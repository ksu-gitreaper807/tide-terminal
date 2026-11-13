use crate::theme::{self, ACCENT, BG, CHROME, CURSOR, DANGER, INK, Icon, MUTED, WARNING};
use alacritty_terminal::{term::TermMode, vte::ansi::CursorShape};
use anyhow::{Context, Result};
use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use tide_terminal::{
    append::{AppendMode, FollowUp, Prepared, append_file},
    capture::Recording,
    config::{Config, GpuMode},
    history::{History, HistoryEntry},
    rules::RuleSpec,
    session::{Event, Launch, Options, Session},
};

#[derive(Default)]
pub struct UiOptions {
    pub gpu: Option<GpuMode>,
    pub shell: Option<String>,
    pub clean: bool,
    pub private: bool,
    pub demo: bool,
    pub qa: bool,
    pub screenshot: Option<PathBuf>,
    pub exit_after: Option<Duration>,
    pub event_log: Option<PathBuf>,
}

pub fn launch(options: UiOptions) -> Result<()> {
    let title = if options.qa {
        "Tide Terminal QA"
    } else {
        "Tide Terminal"
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120., 740.])
            .with_min_inner_size([640., 420.])
            .with_transparent(true)
            .with_title(title),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: crate::gpu::configuration(options.gpu.unwrap_or(Config::load()?.gpu)),
        ..Default::default()
    };
    eframe::run_native(
        "Tide Terminal",
        native,
        Box::new(move |cc| Ok(Box::new(App::new(cc, options)))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

struct Pane {
    session: Session,
    label: String,
    cursor: Option<Pos2>,
    selection: Option<((usize, usize), (usize, usize))>,
    last_size: (u16, u16),
    visibility: f32,
    mouse_down: Option<u8>,
}
#[derive(Default)]
struct RunForm {
    command: String,
    timeout: String,
    timeout_unit: DurationUnit,
    idle: String,
    idle_unit: DurationUnit,
    marker: String,
    pattern: String,
    force: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum DurationUnit {
    #[default]
    Seconds,
    Minutes,
    Hours,
}

impl DurationUnit {
    fn label(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Seconds => "s",
            Self::Minutes => "m",
            Self::Hours => "h",
        }
    }
}

fn run_duration(text: &str, unit: DurationUnit) -> Result<Option<Duration>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value = if text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        format!("{text}{}", unit.suffix())
    } else {
        text.to_owned()
    };
    let duration = humantime::parse_duration(&value)?;
    anyhow::ensure!(!duration.is_zero(), "Enter a duration greater than zero");
    Ok(Some(duration))
}

fn duration_field(
    ui: &mut egui::Ui,
    rects: &mut std::collections::BTreeMap<String, Rect>,
    id: &str,
    value: &mut String,
    unit: &mut DurationUnit,
) {
    ui.allocate_ui_with_layout(Vec2::new(270., 28.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        track(
            rects,
            id,
            ui.add(
                egui::TextEdit::singleline(value)
                    .id_salt(id)
                    .desired_width(160.)
                    .hint_text(theme::hint("No limit · e.g. 30")),
            )
            .on_hover_text("Enter a number and choose a unit. You can also type 30s, 5m, 2h, or 500ms; an explicit unit overrides the selector."),
        );
        let response = egui::ComboBox::from_id_salt((id, "unit"))
            .width(86.)
            .selected_text(unit.label())
            .show_ui(ui, |ui| {
                for choice in [DurationUnit::Seconds, DurationUnit::Minutes, DurationUnit::Hours] {
                    let response = ui.selectable_value(unit, choice, choice.label());
                    track(rects, &format!("{id}_{}", choice.suffix()), response);
                }
            });
        track(rects, &format!("{id}_unit"), response.response);
    });
}
#[derive(Clone, Copy)]
enum CloseTarget {
    Tab(u64),
    Window,
}
struct CloseRequest {
    target: CloseTarget,
    jobs: Vec<String>,
}

