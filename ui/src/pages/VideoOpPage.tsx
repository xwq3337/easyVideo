import { useEffect, useState } from "react";
import { defaultParams } from "../lib/opDefaults";
import type { JobParamsDto } from "../lib/types";
import { opLabel, type VideoOp } from "../lib/videoOps";
import VideoOpShell from "./ops/VideoOpShell";
import CompressForm from "./ops/CompressForm";
import TranscodeForm from "./ops/TranscodeForm";
import TrimForm from "./ops/TrimForm";

/** 还没实现的两个操作。它们各自带一个独立的设计问题，见 spec 的非目标一节。 */
const NOT_READY: readonly VideoOp[] = ["merge", "crop"];

/**
 * 视频操作页：**只做分派**。
 *
 * 三套参数界面各自成组件（`ops/*Form.tsx`），选文件 / 源信息 / 输出目录 / 产物名
 * 预览 / 开始按钮 / 任务列表那一整套归外框。这个文件以前全是压缩的 UI，
 * 再塞两个操作就是一个巨型文件。
 */
export default function VideoOpPage({ op }: { op: VideoOp }) {
  const [params, setParams] = useState<JobParamsDto | null>(() => defaultParams(op));

  // 切操作时参数必须**整个换掉**，不能合并 —— 从转码切到裁剪还带着 target 字段
  // 提交，会被 core 的反序列化直接拒掉。
  useEffect(() => {
    setParams(defaultParams(op));
  }, [op]);

  if (NOT_READY.includes(op)) {
    // 侧边栏不给未实现的操作导航，所以走不到这里；但类型上 op 是五个的联合。
    // 不假装能用 —— 照实说。
    return (
      <div className="h-full flex items-center justify-center" style={{ color: "var(--fg-muted)" }}>
        {opLabel(op)}还没做
      </div>
    );
  }

  return (
    <VideoOpShell op={op} params={params}>
      {(info) => (
        <>
          {op === "compress" && <CompressForm value={params} onChange={setParams} />}
          {op === "transcode" && <TranscodeForm value={params} onChange={setParams} />}
          {op === "trim" && (
            <TrimForm
              value={params}
              onChange={setParams}
              durationSecs={info?.durationSecs ?? null}
            />
          )}
        </>
      )}
    </VideoOpShell>
  );
}
