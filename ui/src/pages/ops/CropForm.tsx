import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Input } from "@/components/ui/input";
import * as ipc from "../../lib/ipc";
import {
  applyDrag,
  clampCrop,
  evenDown,
  sourceDelta,
  type CropDraft,
  type CropGesture,
} from "../../lib/opForms";
import { PRESETS } from "../../lib/opDefaults";
import { formatTimecode, parseTimeInput } from "../../lib/timecode";
import { run } from "../../store/store";
import type { JobParamsDto, VideoInfoDto } from "../../lib/types";

/** 四个角的手柄。`nw`/`se` 是西北—东南对角线，另两个是东北—西南。 */
// 手柄只往框外露 1px：容器有 `overflow-hidden`（挡那个 9999px 的遮罩），
// 露太多的话框贴到画面边时手柄会被裁掉一半。
const CORNERS: ReadonlyArray<{ c: "nw" | "ne" | "sw" | "se"; style: CSSProperties }> = [
  { c: "nw", style: { left: -1, top: -1, cursor: "nwse-resize" } },
  { c: "ne", style: { right: -1, top: -1, cursor: "nesw-resize" } },
  { c: "sw", style: { left: -1, bottom: -1, cursor: "nesw-resize" } },
  { c: "se", style: { right: -1, bottom: -1, cursor: "nwse-resize" } },
];

type Drag = {
  gesture: CropGesture;
  /** 按下那一刻的框 —— 每次都从它算起，不在实时值上累加（累加会漂）。 */
  from: CropDraft;
  clientX: number;
  clientY: number;
  /** 画面那一块按下时的渲染尺寸。换算分母就是它（不是窗口宽）。 */
  frame: { width: number; height: number };
};

/**
 * 画面裁剪：**拖预览图上的框**来定裁剪区域。
 *
 * 框内拖动 = 移动，四个角 = 缩放（对角固定）。没有数字输入框 —— 当前值以只读的
 * 一行数字显示在下面。
 *
 * **它与 ffmpeg 的静默钳制有本质区别**：越界会**顶住**而不是被悄悄挪走，宽高当场
 * 取偶。实测 ffmpeg 是反过来的 —— 源 1920 宽、请求 x=1200 再裁 1280（2480 已超出），
 * 它把 x 钳到 640 照常输出、stderr 一条提示都没有；奇数宽度同理（请求 641 得到 640）。
 *
 * 叠加框用**百分比**定位，所以不必管预览图实际缩到了多少 —— 比例天然对。
 * 拖动的手势换算与钳制全在 `applyDrag` 里（纯函数，配有测试）。
 */