struct App {
    config: Config,
    options: UiOptions,
    panes: Vec<Pane>,
    active: usize,
    scroll_active_tab: bool,
    tab_view_width: f32,
    error: Option<String>,
    close_request: Option<CloseRequest>,
    close_approved: bool,
    settings: bool,
    run_open: bool,
    append_open: bool,
    append_focus: bool,
    append_source: Option<(u64, Recording)>,
    append_mode: AppendMode,
    append_target: String,
    append_error: Option<String>,
    append_notice: Option<(String, Instant)>,
    followups: Vec<(u64, FollowUp)>,
    file_writes: Vec<crossbeam_channel::Receiver<std::result::Result<String, String>>>,
    history_open: bool,
    history_query: String,
    history_rows: Vec<HistoryEntry>,
    history_focus: bool,
    run_focus: bool,
    settings_saved: Option<Instant>,
    form: RunForm,
    activity: VecDeque<String>,
    started: Instant,
    screenshot_requested: bool,
    event_log: Option<std::fs::File>,
    last_interaction: Instant,
    qa_rects: std::collections::BTreeMap<String, Rect>,
    last_qa: String,
    gpu_name: String,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, options: UiOptions) -> Self {
        let mut config = Config::load().unwrap_or_default();
        if let Some(shell) = &options.shell {
            config.shell = shell.clone();
        }
        if let Some(gpu) = options.gpu {
            config.gpu = gpu;
        }
        config.private |= options.private;
        let font_error = theme::apply_with_font(&cc.egui_ctx, &config.font_file)
            .err()
            .map(|error| {
                theme::apply(&cc.egui_ctx);
                format!(
                    "{error:#}. Using the default font; correct Terminal font file in Settings."
                )
            });
        let mut event_log = options
            .event_log
            .as_ref()
            .and_then(|p| std::fs::File::create(p).ok());
        let gpu_name = if let Some(render) = &cc.wgpu_render_state {
            let info = render.adapter.get_info();
            if let Some(log) = &mut event_log {
                let _ = writeln!(
                    log,
                    "{}",
                    serde_json::json!({"gpu":{"name":info.name,"device_type":format!("{:?}",info.device_type),"backend":format!("{:?}",info.backend),"vendor":info.vendor,"device":info.device,"driver":info.driver}})
                );
            }
            format!("{} ({:?})", info.name, info.backend)
        } else {
            "No GPU adapter".into()
        };
        let mut app = Self {
            config,
            options,
            panes: vec![],
            active: 0,
            scroll_active_tab: false,
            tab_view_width: 0.,
            error: font_error,
            close_request: None,
            close_approved: false,
            settings: false,
            run_open: false,
            append_open: false,
            append_focus: false,
            append_source: None,
            append_mode: AppendMode::default(),
            append_target: String::new(),
            append_error: None,
            append_notice: None,
            followups: vec![],
            file_writes: vec![],
            history_open: false,
            history_query: String::new(),
            history_rows: vec![],
            history_focus: false,
            run_focus: false,
            settings_saved: None,
            form: RunForm {
                force: true,
                ..Default::default()
            },
            activity: VecDeque::new(),
            started: Instant::now(),
            screenshot_requested: false,
            event_log,
            last_interaction: Instant::now(),
            qa_rects: Default::default(),
            last_qa: String::new(),
            gpu_name,
        };
        app.open_shell(&cc.egui_ctx);
        app
    }
    fn base_options(&self) -> Options {
        let cwd = self
            .panes
            .get(self.active)
            .map(|p| PathBuf::from(&p.session.state.lock().unwrap().cwd))
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));
        let mut o = Options::shell(&self.config.shell, cwd);
        o.launch = Launch::Shell {
            path: self.config.shell.clone(),
            clean: self.options.clean,
        };
        o.history = Some(crate::history_path());
        o.private = self.config.private;
        o.notifications = self.config.notifications;
        o.suggestions = self.config.enhanced_suggestions;
        o
    }
    fn push(&mut self, session: Session, label: String, ctx: &egui::Context) {
        let c = ctx.clone();
        session.set_waker(move || c.request_repaint());
        ctx.animate_value_with_time(egui::Id::new(("tab_open", session.id)), 0., 0.18);
        ctx.animate_value_with_time(egui::Id::new(("pane_visible", session.id)), 0., 0.18);
        self.panes.push(Pane {
            session,
            label,
            cursor: None,
            selection: None,
            last_size: (0, 0),
            visibility: 0.,
            mouse_down: None,
        });
        self.active = self.panes.len() - 1;
        self.scroll_active_tab = true;
        ctx.request_repaint();
    }
    fn open_shell(&mut self, ctx: &egui::Context) {
        let o = self.base_options();
        let label = o.shell_name();
        match Session::spawn(o) {
            Ok(s) => self.push(s, label, ctx),
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }
    fn open_run(&mut self, ctx: &egui::Context) -> Result<()> {
        anyhow::ensure!(
            !self.form.command.trim().is_empty(),
            "Enter a command to run"
        );
        let mut o = self.base_options();
        o.launch = Launch::ShellCommand {
            shell: self.config.shell.clone(),
            command: self.form.command.clone(),
        };
        o.rules = RuleSpec {
            timeout: run_duration(&self.form.timeout, self.form.timeout_unit).context(
                "Allowed execution time: use a positive number or a duration such as 30s",
            )?,
            idle: run_duration(&self.form.idle, self.form.idle_unit)
                .context("Output idle limit: use a positive number or a duration such as 10s")?,
            literal: (!self.form.marker.is_empty()).then(|| self.form.marker.clone()),
            pattern: (!self.form.pattern.is_empty()).then(|| self.form.pattern.clone()),
            grace: Duration::from_secs(3),
            force: self.form.force,
        };
        let s = Session::spawn(o)?;
        let label = self
            .form
            .command
            .split_whitespace()
            .next()
            .unwrap_or("Command");
        let label = std::path::Path::new(label)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.push(s, label, ctx);
        Ok(())
    }
    fn load_history(&mut self) {
        if self.config.private {
            self.history_rows.clear();
            return;
        }
        let cwd = self
            .panes
            .get(self.active)
            .map(|p| p.session.state.lock().unwrap().cwd.clone())
            .unwrap_or_default();
        match History::open(&crate::history_path())
            .and_then(|h| h.search(&self.history_query, &cwd, 50))
        {
            Ok(rows) => self.history_rows = rows,
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn show_append(&mut self) {
        self.append_source = self.panes.get(self.active).and_then(|pane| {
            pane.session
                .state
                .lock()
                .unwrap()
                .recording
                .clone()
                .map(|source| (pane.session.id, source))
        });
        self.append_error = None;
        self.append_open = true;
        self.append_focus = true;
        self.run_open = false;
        self.history_open = false;
        self.settings = false;
    }
    fn queue_append(&mut self) -> Result<()> {
        let (session, source) = self
            .append_source
            .as_ref()
            .context("No tracked command is available in this tab")?;
        anyhow::ensure!(
            !self
                .followups
                .iter()
                .any(|(_, queued)| std::sync::Arc::ptr_eq(source, &queued.source)),
            "An action is already queued for this command"
        );
        let followup = FollowUp::new(
            source.clone(),
            self.append_mode,
            self.append_target.clone(),
            self.base_options(),
        )?;
        self.followups.push((*session, followup));
        self.append_notice = Some((
            "Appended action queued · waiting for source completion".into(),
            Instant::now(),
        ));
        self.activity
            .push_front("Appended action queued · original command will not be rerun".into());
        Ok(())
    }
    fn poll_followups(&mut self, ctx: &egui::Context) {
        let mut pending = Vec::new();
        for (source_session, followup) in std::mem::take(&mut self.followups) {
            if !self
                .panes
                .iter()
                .any(|pane| pane.session.id == source_session)
            {
                self.append_notice = Some((
                    "Queued append cancelled · source tab was closed".into(),
                    Instant::now(),
                ));
                self.activity
                    .push_front("Queued append cancelled · source tab was closed".into());
                continue;
            }
            match followup.prepare() {
                Ok(None) => pending.push((source_session, followup)),
                Ok(Some(Prepared::Skipped(code))) => {
                    let message = format!("&& action skipped · source exited with status {code}");
                    self.append_notice = Some((message.clone(), Instant::now()));
                    self.activity.push_front(message);
                }
                Ok(Some(Prepared::Command(options))) => {
                    if followup.mode == AppendMode::Pipe
                        && followup.source.lock().unwrap().truncated
                    {
                        self.activity.push_front(
                            "Processing retained output · source capture was truncated".into(),
                        );
                    }
                    let label = options
                        .label()
                        .split_whitespace()
                        .next()
                        .unwrap_or("Appended command")
                        .to_owned();
                    match Session::spawn(*options) {
                        Ok(session) => {
                            self.append_notice =
                                Some(("Appended command started".into(), Instant::now()));
                            self.push(session, label, ctx);
                        }
                        Err(error) => {
                            self.error = Some(format!("Could not run appended command: {error:#}"))
                        }
                    }
                }
                Ok(Some(Prepared::File {
                    path,
                    output,
                    truncated,
                })) => {
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    let ctx = ctx.clone();
                    self.file_writes.push(rx);
                    std::thread::spawn(move || {
                        let result = append_file(&path, &output)
                            .map(|size| {
                                format!(
                                    "Appended {size} bytes to {}{}",
                                    path.display(),
                                    if truncated {
                                        " · source capture truncated"
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .map_err(|error| {
                                format!("Could not append to {}: {error:#}", path.display())
                            });
                        let _ = tx.send(result);
                        ctx.request_repaint();
                    });
                }
                Err(error) => {
                    self.error = Some(format!("Could not prepare appended action: {error:#}"))
                }
            }
        }
        self.followups = pending;
        self.file_writes
            .retain(|receiver| match receiver.try_recv() {
                Ok(Ok(message)) => {
                    self.append_notice = Some((message.clone(), Instant::now()));
                    self.activity.push_front(message);
                    false
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    false
                }
                Err(crossbeam_channel::TryRecvError::Empty) => true,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.error = Some("File append worker stopped unexpectedly".into());
                    false
                }
            });
        self.activity.truncate(30);
        if !self.followups.is_empty() || !self.file_writes.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    fn poll(&mut self) {
        for (i, pane) in self.panes.iter_mut().enumerate() {
            if let Some(label) = command_label(&pane.session.state.lock().unwrap().last_command) {
                pane.label = label;
            }
            for event in pane.session.events.try_iter() {
                if let Some(log) = &mut self.event_log {
                    let _ = writeln!(log, "{}", serde_json::json!({"pane":i,"event":event}));
                }
                let msg = match event {
                    Event::Attention { message, .. } => Some(message),
                    Event::RuleTriggered { reason, .. } => Some(reason),
                    Event::Stopping { reason, .. } => Some(reason),
                    Event::Escalated { .. } => Some("Grace period ended · forced stop".into()),
                    Event::Exited { code, .. } => Some(format!("Run exited with status {code}")),
                    Event::Error { message } => {
                        self.error = Some(message.clone());
                        Some(message)
                    }
                    _ => None,
                };
                if let Some(msg) = msg {
                    self.activity.push_front(msg);
                    self.activity.truncate(30);
                }
            }
        }
    }
    fn request_close(&mut self, target: CloseTarget, ctx: &egui::Context) {
        let jobs = self
            .panes
            .iter()
            .filter(|pane| match target {
                CloseTarget::Tab(id) => pane.session.id == id,
                CloseTarget::Window => true,
            })
            .flat_map(|pane| pane.session.active_processes())
            .collect::<Vec<_>>();
        if jobs.is_empty() {
            self.finish_close(target, ctx);
        } else {
            self.run_open = false;
            self.append_open = false;
            self.settings = false;
            self.history_open = false;
            self.error = None;
            self.close_request = Some(CloseRequest { target, jobs });
        }
    }
    fn finish_close(&mut self, target: CloseTarget, ctx: &egui::Context) {
        self.close_request = None;
        match target {
            CloseTarget::Tab(id) => {
                if let Some(index) = self.panes.iter().position(|pane| pane.session.id == id) {
                    self.panes.remove(index);
                    if index < self.active {
                        self.active -= 1;
                    }
                    self.active = self.active.min(self.panes.len().saturating_sub(1));
                    self.scroll_active_tab = true;
                    ctx.request_repaint();
                }
            }
            CloseTarget::Window => {
                self.close_approved = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = &self.close_request else {
            return;
        };
        let target = request.target;
        let mut cancel = false;
        let mut confirm = false;
        theme::dialog(ctx, "close_confirmation").show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 96.).clamp(300., 480.));
            cancel = theme::heading(
                ui,
                match target {
                    CloseTarget::Tab(_) => "Close this terminal?",
                    CloseTarget::Window => "Close Tide?",
                },
            );
            ui.label(
                "Programs are still running. Closing will stop them and may discard unsaved work.",
            );
            ui.add_space(12.);
            egui::ScrollArea::vertical()
                .max_height(130.)
                .show(ui, |ui| {
                    for job in &request.jobs {
                        ui.label(theme::hint(job));
                    }
                });
            ui.add_space(20.);
            ui.horizontal(|ui| {
                cancel |=
                    track(&mut self.qa_rects, "close_cancel", ui.button("Keep open")).clicked();
                confirm = track(
                    &mut self.qa_rects,
                    "close_confirm",
                    ui.add(theme::primary("Stop and close")),
                )
                .clicked();
            });
        });
        // Require an explicit decision. The click that opens a reused modal must
        // not immediately dismiss it through the backdrop. Escape remains cancel.
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if confirm {
            self.finish_close(target, ctx);
        } else if cancel {
            self.close_request = None;
        }
    }
    fn modal_open(&self) -> bool {
        self.close_request.is_some()
            || self.settings
            || self.run_open
            || self.history_open
            || self.append_open
            || self.error.is_some()
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.options.screenshot.is_some()
            && ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers {
                        ctrl: true,
                        shift: true,
                        ..Default::default()
                    },
                    Key::S,
                )
            })
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.close_request.is_some() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                self.close_request = None;
            }
            return;
        }
        let (new, run, history, settings, close, next, zoom_in, append) = ctx.input_mut(|i| {
            let m = egui::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            };
            (
                i.consume_key(m, Key::T),
                i.consume_key(m, Key::R),
                i.consume_key(m, Key::H),
                i.consume_key(m, Key::Comma),
                i.consume_key(m, Key::W),
                i.consume_key(egui::Modifiers::CTRL, Key::Tab),
                i.consume_key(egui::Modifiers::CTRL, Key::Plus),
                i.consume_key(m, Key::A),
            )
        });
        if new {
            self.open_shell(ctx);
        }
        if append {
            self.show_append();
        }
        if run {
            self.append_open = false;
            self.run_open = true;
            self.run_focus = true;
            self.settings = false;
            self.history_open = false;
        }
        if history {
            self.append_open = false;
            self.history_open = true;
            self.history_focus = true;
            self.settings = false;
            self.run_open = false;
            self.load_history();
        }
        if settings {
            self.append_open = false;
            self.settings = !self.settings;
            self.run_open = false;
            self.history_open = false;
        }
        if close && !self.panes.is_empty() {
            self.request_close(CloseTarget::Tab(self.panes[self.active].session.id), ctx);
        }
        if next && !self.panes.is_empty() {
            self.active = (self.active + 1) % self.panes.len();
            self.scroll_active_tab = true;
        }
        if (self.settings || self.history_open || self.run_open || self.append_open)
            && !ctx.memory(|memory| memory.any_popup_open())
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            self.settings = false;
            self.history_open = false;
            self.run_open = false;
            self.append_open = false;
        }
        if zoom_in {
            self.config.font_size = (self.config.font_size + 1.).min(32.);
        }
    }
    fn toolbar(&mut self, ctx: &egui::Context, reveal: f32) {
        let mut close = None;
        let mut new_shell = false;
        if reveal > 0.001 {
            // Reserve only the space the revealed tabs need. The action panel
            // floats independently so one terminal can use the top of the window.
            egui::TopBottomPanel::top("session_bar")
                .show_separator_line(false)
                .exact_height(48. * reveal)
                .frame(egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(
                    BG.r(),
                    BG.g(),
                    BG.b(),
                    (self.config.opacity * 255.) as u8,
                )))
                .show(ctx, |_| {});
            egui::Area::new("terminal_tabs".into())
                .anchor(Align2::LEFT_TOP, Vec2::new(10., 6.))
                .order(egui::Order::Middle)
                .movable(false)
                .fade_in(false)
                .show(ctx, |ui| {
                    if self.close_request.is_some() {
                        ui.disable();
                    }
                    let mut clip = ctx.screen_rect();
                    clip.max.y = clip.top() + 48. * reveal;
                    ui.set_clip_rect(clip);
                    ui.spacing_mut().item_spacing.x = 8.;
                    let tab_width = (ctx.screen_rect().width() - 312.).max(100.);
                    ui.set_width(tab_width);
                    if (self.tab_view_width - tab_width).abs() > 1. {
                        self.scroll_active_tab = true;
                        self.tab_view_width = tab_width;
                    }
                    ui.allocate_ui_with_layout(
                        Vec2::new(tab_width, 40.),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(Vec2::new(tab_width, 40.));
                            self.qa_rects.insert("tab_strip".into(), ui.max_rect());
                            egui::ScrollArea::horizontal()
                                .id_salt("session_tabs")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                )
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.;
                                    ui.horizontal(|ui| {
                                        for (i, pane) in self.panes.iter().enumerate() {
                                            let opening = ctx.animate_value_with_time(
                                                egui::Id::new(("tab_open", pane.session.id)),
                                                1.,
                                                0.18,
                                            );
                                            let selected = ctx.animate_value_with_time(
                                                egui::Id::new(("tab_selected", pane.session.id)),
                                                f32::from(i == self.active),
                                                0.16,
                                            );
                                            ui.scope(|ui| {
                                                ui.multiply_opacity(
                                                    reveal
                                                        * (0.74 + 0.26 * selected)
                                                        * (0.45 + 0.55 * opening),
                                                );

                                                let state = pane.session.state.lock().unwrap();
                                                let active = i == self.active;
                                                let (rect, response) = ui.allocate_exact_size(
                                                    Vec2::new(142., 30.),
                                                    Sense::click(),
                                                );
                                                response.widget_info(|| {
                                                    egui::WidgetInfo::selected(
                                                        egui::WidgetType::SelectableLabel,
                                                        ui.is_enabled(),
                                                        active,
                                                        &pane.label,
                                                    )
                                                });
                                                let close_rect = Rect::from_center_size(
                                                    Pos2::new(rect.right() - 15., rect.center().y),
                                                    Vec2::splat(22.),
                                                );
                                                let close_response = ui
                                                    .interact(
                                                        close_rect,
                                                        ui.id().with(("close_tab", i)),
                                                        Sense::click(),
                                                    )
                                                    .on_hover_text("Close terminal · Ctrl+Shift+W");
                                                if selected > 0.
                                                    || response.hovered()
                                                    || close_response.hovered()
                                                {
                                                    ui.painter().rect_filled(
                                                        rect,
                                                        5.,
                                                        if selected > 0. {
                                                            Color32::from_rgba_unmultiplied(
                                                                43,
                                                                48,
                                                                56,
                                                                (255. * selected) as u8,
                                                            )
                                                        } else {
                                                            Color32::from_rgb(34, 38, 45)
                                                        },
                                                    );
                                                }
                                                if selected > 0. {
                                                    ui.painter().line_segment(
                                                        [
                                                            rect.left_bottom() + Vec2::new(9., -1.),
                                                            rect.right_bottom()
                                                                + Vec2::new(-9., -1.),
                                                        ],
                                                        Stroke::new(
                                                            1.5_f32,
                                                            ACCENT.gamma_multiply(selected),
                                                        ),
                                                    );
                                                }
                                                if response.has_focus() {
                                                    ui.painter().rect_stroke(
                                                        rect.shrink(1.),
                                                        5.,
                                                        Stroke::new(1.0_f32, ACCENT),
                                                        egui::StrokeKind::Inside,
                                                    );
                                                }
                                                let icon_rect = Rect::from_center_size(
                                                    Pos2::new(rect.left() + 16., rect.center().y),
                                                    Vec2::splat(16.),
                                                );
                                                if state.attention.is_some() {
                                                    ui.painter().circle_filled(
                                                        icon_rect.center(),
                                                        3.,
                                                        WARNING,
                                                    );
                                                } else {
                                                    theme::icon(
                                                        ui,
                                                        icon_rect,
                                                        Icon::Terminal,
                                                        if active { INK } else { MUTED },
                                                    );
                                                }
                                                let text_color = if active { INK } else { MUTED };
                                                let text_left = rect.left() + 31.;
                                                let title = ui.fonts(|fonts| {
                                                    fonts.layout_job(tab_title_job(
                                                        &pane.label,
                                                        (close_rect.left() - text_left - 4.)
                                                            .max(1.),
                                                        text_color,
                                                    ))
                                                });
                                                ui.painter().galley(
                                                    Pos2::new(
                                                        text_left,
                                                        rect.center().y - title.size().y / 2.,
                                                    ),
                                                    title,
                                                    text_color,
                                                );
                                                if selected > 0.
                                                    || response.hovered()
                                                    || close_response.hovered()
                                                {
                                                    theme::icon(
                                                        ui,
                                                        close_rect,
                                                        Icon::Close,
                                                        if close_response.hovered() {
                                                            INK
                                                        } else {
                                                            MUTED
                                                        },
                                                    );
                                                }
                                                self.qa_rects.insert(format!("tab_{i}"), rect);
                                                self.qa_rects
                                                    .insert(format!("tab_close_{i}"), close_rect);
                                                if close_response.clicked() {
                                                    close = Some(i);
                                                } else if response.clicked() {
                                                    self.active = i;
                                                    self.scroll_active_tab = true;
                                                }
                                                if self.scroll_active_tab && active {
                                                    response
                                                        .scroll_to_me(Some(egui::Align::Center));
                                                    self.scroll_active_tab = false;
                                                }
                                                response.on_hover_text(format!(
                                                    "{}\n{}",
                                                    pane.label,
                                                    theme::short_path(&state.cwd)
                                                ));
                                            });
                                        }
                                    });
                                });
                        },
                    );
                });
        }
        egui::Area::new("terminal_actions".into())
            .anchor(Align2::RIGHT_TOP, Vec2::new(-10., 6.))
            .order(egui::Order::Middle)
            .movable(false)
            .fade_in(false)
            .show(ctx, |ui| {
                    if self.close_request.is_some() { ui.disable(); }
                    let panel = egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(35, 41, 50, 218))
                        .stroke(Stroke::new(1_f32, Color32::from_white_alpha(24)))
                        .corner_radius(14)
                        .shadow(egui::epaint::Shadow { offset: [0, 5], blur: 16, spread: 0, color: Color32::from_black_alpha(60) })
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 6.;
                            ui.spacing_mut().button_padding = Vec2::new(10., 6.);
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.inactive.weak_bg_fill = Color32::from_rgba_unmultiplied(62, 71, 84, 135);
                            widgets.inactive.bg_stroke = Stroke::new(1_f32, Color32::from_white_alpha(8));
                            widgets.hovered.weak_bg_fill = Color32::from_rgba_unmultiplied(81, 103, 132, 190);
                            widgets.hovered.bg_stroke = Stroke::new(1_f32, Color32::from_white_alpha(28));
                            widgets.active.weak_bg_fill = Color32::from_rgba_unmultiplied(77, 112, 159, 210);
                            ui.horizontal(|ui| {
                                new_shell = track(&mut self.qa_rects, "new_tab", glass_icon_button(ui, Icon::Plus, "New terminal · Ctrl+Shift+T")).clicked();
                                if track(&mut self.qa_rects, "run", glass_text_button(ui, "Run", 48.).on_hover_text("Run a command with stop rules · Ctrl+Shift+R")).clicked() {
                                    self.append_open = false; self.run_open = true; self.run_focus = true; self.settings = false; self.history_open = false;
                                }
                                if track(&mut self.qa_rects, "append", glass_text_button(ui, "Append", 66.).on_hover_text("Append &&, process captured output with |, or append to a file with >> · Ctrl+Shift+A")).clicked() { self.show_append(); }
                                if track(&mut self.qa_rects, "history", glass_text_button(ui, "History", 66.).on_hover_text("Search command history · Ctrl+Shift+H")).clicked() {
                                    self.append_open = false; self.history_open = true; self.history_focus = true; self.run_open = false; self.settings = false; self.load_history();
                                }
                                if track(&mut self.qa_rects, "settings", glass_icon_button(ui, Icon::Settings, "Settings · Ctrl+Shift+," )).clicked() {
                                    self.append_open = false; self.settings = !self.settings; self.run_open = false; self.history_open = false;
                                }
                            });
                        });
                    self.qa_rects.insert("action_panel".into(), panel.response.rect);
            });
        if let Some(index) = close {
            self.request_close(CloseTarget::Tab(self.panes[index].session.id), ctx);
        }
        if new_shell {
            self.open_shell(ctx);
        }
        egui::TopBottomPanel::bottom("status")
            .exact_height(26.)
            .frame(
                egui::Frame::NONE
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(14, 3)),
            )
            .show(ctx, |ui| {
                if let Some(pane) = self.panes.get(self.active) {
                    let state = pane.session.state.lock().unwrap();
                    ui.horizontal(|ui| {
                        let (folder, _) =
                            ui.allocate_exact_size(Vec2::new(16., 16.), Sense::hover());
                        theme::icon(ui, folder, Icon::Folder, MUTED);
                        let width = (ui.available_width() - 220.).max(100.);
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, 18.),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(theme::short_path(&state.cwd))
                                            .size(12.)
                                            .color(MUTED),
                                    )
                                    .halign(egui::Align::Min)
                                    .truncate(),
                                )
                                .on_hover_text(&state.cwd);
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (status, color) = if state.attention.is_some() {
                                ("Input requested".to_owned(), WARNING)
                            } else if let Some(code) = state.exit_code {
                                (
                                    format!(
                                        "Exited {}",
                                        state
                                            .exit_signal
                                            .clone()
                                            .unwrap_or_else(|| code.to_string())
                                    ),
                                    if code == 0 { MUTED } else { DANGER },
                                )
                            } else if state.stop_reason.is_some() {
                                ("Stopping".into(), WARNING)
                            } else if state.at_prompt {
                                ("Ready".into(), MUTED)
                            } else {
                                ("Running".into(), INK)
                            };
                            ui.label(RichText::new(status).size(12.).color(color));
                            if !state.at_prompt {
                                ui.label(
                                    RichText::new(theme::duration(state.elapsed))
                                        .size(12.)
                                        .color(MUTED),
                                );
                            }
                            if self.config.private {
                                ui.label(RichText::new("Private").size(12.).color(MUTED));
                            }
                        });
                    });
                } else {
                    ui.label(RichText::new("No open terminals").size(12.).color(MUTED));
                }
            });
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.close_request.is_some() {
            self.close_dialog(ctx);
            return;
        }
        let width = (ctx.screen_rect().width() - 96.).clamp(320., 568.);
        let body_height = (ctx.screen_rect().height() - 230.).max(120.);
        if self.settings {
            let mut close = false;
            let mut changed = false;
            let modal = theme::dialog(ctx, "settings_dialog").show(ctx, |ui| {
                ui.set_width(width);
                close = theme::heading(ui, "Settings");
                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .max_height(body_height)
                    .show(ui, |ui| {
                        theme::section(ui, "Appearance");
                        egui::Grid::new("appearance_grid")
                            .num_columns(2)
                            .spacing([24., 14.])
                            .show(ui, |ui| {
                                ui.label("Background opacity");
                                ui.allocate_ui_with_layout(
                                    Vec2::new(248., 28.),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        changed |= track(
                                            &mut self.qa_rects,
                                            "opacity",
                                            ui.add(
                                                egui::Slider::new(
                                                    &mut self.config.opacity,
                                                    0.3..=1.,
                                                )
                                                .show_value(false),
                                            ),
                                        )
                                        .changed();
                                        ui.label(
                                            RichText::new(format!(
                                                "{:.0}%",
                                                self.config.opacity * 100.
                                            ))
                                            .color(MUTED),
                                        );
                                    },
                                );
                                ui.end_row();
                                ui.label("Terminal font size");
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut self.config.font_size, 10.0..=32.0)
                                            .fixed_decimals(0)
                                            .suffix(" px"),
                                    )
                                    .changed();
                                ui.end_row();
                            });
                        ui.label("Terminal font file (restart required)");
                        changed |= track(&mut self.qa_rects, "font_file", ui.add(egui::TextEdit::singleline(&mut self.config.font_file).desired_width(f32::INFINITY).hint_text("Default, or an absolute path to a monospace .ttf / .otf font"))).changed();
                        ui.add_space(8.);
                        changed |= ui
                            .checkbox(&mut self.config.cursor_animation, "Smooth cursor motion")
                            .changed();
                        ui.label(theme::help(format!("Save preferences to {}. Font-file and renderer changes apply after restarting Tide.", tide_terminal::config::config_path().display())));
                        ui.add_space(8.);
                        ui.separator();
                        theme::section(ui, "Shell & sessions");
                        ui.horizontal(|ui| {
                            ui.label("Default shell");
                            let name = std::path::Path::new(&self.config.shell)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy();
                            egui::ComboBox::from_id_salt("shell")
                                .width(200.)
                                .selected_text(name)
                                .show_ui(ui, |ui| {
                                    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/fish"] {
                                        changed |= ui
                                            .selectable_value(
                                                &mut self.config.shell,
                                                shell.into(),
                                                shell,
                                            )
                                            .changed();
                                    }
                                });
                        });
                        ui.add_space(6.);
                        changed |= ui
                            .checkbox(
                                &mut self.config.enhanced_suggestions,
                                "Inline command suggestions",
                            )
                            .changed();
                        changed |= ui
                            .checkbox(
                                &mut self.config.private,
                                "Start new sessions without history",
                            )
                            .changed();
                        changed |= ui
                            .checkbox(
                                &mut self.config.notifications,
                                "Desktop notifications for input prompts",
                            )
                            .changed();
                        ui.label(
                            theme::help("The default shell applies to new tabs and Run commands. Existing shells keep running."),
                        );
                        ui.add_space(12.);
                        ui.separator();
                        ui.add_space(8.);
                        ui.horizontal(|ui| {
                            ui.label("Renderer (restart required)");
                            let combo = egui::ComboBox::from_id_salt("gpu_mode")
                                .selected_text(self.config.gpu.label())
                                .show_ui(ui, |ui| {
                                    for mode in [GpuMode::Integrated, GpuMode::Discrete, GpuMode::Auto, GpuMode::Software] {
                                        changed |= ui.selectable_value(&mut self.config.gpu, mode, mode.label()).changed();
                                    }
                                });
                            self.qa_rects.insert("gpu_mode".into(), combo.response.rect);
                        });
                        ui.label(theme::help("Save, then restart Tide to change renderer. Integrated and Discrete require that GPU; Software requires an available CPU graphics driver."));
                        ui.label(
                            RichText::new(format!("Active renderer: {}", self.gpu_name))
                                .small()
                                .color(MUTED),
                        );
                    });
                if changed {
                    self.settings_saved = None;
                }
                ui.add_space(16.);
                ui.horizontal(|ui| {
                    if self
                        .settings_saved
                        .is_some_and(|when| when.elapsed() < Duration::from_secs(3))
                    {
                        ui.label(RichText::new("Settings saved").small().color(MUTED));
                        ctx.request_repaint_after(Duration::from_secs(3));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if track(
                            &mut self.qa_rects,
                            "save_settings",
                            ui.add(theme::primary("Save changes")),
                        )
                        .clicked()
                        {
                            match theme::validate_terminal_font(&self.config.font_file).and_then(|_| self.config.save()) {
                                Ok(()) => self.settings_saved = Some(Instant::now()),
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                    });
                });
            });
            if close || modal.should_close() {
                self.settings = false;
            }
        }
        if self.run_open {
            let mut close = false;
            let mut launch = false;
            let mut cancel = false;
            let modal = theme::dialog(ctx, "run_dialog").show(ctx, |ui| {
                ui.set_width(width);
                close = theme::heading(ui, "Run command");
                egui::ScrollArea::vertical().id_salt("run_scroll").max_height(body_height).show(ui, |ui| {
                    ui.label(theme::help("Start a command in its own terminal. Stop it when a time limit or output rule matches."));
                    ui.add_space(8.);
                    track(&mut self.qa_rects, "run_shell", ui.label(theme::help(format!("Shell: {} · Change in Settings", self.config.shell))).on_hover_text("Chosen in Settings → Default shell, or with the --shell launch option. Use this shell’s command syntax. Run starts a fresh instance of it with -c."));
                    ui.add_space(12.);
                    ui.label("Command");
                    let command = track(&mut self.qa_rects, "command", ui.add(egui::TextEdit::multiline(&mut self.form.command).id_salt("run_command").font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(3).margin(egui::Margin::same(10)).hint_text(theme::hint("Enter a command to run, for example:\nsleep 30"))));
                    // A new Area first runs a disabled sizing pass. Keep the
                    // focus request pending until the actual input widget exists.
                    if self.run_focus && ui.is_enabled() && !ui.is_sizing_pass() {
                        command.request_focus();
                        self.run_focus = false;
                        ctx.request_repaint();
                    }
                    theme::section(ui, "Stop conditions");
                    egui::Grid::new("rules_form").num_columns(2).spacing([24., 12.]).show(ui, |ui| {
                        ui.label("Allowed execution time").on_hover_text("Stop after this much time has elapsed since the command started. Leave empty for no runtime limit.");
                        duration_field(ui, &mut self.qa_rects, "timeout", &mut self.form.timeout, &mut self.form.timeout_unit); ui.end_row();
                        ui.label("Output idle limit").on_hover_text("Stop if the program produces no visible output for this long. Output resets this timer; typing alone does not.");
                        duration_field(ui, &mut self.qa_rects, "idle", &mut self.form.idle, &mut self.form.idle_unit); ui.end_row();
                        ui.label("Output contains").on_hover_text("Stop when visible output contains this exact text. Matching is case sensitive.");
                        track(&mut self.qa_rects, "marker", ui.add(egui::TextEdit::singleline(&mut self.form.marker).id_salt("run_marker").desired_width(270.).hint_text(theme::hint("Exact text to stop on, e.g. DONE")))); ui.end_row();
                        ui.label("Output matches regex").on_hover_text("Stop when visible output matches a Rust regular expression. Leave empty to disable this rule.");
                        track(&mut self.qa_rects, "pattern", ui.add(egui::TextEdit::singleline(&mut self.form.pattern).id_salt("run_pattern").desired_width(270.).hint_text(theme::hint("Pattern to stop on, e.g. ERROR|FAILED")))); ui.end_row();
                    });
                    ui.label(theme::help("Choose seconds, minutes, or hours. You can also type 30s, 5m, 2h, or 500ms."));
                    ui.add_space(10.);
                    ui.checkbox(&mut self.form.force, "Force quit 3 seconds after a stop request");
                    ui.label(theme::help("First match interrupts the command. Leave all rules empty for manual control."));
                    ui.add_space(8.);
                    ui.label(theme::help("Uses this tab’s working directory and a fresh shell. Aliases and local variables from an existing session are not copied."));
                });
                if let Some(error) = &self.form.error {
                    ui.add_space(8.);
                    track(&mut self.qa_rects, "run_error", ui.colored_label(DANGER, error));
                }
                ui.add_space(16.);
                ui.horizontal(|ui| {
                    ui.label(theme::help("Ctrl+C interrupts a running command."));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        launch = track(&mut self.qa_rects, "start_run", ui.add(theme::primary("Run command"))).clicked();
                        cancel = track(&mut self.qa_rects, "run_cancel", ui.add(egui::Button::new("Cancel").frame(false))).clicked();
                    });
                });
            });
            if close || cancel || modal.should_close() {
                self.run_open = false;
            }
            if launch {
                match self.open_run(ctx) {
                    Ok(()) => {
                        self.run_open = false;
                        self.form.error = None;
                    }
                    Err(error) => self.form.error = Some(format!("{error:#}")),
                }
            }
        }
        if self.history_open {
            let mut close = false;
            let mut selected = None;
            let modal = theme::dialog(ctx, "history_dialog").show(ctx, |ui| {
                ui.set_width(width);
                close = theme::heading(ui, "Command history");
                let query = track(
                    &mut self.qa_rects,
                    "history_query",
                    ui.add(
                        egui::TextEdit::singleline(&mut self.history_query)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::same(10))
                            .hint_text(theme::hint("Search saved commands by name or arguments")),
                    ),
                );
                if self.history_focus {
                    query.request_focus();
                    self.history_focus = false;
                }
                if query.changed() {
                    self.load_history();
                }
                ui.add_space(8.);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("history_scroll")
                    .max_height(body_height.min(360.))
                    .min_scrolled_height(120.)
                    .show(ui, |ui| {
                        if self.history_rows.is_empty() {
                            ui.add_space(20.);
                            ui.label(if self.config.private {
                                "History is paused for private sessions."
                            } else {
                                "No matching commands."
                            });
                            ui.add_space(20.);
                        }
                        for (index, entry) in self.history_rows.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 58.),
                                Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    &entry.command,
                                )
                            });
                            if response.hovered() || response.has_focus() {
                                ui.painter()
                                    .rect_filled(rect, 5., Color32::from_rgb(39, 44, 53));
                            }
                            let painter = ui.painter().with_clip_rect(rect.shrink(6.));
                            painter.text(
                                rect.min + Vec2::new(10., 9.),
                                Align2::LEFT_TOP,
                                &entry.command,
                                FontId::monospace(13.),
                                INK,
                            );
                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    rect.min,
                                    rect.max - Vec2::new(84., 0.),
                                ))
                                .text(
                                    rect.min + Vec2::new(10., 33.),
                                    Align2::LEFT_TOP,
                                    format!("{}    {}", entry.shell, theme::short_path(&entry.cwd)),
                                    FontId::proportional(11.5),
                                    MUTED,
                                );
                            painter.text(
                                rect.right_top() + Vec2::new(-10., 33.),
                                Align2::RIGHT_TOP,
                                format!("Exit {}", entry.exit_code),
                                FontId::proportional(11.5),
                                if entry.exit_code == 0 { MUTED } else { DANGER },
                            );
                            let clicked = response.clicked();
                            track(
                                &mut self.qa_rects,
                                &format!("history_result_{index}"),
                                response.on_hover_text(&entry.command),
                            );
                            if clicked {
                                selected = Some(entry.command.clone());
                            }
                        }
                    });
                ui.add_space(12.);
                ui.label(
                    theme::help("Select a result to insert it at the shell prompt. Press Enter in the terminal when you’re ready to run it."),
                );
            });
            if close || modal.should_close() {
                self.history_open = false;
            }
            if let Some(command) = selected
                && let Some(pane) = self.panes.get(self.active)
            {
                if pane.session.state.lock().unwrap().at_prompt {
                    match pane.session.insert_command(&command) {
                        Ok(()) => self.history_open = false,
                        Err(error) => self.error = Some(error.to_string()),
                    }
                } else {
                    self.error = Some(
                        "Select a terminal at its shell prompt before inserting a command.".into(),
                    );
                }
            }
        }
        if self.append_open {
            let mut close = false;
            let mut submit = false;
            let mut cancel_queued = false;
            let modal = theme::dialog(ctx, "append_dialog").show(ctx, |ui| {
                ui.set_width(width);
                close = theme::heading(ui, "Append to command");
                let pending=self.append_source.as_ref().is_some_and(|(_,source)| self.followups.iter().any(|(_,queued)|std::sync::Arc::ptr_eq(source,&queued.source)));
                egui::ScrollArea::vertical().id_salt("append_scroll").max_height(body_height).show(ui, |ui| {
                if let Some((_, source)) = &self.append_source {
                    let run = source.lock().unwrap();
                    ui.label(theme::help("Original command"));
                    ui.add(egui::Label::new(RichText::new(&run.command).monospace()).wrap());
                    ui.label(theme::help(match run.exit_code {Some(code)=>format!("Finished · exit status {code}"),None=>"Running · the appended action waits for completion".into()}));
                    if run.truncated {ui.label(RichText::new("Captured output was truncated; only retained text is available.").color(WARNING));}
                } else {
                    ui.label(theme::help("Run a command in this tab first. Shell command tracking is required."));
                }
                ui.add_space(12.);
                let mode_selector = egui::ComboBox::from_id_salt("append_mode").selected_text(self.append_mode.label()).width(300.).show_ui(ui, |ui| {
                    for mode in [AppendMode::AfterSuccess, AppendMode::Pipe, AppendMode::File] {
                        track(&mut self.qa_rects, match mode {AppendMode::AfterSuccess=>"append_and",AppendMode::Pipe=>"append_pipe",AppendMode::File=>"append_file"}, ui.selectable_value(&mut self.append_mode, mode, mode.label()));
                    }
                });
                track(&mut self.qa_rects, "append_mode", mode_selector.response);
                ui.label(theme::help(match self.append_mode {
                    AppendMode::AfterSuccess=>"Run a fresh shell command only if the original exits successfully. The original is never rerun.",
                    AppendMode::Pipe=>"Send the original's captured terminal text to a fresh command as stdin after it finishes, even if it fails. stdout and stderr are combined; this is not a live or binary-clean pipe.",
                    AppendMode::File=>"Append captured terminal text to this file after the original finishes, even if it fails. Existing file contents are preserved. Relative paths use the source command's directory.",
                }));
                ui.add_space(10.);
                ui.label(if self.append_mode==AppendMode::File {"File path"} else {"Command to append"});
                let input = track(&mut self.qa_rects, "append_target", ui.add(egui::TextEdit::multiline(&mut self.append_target).id_salt("append_target").font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(3).hint_text(theme::hint(if self.append_mode==AppendMode::File {"logs/search-results.txt"} else {"For example: grep -i error"}))));
                if self.append_focus && ui.is_enabled() && !ui.is_sizing_pass() {input.request_focus();self.append_focus=false;ctx.request_repaint();}
                ui.label(theme::help("Retains terminal text, up to 1 MiB. New commands use a fresh source shell; interactive aliases and unexported variables are not copied."));
                if let Some(error)=&self.append_error {ui.label(RichText::new(error).color(DANGER));}

                if pending {
                    ui.label(theme::help("An action is already queued for this command. Add another step from its resulting tab."));

                }
                });
                if pending {cancel_queued=track(&mut self.qa_rects,"append_cancel_queued",ui.button("Cancel queued action")).clicked();}
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    close |= ui.button("Cancel").clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        submit=track(&mut self.qa_rects,"append_submit",ui.add_enabled(self.append_source.is_some() && !pending, theme::primary(if self.append_mode==AppendMode::File {"Append to file"} else {"Append command"}))).clicked();
                    });
                });
            });
            if cancel_queued && let Some((_, source)) = &self.append_source {
                self.followups
                    .retain(|(_, queued)| !std::sync::Arc::ptr_eq(source, &queued.source));
                self.append_notice = Some(("Queued append cancelled".into(), Instant::now()));
            }
            if submit {
                match self.queue_append() {
                    Ok(()) => self.append_open = false,
                    Err(error) => self.append_error = Some(format!("{error:#}")),
                }
            }
            if close || modal.should_close() {
                self.append_open = false;
            }
        }
        if let Some(message) = self.error.clone() {
            let mut close = false;
            let modal = theme::dialog(ctx, "error_dialog").show(ctx, |ui| {
                ui.set_width(width.min(450.));
                close = theme::heading(ui, "Unable to complete action");
                ui.label(message);
                ui.add_space(16.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= ui.add(theme::primary("Dismiss")).clicked();
                });
            });
            if close || modal.should_close() {
                self.error = None;
            }
        }
    }
    fn terminal(&mut self, ctx: &egui::Context) {
        let alpha = (self.config.opacity * 255.) as u8;
        let fill = Color32::from_rgba_unmultiplied(BG.r(), BG.g(), BG.b(), alpha);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(fill).inner_margin(egui::Margin {
                left: 18,
                right: 18,
                top: 6,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                if self.panes.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.35).max(40.));
                        ui.label(RichText::new("No open terminals").size(20.).color(INK));
                        ui.add_space(15.);
                        if ui.add(theme::primary("New terminal")).clicked() {
                            self.open_shell(ctx);
                        }
                    });
                    return;
                }
                let p = &mut self.panes[self.active];
                {
                    let state = p.session.state.lock().unwrap();
                    if state.managed {
                        egui::Frame::NONE
                            .fill(CHROME)
                            .corner_radius(5)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                if let Some(actions) = self.qa_rects.get("action_panel")
                                    && ui.max_rect().top() < actions.bottom()
                                {
                                    ui.set_max_width(
                                        (actions.left() - ui.max_rect().left() - 12.)
                                            .min(ui.available_width())
                                            .max(80.),
                                    );
                                }
                                ui.horizontal(|ui| {
                                    let width = (ui.available_width() - 70.).max(80.);
                                    ui.add_sized(
                                        [width, 22.],
                                        egui::Label::new(
                                            RichText::new(&state.command)
                                                .monospace()
                                                .size(12.)
                                                .color(MUTED),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&state.command);
                                    if state.running
                                        && track(
                                            &mut self.qa_rects,
                                            "stop_run",
                                            ui.add(
                                                egui::Button::new(
                                                    RichText::new("Stop").color(DANGER),
                                                )
                                                .frame(false),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        p.session.stop();
                                    }
                                });
                                if let Some(reason) = &state.stop_reason {
                                    ui.label(RichText::new(reason).small().color(MUTED));
                                }
                            });
                        ui.add_space(10.);
                    }
                }
                let attention = p.session.state.lock().unwrap().attention.clone();
                if let Some(message) = attention {
                    let (title, help) = if message.contains("fingerprint") {
                        (
                            "Fingerprint prompt",
                            "Use the fingerprint reader to continue.",
                        )
                    } else if message.contains("password") {
                        (
                            "Password prompt",
                            "Enter your password in the terminal below.",
                        )
                    } else if message.contains("confirmation") {
                        (
                            "Confirmation prompt",
                            "Review the question in the terminal below.",
                        )
                    } else {
                        ("Input requested", "Check the program's prompt to continue.")
                    };
                    let banner = egui::Frame::NONE
                        .fill(Color32::from_rgb(43, 38, 29))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (indicator, _) =
                                    ui.allocate_exact_size(Vec2::new(12., 28.), Sense::hover());
                                ui.painter().circle_filled(indicator.center(), 3., WARNING);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(title).strong().color(WARNING));
                                    ui.label(RichText::new(help).small().color(MUTED));
                                });
                            });
                        });
                    let rect = banner.response.rect;
                    ui.painter().line_segment(
                        [
                            rect.left_top() + Vec2::new(0., 8.),
                            rect.left_bottom() - Vec2::new(0., 8.),
                        ],
                        Stroke::new(2.0_f32, WARNING.gamma_multiply(0.6)),
                    );
                    ui.add_space(12.);
                }
                let font = FontId::monospace(self.config.font_size);
                let cell = ui
                    .fonts(|f| Vec2::new(f.glyph_width(&font, 'M'), f.row_height(&font)))
                    .max(Vec2::new(1., 1.));
                let available = ui.available_size();
                let cols = (available.x / cell.x).floor().max(2.) as u16;
                let rows = (available.y / cell.y).floor().max(2.) as u16;
                if p.last_size != (rows, cols) {
                    p.session.resize(rows, cols);
                    p.last_size = (rows, cols);
                }
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(cols as f32 * cell.x, rows as f32 * cell.y),
                    Sense::hover(),
                );
                let response = ui.interact(
                    rect,
                    terminal_input_id(p.session.id),
                    Sense::click_and_drag(),
                );
                self.qa_rects.insert("terminal".into(), rect);
                let view = p.session.snapshot();
                let mut painter = ui.painter_at(rect);
                painter.multiply_opacity(0.65 + 0.35 * p.visibility);
                let point = |pos: Pos2| -> (usize, usize) {
                    (
                        ((pos.y - rect.top()) / cell.y)
                            .floor()
                            .clamp(0., rows.saturating_sub(1) as f32)
                            as usize,
                        ((pos.x - rect.left()) / cell.x)
                            .floor()
                            .clamp(0., cols.saturating_sub(1) as f32)
                            as usize,
                    )
                };
                let modal = self.settings
                    || self.run_open
                    || self.history_open
                    || self.append_open
                    || self.error.is_some();
                if !modal && !ctx.memory(|memory| memory.any_popup_open()) {
                    focus_terminal(ctx, response.id);
                }
                let mouse_mode = view.mode.intersects(TermMode::MOUSE_MODE)
                    && !ctx.input(|i| i.modifiers.shift)
                    && !modal;
                if response.clicked() {
                    if !modal {
                        focus_terminal(ctx, response.id);
                    }
                    p.selection = None;
                    self.last_interaction = Instant::now();
                }
                if !mouse_mode
                    && response.drag_started()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let a = point(pos);
                    p.selection = Some((a, a));
                }
                if !mouse_mode
                    && response.dragged()
                    && let (Some((a, _)), Some(pos)) =
                        (p.selection, response.interact_pointer_pos())
                {
                    p.selection = Some((a, point(pos)));
                }
                if mouse_mode {
                    for event in ctx.input(|i| i.events.clone()) {
                        match event {
                            egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                modifiers,
                            } if rect.contains(pos) || (!pressed && p.mouse_down.is_some()) => {
                                let button = match button {
                                    egui::PointerButton::Primary => 0,
                                    egui::PointerButton::Middle => 1,
                                    egui::PointerButton::Secondary => 2,
                                    _ => continue,
                                };
                                let (row, col) = point(pos);
                                let bits = button
                                    + 4 * u8::from(modifiers.shift)
                                    + 8 * u8::from(modifiers.alt)
                                    + 16 * u8::from(modifiers.ctrl);
                                let _ = p
                                    .session
                                    .write(mouse_bytes(bits, row, col, pressed, view.mode));
                                p.mouse_down = if pressed { Some(button) } else { None };
                            }
                            egui::Event::PointerMoved(pos)
                                if rect.contains(pos)
                                    && (view.mode.contains(TermMode::MOUSE_MOTION)
                                        || (view.mode.contains(TermMode::MOUSE_DRAG)
                                            && p.mouse_down.is_some())) =>
                            {
                                let (row, col) = point(pos);
                                let _ = p.session.write(mouse_bytes(
                                    32 + p.mouse_down.unwrap_or(3),
                                    row,
                                    col,
                                    true,
                                    view.mode,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                if response.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.1 {
                        if mouse_mode {
                            if let Some(pos) = response.hover_pos() {
                                let (row, col) = point(pos);
                                let _ = p.session.write(mouse_bytes(
                                    if scroll > 0. { 64 } else { 65 },
                                    row,
                                    col,
                                    true,
                                    view.mode,
                                ));
                            }
                        } else if view.mode.contains(TermMode::ALT_SCREEN) {
                            let key = if scroll > 0. {
                                Key::ArrowUp
                            } else {
                                Key::ArrowDown
                            };
                            if let Some(bytes) = key_bytes(key, egui::Modifiers::NONE, view.mode) {
                                for _ in 0..((scroll.abs() / cell.y).ceil() as usize).min(10) {
                                    let _ = p.session.write(bytes.clone());
                                }
                            }
                        } else {
                            p.session.scroll((scroll / cell.y).round() as i32);
                            ctx.request_repaint();
                        }
                    }
                }
                if let Some((a, b)) = p.selection {
                    let (a, b) = if a <= b { (a, b) } else { (b, a) };
                    for row in a.0..=b.0 {
                        let start = if row == a.0 { a.1 } else { 0 };
                        let end = if row == b.0 { b.1 + 1 } else { cols as usize };
                        painter.rect_filled(
                            Rect::from_min_size(
                                rect.min + Vec2::new(start as f32 * cell.x, row as f32 * cell.y),
                                Vec2::new((end - start) as f32 * cell.x, cell.y),
                            ),
                            0.,
                            Color32::from_rgb(37, 65, 79),
                        );
                    }
                }
                for c in &view.cells {
                    let pos = rect.min + Vec2::new(c.col as f32 * cell.x, c.row as f32 * cell.y);
                    let width = cell.x * if c.wide { 2. } else { 1. };
                    if let Some(bg) = c.bg {
                        painter.rect_filled(
                            Rect::from_min_size(pos, Vec2::new(width, cell.y)),
                            0.,
                            Color32::from_rgb(bg[0], bg[1], bg[2]),
                        );
                    }
                    let color = Color32::from_rgb(c.fg[0], c.fg[1], c.fg[2]);
                    if let Some(icon_font) =
                        theme::terminal_icon_font(ctx, &c.text, Vec2::new(width, cell.y))
                    {
                        let galley = painter.layout_no_wrap(c.text.clone(), icon_font, color);
                        let offset =
                            (Vec2::new(width, cell.y) - galley.size()).max(Vec2::ZERO) * 0.5;
                        painter
                            .with_clip_rect(
                                Rect::from_min_size(pos, Vec2::new(width, cell.y)).intersect(rect),
                            )
                            .galley(pos + offset, galley, color);
                    } else {
                        painter.text(pos, Align2::LEFT_TOP, &c.text, font.clone(), color);
                    }
                    if c.underline {
                        painter.line_segment(
                            [
                                pos + Vec2::new(0., cell.y - 2.),
                                pos + Vec2::new(width, cell.y - 2.),
                            ],
                            Stroke::new(1_f32, color),
                        );
                    }
                    if c.strike {
                        painter.line_segment(
                            [
                                pos + Vec2::new(0., cell.y * 0.55),
                                pos + Vec2::new(width, cell.y * 0.55),
                            ],
                            Stroke::new(1_f32, color),
                        );
                    }
                }
                if view.mode.contains(TermMode::SHOW_CURSOR)
                    && view.cursor.0 >= 0
                    && view.cursor.0 < rows as i32
                    && view.offset == 0
                    && p.session.is_running()
                {
                    let target = rect.min
                        + Vec2::new(view.cursor.1 as f32 * cell.x, view.cursor.0 as f32 * cell.y);
                    let old = p.cursor.unwrap_or(target);
                    let dt = ctx.input(|i| i.stable_dt).min(0.1);
                    let pos = if self.config.cursor_animation {
                        old.lerp(target, 1. - (-22. * dt).exp())
                    } else {
                        target
                    };
                    if pos.distance(target) > 0.3 {
                        ctx.request_repaint();
                    }
                    p.cursor = Some(pos);
                    let visible = self.last_interaction.elapsed().as_millis() % 1000 < 650;
                    if visible || pos.distance(target) > 0.3 {
                        let cursor_rect = match view.cursor_shape {
                            CursorShape::Beam => Rect::from_min_size(pos, Vec2::new(2., cell.y)),
                            CursorShape::Underline => Rect::from_min_size(
                                pos + Vec2::new(0., cell.y - 2.),
                                Vec2::new(cell.x, 2.),
                            ),
                            _ => Rect::from_min_size(pos, cell),
                        };
                        painter.rect_filled(
                            cursor_rect,
                            0.,
                            CURSOR.gamma_multiply(
                                if matches!(view.cursor_shape, CursorShape::Block) {
                                    0.55
                                } else {
                                    0.9
                                },
                            ),
                        );
                    }
                    ctx.request_repaint_after(Duration::from_millis(350));
                }
                if !modal && !ctx.memory(|memory| memory.any_popup_open()) && response.has_focus() {
                    let events = terminal_events(ctx.input(|i| i.events.clone()));
                    for event in events {
                        match event {
                            egui::Event::Copy => {
                                if let Some((a, b)) = p.selection {
                                    ctx.copy_text(p.session.select(a, b));
                                }
                            }
                            egui::Event::Paste(text) => {
                                let _ = p.session.write(paste(&text, view.mode));
                                self.last_interaction = Instant::now();
                            }
                            egui::Event::Text(text) => {
                                let _ = p.session.write(text.into_bytes());
                                self.last_interaction = Instant::now();
                            }
                            egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } => {
                                if modifiers.ctrl && modifiers.shift && key == Key::C {
                                    if let Some((a, b)) = p.selection {
                                        ctx.copy_text(p.session.select(a, b));
                                    }
                                    continue;
                                }
                                if let Some(bytes) = key_bytes(key, modifiers, view.mode) {
                                    let _ = p.session.write(bytes);
                                    self.last_interaction = Instant::now();
                                }
                            }
                            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                                let _ = p.session.write(text.into_bytes());
                            }
                            egui::Event::WindowFocused(focused)
                                if view.mode.contains(TermMode::FOCUS_IN_OUT) =>
                            {
                                let _ = p.session.write(if focused {
                                    b"\x1b[I".to_vec()
                                } else {
                                    b"\x1b[O".to_vec()
                                });
                            }
                            _ => {}
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::IMERect(Rect::from_min_size(
                        rect.min
                            + Vec2::new(
                                view.cursor.1 as f32 * cell.x,
                                view.cursor.0 as f32 * cell.y,
                            ),
                        cell,
                    )));
                }
                if p.session.is_running() {
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
            });
    }
}
impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0., 0., 0., 0.]
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.qa_rects.clear();
        // Sample targets before actions that can spend time launching or reaping
        // a shell. Their changes start animating on the next, freshly timed frame.
        let reveal = ctx.animate_value_with_time(
            egui::Id::new("multiple_terminal_tabs"),
            f32::from(self.panes.len() > 1),
            0.18,
        );
        for (index, pane) in self.panes.iter_mut().enumerate() {
            pane.visibility = ctx.animate_value_with_time(
                egui::Id::new(("pane_visible", pane.session.id)),
                f32::from(index == self.active),
                0.18,
            );
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_approved {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close(CloseTarget::Window, ctx);
        }
        self.poll();
        self.poll_followups(ctx);
        if self.options.demo
            && self.started.elapsed() > Duration::from_millis(600)
            && let Some(pane) = self.panes.first()
            && pane.session.state.lock().unwrap().at_prompt
        {
            let cmd = "printf '\\033[2J\\033[H'; pwd; printf '\\n'; ls --color=always\n";
            let _ = pane.session.write(cmd.as_bytes().to_vec());
            self.options.demo = false;
        }
        self.shortcuts(ctx);
        if !self.modal_open()
            && !ctx.memory(|memory| memory.any_popup_open())
            && let Some(pane) = self.panes.get(self.active)
        {
            // Reclaim focus before toolbar widgets can consume Enter/Space. Dialogs
            // retain their own focus and normal Tab navigation while they are open.
            focus_terminal(ctx, terminal_input_id(pane.session.id));
        }
        self.toolbar(ctx, reveal);
        self.terminal(ctx);
        self.dialogs(ctx);
        // egui keeps modal layers and hit-test data from the previous pass.
        // After dismissal, render through their removal even if the child is
        // silent; otherwise the next click can hit an invisible modal backdrop.
        if !self.modal_open() && ctx.memory(|memory| memory.top_modal_layer().is_some()) {
            ctx.request_repaint();
        }
        if let Some((message, since)) = self.append_notice.clone() {
            if since.elapsed() < Duration::from_secs(6) {
                let mut dismiss = false;
                egui::Area::new("append_notice".into())
                    .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-18., -40.))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width((ctx.screen_rect().width() - 70.).min(600.));
                            ui.horizontal_wrapped(|ui| {
                                track(&mut self.qa_rects, "append_notice", ui.label(&message));
                                dismiss = theme::icon_button(ui, Icon::Close, "Dismiss message")
                                    .clicked();
                            });
                        });
                    });
                if dismiss {
                    self.append_notice = None;
                } else {
                    ctx.request_repaint_after(
                        Duration::from_secs(6).saturating_sub(since.elapsed()),
                    );
                }
            } else {
                self.append_notice = None;
            }
        }
        if self.options.qa {
            let rects: std::collections::BTreeMap<_, _> = self
                .qa_rects
                .iter()
                .map(|(name, r)| (name.clone(), [r.min.x, r.min.y, r.max.x, r.max.y]))
                .collect();
            let value = serde_json::json!({"ui":rects,"scale":ctx.pixels_per_point(),
                "run_command":self.form.command,"run_timeout":self.form.timeout,"active_tab":self.active,
                "tabs":self.panes.iter().map(|pane| &pane.label).collect::<Vec<_>>(),
                "pane_transition":self.panes.get(self.active).map(|pane| pane.visibility)})
            .to_string();
            if value != self.last_qa {
                if let Some(log) = &mut self.event_log {
                    let _ = writeln!(log, "{value}");
                }
                self.last_qa = value;
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event
                && let Some(path) = &self.options.screenshot
            {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let _ = image::save_buffer(
                    path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                );
            }
        }
        if self.options.screenshot.is_some()
            && !self.screenshot_requested
            && !self.options.demo
            && self.started.elapsed() > Duration::from_millis(3500)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
        }
        if self.options.screenshot.is_some() && !self.screenshot_requested {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(duration) = self.options.exit_after {
            if self.started.elapsed() >= duration {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}

fn glass_text_button(ui: &mut egui::Ui, label: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.).color(INK))
            .min_size(Vec2::new(width, 30.))
            .corner_radius(8),
    )
}

