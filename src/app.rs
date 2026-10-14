use std::time::{Duration, Instant};

use eframe::egui;
use easydownvideo::core::engine::Engine;
use easydownvideo::core::events::{Choice, ChoiceKind, EngineEvent, ProgressUnit, TaskId, TaskState};
use easydownvideo::settings::Settings;

pub fn build_engine(
    rt: tokio::runtime::Handle,
    settings: Settings,
    data_dir: std::path::PathBuf,
) -> Engine {
    Engine::new(rt, settings, data_dir)
}

struct UiTask {
    id: TaskId,
    url: String,
    filename: String,
    state: TaskState,
    downloaded: u64,
    total: u64,
    unit: ProgressUnit,
    speed_bps: f64,
    last_downloaded: u64,
    last_tick: Instant,
    choices: Option<ChoiceKind>,
    show_choices: bool,
}

impl UiTask {
    fn progress_text(&self) -> String {
        let pct = if self.total > 0 {
            self.downloaded as f64 / self.total as f64 * 100.0
        } else {
            0.0
        };
        match self.unit {
            ProgressUnit::Bytes => format!(
                "{} / {} ({:.1}%)",
                format_size(self.downloaded),
                format_size(self.total),
                pct
            ),
            ProgressUnit::Segments => format!(
                "分片 {} / {} ({:.1}%)",
                self.downloaded, self.total, pct
            ),
        }
    }

