import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PRESETS } from "../../lib/opDefaults";
import type { JobParamsDto } from "../../lib/types";

/**
 * 目标格式是**整组打包**的：容器 + 视频编码 + 音频编码三样绑死。
 *
 * 不做成三根自由的轴 —— WebM 只收 VP8/VP9/AV1 + Vorbis/Opus，硬塞 aac 进去是
 * `Could not write header` 的硬失败（实测）。自由的轴等于允许用户拼出这种组合，
 * 而且要到任务失败才告诉他。这个表与 core 的 `TargetFormat` 一一对应。
 */
const TARGETS = [
  { value: "mp4h264", label: "MP4 · H.264 + AAC（兼容性最好）" },
  { value: "mp4h265", label: "MP4 · H.265 + AAC（同画质体积更小）" },
  { value: "webmvp9", label: "WebM · VP9 + Opus（网页友好）" },
  { value: "mkvh264", label: "MKV · H.264 + AAC（容器最宽容）" },
] as const;

type TranscodeParams = Extract<JobParamsDto, { op: "transcode" }>;

export default function TranscodeForm({
  value,
  onChange,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
}) {
  const v: TranscodeParams =
    value?.op === "transcode" ? value : { op: "transcode", target: "mp4h264", preset: "balanced" };
  function update(patch: Partial<TranscodeParams>) {
    onChange({ ...v, ...patch });
  }

  return (
    <>
      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>目标格式</span>
        <Select value={v.target} onValueChange={(target) => update({ target })}>
          <SelectTrigger className="w-72 mt-1.5">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {TARGETS.map((t) => (
              <SelectItem key={t.value} value={t.value}>
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          转码是"换格式"。只想把文件变小、格式不变，用「压缩」。
          {v.target === "webmvp9" && " WebM 用 VP9 软件编码，明显更慢。"}
        </span>
      </label>

      <div className="mb-5">
        <div style={{ color: "var(--fg-muted)" }}>质量</div>
        <div className="flex gap-1 mt-1.5">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              onClick={() => update({ preset: p.value })}
              aria-pressed={v.preset === p.value}
              className="h-8 px-3 rounded-md text-[13px] border transition-colors"
              style={{
                borderColor: "var(--line-strong)",
                background: v.preset === p.value ? "var(--nav-sel)" : "transparent",
                color: "var(--fg)",
                fontWeight: v.preset === p.value ? 500 : 400,
              }}
            >
              {p.label}
            </button>
          ))}
        </div>
      </div>
    </>
  );
}
