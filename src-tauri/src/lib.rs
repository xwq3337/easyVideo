// 应用外壳的 lib crate。模块声明放这里,`main.rs` 是**另一个** crate root,
// 它用 `easydownvideo_tauri::...` 这样的路径访问这些模块。
//
// 本任务(脚手架)只有空窗口,逻辑都在 `main.rs`,所以这里暂时没有模块;
// 后续任务追加 `pub mod dto;` / `bridge` / `commands` / `state`。
