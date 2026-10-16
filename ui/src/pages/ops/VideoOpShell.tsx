import { useEffect, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import JobList from "../../components/JobList";
import * as ipc from "../../lib/ipc";
import { formatSize } from "../../lib/format";
import { formatTimecode } from "../../lib/timecode";
import { planKey, realignPicks } from "../../lib/opForms";
import { run, useSettings } from "../../store/store";
import type { JobParamsDto, VideoInfoDto } from "../../lib/types";
import { opLabel, type VideoOp } from "../../lib/videoOps";

/** 表单拿到的东西：已选的文件、它们的媒体信息，以及改这个列表的入口。 */
export interface OpSource {
  inputs: string[];
  /**
   * 与 `inputs` **逐位对应** —— 还没探完或探测失败时是 `null`，**绝不过滤**。
   * 过滤掉会把后面所有行的信息整体前移，而这一页唯一的任务就是让用户按看到的
   * 信息定顺序。
   */
  infos: (VideoInfoDto | null)[];
  /** 整组替换。**按路径找回已有信息** —— 调顺序、删一项都不会错位。 */
  setInputs: (next: string[]) => void;
}

/** 一个已选文件与它的媒体信息。两者必须一起走，分开存必然错位。 */
type Pick = { path: string; info: VideoInfoDto | null };

/**
 * 各操作页共用的外框：选文件 / 源信息 / 输出目录 / 产物名预览 / 开始按钮 / 任务列表。
 *
 * 每个操作只写自己的参数区（`children`）—— 否则"选文件"那段要抄几遍，
 * 而其中任何一处将来都会各自漂移。
 *
 * **输入是一组**（合并要按顺序拼 N 个文件），但选法分两种：合并走多选、且**追加**
 * （用户常分几次挑），其余四个仍然单选（选一个就把列表整个换成它）。
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
   * **render prop 而不是普通 children**：裁剪表单要源时长（显示"共 8 秒"、
   * 做基本校验），裁剪页还要源宽高（裁剪框的坐标系），而 probe 结果归外框所有。
   * 传一个节点进来的话，这些就得在 VideoOpPage 里再探一次 —— 两个来源迟早不一致。
   */
  children: (src: OpSource) => ReactNode;
}) {
  const settings = useSettings();
  const [sel, setSel] = useState<Pick[]>([]);
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [planned, setPlanned] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const multi = op === "merge";

  // 输出目录默认跟随设置里的下载目录，但允许这一页临时改（不落盘、不污染设置）。
  useEffect(() => {
    if (settings && outputDir === null) setOutputDir(settings.downloadDir);
  }, [settings, outputDir]);

  const inputs = sel.map((s) => s.path);
  const infos = sel.map((s) => s.info);

  /** 整组替换：**按路径**保留已有信息，只探缺的那些。 */
  async function apply(next: string[]) {
    const aligned = realignPicks(sel, next);
    setSel(aligned);
    setPlanned(null);

    const missing = aligned.filter((p) => p.info === null);
    if (missing.length === 0) return;
    const probed = new Map<string, VideoInfoDto>();
    await Promise.all(
      missing.map(async (p) => {
        const i = await run(ipc.probeVideo(p.path));
        if (i) probed.set(p.path, i);
      })
    );
    // 只补信息，**不动顺序** —— 探测期间用户可能又改了列表
    setSel((cur) =>
      cur.map((p) =>
        p.info === null && probed.has(p.path) ? { path: p.path, info: probed.get(p.path)! } : p
      )
    );
  }

  function reset() {
    setSel([]);
    setPlanned(null);
  }

  /** 单文件的操作：选一个就把列表整个换成它。 */
  async function pickOne() {
    const p = await run(ipc.pickVideoFile());
    if (!p) return;
    await apply([p]);
  }

  /** 合并：多选，**追加**到现有列表。 */
  async function pickMany() {
    const ps = await run(ipc.pickVideoFiles());
    if (!ps || ps.length === 0) return;
    await apply([...inputs, ...ps]);
  }

  // **只在影响产物名的东西变化时重算** —— 键的构成在 `planKey` 里，并配有测试。
  // 其中**保存目录必须在**：它是产物名的一部分（重名让路就是在它里面找空位），
  // 漏了它的话用户改了保存位置、预览还指着旧目录。
  //
  // 第一项传的是**整组路径拼起来的串**：合并的顺序变了产物名不变，但列表变了
  // 就得重算（换了第一段，`<原名>` 就变了）。`planKey` 只用它做"变没变"的判断。
  const namer = planKey(inputs.length ? inputs.join("\u0000") : null, op, params, outputDir);

  useEffect(() => {
    if (namer === null || inputs.length === 0 || !outputDir || params === null) {
      setPlanned(null);
      return;
    }
    let alive = true;
    void run(ipc.planOutput(inputs, outputDir, params)).then((v) => {
      if (alive && v !== undefined) setPlanned(v);
    });
    return () => {
      alive = false;
    };
    // 依赖是这个字符串而不是 [inputs, outputDir, params] —— 后者每帧都是新对象，
    // 会把 effect 变成死循环。
  }, [namer]);

  async function start() {
    if (inputs.length === 0 || !outputDir || params === null || busy) return;
    setBusy(true);
    try {
      const id = await run(ipc.addVideoJob(inputs, outputDir, params));
      // 入队成功才清空选择：失败时把文件留在框里，用户要重试的就是它
      if (id !== undefined) reset();
    } finally {
      setBusy(false);
    }
  }

  const one = infos[0] ?? null;
  const srcLine =
    inputs.length === 0
      ? null
      : multi
        ? `已选 ${inputs.length} 个文件 · 共 ${formatSize(
            infos.reduce((n, i) => n + (i?.sizeBytes ?? 0), 0)
          )}`
        : one
          ? `源文件 ${formatSize(one.sizeBytes)} · ${
              one.durationSecs != null ? formatTimecode(one.durationSecs) : "时长未知"
            } · ${one.videoCodec ?? "编码未知"} · ${one.container ?? "容器未知"}`
          : null;

  return (
    <>
      <div className="flex items-center gap-2 py-4">
        <Input
          readOnly
          value={
            multi
              ? inputs.length
                ? `已选 ${inputs.length} 个文件`
                : ""
              : (inputs[0] ?? "")
          }
          placeholder={multi ? "选几个视频文件（可以分几次挑）…" : "选一个视频文件…"}
        />
        <Button onClick={() => void (multi ? pickMany() : pickOne())}>选择文件</Button>
        {inputs.length > 0 && (
          <Button variant="ghost" onClick={reset}>
            清空
          </Button>
        )}
      </div>

      {srcLine && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          {srcLine}
          {!multi && one && !one.hasVideo && " · 这个文件里没有视频流"}
        </p>
      )}

      {children({ inputs, infos, setInputs: (next) => void apply(next) })}

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
        disabled={
          busy ||
          inputs.length === 0 ||
          !outputDir ||
          params === null ||
          one?.hasVideo === false
        }
      >
        {busy ? "提交中…" : `开始${opLabel(op)}`}
      </Button>

      <div className="mt-6 pb-8">
        <JobList />
      </div>
    </>
  );
}
