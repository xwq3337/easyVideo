import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import JobList from "../components/JobList";
import * as ipc from "../lib/ipc";
import { formatSize } from "../lib/format";
import { run, useSettings } from "../store/store";
import { VIDEO_OPS, opLabel, type VideoOp } from "../lib/videoOps";

const PRESETS = [
  { value: "small", label: "小体积" },
  { value: "balanced", label: "均衡" },
  { value: "high", label: "高画质" },
] as const;

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

function formatDuration(secs: number | null): string {
  if (secs === null) return "时长未知";
  const total = Math.round(secs);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s} 秒`;
}

export default function VideoOpPage({ op }: { op: VideoOp }) {
  const settings = useSettings();
  const [input, setInput] = useState<string | null>(null);
  const [info, setInfo] = useState<{ durationSecs: number | null; hasVideo: boolean; sizeBytes: number } | null>(null);
  const [preset, setPreset] = useState<string>("balanced");
  const [encoder, setEncoder] = useState<string>("auto");
  const [maxHeight, setMaxHeight] = useState<number>(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  // **空字符串 = 跟随档位。** 不在这里写一份档位数值表：那份表要与 core 的
  // `preset_defaults` 手动保持同步，而它同步不了（两个 crate）。留空让 core 自己取，
  // 界面就永远不需要知道 24 还是 28。
  const [quality, setQuality] = useState<string>("");
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // 输出目录默认跟随设置里的下载目录，但允许这一页临时改（不落盘、不污染设置）。
  useEffect(() => {
    if (settings && outputDir === null) setOutputDir(settings.downloadDir);
  }, [settings, outputDir]);

  async function pick() {
    const p = await run(ipc.pickVideoFile());
    if (!p) return;
    setInput(p);
    setInfo(null);
    const i = await run(ipc.probeVideo(p));
    if (i) setInfo(i);
  }

  async function start() {
    if (!input || !outputDir || busy) return;
    setBusy(true);
    try {
      const q = quality.trim() === "" ? null : Number(quality);
      const id = await run(
        ipc.addCompressJob({
          input,
          outputDir,
          preset,
          encoder,
          maxHeight: maxHeight === 0 ? null : maxHeight,
          // 硬件编码的质量旋钮是 -q:v、软件是 -crf，两者**方向相反**，
          // 所以同一个输入框按编码器送进不同的字段。core 那边是两条明确路径。
          crf: encoder === "hardware" ? null : q,
          qv: encoder === "hardware" ? q : null,
          audioKbps: null,
        })
      );
      // 入队成功才清空选择：失败时把文件留在框里，用户要重试的就是它。
      if (id !== undefined) {
        setInput(null);
        setInfo(null);
      }
    } finally {
      setBusy(false);
    }
  }

  const ready = VIDEO_OPS.find((o) => o.op === op)?.ready ?? false;
  if (!ready) {
    // 侧边栏不给未实现的操作导航，所以走不到这里；但类型上 op 是五个的联合。
    // 不假装能用 —— 照实说。
    return (
      <div className="h-full flex items-center justify-center" style={{ color: "var(--fg-muted)" }}>
        {opLabel(op)}还没做
      </div>
    );
  }

  return (
    // 左右内边距由 App 的滚动容器提供，这里只留纵向的
    <>
      <div className="flex items-center gap-2 py-4">
        <Input readOnly value={input ?? ""} placeholder="选一个视频文件…" />
        <Button onClick={() => void pick()}>选择文件</Button>
      </div>

      {info && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          源文件 {formatSize(info.sizeBytes)} · {formatDuration(info.durationSecs)}
          {!info.hasVideo && " · 这个文件里没有视频流，压不了"}
        </p>
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
      </div>

      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>保存到</span>
        <Input
          className="mt-1.5"
          value={outputDir ?? ""}
          onChange={(e) => setOutputDir(e.target.value)}
        />
        <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
          只影响这次操作，不改设置里的下载目录。重名会自动让路，源文件不会被覆盖。
        </span>
      </label>

      <button
        className="flex items-center gap-1 text-[13px] mb-3"
        onClick={() => setShowAdvanced((v) => !v)}
        aria-expanded={showAdvanced}
      >
        {showAdvanced ? "▾" : "▸"} 高级
      </button>

      {showAdvanced && (
        <div className="mb-5 pl-3 border-l" style={{ borderColor: "var(--line)" }}>
          <label className="block mb-4">
            <span style={{ color: "var(--fg-muted)" }}>编码器</span>
            <Select value={encoder} onValueChange={setEncoder}>
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
            <Select value={String(maxHeight)} onValueChange={(v) => setMaxHeight(Number(v))}>
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
              画质{encoder === "hardware" ? "（数值越大体积越大）" : "（数值越小体积越小）"}
            </span>
            <Input
              type="number"
              className="w-28 mt-1.5"
              placeholder="跟随档位"
              value={quality}
              onChange={(e) => setQuality(e.target.value)}
            />
            <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
              留空就用所选档位自带的值。
            </span>
          </label>
        </div>
      )}

      <Button
        variant="primary"
        onClick={() => void start()}
        disabled={busy || !input || !outputDir || info?.hasVideo === false}
      >
        {busy ? "提交中…" : "开始压缩"}
      </Button>

      <div className="mt-6 pb-8">
        <JobList />
      </div>
    </>
  );
}
