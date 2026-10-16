import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import * as ipc from "../../lib/ipc";
import { clampCrop, evenDown, type CropDraft } from "../../lib/opForms";
import { PRESETS } from "../../lib/opDefaults";
import { formatTimecode, parseTimeInput } from "../../lib/timecode";
import { run } from "../../store/store";
import type { JobParamsDto, VideoInfoDto } from "../../lib/types";

/**
 * 画面裁剪：预览图 + 叠加框 + 四个数字 + 时间 + 质量档位。
 *
 * **它与 ffmpeg 的静默钳制有本质区别**：这里钳完的结果会**写回输入框**，用户看得见
 * 那几个数字变了。实测 ffmpeg 是悄悄来的 —— 源 1920 宽、请求 x=1200 再裁 1280
 * （2480 已超出），它把 x 钳到 640 照常输出，stderr 一条提示都没有；奇数宽度同理
 * （请求 641 得到 640）。所以越界与奇数必须我们自己判、自己说。
 *
 * 叠加框用**百分比**定位，所以不必管预览图实际缩到了多少 —— 比例天然对。
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
    // 依赖用算好的秒数而不是 src 对象 —— 后者每帧都是新引用
  }, [inputs[0], timeKey]);

  // 合法时把参数交上去；非法时 null（外框禁用开始按钮）。
  // 依赖逐项列出来：写 [box] 的话每帧都是新对象，会把 effect 变成死循环。
  useEffect(() => {
    onChange(box ? { op: "crop", ...box, preset } : null);
  }, [box?.width, box?.height, box?.x, box?.y, preset]);

  /**
   * 改一个数字。**位置当场钳、尺寸留到失焦再钳。**
   *
   * 尺寸若也当场钳，打 "1920" 的途中会先被算成 0 再写回输入框（源宽 1920 时
   * `min(1, 1920)` 取偶就是 0），用户根本没法把数字打完。而左/上**必须**当场钳 ——
   * "宽改小之后左越界"正是最需要看见的那一下。
   */
  function setField(k: keyof CropDraft, raw: number) {
    const next: CropDraft = { ...draft, [k]: Number.isFinite(raw) ? raw : 0 };
    const c = clampCrop(next, sw, sh);
    setDraft(c ? { ...next, x: c.x, y: c.y } : next);
  }

  /** 失焦时把尺寸也钳回来 —— 越界/奇数的数字当场变成实际会用的那个。 */
  function settle() {
    const c = clampCrop(draft, sw, sh);
    if (c) setDraft(c);
  }

  const num = (k: keyof CropDraft, label: string) => (
    <label className="block">
      <span className="text-[12px]" style={{ color: "var(--fg-muted)" }}>
        {label}
      </span>
      <Input
        className="w-24 mt-1"
        inputMode="numeric"
        value={String(draft[k])}
        onChange={(e) => setField(k, Number(e.target.value))}
        onBlur={settle}
      />
    </label>
  );

  const pct = (v: number, whole: number) => `${(v / whole) * 100}%`;

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

      {inputs[0] && preview && (
        <div
          className="relative inline-block mb-3 rounded-xl overflow-hidden"
          style={{ border: "1px solid var(--line)" }}
        >
          <img src={preview} alt="预览帧" className="block w-full max-w-160" />
          {/* 框画的是**校验通过之后**的矩形：数字越界时不画一个假的框 */}
          {box && sw && sh && (
            <div
              className="absolute pointer-events-none"
              style={{
                left: pct(box.x, sw),
                top: pct(box.y, sh),
                width: pct(box.width, sw),
                height: pct(box.height, sh),
                border: "2px solid var(--accent-blue)",
                boxShadow: "0 0 0 9999px rgba(0,0,0,0.35)",
              }}
            />
          )}
        </div>
      )}

      {inputs[0] && previewErr && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          取预览失败（不影响裁剪本身，下面照样可以填数字提交）。
        </p>
      )}

      {inputs[0] && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          {sw != null && sh != null ? `画面 ${sw}x${sh}` : "画面尺寸未知"} ·
          预览取自 {formatTimecode(timeKey)} 处
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

      <div className="flex items-end gap-3 mb-1">
        {num("width", "宽")}
        {num("height", "高")}
        {num("x", "左")}
        {num("y", "上")}
      </div>
      <p className="text-[12px] mb-5" style={{ color: "var(--fg-faint)" }}>
        宽高必须是偶数：奇数会被编码器静默改成相邻的偶数（请求 641 得到 640），
        所以这里当场帮你取偶。左/上越界也会当场钳到上限。
      </p>

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
          裁剪要重新编码，所以会比「合并」慢。**时长一点不变** —— 只换画面尺寸。
        </span>
      </div>
    </>
  );
}
