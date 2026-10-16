import type { JobParamsDto } from "./types";
import type { VideoOp } from "./videoOps";

/** 压缩的默认参数。抽出来是因为外框与表单都要用它兜底。 */
export const DEFAULT_COMPRESS: Extract<JobParamsDto, { op: "compress" }> = {
  op: "compress",
  preset: "balanced",
  encoder: "auto",
  maxHeight: null,
  crf: null,
  qv: null,
  audioKbps: null,
};

/**
 * 每个操作的初始参数。
 *
 * 放在一处，是因为切操作时参数必须**整个换掉**而不是合并：从转码切到裁剪还带着
 * 一个 `target` 字段提交，会被 core 的反序列化直接拒掉。
 */
export function defaultParams(op: VideoOp): JobParamsDto {
  switch (op) {
    case "compress":
      return { ...DEFAULT_COMPRESS };
    case "transcode":
      return { op: "transcode", target: "mp4h264", preset: "balanced" };
    case "trim":
      return { op: "trim", startSecs: 0, endSecs: null, mode: "fast" };
    case "merge":
      // 合并**没有参数** —— 顺序就是已选文件的顺序
      return { op: "merge" };
    case "crop":
      // 宽高默认 0：表单拿到源尺寸后会把它填满整个画面（`CropForm` 那个 effect）。
      // 这里填不了真实的源尺寸 —— 选文件之前根本不知道源有多大。
      return { op: "crop", width: 0, height: 0, x: 0, y: 0, preset: "balanced" };
  }
}

/**
 * 质量三档。**压缩与转码共用同一组档位名** —— 它们说的是同一件事（更小 ↔ 更好），
 * 而两处各写一份的话，将来加一档必然漏掉一边。
 */
export const PRESETS = [
  { value: "small", label: "小体积" },
  { value: "balanced", label: "均衡" },
  { value: "high", label: "高画质" },
] as const;
