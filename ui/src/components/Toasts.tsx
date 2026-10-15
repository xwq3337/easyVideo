import { dismissToast, useToasts } from "../store/store";

/** 命令级失败的落点：右下角一条可关掉的提示，不打断正在做的事。 */
export default function Toasts() {
  const toasts = useToasts();
  if (toasts.length === 0) return null;
  return (
    <div className="fixed right-4 bottom-12 flex flex-col gap-2 max-w-[420px]">
      {toasts.map((text, i) => (
        <button
          key={`${i}-${text}`}
          onClick={() => dismissToast(i)}
          className="text-left rounded-md px-3 py-2 shadow-lg"
          style={{
            background: "var(--bg)",
            border: "1px solid var(--accent-err)",
            color: "var(--accent-err)",
          }}
          title="点击关闭"
        >
          {text}
        </button>
      ))}
    </div>
  );
}
