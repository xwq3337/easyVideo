// 应用外壳的 lib crate。模块声明放这里,`main.rs` 是**另一个** crate root,
// 它用 `easydownvideo_tauri::...` 这样的路径访问这些模块。
//
// **绝不要在 `main.rs` 里重复这些 `mod` 声明**:两个 root 各声明一次会得到两套互不相干的
// 类型,然后是极难定位的 `expected 'dto::X', found 'dto::X'`。
pub mod bridge;
pub mod commands;
pub mod dto;
pub mod state;