fn glass_icon_button(ui: &mut egui::Ui, kind: Icon, tooltip: &str) -> egui::Response {
    let response = ui
        .add(
            egui::Button::new("")
                .min_size(Vec2::splat(30.))
                .corner_radius(8),
        )
        .on_hover_text(tooltip);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    theme::icon(
        ui,
        response.rect,
        kind,
        if response.hovered() { INK } else { MUTED },
    );
    response
}

fn terminal_input_id(session: u64) -> egui::Id {
    egui::Id::new(("terminal_input", session))
}

fn tab_title_job(label: &str, width: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        FontId::proportional(13.),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    job
}

fn command_label(command: &str) -> Option<String> {
    let label = command
        .trim()
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect::<String>();
    (!label.is_empty()).then_some(label)
}

fn focus_terminal(ctx: &egui::Context, id: egui::Id) {
    let changed = ctx.memory_mut(|memory| {
        let changed = memory.focused() != Some(id);
        if changed {
            memory.request_focus(id);
        }
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
        changed
    });
    if changed {
        // egui installs a focus filter once the widget also had focus last frame.
        ctx.request_repaint();
    }
}

fn track(
    rects: &mut std::collections::BTreeMap<String, Rect>,
    name: &str,
    response: egui::Response,
) -> egui::Response {
    rects.insert(name.into(), response.rect);
    response
}

pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    let clean = text.replace('\x1b', "");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        clean.replace(['\r', '\n'], " ").into_bytes()
    }
}
// The patched winit bridge supplies a clipboard event immediately before the
// corresponding key press. Only terminal input bypasses those clipboard events;
// modal text fields keep egui's normal copy/cut/paste behavior.
fn terminal_events(events: Vec<egui::Event>) -> Vec<egui::Event> {
    let mut events = events.into_iter().peekable();
    let mut terminal = Vec::new();
    while let Some(event) = events.next() {
        let clipboard_shortcut = match (&event, events.peek()) {
            (
                egui::Event::Copy,
                Some(egui::Event::Key {
                    key: Key::C,
                    pressed: true,
                    modifiers,
                    ..
                }),
            )
            | (
                egui::Event::Cut,
                Some(egui::Event::Key {
                    key: Key::X,
                    pressed: true,
                    modifiers,
                    ..
                }),
            ) => modifiers.ctrl,
            (
                egui::Event::Paste(_),
                Some(egui::Event::Key {
                    key: Key::V,
                    pressed: true,
                    modifiers,
                    ..
                }),
            ) => modifiers.ctrl && !modifiers.shift,
            _ => false,
        };
        if !clipboard_shortcut {
            terminal.push(event);
        }
    }
    terminal
}

