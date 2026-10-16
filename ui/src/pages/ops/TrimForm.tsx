import { useState } from "react";
import { Input } from "@/components/ui/input";
import { formatTimecode, parseTimeInput } from "../../lib/timecode";
import type { JobParamsDto } from "../../lib/types";

/**
 * 裁剪视频。两个档位的差别按**实测到的事实**写，不按流传的说法写：
 *
 * - 流传的说法是 `-c copy` 会把切口吸附到关键帧（起点提前、片段变长）。
 *   我们用强制 GOP=10 秒的素材实测**复现不出来**（`tests/videoops_trim_test.rs`
 *   里那条 `fast_trim_cut_is_frame_accurate_on_a_sparse_gop_source` 守着它），
 *   所以偏移只能写成"可能"而不是"会"。
 * - 精确档"多一次质量损失"是重编码的必然，写成确定。
 *
 * 这个不对称是刻意的。
 */
export default function TrimForm({
  value,
  onChange,
  durationSecs,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
  /** 源时长，用来显示与做基本校验。读不出来时 null。 */
  durationSecs: number | null;
}) {
  const v = value?.op === "trim" ? value : { op: "trim" as const, startSecs: 0, endSecs: null, mode: "fast" };
  // 输入框的原始文本留在本地：用户打到一半的 "1:" 是非法值，但不该被抹掉
  const [startText, setStartText] = useState("0");
  const [endText, setEndText] = useState("");

  const start = parseTimeInput(startText);
  const endRaw = endText.trim() === "" ? null : parseTimeInput(endText);
  // 非法：起始解析不出来 / 结束填了但解析不出来 / 结束不晚于起始
  const bad =
    start === null ||
    (endText.trim() !== "" && endRaw === null) ||
    (start !== null && endRaw !== null && endRaw <= start);

  /** 把当前文本重新解析后交上去；不合法就交 null（外框据此禁掉开始按钮）。 */
  function push(nextStartText: string, nextEndText: string, mode: string) {
    const s = parseTimeInput(nextStartText);
    const e = nextEndText.trim() === "" ? null : parseTimeInput(nextEndText);
    const invalid =
      s === null ||
      (nextEndText.trim() !== "" && e === null) ||
      (s !== null && e !== null && e <= s);
    onChange(invalid ? null : { op: "trim", startSecs: s!, endSecs: e, mode });
  }

  const span =
    !bad && start !== null
      ? endRaw !== null
        ? `共 ${formatTimecode(endRaw - start)}`
        : durationSecs != null
          ? `共 ${formatTimecode(durationSecs - start)}`
          : "到结尾"
      : null;

  return (
    <>
      <div className="flex items-end gap-3 mb-1.5">
        <label className="block">
          <span style={{ color: "var(--fg-muted)" }}>起始</span>
          <Input
            className="w-28 mt-1.5"
            placeholder="0:00"
            value={startText}
            onChange={(e) => {
              setStartText(e.target.value);
              push(e.target.value, endText, v.mode);
            }}
          />
        </label>
        <label className="block">
          <span style={{ color: "var(--fg-muted)" }}>结束</span>
          <Input
            className="w-28 mt-1.5"
            placeholder="留空=到结尾"
            value={endText}
            onChange={(e) => {
              setEndText(e.target.value);
              push(startText, e.target.value, v.mode);
            }}
          />
        </label>
        {span && (
          <span className="pb-2 text-[12px]" style={{ color: "var(--fg-muted)" }}>
            {span}
          </span>
        )}
      </div>
      <p className="text-[12px] mb-5" style={{ color: "var(--fg-faint)" }}>
        可以写 <code>1:30</code>、<code>0:01:30</code> 或纯秒数 <code>90</code>。
        源时长{durationSecs != null ? ` ${formatTimecode(durationSecs)}` : "读不出来"}。
      </p>

      <div className="mb-5">
        <div style={{ color: "var(--fg-muted)" }}>切法</div>
        <div className="flex gap-1 mt-1.5">
          {[
            { v: "fast", label: "快速" },
            { v: "accurate", label: "精确" },
          ].map((m) => (
            <button
              key={m.v}
              onClick={() => {
                push(startText, endText, m.v);
              }}
              aria-pressed={v.mode === m.v}
              className="h-8 px-3 rounded-md text-[13px] border transition-colors"
              style={{
                borderColor: "var(--line-strong)",
                background: v.mode === m.v ? "var(--nav-sel)" : "transparent",
                color: "var(--fg)",
                fontWeight: v.mode === m.v ? 500 : 400,
              }}
            >
              {m.label}
            </button>
          ))}
        </div>
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          {v.mode === "fast"
            ? "不重编码：秒完，画质一点不丢。个别源上切口可能略有偏移。"
            : "重新编码：慢一些，会多一次质量损失。快速切的结果不对时用它。"}
        </span>
      </div>
    </>
  );
}
