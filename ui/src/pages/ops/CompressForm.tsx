import { useState } from "react";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { DEFAULT_COMPRESS, PRESETS } from "../../lib/opDefaults";
import type { JobParamsDto } from "../../lib/types";

const ENCODERS = [
  { value: "auto", label: "自动" },
  { value: "hardware", label: "硬件加速" },
  { value: "software", label: "软件" },
] as const;

const HEIGHTS = [
  { value: 0, label: "保持原样" },
  { value: 1080, label: "1080p" },
  { value: 720, label: "720p" },
] as const;

type CompressParams = Extract<JobParamsDto, { op: "compress" }>;

export default function CompressForm({
  value,
  onChange,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
}) {
  // 本组件只在 op === "compress" 时被渲染（见 VideoOpPage 的分派），这里收窄一次
  const v: CompressParams = value?.op === "compress" ? value : DEFAULT_COMPRESS;

  // 只属于界面的状态：展开与否、以及"画质"那个可以留空的输入框的原始文本。
  // 它进不了 params —— 留空表示"跟随档位"，那时 crf/qv 都是 null。
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [quality, setQuality] = useState("");

  /** 补一个字段并交上去。 */
  function update(patch: Partial<CompressParams>) {
    onChange({ ...v, ...patch });
  }

  /** 画质输入框变了：文本留在本地，派生值（crf 或 qv）进 params。 */
  function onQuality(text: string) {
    setQuality(text);
    const q = text.trim() === "" ? null : Number(text);
    update({ crf: v.encoder === "hardware" ? null : q, qv: v.encoder === "hardware" ? q : null });
  }

  return (
    <>
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

      <button
        className="flex items-center gap-1 text-[13px] mb-3"
        onClick={() => setShowAdvanced((x) => !x)}
        aria-expanded={showAdvanced}
      >
        {showAdvanced ? "▾" : "▸"} 高级
      </button>

      {showAdvanced && (
        <div className="mb-5 pl-3 border-l" style={{ borderColor: "var(--line)" }}>
          <label className="block mb-4">
            <span style={{ color: "var(--fg-muted)" }}>编码器</span>
            <Select value={v.encoder} onValueChange={(encoder) => update({ encoder })}>
              <SelectTrigger className="w-48 mt-1.5">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ENCODERS.map((x) => (
                  <SelectItem key={x.value} value={x.value}>
                    {x.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
              「自动」用软件编码：同档位体积最小。硬件快得多，但同档位体积更大。
            </span>
          </label>

          <label className="block mb-4">
            <span style={{ color: "var(--fg-muted)" }}>分辨率</span>
            {/* radix 的 Select 只认字符串值，所以这里转一道 */}
            <Select
              value={String(v.maxHeight ?? 0)}
              onValueChange={(x) => update({ maxHeight: Number(x) === 0 ? null : Number(x) })}
            >
              <SelectTrigger className="w-48 mt-1.5">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {HEIGHTS.map((x) => (
                  <SelectItem key={x.value} value={String(x.value)}>
                    {x.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>

          {/* 质量微调只在软件编码下出现：硬件根本不看 CRF，显示出来就是误导。
              两种刻度的方向相反，所以标签必须写清哪头是"更小体积"。 */}
          <label className="block">
            <span style={{ color: "var(--fg-muted)" }}>
              画质{v.encoder === "hardware" ? "（数值越大体积越大）" : "（数值越小体积越小）"}
            </span>
            <Input
              type="number"
              className="w-28 mt-1.5"
              placeholder="跟随档位"
              value={quality}
              onChange={(e) => onQuality(e.target.value)}
            />
            <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
              留空就用所选档位自带的值。
            </span>
          </label>
        </div>
      )}
    </>
  );
}