    fn progress_ratio(&self) -> f32 {
        if self.total > 0 {
            (self.downloaded as f32 / self.total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

pub struct App {
    engine: Engine,
    events: tokio::sync::broadcast::Receiver<EngineEvent>,
    settings: Settings,
    input_url: String,
    status: String,
    tasks: Vec<UiTask>,
    settings_open: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: Engine,
        settings: Settings,
    ) -> Self {
        // 装中文字体：egui 自带字体没有 CJK 字形，界面全是中文，不装就是满屏方框。
        // 放在最前面 —— 越早装，第一帧就越不会是方框。
        if !easydownvideo::ui_font::install_cjk_font(&cc.egui_ctx) {
            // 已经 warn 过；界面会退化成方框，但程序仍可用
        }
        let events = engine.events();
        // 水合：把引擎中已存在的任务（含 scan_resume 恢复的）载入 UI 列表
        let tasks: Vec<UiTask> = engine
            .list_tasks()
            .into_iter()
            .map(|s| {
                let (downloaded, total) = s.progress.snapshot();
                UiTask {
                    id: s.id,
                    url: s.url,
                    filename: s.filename,
                    state: s.state,
                    downloaded,
                    total,
                    unit: s.progress.unit(),
                    speed_bps: 0.0,
                    last_downloaded: downloaded,
                    last_tick: Instant::now(),
                    choices: None,
                    show_choices: false,
                }
            })
            .collect();
        let mut app = Self {
            engine,
            events,
            settings,
            input_url: String::new(),
            status: String::new(),
            tasks,
            settings_open: false,
        };
        app.restore_clipboard(cc);
        app
    }

    fn restore_clipboard(&mut self, _cc: &eframe::CreationContext<'_>) {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                if text.starts_with("http://") || text.starts_with("https://") {
                    self.input_url = text.trim().to_string();
                }
            }
        }
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                EngineEvent::StateChanged { id, state } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.state = state.clone();
                        if matches!(
                            state,
                            TaskState::Done | TaskState::Failed(_) | TaskState::Canceled
                        ) {
                            t.choices = None;
                            t.show_choices = false;
                        }
                    }
                }
                EngineEvent::Progress {
                    id,
                    downloaded,
                    total,
                    unit,
                } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.downloaded = downloaded;
                        t.total = total;
                        t.unit = unit;
                    }
                }
                EngineEvent::ChoicesOffered { id, kind } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.choices = Some(kind);
                        t.show_choices = true;
                    }
                }
            }
        }
    }

    fn tick_speeds(&mut self) {
        for t in &mut self.tasks {
            let now = Instant::now();
            let dt = now.duration_since(t.last_tick).as_secs_f64();
            if dt >= 1.0 {
                let delta = t.downloaded.saturating_sub(t.last_downloaded);
                t.speed_bps = delta as f64 / dt;
                t.last_downloaded = t.downloaded;
                t.last_tick = now;
            }
        }
    }

    fn add_task(&mut self) {
        let url = self.input_url.trim().to_string();
        if url.is_empty() {
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.status = "请输入 http(s) 链接".into();
            return;
        }
        let id = self.engine.add_url(url.clone());
        // 同一份推导逻辑，来自 probe 模块（不要在 GUI 里再抄一遍）
        let filename = easydownvideo::core::probe::filename_from_url(&url);
        self.tasks.push(UiTask {
            id,
            url,
            filename,
            state: TaskState::Queued,
            downloaded: 0,
            total: 0,
            unit: ProgressUnit::Bytes,
            speed_bps: 0.0,
            last_downloaded: 0,
            last_tick: Instant::now(),
            choices: None,
            show_choices: false,
        });
        self.input_url.clear();
        self.status.clear();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.tick_speeds();
        ctx.request_repaint_after(Duration::from_millis(200));

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // **这里不能用 `desired_width(f32::INFINITY)`**：水平布局里输入框会把整行
                // 一次吃光，紧随其后的「下载」「设置」被排到裁剪矩形之外 —— 屏幕上**完全看不到**
                // 这两个按钮（实测截图确认：整行只有输入框，右侧空白，设置窗口因此无从打开）。
                // 给后面两个按钮留出固定宽度，输入框只占剩余部分。
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.input_url)
                        .desired_width(ui.available_width() - 150.0)
                        .hint_text("粘贴视频链接（mp4 / m3u8 / 网页）后回车"),
                );
                let submit = ui.button("下载").clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit {
                    self.add_task();
                }
                if ui.button("设置").clicked() {
                    self.settings_open = !self.settings_open;
                }
            });
            ui.add_space(6.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let total_speed: f64 = self.tasks.iter().map(|t| t.speed_bps).sum();
                ui.label(format!("总速度: {}", format_speed(total_speed)));
                let active = self
                    .tasks
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.state,
                            TaskState::Downloading | TaskState::Probing | TaskState::Merging
                        )
                    })
                    .count();
                ui.label(format!("活跃任务: {active}"));
                if !self.status.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut remove_ids = Vec::new();
                    for i in 0..self.tasks.len() {
                        let t = &mut self.tasks[i];
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let name_edit = ui.add(
                                    egui::TextEdit::singleline(&mut t.filename)
                                        .desired_width(220.0),
                                );
                                if name_edit.lost_focus() {
                                    self.engine.set_filename(t.id, t.filename.clone());
                                }
                                ui.colored_label(state_color(&t.state), state_text(&t.state));
                            });
                            ui.horizontal(|ui| {
                                let bar = egui::ProgressBar::new(t.progress_ratio())
                                    .desired_width(420.0)
                                    .text(t.progress_text());
                                ui.add(bar);
                                // 分片计数模式下速度无意义（单位为分片），只在字节模式下显示
                                if t.unit == ProgressUnit::Bytes {
                                    ui.label(format_speed(t.speed_bps));
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        match t.state {
                                            TaskState::Downloading
                                            | TaskState::Probing
                                            | TaskState::Merging => {
                                                if ui.button("暂停").clicked() {
                                                    self.engine.pause(t.id);
                                                }
                                            }
                                            TaskState::Paused => {
                                                if ui.button("继续").clicked() {
                                                    self.engine.unpause(t.id);
                                                }
                                            }
                                            TaskState::Failed(_) => {
                                                if ui.button("重试").clicked() {
                                                    // 必须调 retry：unpause 只认 Paused，对 Failed 毫无反应，
                                                    // 那样这个按钮就是个点了没任何动静的死按钮。
                                                    self.engine.retry(t.id);
                                                }
                                            }
                                            _ => {}
                                        }
                                        if ui.button("删除").clicked() {
                                            self.engine.cancel(t.id);
                                            remove_ids.push(t.id);
                                        }
                                        if t.choices.is_some() && !t.show_choices {
                                            if ui.button("选择格式").clicked() {
                                                t.show_choices = true;
                                            }
                                        }
                                    },
                                );
                            });
                            ui.label(egui::RichText::new(&t.url).small().weak());
                        });
                        ui.add_space(4.0);

                        // 选择窗口（模态）
                        if t.show_choices {
                            if let Some(kind) = t.choices.clone() {
                                let mut open = t.show_choices;
                                let mut chosen: Option<Choice> = None;
                                egui::Window::new("选择下载格式")
                                    .collapsible(false)
                                    .resizable(false)
                                    .open(&mut open)
                                    .show(ctx, |ui| match &kind {
                                        ChoiceKind::HlsVariants(vars) => {
                                            ui.label("选择清晰度：");
                                            // 默认最高码率排最前
                                            let mut order: Vec<(usize, &_)> =
                                                vars.iter().enumerate().collect();
                                            order.sort_by(|a, b| {
                                                b.1.bandwidth.cmp(&a.1.bandwidth)
                                            });
                                            for (idx, v) in order {
                                                let label = match v.resolution {
                                                    Some((w, h)) => format!(
                                                        "{}x{} ({} kbps)",
                                                        w,
                                                        h,
                                                        v.bandwidth / 1000
                                                    ),
                                                    None => {
                                                        format!("{} kbps", v.bandwidth / 1000)
                                                    }
                                                };
                                                if ui.button(label).clicked() {
                                                    chosen = Some(Choice::Variant(idx));
                                                }
                                            }
                                        }
                                        ChoiceKind::YtdlpFormats(fmt) => {
                                            ui.label("yt-dlp 可用格式：");
                                            for (i, f) in fmt.iter().enumerate() {
                                                let size = f
                                                    .filesize
                                                    .map(format_size)
                                                    .unwrap_or_else(|| "未知大小".into());
                                                if ui
                                                    .button(format!(
                                                        "[{}] {} - {}",
                                                        f.id, f.note, size
                                                    ))
                                                    .clicked()
                                                {
                                                    chosen = Some(Choice::YtdlpFormat(i));
                                                }
                                            }
                                        }
                                    });
                                // 选中后必须关窗。原稿在 arm 里置 `show_choices = false`，紧接着
                                // 又 `t.show_choices = open` —— 而 `open` 只在用户点窗口的 [x] 时才变
                                // false，选完一个格式它仍是 true，于是把窗口**又置回打开**：模态框
                                // 赖着不走，里面每个按钮都已失效（`set_choice` 已把 pending_choice 取走）。
                                if let Some(c) = chosen {
                                    self.engine.set_choice(t.id, c);
                                    // 候选列表已被 `set_choice` 消费掉，清掉本地副本，
                                    // 否则"选择格式"按钮会再次出现（且点了只会开出一个失效窗口）。
                                    t.choices = None;
                                    t.show_choices = false;
                                } else {
                                    t.show_choices = open;
                                }
                            }
                        }
                    }
                    for id in remove_ids {
                        self.tasks.retain(|t| t.id != id);
                    }
                    if self.tasks.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(egui::RichText::new("暂无任务，粘贴链接开始下载").weak());
                        });
                    }
                });
        });

        // 设置窗口
        if self.settings_open {
            egui::Window::new("设置")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("下载目录：");
                        ui.label(self.settings.download_dir.display().to_string());
                        if ui.button("更改…").clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.settings.download_dir = dir;
                                let _ = self.settings.save();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("单任务并发：");
                        ui.add(egui::Slider::new(
                            &mut self.settings.segment_concurrency,
                            1..=16,
                        ));
                    });
                    ui.horizontal(|ui| {
                        ui.label("同时下载任务数：");
                        ui.add(egui::Slider::new(&mut self.settings.max_active_tasks, 1..=8));
                    });
                    ui.checkbox(&mut self.settings.ytdlp_enabled, "启用 yt-dlp 网页解析");
                    // 立即推给引擎（其余任务用新并发/开关，无需重启）
                    self.engine.update_settings(self.settings.clone());
                    if self.settings.save().is_err() {
                        ui.colored_label(egui::Color32::LIGHT_RED, "设置保存失败");
                    }
                });
        }
    }
}

fn state_text(s: &TaskState) -> String {
    match s {
        TaskState::Queued => "排队中".into(),
        TaskState::Probing => "探测中".into(),
        TaskState::AwaitingChoice => "等待选择".into(),
        TaskState::Downloading => "下载中".into(),
        TaskState::Merging => "合并中".into(),
        TaskState::Paused => "已暂停".into(),
        TaskState::Done => "完成".into(),
        TaskState::Failed(e) => format!("失败: {e}"),
        TaskState::Canceled => "已取消".into(),
    }
}

fn state_color(s: &TaskState) -> egui::Color32 {
    match s {
        TaskState::Done => egui::Color32::LIGHT_GREEN,
        TaskState::Failed(_) => egui::Color32::LIGHT_RED,
        TaskState::Downloading | TaskState::Merging => egui::Color32::LIGHT_BLUE,
        TaskState::Paused | TaskState::Canceled => egui::Color32::GRAY,
        _ => egui::Color32::YELLOW,
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn format_speed(bps: f64) -> String {
    format!("{}/s", format_size(bps as u64))
}