pub fn key_bytes(key: Key, m: egui::Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if m.ctrl && m.shift && matches!(key, Key::C | Key::V) {
        return None;
    }
    if m.ctrl {
        let c = match key {
            Key::A => 1,
            Key::B => 2,
            Key::C => 3,
            Key::D => 4,
            Key::E => 5,
            Key::F => 6,
            Key::G => 7,
            Key::H | Key::Backspace => 8,
            Key::I => 9,
            Key::J => 10,
            Key::K => 11,
            Key::L => 12,
            Key::M => 13,
            Key::N => 14,
            Key::O => 15,
            Key::P => 16,
            Key::Q => 17,
            Key::R => 18,
            Key::S => 19,
            Key::T => 20,
            Key::U => 21,
            Key::V => 22,
            Key::W => 23,
            Key::X => 24,
            Key::Y => 25,
            Key::Z => 26,
            Key::Space | Key::Num2 | Key::Backtick => 0,
            Key::OpenBracket | Key::OpenCurlyBracket | Key::Num3 => 27,
            Key::Backslash | Key::Pipe | Key::Num4 => 28,
            Key::CloseBracket | Key::CloseCurlyBracket | Key::Num5 => 29,
            Key::Num6 => 30,
            Key::Minus | Key::Num7 => 31,
            Key::Slash if !m.shift => 31,
            Key::Slash | Key::Questionmark | Key::Num8 => 127,
            _ => 255,
        };
        if c != 255 {
            return Some(if m.alt { vec![0x1b, c] } else { vec![c] });
        }
    }
    let app = mode.contains(TermMode::APP_CURSOR);
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let arrow = match key {
        Key::ArrowUp => Some('A'),
        Key::ArrowDown => Some('B'),
        Key::ArrowRight => Some('C'),
        Key::ArrowLeft => Some('D'),
        _ => None,
    };
    if let Some(a) = arrow {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{a}")
            } else if app {
                format!("\x1bO{a}")
            } else {
                format!("\x1b[{a}")
            }
            .into_bytes(),
        );
    }
    let s = match key {
        Key::Enter => "\r",
        Key::Backspace => "\x7f",
        Key::Tab if m.shift => "\x1b[Z",
        Key::Tab => "\t",
        Key::Escape => "\x1b",
        Key::Home if app => "\x1bOH",
        Key::End if app => "\x1bOF",
        Key::Home => "\x1b[H",
        Key::End => "\x1b[F",
        Key::Insert => "\x1b[2~",
        Key::Delete => "\x1b[3~",
        Key::PageUp => "\x1b[5~",
        Key::PageDown => "\x1b[6~",
        Key::F1 => "\x1bOP",
        Key::F2 => "\x1bOQ",
        Key::F3 => "\x1bOR",
        Key::F4 => "\x1bOS",
        Key::F5 => "\x1b[15~",
        Key::F6 => "\x1b[17~",
        Key::F7 => "\x1b[18~",
        Key::F8 => "\x1b[19~",
        Key::F9 => "\x1b[20~",
        Key::F10 => "\x1b[21~",
        Key::F11 => "\x1b[23~",
        Key::F12 => "\x1b[24~",
        _ => return None,
    };
    Some(s.as_bytes().to_vec())
}

