import { useEffect, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import JobList from "../../components/JobList";
import * as ipc from "../../lib/ipc";
import { formatSize } from "../../lib/format";
import { formatTimecode } from "../../lib/timecode";
import { run, useSettings } from "../../store/store";
import type { JobParamsDto, VideoInfoDto } from "../../lib/types";
import { opLabel, type VideoOp } from "../../lib/videoOps";

/**
 * 三个操作页共用的外框：选文件 / 源信息 / 输出目录 / 产物名预览 / 开始按钮 / 任务列表。
 *
 * 每个操作只写自己的参数区（`children`）—— 否则"选文件"那段要抄三遍，
 * 而其中任何一处将来都会各自漂移。
 */
export default function VideoOpShell({
  op,
  params,
  children,
}: {
  op: VideoOp;
  /** 为 `null` 表示参数还不合法（比如裁剪的起止填错了），开始按钮要禁掉。 */
  params: JobParamsDto | null;
  /**
   * **render prop 而不是普通 children**：裁剪表单要用源时长（显示"共 8 秒"、
   * 做基本校验），而 probe 结果归外框所有。传一个节点进来的话，时长就得在
   * VideoOpPage 里再探一次 —— 两个来源迟早不一致。
   */
  children: (info: VideoInfoDto | null) => ReactNode;
}) {
  const settings = useSettings();
  const [input, setInput] = useState<string | null>(null);
  const [info, setInfo] = useState<VideoInfoDto | null>(null);
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [planned, setPlanned] = useState<string | null>(null);
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
    setPlanned(null);
    const i = await run(ipc.probeVideo(p));
    if (i) setInfo(i);
  }

  // **只在影响产物名的东西变化时重算**：源文件、操作、以及参数里决定扩展名的那两项
  // （转码的 target、裁剪的 mode）。时间与画质不影响名字，改它们不触发 ——
  // 否则用户每敲一个数字就要跑一次 `ffmpeg -i`。
  const namer =
    params === null
      ? null
      : JSON.stringify([
          input,
          op,
          params.op === "transcode" ? params.target : null,
          params.op === "trim" ? params.mode : null,
        ]);

  useEffect(() => {
    if (!input || !outputDir || params === null) {
      setPlanned(null);
      return;
    }
    let alive = true;
    void run(ipc.planOutput(input, outputDir, params)).then((v) => {
      if (alive && v !== undefined) setPlanned(v);
    });
    return () => {
      alive = false;
    };
    // 依赖刻意是上面那个拼出来的字符串，不是 [input, outputDir, params] ——
    // 后者的每个成员都可能在每帧重建，会把 effect 变成死循环。
    // eslint 之外的规矩：真实的依赖语义由 `namer` 这个字符串代表。
  }, [namer]);

  async function start() {
    if (!input || !outputDir || params === null || busy) return;
    setBusy(true);
    try {
      const id = await run(ipc.addVideoJob(input, outputDir, params));
      // 入队成功才清空选择：失败时把文件留在框里，用户要重试的就是它
      if (id !== undefined) {
        setInput(null);
        setInfo(null);
        setPlanned(null);
      }
    } finally {
      setBusy(false);
    }
  }

  const srcLine = info
    ? `源文件 ${formatSize(info.sizeBytes)} · ${
        info.durationSecs != null ? formatTimecode(info.durationSecs) : "时长未知"
      } · ${info.videoCodec ?? "编码未知"} · ${info.container ?? "容器未知"}`
    : null;

  return (
    <>
      <div className="flex items-center gap-2 py-4">
        <Input readOnly value={input ?? ""} placeholder="选一个视频文件…" />
        <Button onClick={() => void pick()}>选择文件</Button>
      </div>

      {srcLine && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          {srcLine}
          {info && !info.hasVideo && " · 这个文件里没有视频流"}
        </p>
      )}

      {children(info)}

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

      {/* 产物名预览：让路命名（" (1)"）与扩展名变化（同一个裁剪选快速可能出 .mkv、
          选精确出 .mp4）都在提交前就看得见。名字由 core 算，前端不自己拼。 */}
      {planned && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          产物：{planned}
        </p>
      )}

      <Button
        variant="primary"
        onClick={() => void start()}
        disabled={busy || !input || !outputDir || params === null || info?.hasVideo === false}
      >
        {busy ? "提交中…" : `开始${opLabel(op)}`}
      </Button>

      <div className="mt-6 pb-8">
        <JobList />
      </div>
    </>
  );
}
