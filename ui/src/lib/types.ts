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
}

export type EventDto =
  | { type: "stateChanged"; id: number; state: StateDto }
  | { type: "progress"; id: number; downloaded: number; total: number; unit: UnitDto }
  | { type: "choicesOffered"; id: number; kind: ChoiceKindDto };

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
}

/** 界面用的一行。比 TaskDto 多几样本地算出来的东西。 */
export interface TaskView extends TaskDto {
  /** 候选列表的本地副本。重同步时可能是 null —— 那就要去问 core。 */
  choices: ChoiceKindDto | null;
  /** 用户正在编辑文件名时为 true，此时不被进度事件刷新覆盖。 */
  editing: boolean;
  /** 由前后两次进度快照做差得出。分片单位一律为 null（那是片/秒，不是字节/秒）。 */
  speedBps: number | null;
  /** 形如 "剩 1 分 32 秒"；算不出来时为 null。 */
  etaText: string | null;
  /** 上一次进度事件的时间戳，用于算速度差。 */
  lastTickAt: number;
}