pub fn mouse_bytes(button: u8, row: usize, col: usize, pressed: bool, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::SGR_MOUSE) {
        format!(
            "\x1b[<{};{};{}{}",
            button,
            col + 1,
            row + 1,
            if pressed { 'M' } else { 'm' }
        )
        .into_bytes()
    } else {
        let code = if pressed { button } else { 3 };
        vec![
            0x1b,
            b'[',
            b'M',
            32 + code,
            33 + col.min(222) as u8,
            33 + row.min(222) as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_unicode_tab_titles_fit_without_covering_the_close_button() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            for label in [
                "WWWWWWWWWWWWWWWWWWWWWWWW",
                "日本語の長いコマンドと引数 --option",
                "grep -i hello /a/very/long/path",
            ] {
                let galley = ctx.fonts(|fonts| fonts.layout_job(tab_title_job(label, 85., INK)));
                assert_eq!(galley.rows.len(), 1);
                assert!(galley.size().x <= 85.);
            }
        });
    }

    #[test]
    fn tab_command_labels_preserve_arguments_and_clean_control_characters() {
        assert_eq!(
            command_label("  grep -i 'hello world' file.txt\n"),
            Some("grep -i 'hello world' file.txt".into())
        );
        assert_eq!(
            command_label("printf a\n\tprintf b"),
            Some("printf a  printf b".into())
        );
        assert_eq!(
            command_label("日本語 --flag\x07"),
            Some("日本語 --flag".into())
        );
        assert_eq!(
            command_label("printf 'a   b'"),
            Some("printf 'a   b'".into())
        );
        assert_eq!(command_label(" \n\t"), None);
    }

    #[test]
    fn terminal_focus_survives_navigation_and_escape_across_frames() {
        let ctx = egui::Context::default();
        let id = terminal_input_id(42);
        let frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    focus_terminal(ctx, id);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = ui.button("Another focusable control");
                        ui.interact(
                            Rect::from_min_size(Pos2::new(0., 40.), Vec2::splat(200.)),
                            id,
                            Sense::click_and_drag(),
                        );
                        focus_terminal(ctx, id);
                    });
                },
            );
            assert_eq!(ctx.memory(|memory| memory.focused()), Some(id));
        };
        frame(vec![]);
        frame(vec![]);
        for (key, modifiers) in [
            (Key::Tab, egui::Modifiers::NONE),
            (Key::Tab, egui::Modifiers::SHIFT),
            (Key::ArrowLeft, egui::Modifiers::NONE),
            (Key::ArrowRight, egui::Modifiers::NONE),
            (Key::ArrowUp, egui::Modifiers::NONE),
            (Key::ArrowDown, egui::Modifiers::NONE),
            (Key::Escape, egui::Modifiers::NONE),
        ] {
            frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }]);
        }
    }

    #[test]
    fn run_durations_use_selected_units_and_preserve_explicit_durations() {
        for (value, unit, expected) in [
            ("30", DurationUnit::Seconds, Duration::from_secs(30)),
            ("0.7", DurationUnit::Seconds, Duration::from_millis(700)),
            ("2", DurationUnit::Minutes, Duration::from_secs(120)),
            ("1.5", DurationUnit::Hours, Duration::from_secs(5400)),
            ("500ms", DurationUnit::Hours, Duration::from_millis(500)),
            ("1m 30s", DurationUnit::Seconds, Duration::from_secs(90)),
        ] {
            assert_eq!(run_duration(value, unit).unwrap(), Some(expected));
        }
        assert_eq!(run_duration(" ", DurationUnit::Minutes).unwrap(), None);
        for invalid in ["0", "0s", "-1", "not a duration"] {
            assert!(run_duration(invalid, DurationUnit::Seconds).is_err());
        }
    }

    #[test]
    fn legacy_control_keys_and_clipboard_shortcuts_stay_separate() {
        let letters = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
        ];
        for (index, key) in letters.into_iter().enumerate() {
            assert_eq!(
                key_bytes(key, egui::Modifiers::CTRL, TermMode::empty()),
                Some(vec![index as u8 + 1])
            );
        }
        for (key, shift, byte) in [
            (Key::Space, false, 0),
            (Key::Num2, true, 0),
            (Key::OpenBracket, false, 27),
            (Key::Backslash, false, 28),
            (Key::Pipe, true, 28),
            (Key::CloseBracket, false, 29),
            (Key::Num6, true, 30),
            (Key::Minus, true, 31),
            (Key::Slash, false, 31),
            (Key::Questionmark, true, 127),
            (Key::Num8, false, 127),
            (Key::Backspace, false, 8),
        ] {
            assert_eq!(
                key_bytes(
                    key,
                    egui::Modifiers {
                        ctrl: true,
                        shift,
                        ..Default::default()
                    },
                    TermMode::empty()
                ),
                Some(vec![byte]),
                "{key:?}"
            );
        }
        let event = |key, shift| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                shift,
                ..Default::default()
            },
        };
        for (clipboard, key, byte) in [
            (egui::Event::Copy, Key::C, 3),
            (egui::Event::Cut, Key::X, 24),
            (
                egui::Event::Paste("clipboard must not be typed".into()),
                Key::V,
                22,
            ),
        ] {
            let events = terminal_events(vec![clipboard, event(key, false)]);
            assert_eq!(events.len(), 1);
            assert_eq!(
                key_bytes(key, egui::Modifiers::CTRL, TermMode::empty()),
                Some(vec![byte])
            );
        }
        assert_eq!(
            terminal_events(vec![event(Key::V, false)]).len(),
            1,
            "An empty clipboard must still deliver Ctrl+V"
        );
        let shifted = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert!(key_bytes(Key::V, shifted, TermMode::empty()).is_none());
        assert!(key_bytes(Key::C, shifted, TermMode::empty()).is_none());
        let events = terminal_events(vec![
            egui::Event::Paste("pasted once".into()),
            event(Key::V, true),
        ]);
        assert!(matches!(&events[0], egui::Event::Paste(text) if text == "pasted once"));
        assert_eq!(terminal_events(vec![egui::Event::Copy]).len(), 1);
    }

    #[test]
    fn keyboard_obeys_terminal_modes_and_paste_cannot_execute_unbracketed_lines() {
        assert_eq!(
            key_bytes(Key::ArrowUp, egui::Modifiers::NONE, TermMode::APP_CURSOR).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(Key::C, egui::Modifiers::CTRL, TermMode::empty()).unwrap(),
            vec![3]
        );
        assert_eq!(paste("echo a\necho b", TermMode::empty()), b"echo a echo b");
        assert_eq!(
            paste("a\x1b[201~b", TermMode::BRACKETED_PASTE),
            b"\x1b[200~a[201~b\x1b[201~"
        );
        assert_eq!(
            mouse_bytes(0, 4, 7, true, TermMode::SGR_MOUSE),
            b"\x1b[<0;8;5M"
        );
    }
}
