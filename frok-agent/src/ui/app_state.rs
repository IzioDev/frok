use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tracing::{error, info, warn};

use frok_protocol::{BuildInfo, IngressMode, RegisteredIngress};

use super::theme::{Texture, ThemeKind};

const MAX_LOG_LINES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewMode {
    Dashboard,
    Logs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteStatus {
    Pending,
    Live,
    Error,
}

#[derive(Debug, Clone)]
pub(crate) struct RouteEntry {
    pub(crate) name: String,
    pub(crate) public_url: String,
    pub(crate) local_addr: String,
    pub(crate) mode: IngressMode,
    pub(crate) status: RouteStatus,
}

#[derive(Debug, Clone)]
pub(crate) enum AuthStatus {
    Unknown,
    Pending,
    Authenticated { subject: String },
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub(crate) struct RouteView {
    pub(crate) name: String,
    pub(crate) full_host: String,
    pub(crate) ingress: RegisteredIngress,
}

#[derive(Debug, Clone)]
pub(crate) enum RouteEvent {
    Registered(RouteView),
    RegisterFailed {
        name: String,
        full_host: String,
        reason: String,
    },
}

pub struct AppState {
    status: RwLock<String>,
    routes: RwLock<BTreeMap<String, RouteEntry>>,
    logs: Mutex<VecDeque<Cow<'static, str>>>,
    view_mode: Mutex<ViewMode>,
    public_domain: RwLock<String>,
    edge: RwLock<String>,
    agent_label: RwLock<String>,
    agent_build: RwLock<BuildInfo>,
    edge_build: RwLock<Option<BuildInfo>>,
    auth_status: RwLock<AuthStatus>,
    auth_url: RwLock<Option<String>>,
    auth_code_tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    route_events: broadcast::Sender<RouteEvent>,
}

impl AppState {
    pub fn new() -> std::sync::Arc<Self> {
        let (route_events, _) = broadcast::channel(64);
        std::sync::Arc::new(Self {
            status: RwLock::new("starting".to_string()),
            routes: RwLock::new(BTreeMap::new()),
            logs: Mutex::new(VecDeque::new()),
            view_mode: Mutex::new(ViewMode::Dashboard),
            public_domain: RwLock::new(String::new()),
            edge: RwLock::new(String::new()),
            agent_label: RwLock::new(String::new()),
            agent_build: RwLock::new(BuildInfo::default()),
            edge_build: RwLock::new(None),
            auth_status: RwLock::new(AuthStatus::Unknown),
            auth_url: RwLock::new(None),
            auth_code_tx: Mutex::new(None),
            route_events,
        })
    }

    pub async fn set_status(&self, value: impl Into<String>) {
        let mut guard = self.status.write().await;
        *guard = value.into();
    }

    pub async fn set_public_domain(&self, value: impl Into<String>) {
        let mut guard = self.public_domain.write().await;
        *guard = value.into();
    }

    pub async fn set_edge(&self, value: impl Into<String>) {
        let mut guard = self.edge.write().await;
        *guard = value.into();
    }

    pub async fn set_agent_label(&self, value: impl Into<String>) {
        let mut guard = self.agent_label.write().await;
        *guard = value.into();
    }

    pub async fn set_agent_build(&self, build: BuildInfo) {
        let mut guard = self.agent_build.write().await;
        *guard = build;
    }

    pub async fn set_edge_build(&self, build: Option<BuildInfo>) {
        let mut guard = self.edge_build.write().await;
        *guard = build;
    }

    pub async fn set_auth_status(&self, status: AuthStatus) {
        let mut guard = self.auth_status.write().await;
        *guard = status;
    }

    pub async fn set_auth_url(&self, value: Option<String>) {
        let mut guard = self.auth_url.write().await;
        *guard = value;
    }

    pub async fn set_auth_code_sender(&self, value: Option<mpsc::UnboundedSender<String>>) {
        let mut guard = self.auth_code_tx.lock().await;
        *guard = value;
    }

    pub fn send_auth_code_blocking(&self, input: String) -> bool {
        let guard = self.auth_code_tx.blocking_lock();
        if let Some(sender) = guard.as_ref() {
            sender.send(input).is_ok()
        } else {
            false
        }
    }

    pub fn subscribe_routes(&self) -> broadcast::Receiver<RouteEvent> {
        self.route_events.subscribe()
    }

    pub fn publish_route_event(&self, event: RouteEvent) {
        let _ = self.route_events.send(event);
    }

    pub async fn push_log(&self, line: impl Into<Cow<'static, str>>) {
        let mut guard = self.logs.lock().await;
        push_log_inner(&mut guard, line.into());
    }

    pub async fn log_info(&self, line: impl Into<Cow<'static, str>>) {
        let line = line.into();
        info!("{line}");
        self.push_log(line).await;
    }

    pub async fn log_warn(&self, line: impl Into<Cow<'static, str>>) {
        let line = line.into();
        warn!("{line}");
        self.push_log(line).await;
    }

    pub async fn log_error(&self, line: impl Into<Cow<'static, str>>) {
        let line = line.into();
        error!("{line}");
        self.push_log(line).await;
    }

    pub async fn clear_logs(&self) {
        let mut guard = self.logs.lock().await;
        guard.clear();
    }

    pub async fn register_route(&self, entry: RouteEntry) {
        let mut guard = self.routes.write().await;
        guard.insert(entry.name.clone(), entry);
    }

    pub async fn set_route_status(&self, name: &str, status: RouteStatus) {
        let mut guard = self.routes.write().await;
        if let Some(route) = guard.get_mut(name) {
            route.status = status;
        }
    }

    pub async fn unregister_route(&self, name: &str) {
        let mut guard = self.routes.write().await;
        guard.remove(name);
    }

    pub async fn show_logs(&self) {
        let mut guard = self.view_mode.lock().await;
        *guard = ViewMode::Logs;
    }

    pub async fn hide_logs(&self) {
        let mut guard = self.view_mode.lock().await;
        *guard = ViewMode::Dashboard;
    }

    pub async fn toggle_logs(&self) {
        let mut guard = self.view_mode.lock().await;
        *guard = match *guard {
            ViewMode::Dashboard => ViewMode::Logs,
            ViewMode::Logs => ViewMode::Dashboard,
        };
    }

    pub fn show_logs_blocking(&self) {
        let mut guard = self.view_mode.blocking_lock();
        *guard = ViewMode::Logs;
    }

    pub fn hide_logs_blocking(&self) {
        let mut guard = self.view_mode.blocking_lock();
        *guard = ViewMode::Dashboard;
    }

    pub(crate) fn snapshot(&self, ui: &UiState) -> UiSnapshot {
        let status = self.status.blocking_read().clone();
        let routes = self.routes.blocking_read().values().cloned().collect();
        let logs = self.logs.blocking_lock().clone();
        let view_mode = *self.view_mode.blocking_lock();
        let public_domain = self.public_domain.blocking_read().clone();
        let edge = self.edge.blocking_read().clone();
        let agent_label = self.agent_label.blocking_read().clone();
        let agent_build = self.agent_build.blocking_read().clone();
        let edge_build = self.edge_build.blocking_read().clone();
        let auth_status = self.auth_status.blocking_read().clone();
        let auth_url = self.auth_url.blocking_read().clone();

        UiSnapshot {
            status,
            routes,
            logs,
            view_mode,
            public_domain,
            edge,
            agent_label,
            agent_build,
            edge_build,
            auth_status,
            auth_url,
            started_at: ui.started_at,
            route_index: ui.route_index,
            overlay: ui.overlay.clone(),
            logs_state: ui.logs.clone(),
            toast: ui.toast.clone(),
        }
    }
}

fn push_log_inner(logs: &mut VecDeque<Cow<'static, str>>, line: Cow<'static, str>) {
    if logs.len() >= MAX_LOG_LINES {
        logs.pop_front();
    }
    logs.push_back(line);
}

#[derive(Debug, Clone)]
pub(crate) struct TextInput {
    pub(crate) value: String,
    pub(crate) cursor: usize,
}

impl TextInput {
    pub(crate) fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub(crate) fn set(&mut self, value: String) {
        self.value = value;
        self.cursor = self.value.chars().count();
    }

    pub(crate) fn insert_str(&mut self, value: &str) {
        insert_at_cursor(&mut self.value, &mut self.cursor, value);
    }

    pub(crate) fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.insert_str(s);
    }

    pub(crate) fn backspace(&mut self) {
        remove_before_cursor(&mut self.value, &mut self.cursor);
    }

    pub(crate) fn delete(&mut self) {
        remove_at_cursor(&mut self.value, &mut self.cursor);
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        let len = self.value.chars().count();
        self.cursor = (self.cursor + 1).min(len);
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.value.chars().count();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Toast {
    pub(crate) message: String,
    pub(crate) created_at: Instant,
    pub(crate) kind: ToastKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogsFilter {
    All,
    Events,
    Errors,
}

#[derive(Debug, Clone)]
pub(crate) struct LogsState {
    pub(crate) filter: LogsFilter,
    pub(crate) search: TextInput,
    pub(crate) search_active: bool,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
}

impl LogsState {
    pub(crate) fn new() -> Self {
        Self {
            filter: LogsFilter::All,
            search: TextInput::new(),
            search_active: false,
            selected: 0,
            scroll: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PaletteState {
    pub(crate) input: TextInput,
    pub(crate) selection: usize,
}

impl PaletteState {
    pub(crate) fn new() -> Self {
        Self {
            input: TextInput::new(),
            selection: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddRouteStep {
    Target,
    Mode,
    Name,
    Confirm,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalTarget {
    pub(crate) label: String,
    pub(crate) addr: String,
    pub(crate) open: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct AddRouteState {
    pub(crate) step: AddRouteStep,
    pub(crate) targets: Vec<LocalTarget>,
    pub(crate) target_selection: usize,
    pub(crate) target_manual: TextInput,
    pub(crate) mode_selection: usize,
    pub(crate) name_input: TextInput,
    pub(crate) error: Option<String>,
}

impl AddRouteState {
    pub(crate) fn new(targets: Vec<LocalTarget>) -> Self {
        Self {
            step: AddRouteStep::Target,
            targets,
            target_selection: 0,
            target_manual: TextInput::new(),
            mode_selection: 0,
            name_input: TextInput::new(),
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FirstRunState {
    pub(crate) auth_input: TextInput,
}

impl FirstRunState {
    pub(crate) fn new() -> Self {
        Self {
            auth_input: TextInput::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum UiOverlay {
    None,
    CommandPalette(PaletteState),
    AddRoute(AddRouteState),
    FirstRun(FirstRunState),
}

#[derive(Debug)]
pub(crate) struct UiState {
    pub(crate) started_at: Instant,
    pub(crate) route_index: usize,
    pub(crate) overlay: UiOverlay,
    pub(crate) logs: LogsState,
    pub(crate) toast: Option<Toast>,
    pub(crate) first_run_seen: bool,
    pub(crate) theme_kind: ThemeKind,
    pub(crate) texture: Texture,
    pub(crate) target_scan: Option<std::sync::mpsc::Receiver<Vec<LocalTarget>>>,
}

impl Default for UiState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            route_index: 0,
            overlay: UiOverlay::None,
            logs: LogsState::new(),
            toast: None,
            first_run_seen: false,
            theme_kind: ThemeKind::NeonSunset,
            texture: Texture::None,
            target_scan: None,
        }
    }
}

impl UiState {
    pub(crate) fn set_toast(&mut self, message: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast {
            message: message.into(),
            created_at: Instant::now(),
            kind,
        });
    }

    pub(crate) fn clamp_route_index(&mut self, len: usize) {
        if len == 0 {
            self.route_index = 0;
        } else if self.route_index >= len {
            self.route_index = len - 1;
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UiSnapshot {
    pub(crate) status: String,
    pub(crate) routes: Vec<RouteEntry>,
    pub(crate) logs: VecDeque<Cow<'static, str>>,
    pub(crate) view_mode: ViewMode,
    pub(crate) public_domain: String,
    pub(crate) edge: String,
    pub(crate) agent_label: String,
    pub(crate) agent_build: BuildInfo,
    pub(crate) edge_build: Option<BuildInfo>,
    pub(crate) auth_status: AuthStatus,
    pub(crate) auth_url: Option<String>,
    pub(crate) started_at: Instant,
    pub(crate) route_index: usize,
    pub(crate) overlay: UiOverlay,
    pub(crate) logs_state: LogsState,
    pub(crate) toast: Option<Toast>,
}

impl UiSnapshot {
    pub(crate) fn is_logs(&self) -> bool {
        self.view_mode == ViewMode::Logs
    }

    pub(crate) fn selected_route(&self) -> Option<RouteEntry> {
        self.routes.get(self.route_index).cloned()
    }
}

fn char_to_byte_idx(input: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    input
        .char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| input.len())
}

fn insert_at_cursor(input: &mut String, cursor: &mut usize, value: &str) {
    let idx = char_to_byte_idx(input, *cursor);
    input.insert_str(idx, value);
    *cursor += value.chars().count();
}

fn remove_before_cursor(input: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    *cursor -= 1;
    remove_at_cursor(input, cursor);
}

fn remove_at_cursor(input: &mut String, cursor: &mut usize) {
    let start = char_to_byte_idx(input, *cursor);
    let end = char_to_byte_idx(input, *cursor + 1);
    if start < end {
        input.replace_range(start..end, "");
    }
}