export default function CropForm({
  value,
  onChange,
  inputs,
  infos,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
  inputs: string[];
  infos: (VideoInfoDto | null)[];
}) {
  const src = infos[0] ?? null;
  const sw = src?.width ?? null;
  const sh = src?.height ?? null;

  const [draft, setDraft] = useState<CropDraft>({ width: 0, height: 0, x: 0, y: 0 });
  const [preset, setPreset] = useState<string>(
    value?.op === "crop" ? value.preset : "balanced"
  );
  const [preview, setPreview] = useState<string | null>(null);
  const [previewErr, setPreviewErr] = useState(false);
  const [atSecs, setAtSecs] = useState<number | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);

  /** 量的是**画面那一块**的渲染尺寸 —— client 像素换算成源像素的分母。 */
  const frameRef = useRef<HTMLDivElement | null>(null);

  const box = clampCrop(draft, sw, sh);

  // 源信息一到手就把整个画面当默认裁剪框（取偶：源可能是奇数尺寸）
  useEffect(() => {
    if (sw && sh && draft.width === 0 && draft.height === 0) {
      setDraft({ width: evenDown(sw), height: evenDown(sh), x: 0, y: 0 });
    }
  }, [sw, sh]);

  // 预览帧：**只在文件或时间变化时重抽** —— 它要跑一次 ffmpeg。
  // 默认取 10% 处，避开开头的黑场/台标；时长读不出来时退回首帧。
  const timeKey = atSecs ?? (src?.durationSecs != null ? src.durationSecs * 0.1 : 0);
  useEffect(() => {
    if (!inputs[0]) {
      setPreview(null);
      return;
    }
    let alive = true;
    setPreviewErr(false);
    void run(ipc.previewFrame(inputs[0], timeKey)).then((u) => {
      if (!alive) return;
      if (u) setPreview(u);
      else setPreviewErr(true);
    });
    return () => {
      alive = false;
    };
  }, [inputs[0], timeKey]);

  // 合法时把参数交上去；非法时 null（外框禁用开始按钮）。
  // 依赖逐项列出来：写 [box] 的话每帧都是新对象，会把 effect 变成死循环。
  useEffect(() => {
    onChange(box ? { op: "crop", ...box, preset } : null);
  }, [box?.width, box?.height, box?.x, box?.y, preset]);

  function onPointerDown(e: ReactPointerEvent<HTMLElement>, gesture: CropGesture) {
    if (!box || sw == null || sh == null) return;
    const rect = frameRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return;
    // 指针可能被拖到框外甚至图外，捕获之后事件仍然回到这里
    e.currentTarget.setPointerCapture(e.pointerId);
    e.preventDefault();
    setDrag({
      gesture,
      from: box,
      clientX: e.clientX,
      clientY: e.clientY,
      frame: { width: rect.width, height: rect.height },
    });
  }

  function onPointerMove(e: ReactPointerEvent<HTMLDivElement>) {
    if (!drag || sw == null || sh == null) return;
    const d = sourceDelta(
      e.clientX - drag.clientX,
      e.clientY - drag.clientY,
      drag.frame,
      sw,
      sh
    );
    setDraft(applyDrag(drag.from, drag.gesture, d.dx, d.dy, sw, sh));
  }

  function endDrag() {
    setDrag(null);
  }

  const pct = (v: number, whole: number) => `${(v / whole) * 100}%`;
  const ready = inputs[0] && sw != null && sh != null;

  return (
    <>
      {!inputs[0] && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          先选一个视频文件。
        </p>
      )}

      {inputs[0] && sw == null && (
        <p className="text-[12px] mb-4" style={{ color: "var(--accent-err)" }}>
          读不出这个视频的画面尺寸，无法裁剪。
        </p>
      )}

      {ready && (
        <div
          ref={frameRef}
          className="relative inline-block mb-3 rounded-xl overflow-hidden select-none"
          style={{ border: "1px solid var(--line)", touchAction: "none" }}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          {preview ? (
            <img src={preview} alt="预览帧" className="block w-full max-w-160" draggable={false} />
          ) : (
            // 抽帧失败时**仍然能拖**：按源宽高比撑出一块空画布。
            // 否则「只留拖动」就等于完全没有输入手段了。
            <div
              className="block"
              style={{
                width: 640,
                maxWidth: "100%",
                aspectRatio: `${sw} / ${sh}`,
                background: "var(--bg-subtle)",
              }}
            />
          )}

          {/* 框画的是**校验通过之后**的矩形：状态非法时不画一个假的框 */}
          {box && (
            <div
              className="absolute"
              style={{
                left: pct(box.x, sw!),
                top: pct(box.y, sh!),
                width: pct(box.width, sw!),
                height: pct(box.height, sh!),
                border: "2px solid var(--accent-blue)",
                boxShadow: "0 0 0 9999px rgba(0,0,0,0.35)",
                cursor: drag ? "grabbing" : "move",
              }}
              onPointerDown={(e) => onPointerDown(e, { kind: "move" })}
              title="拖动可移动裁剪区域"
            >
              {CORNERS.map(({ c, style }) => (
                <span
                  key={c}
                  className="absolute block size-3 rounded-sm"
                  style={{
                    ...style,
                    background: "var(--accent-blue)",
                    border: "2px solid var(--bg)",
                  }}
                  // 手柄要自己吃掉按下事件，否则会同时触发整框移动
                  onPointerDown={(e) => {
                    e.stopPropagation();
                    onPointerDown(e, { kind: "resize", corner: c });
                  }}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {ready && box && (
        <p className="text-[12px] mb-1 tnum" style={{ color: "var(--fg-muted)" }}>
          宽 {box.width} · 高 {box.height} · 左 {box.x} · 上 {box.y}
        </p>
      )}

      {ready && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          框内拖动移动，四个角拖动缩放。越界会顶住、宽高当场取偶 —— 这两件事 ffmpeg
          是悄悄做掉的，所以在这里先做给你看。
          {sw != null && sh != null && ` 画面 ${sw}x${sh}。`}
        </p>
      )}

      {inputs[0] && previewErr && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          取预览失败（看不见画面，但框照样能拖 —— 时间拿不准的话可以先换一个预览时间）。
        </p>
      )}

      {inputs[0] && (
        <label className="block mb-5">
          <span className="text-[12px]" style={{ color: "var(--fg-muted)" }}>
            预览时间（可以写 <code>1:30</code> 或纯秒数）
          </span>
          <Input
            className="w-28 mt-1"
            placeholder={formatTimecode(timeKey)}
            onChange={(e) => {
              const t = e.target.value.trim() === "" ? null : parseTimeInput(e.target.value);
              setAtSecs(t);
            }}
          />
        </label>
      )}

      <div className="mb-5">
        <div style={{ color: "var(--fg-muted)" }}>质量</div>
        <div className="flex gap-1 mt-1.5">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              onClick={() => setPreset(p.value)}
              aria-pressed={preset === p.value}
              className="h-8 px-3 rounded-md text-[13px] border transition-colors"
              style={{
                borderColor: "var(--line-strong)",
                background: preset === p.value ? "var(--nav-sel)" : "transparent",
                color: "var(--fg)",
                fontWeight: preset === p.value ? 500 : 400,
              }}
            >
              {p.label}
            </button>
          ))}
        </div>
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          裁剪要重新编码，所以会比「合并」慢。时长一点不变 —— 只换画面尺寸。
        </span>
      </div>
    </>
  );
}
