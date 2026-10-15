// 应用外壳的 lib crate。模块声明放这里,`main.rs` 是**另一个** crate root,
// 它用 `easydownvideo_tauri::...` 这样的路径访问这些模块。
//
// 后续任务追加 `bridge` / `commands` / `state`。
pub mod bridge;
pub mod dto;
