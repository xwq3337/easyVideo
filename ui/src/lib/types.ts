// 与 src-tauri/src/dto.rs 一一对应。改这里之前先看 dto.rs 末尾那几条测试——
// 它们就是这份契约的说明书。真实序列化出来的 JSON 见 Task 4 报告第 3 节。

export type StateDto =
  | { kind: "queued" }
  | { kind: "probing" }
  | { kind: "awaitingChoice" }
  | { kind: "downloading" }
  | { kind: "merging" }
  | { kind: "paused" }
  | { kind: "done" }
  | { kind: "failed"; message: string }
  | { kind: "canceled" };

export type UnitDto = "bytes" | "segments";

export interface TaskDto {
  id: number;
  url: string;
  filename: string;
  state: StateDto;
  downloaded: number;
  total: number;
  unit: UnitDto;
  /** 本轮已下字节数。与 `unit` 无关，恒为字节 —— 速度只能从它算。 */
  bytesDownloaded: number;
}

export type EventDto =
  | { type: "stateChanged"; id: number; state: StateDto }
  | {
      type: "progress";
      id: number;
      downloaded: number;
      total: number;
      unit: UnitDto;
      bytesDownloaded: number;
    }
  | { type: "choicesOffered"; id: number; kind: ChoiceKindDto }
  | { type: "taskRemoved"; id: number };

export interface VariantDto {
  uri: string;
  bandwidth: number;
  resolution: [number, number] | null;
}

export interface FormatDto {
  id: string;
  ext: string;
  note: string;
  filesize: number | null;
  url: string;
}

// `ChoiceKindDto` 是**结构体变体**，不是新类型变体。dto.rs 里若用新类型变体
// (`HlsVariants(Vec<..>)`)，序列化会在运行时炸；真实形状是内部标签 + camelCase 数组字段：
//   { "kind": "hlsVariants", "hlsVariants": [...] }
export type ChoiceKindDto =
  | { kind: "hlsVariants"; hlsVariants: VariantDto[] }
  | { kind: "ytdlpFormats"; ytdlpFormats: FormatDto[] };

export type ChoiceDto = { kind: "variant"; index: number } | { kind: "ytdlpFormat"; index: number };

export interface SettingsDto {
  downloadDir: string;
  segmentConcurrency: number;
  maxActiveTasks: number;
  ytdlpEnabled: boolean;
  ffmpegPathOverride: string | null;
  /** 浏览器名（chrome/firefox/safari…）。null = 不读浏览器 cookie。 */
  cookiesFromBrowser: string | null;
  /** 伪装浏览器指纹：绕过 Cloudflare 的主要手段，需要 yt-dlp 装了 curl_cffi。 */
  impersonate: boolean;
}

/** 界面用的一行。比 TaskDto 多几样本地算出来的东西。 */
export interface TaskView extends TaskDto {
  /** 候选列表的本地副本。重同步时可能是 null —— 那就要去问 core。 */
  choices: ChoiceKindDto | null;
  /** 用户正在编辑文件名时为 true，此时不被进度事件刷新覆盖。 */
  editing: boolean;
  /** 由前后两次快照的**字节数**做差得出，对分片任务同样有效。算不出时为 null。 */
  speedBps: number | null;
  /** 形如 "剩 1 分 32 秒"；算不出来时为 null。 */
  etaText: string | null;
  /** 上一次进度事件的时间戳，用于算速度差。 */
  lastTickAt: number;
}

// ===== 视频操作 =====
//
// 与下载那套刻意分开：事件走自己的通道，DTO 不塞进 TaskDto 的联合里，
// 免得两边互相污染。

export type JobStateDto =
  | { kind: "queued" }
  | { kind: "running" }
  | { kind: "done" }
  | { kind: "failed"; message: string }
  | { kind: "canceled" };

export interface JobDto {
  id: number;
  /** 形如 "压缩 · 均衡"，列表直接显示，不在前端拼。 */
  label: string;
  input: string;
  output: string;
  state: JobStateDto;
  /** 0–1。null = 时长未知，界面显示不确定态进度条。 */
  progress: number | null;
}

/** 界面直接用，不需要额外的本地字段 —— 视频任务没有"正在改名"这种局部态。 */
export type JobView = JobDto;

export type VideoOpEventDto =
  | { type: "added"; id: number }
  | { type: "removed"; id: number }
  | { type: "progress"; id: number; fraction: number }
  | { type: "stateChanged"; id: number; state: JobStateDto };

// ===== 视频操作（其二）：转码与裁剪 =====

/** `probe_video` 的返回。编码/容器取不到时为 null —— 界面显示"未知"，不编。 */
export interface VideoInfoDto {
  durationSecs: number | null;
  hasVideo: boolean;
  videoCodec: string | null;
  container: string | null;
  sizeBytes: number;
}

/**
 * 视频操作的入参。**带 `op` 判别标签**，与 Rust 侧的 `JobParamsDto` 一一对应
 * （`#[serde(tag = "op", rename_all_fields = "camelCase")]`）。
 *
 * 加一个操作就在这里加一个成员，`switch` 会替你找齐所有该改的地方。
 * 注意取值（target / mode / preset）在 core 那边是 `String`，非法取值要到
 * 入队时才被拒 —— 前端据此在提交前先把明显的错误挡掉。
 */
export type JobParamsDto =
  | {
      op: "compress";
      preset: string;
      encoder: string;
      maxHeight: number | null;
      crf: number | null;
      qv: number | null;
      audioKbps: number | null;
    }
  | { op: "transcode"; target: string; preset: string }
  | { op: "trim"; startSecs: number; endSecs: number | null; mode: string };
