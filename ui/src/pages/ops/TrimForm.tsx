import { useState } from "react";
import { Input } from "@/components/ui/input";
import { trimFormReduce, trimParamsOf, type TrimAction, type TrimFormState } from "../../lib/opForms";
import { formatTimecode, parseTimeInput } from "../../lib/timecode";
import type { JobParamsDto } from "../../lib/types";

/**
 * 裁剪视频。两个档位的差别按**实测到的事实**写，不按流传的说法写：
 *
 * - 流传的说法是 `-c copy` 会把切口吸附到关键帧（起点提前、片段变长）。我们用强制
 *   GOP=10 秒的素材实测**复现不出来**（`tests/videoops_trim_test.rs` 里那条
 *   `fast_trim_cut_is_frame_accurate_on_a_sparse_gop_source` 守着它），所以偏移只能
 *   写成"可能"而不是"会"。
 * - 精确档"多一次质量损失"是重编码的必然，写成确定。
 *
 * 这个不对称是刻意的。
 *
 * 表单状态（起始文本 / 结束文本 / **档位**）由一个纯 reducer 管，逻辑在 `opForms.ts`
 * 里并配有测试。档位住在这里、不从父级的 `params` 推，是因为 `params` 在输入中途
 * 会是 `null`（非法中间态）—— 从它推的话用户点「精确」再输时间，最后提交的会是
 * `-c copy`，而界面还会跟着翻回"快速"。
 */
export default function TrimForm({
  value,
  onChange,
  durationSecs,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
  /** 源时长，用来显示"共 X 秒"与做基本校验。读不出来时 null。 */
  durationSecs: number | null;
}) {
  const [state, setState] = useState<TrimFormState>(() => ({
    startText: "0",
    endText: "",
    mode: value?.op === "trim" ? value.mode : "fast",
  }));

  function dispatch(a: TrimAction) {
    const next = trimFormReduce(state, a);
    setState(next);
    onChange(trimParamsOf(next));
  }

  const params = trimParamsOf(state);
  const start = parseTimeInput(state.startText);
  const end = state.endText.trim() === "" ? null : parseTimeInput(state.endText);
  const span =
    params !== null && start !== null
      ? end !== null
        ? `共 ${formatTimecode(end - start)}`
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
            value={state.startText}
            onChange={(e) => dispatch({ kind: "start", text: e.target.value })}
          />
        </label>
        <label className="block">
          <span style={{ color: "var(--fg-muted)" }}>结束</span>
          <Input
            className="w-28 mt-1.5"
            placeholder="留空=到结尾"
            value={state.endText}
            onChange={(e) => dispatch({ kind: "end", text: e.target.value })}
          />
        </label>
        {span && (
          <span className="pb-2 text-[12px]" style={{ color: "var(--fg-muted)" }}>
            {span}
          </span>
        )}
      </div>
      <p className="text-[12px] mb-5" style={{ color: "var(--fg-faint)" }}>
        可以写 <code>1:30</code>、<code>0:01:30</code> 或纯秒数 <code>90</code>。源时长
        {durationSecs != null ? ` ${formatTimecode(durationSecs)}` : "读不出来"}。
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
              onClick={() => dispatch({ kind: "mode", mode: m.v })}
              aria-pressed={state.mode === m.v}
              className="h-8 px-3 rounded-md text-[13px] border transition-colors"
              style={{
                borderColor: "var(--line-strong)",
                background: state.mode === m.v ? "var(--nav-sel)" : "transparent",
                color: "var(--fg)",
                fontWeight: state.mode === m.v ? 500 : 400,
              }}
            >
              {m.label}
            </button>
          ))}
        </div>
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          {state.mode === "fast"
            ? "不重编码：秒完，画质一点不丢。个别源上切口可能略有偏移。"
            : "重新编码：慢一些，会多一次质量损失。快速切的结果不对时用它。"}
        </span>
      </div>
    </>
  );
}
