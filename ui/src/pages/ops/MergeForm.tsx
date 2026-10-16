import { useEffect, useState } from "react";
import * as ipc from "../../lib/ipc";
import { moveDown, moveUp } from "../../lib/opForms";
import { run } from "../../store/store";
import type { JobParamsDto, MergeCheckDto, VideoInfoDto } from "../../lib/types";

const ACTION_BTN =
  "flex items-center gap-1 text-[12px] transition-colors hover:text-[var(--fg)] disabled:opacity-30 disabled:pointer-events-none";

/** 从路径里取文件名。路径分隔符两种都要认（Windows 上也有人用 `/`）。 */
function basename(p: string): string {
  return p.split(/[/\\]/).pop() || p;
}

/**
 * 合并：文件列表（顺序即拼接顺序）+ 兼容性结果。
 *
 * **兼容性规则只在 core 一处** —— 这里只负责把 `check_merge` 的结果画出来：
 * blockers 红字、warnings 灰字。抄一份规则进来必然漂移（时间解析那边已经吃过一次亏）。
 *
 * 有 blocker 时把参数交 `null`，外框据此禁用开始按钮 —— 那个按钮点下去只会
 * 产出一个打不开的文件，而 ffmpeg 会报 exit 0。
 */
export default function MergeForm({
  value,
  onChange,
  inputs,
  setInputs,
  infos,
}: {
  value: JobParamsDto | null;
  onChange: (p: JobParamsDto | null) => void;
  inputs: string[];
  setInputs: (next: string[]) => void;
  /** 与 `inputs` 逐位对应 —— 还没探完时是 null（界面显示「读取中…」）。 */
  infos: (VideoInfoDto | null)[];
}) {
  const [check, setCheck] = useState<MergeCheckDto | null>(null);

  // 文件列表一变就去问 core。依赖用拼起来的串而不是数组本身 —— 数组每帧都是新引用，
  // 会把这个 effect 变成每次都跑（每次都起 N 次 ffmpeg -i）。
  const listKey = inputs.join("\u0000");
  useEffect(() => {
    if (inputs.length < 2) {
      setCheck(null);
      return;
    }
    let alive = true;
    void run(ipc.checkMerge(inputs)).then((c) => {
      if (alive && c) setCheck(c);
    });
    return () => {
      alive = false;
    };
  }, [listKey]);

  // 少于两段就不叫合并了 —— 交 null 禁用开始按钮，而不是让用户拿到一份复制品。
  const blocked = check ? check.blockers.length > 0 : false;
  useEffect(() => {
    onChange(inputs.length >= 2 && !blocked ? { op: "merge" } : null);
  }, [inputs.length, blocked]);

  void value; // 合并没有参数，`value` 只在类型上参与（外框的契约）

  return (
    <>
      <div className="flex items-baseline gap-2 mb-2">
        <span style={{ color: "var(--fg-muted)" }}>按此顺序拼接</span>
        <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
          第一段的容器决定产物的容器
        </span>
      </div>

      {inputs.length === 0 ? (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          还没选文件。可以分几次挑，选中的会追加到这个列表里。
        </p>
      ) : (
        <div
          className="rounded-xl overflow-hidden mb-3"
          style={{ background: "var(--bg-subtle)" }}
        >
          {inputs.map((p, i) => (
            <div
              // key 带下标：同一个文件被选两次是**合法意图**（"合"就是把它重复一遍），
              // 只用 path 当 key 会让 React 把两行当成同一行，↑/↓/✕ 作用到错的那一行。
              key={`${i}:${p}`}
              className="flex items-center gap-2 px-3 py-2"
              style={{ borderBottom: "1px solid var(--line)" }}
            >
              <span className="truncate flex-1 text-[13px]" title={p}>
                {basename(p)}
              </span>
              <span
                className="shrink-0 text-[12px]"
                style={{ color: "var(--fg-faint)" }}
              >
                {infos[i]
                  ? `${infos[i].videoCodec ?? "编码未知"} · ${infos[i].width ?? "?"}x${infos[i].height ?? "?"}`
                  : "读取中…"}
              </span>
              <button
                className={ACTION_BTN}
                disabled={i === 0}
                title="上移"
                onClick={() => setInputs(moveUp(inputs, i))}
              >
                ↑
              </button>
              <button
                className={ACTION_BTN}
                disabled={i === inputs.length - 1}
                title="下移"
                onClick={() => setInputs(moveDown(inputs, i))}
              >
                ↓
              </button>
              <button
                className={ACTION_BTN}
                title="移除"
                onClick={() => setInputs(inputs.filter((_, k) => k !== i))}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      {inputs.length === 1 && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          合并至少要两段，再选一个。
        </p>
      )}

      {check?.blockers.map((b) => (
        <p key={b} className="text-[12px] mb-1" style={{ color: "var(--accent-err)" }}>
          {b}
        </p>
      ))}
      {check && check.blockers.length > 0 && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          直接拼接会产出一个打不开的文件。可以先把它「转码」成 MP4 · H.264 + AAC，再回来合并。
        </p>
      )}
      {check?.warnings.map((w) => (
        <p key={w} className="text-[12px] mb-1" style={{ color: "var(--fg-faint)" }}>
          {w}
        </p>
      ))}
      {check && <div className="mb-3" />}
    </>
  );
}
