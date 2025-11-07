use std::{cmp, collections::HashMap, env, fs, io, path::PathBuf, thread, time::Duration};

use anyhow::{anyhow, Context, Result};
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use serde_json::{from_value, to_value, Value};
use tokio::{spawn, sync::mpsc};
use tracing::{debug, error, info};
use tui18_core::{
    manifest::ResourceMetadata,
    models::GameInfo,
    resource::{ResourceLoader, SyncEvent},
    save::{SaveEntry, SaveManager},
    session::{
        Corporation, CorporationTrain, GameSession, MarketCell, MarketPosition, SessionLoader,
        TrainType,
    },
};

use crate::block_font;

const TICK_RATE: Duration = Duration::from_millis(250);
const MAX_SAVE_NAME_LEN: usize = 64;

#[derive(Debug, Clone)]
struct Theme {
    primary_bg: Color,
    primary_fg: Color,
    accent: Color,
    accent_alt: Color,
    muted: Color,
    selection_bg: Color,
    selection_fg: Color,
    success: Color,
    warning: Color,
    danger: Color,
    on_accent: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary_bg: Color::Black,
            primary_fg: Color::White,
            accent: Color::Cyan,
            accent_alt: Color::Blue,
            muted: Color::DarkGray,
            selection_bg: Color::DarkGray,
            selection_fg: Color::White,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            on_accent: Color::Black,
        }
    }
}

fn load_theme() -> (Theme, String) {
    let mut theme = Theme::default();
    let candidates = omarchy_theme_candidates();
    let path = match candidates.into_iter().find(|candidate| candidate.exists()) {
        Some(path) => path,
        None => {
            return (
                theme,
                "Omarchy theme not found; using default palette.".to_string(),
            )
        }
    };

    let data = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            return (
                theme,
                format!(
                    "Failed to read {} ({err}); using default palette.",
                    path.display()
                ),
            )
        }
    };

    let json: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(err) => {
            return (
                theme,
                format!(
                    "Failed to parse {} ({err}); using default palette.",
                    path.display()
                ),
            )
        }
    };

    let mut applied: Vec<&str> = Vec::new();

    if let Some(color) = color_at_path(&json, &["colors", "primary", "background"])
        .or_else(|| color_at_path(&json, &["apps", "alacritty", "colors", "primary", "background"]))
    {
        theme.primary_bg = color;
        applied.push("primary.background");
    }

    if let Some(color) = color_at_path(&json, &["colors", "primary", "foreground"])
        .or_else(|| color_at_path(&json, &["apps", "alacritty", "colors", "primary", "foreground"]))
    {
        theme.primary_fg = color;
        applied.push("primary.foreground");
    }

    if let Some(color) =
        color_at_path(&json, &["colors", "primary", "dim_foreground"])
            .or_else(|| {
                color_at_path(
                    &json,
                    &["apps", "alacritty", "colors", "primary", "dim_foreground"],
                )
            })
    {
        theme.muted = color;
        applied.push("primary.dim_foreground");
    }

    if let Some(color) = color_at_path(&json, &["colors", "terminal", "cyan"]) {
        theme.accent = color;
        applied.push("terminal.cyan");
    }

    if let Some(color) = color_at_path(&json, &["colors", "terminal", "blue"])
        .or_else(|| color_at_path(&json, &["colors", "terminal", "magenta"]))
    {
        theme.accent_alt = color;
        applied.push("terminal.blue");
    }

    if let Some(color) = color_at_path(&json, &["colors", "terminal", "green"]) {
        theme.success = color;
        applied.push("terminal.green");
    }

    if let Some(color) = color_at_path(&json, &["colors", "terminal", "yellow"]) {
        theme.warning = color;
        applied.push("terminal.yellow");
    }

    if let Some(color) = color_at_path(&json, &["colors", "terminal", "red"]) {
        theme.danger = color;
        applied.push("terminal.red");
    }

    if let Some(color) = color_at_path(
        &json,
        &["apps", "alacritty", "colors", "selection", "background"],
    ) {
        theme.selection_bg = color;
        applied.push("selection.background");
    }

    if let Some(color) = color_at_path(
        &json,
        &["apps", "alacritty", "colors", "selection", "foreground"],
    ) {
        theme.selection_fg = color;
        applied.push("selection.foreground");
    }

    theme.on_accent = contrast_color(&theme.accent, Color::Black);
    if applied.iter().all(|entry| *entry != "selection.foreground") {
        theme.selection_fg = contrast_color(&theme.selection_bg, theme.selection_fg);
    }

    let summary = if applied.is_empty() {
        format!(
            "Loaded Omarchy theme from {} but no recognized color keys were applied.",
            path.display()
        )
    } else {
        format!(
            "Loaded Omarchy theme from {} (applied {}).",
            path.display(),
            applied.join(", ")
        )
    };

    (theme, summary)
}

fn omarchy_theme_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        let base = PathBuf::from(&xdg).join("omarchy");
        candidates.push(base.join("theme.json"));
        candidates.push(base.join("current").join("theme.json"));
        candidates.push(base.join("current").join("theme").join("custom_theme.json"));
    }
    if let Ok(home) = env::var("HOME") {
        let base = PathBuf::from(home).join(".config").join("omarchy");
        candidates.push(base.join("theme.json"));
        candidates.push(base.join("current").join("theme.json"));
        candidates.push(base.join("current").join("theme").join("custom_theme.json"));
    }
    if let Some(dir) = dirs::config_dir() {
        candidates.push(dir.join("omarchy").join("theme.json"));
        candidates.push(dir.join("omarchy").join("current").join("theme.json"));
        candidates.push(
            dir.join("omarchy")
                .join("current")
                .join("theme")
                .join("custom_theme.json"),
        );
    }
    candidates.push(PathBuf::from("/etc/xdg/omarchy/theme.json"));
    candidates.push(PathBuf::from("/etc/xdg/omarchy/current/theme.json"));
    candidates.push(PathBuf::from("/etc/xdg/omarchy/current/theme/custom_theme.json"));
    candidates
}

fn color_at_path(value: &Value, path: &[&str]) -> Option<Color> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    value_to_color(current)
}

fn value_to_color(value: &Value) -> Option<Color> {
    match value {
        Value::String(text) => parse_hex_color(text),
        Value::Array(items) if items.len() >= 3 => {
            let mut rgb = [0u8; 3];
            for (idx, component) in items.iter().take(3).enumerate() {
                if let Some(val) = component.as_u64() {
                    if val <= 255 {
                        rgb[idx] = val as u8;
                    }
                }
            }
            Some(Color::Rgb(rgb[0], rgb[1], rgb[2]))
        }
        _ => None,
    }
}

fn parse_hex_color(input: &str) -> Option<Color> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn contrast_color(color: &Color, fallback: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => {
            let luminance =
                0.299 * f64::from(*r) + 0.587 * f64::from(*g) + 0.114 * f64::from(*b);
            if luminance > 186.0 {
                Color::Black
            } else {
                Color::White
            }
        }
        _ => fallback,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Filter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Screen {
    Menu,
    Browse,
    Continue,
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PlayMode {
    Idle,
    ParSelect,
    PriceSelect,
    TrainManage,
    TrainRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TrainFocus {
    Owned,
    Pool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrainPurchaseModal {
    cursor: usize,
    offset: usize,
}

#[derive(Debug, Clone)]
struct NamePromptModal {
    input: String,
    cursor: usize,
    game: GameInfo,
    default: String,
}

impl NamePromptModal {
    fn new(game: GameInfo, default: String) -> Self {
        let cursor = default.len();
        Self {
            input: default.clone(),
            cursor,
            game,
            default,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.input.len() as isize;
        let mut next = self.cursor as isize + delta;
        if next < 0 {
            next = 0;
        } else if next > len {
            next = len;
        }
        self.cursor = next as usize;
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.input.len();
    }

    fn insert(&mut self, ch: char) {
        if self.input.len() >= MAX_SAVE_NAME_LEN {
            return;
        }
        if ch.is_ascii() && !ch.is_ascii_control() {
            self.input.insert(self.cursor, ch);
            self.cursor += ch.len_utf8();
        }
    }

    fn backspace(&mut self) {
        if self.cursor > 0 && self.cursor <= self.input.len() {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    fn value(&self) -> String {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            self.default.clone()
        } else {
            trimmed.to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RevenueAction {
    Dividend,
    Withhold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RevenueOutcome {
    corp_sym: String,
    total: i32,
    price_label: String,
    moved: bool,
    action: RevenueAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RevenueError {
    NoCorporation,
    NoMarketPosition,
}

impl std::fmt::Display for RevenueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevenueError::NoCorporation => write!(f, "No corporation selected"),
            RevenueError::NoMarketPosition => {
                write!(f, "Set par price before adjusting stock price")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrainRunState {
    train_index: usize,
    values: Vec<i32>,
    cursor: usize,
    input: String,
    train_name: String,
}

impl TrainRunState {
    fn new(train_index: usize, train_name: String, initial: Vec<i32>) -> Self {
        let base_values = if initial.is_empty() { vec![0] } else { initial };
        Self {
            train_index,
            values: base_values,
            cursor: 0,
            input: String::new(),
            train_name,
        }
    }

    fn current_value(&self) -> i32 {
        self.values.get(self.cursor).copied().unwrap_or_default()
    }

    fn set_current_value(&mut self, value: i32) {
        if let Some(slot) = self.values.get_mut(self.cursor) {
            *slot = value;
        }
    }

    fn append_digit(&mut self, ch: char) {
        if ch.is_ascii_digit() {
            self.input.push(ch);
        }
    }

    fn backspace(&mut self) {
        self.input.pop();
    }

    fn has_pending_input(&self) -> bool {
        !self.input.is_empty()
    }

    fn pending_input(&self) -> &str {
        &self.input
    }

    fn commit_input(&mut self) {
        if self.input.is_empty() {
            return;
        }
        let value = self
            .input
            .parse::<i32>()
            .unwrap_or_else(|_| self.current_value());
        self.set_current_value(value);
        self.input.clear();
    }

    fn move_cursor(&mut self, delta: isize) {
        self.commit_input();
        if self.values.is_empty() {
            self.cursor = 0;
            return;
        }
        let len = self.values.len() as isize;
        let mut idx = self.cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.cursor = idx as usize;
    }

    fn add_stop(&mut self) {
        self.commit_input();
        self.values.push(0);
        self.cursor = self.values.len() - 1;
    }

    fn remove_stop(&mut self) {
        self.commit_input();
        if self.values.len() > 1 {
            self.values.remove(self.cursor);
            if self.cursor >= self.values.len() {
                self.cursor = self.values.len().saturating_sub(1);
            }
        } else if let Some(single) = self.values.first_mut() {
            *single = 0;
        }
    }

    fn clear_current(&mut self) {
        self.input.clear();
        self.set_current_value(0);
    }

    fn total(&self) -> i32 {
        self.values.iter().copied().sum()
    }
}

impl PhaseInfo {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::String(name) => PhaseInfo {
                name: name.clone(),
                operating_rounds: 2,
                raw: value.clone(),
            },
            Value::Object(map) => {
                let name = map
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let operating_rounds = map
                    .get("operating_rounds")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(2);
                PhaseInfo {
                    name,
                    operating_rounds: operating_rounds.max(1),
                    raw: value.clone(),
                }
            }
            _ => PhaseInfo {
                name: "?".to_string(),
                operating_rounds: 2,
                raw: value.clone(),
            },
        }
    }
}

impl OperatingRound {
    fn new(corporations: usize) -> Self {
        OperatingRound {
            revenues: vec![0; corporations],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PhaseInfo {
    name: String,
    operating_rounds: usize,
    raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperatingRound {
    revenues: Vec<i32>,
}

enum AppEvent {
    Input(Event),
    Tick,
    SessionLoaded(Result<GameSession>),
}

/// High-level application state for the Rust TUI.
pub struct Tui18App {
    loader: ResourceLoader,
    metadata: ResourceMetadata,
    state: UiState,
    save_manager: SaveManager,
    saves: Vec<SaveEntry>,
    session_loader: SessionLoader,
    screen: Screen,
    play_state: Option<PlayState>,
    pending_session: bool,
    event_tx: Option<mpsc::Sender<AppEvent>>,
    sync_rx: Option<mpsc::Receiver<SyncEvent>>,
    name_prompt: Option<NamePromptModal>,
    pending_game: Option<GameInfo>,
    pending_save_name: Option<String>,
    pending_save_state: Option<Value>,
    active_save: Option<SaveEntry>,
    theme: Theme,
    theme_status: Option<String>,
}

impl Tui18App {
    pub fn new(
        loader: ResourceLoader,
        metadata: ResourceMetadata,
        session_loader: SessionLoader,
    ) -> Self {
        let (theme, theme_status) = load_theme();
        Self {
            loader,
            metadata,
            state: UiState::default(),
            save_manager: SaveManager::new(SaveManager::default_root()),
            saves: Vec::new(),
            session_loader,
            screen: Screen::Menu,
            play_state: None,
            pending_session: false,
            event_tx: None,
            sync_rx: None,
            name_prompt: None,
            pending_game: None,
            pending_save_name: None,
            pending_save_state: None,
            active_save: None,
            theme,
            theme_status: Some(theme_status),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        self.reload_games()?;
        let mut status = format!("Loaded {} games", self.state.filtered.len());
        if let Some(note) = self.theme_status.as_ref() {
            status.push_str(" • ");
            status.push_str(note);
        }
        self.state.set_status(status);
        if let Err(err) = self.refresh_saves() {
            self.state
                .set_status(format!("Failed to load saves: {err}"));
        } else if let Some(entry) = self.saves.first() {
            if self.state.select_game(&entry.game_id) {
                self.state
                    .set_status(format!("Restored saved selection: {}", entry.name));
            }
        }

        let mut stdout = io::stdout();
        enable_raw_mode().context("failed to enter raw mode")?;
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.hide_cursor()?;
        terminal.clear()?;

        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(128);
        spawn_input_thread(event_tx.clone());
        self.event_tx = Some(event_tx.clone());

        let mut sync_rx = self.sync_rx.take();

        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if self.state.should_quit {
                break;
            }

            if sync_rx.is_some() {
                let mut sync_closed = false;
                let rx = sync_rx.as_mut().unwrap();
                tokio::select! {
                    maybe_event = event_rx.recv() => {
                        if !self.process_app_event(maybe_event) {
                            break;
                        }
                    }
                    maybe_sync = rx.recv() => {
                        match maybe_sync {
                            Some(event) => self.handle_sync_event(event),
                            None => sync_closed = true,
                        }
                    }
                }
                if sync_closed {
                    sync_rx = None;
                }
            } else {
                let maybe_event = event_rx.recv().await;
                if !self.process_app_event(maybe_event) {
                    break;
                }
            }

            if self.state.should_quit {
                break;
            }
        }

        restore_terminal(&mut terminal)?;
        self.event_tx = None;
        Ok(())
    }

    pub fn attach_sync(&mut self, receiver: mpsc::Receiver<SyncEvent>) {
        self.sync_rx = Some(receiver);
    }

    fn reload_games(&mut self) -> Result<()> {
        let games = self.loader.games()?;
        self.state.set_games(games);
        self.state.apply_filter();
        info!(total = self.state.all_games.len(), "Games reloaded");
        Ok(())
    }

    fn handle_tick(&mut self) {
        if self.state.mode == Mode::Filter {
            self.state
                .set_status(format!("Filter: {}", self.state.filter));
        }
    }

    fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::Success { path, metadata } => {
                info!(path = %path.display(), commit = metadata.commit.as_deref().unwrap_or("unknown"), "Sync succeeded");
                self.loader.refresh(path.clone(), metadata.clone());
                self.session_loader.with_root(path);
                if let Err(err) = self.reload_games() {
                    error!(?err, "Reload after sync failed");
                    self.state.set_status(format!("Reload failed: {err}"));
                } else {
                    self.state.set_status("Resources refreshed".to_string());
                }
                self.metadata = metadata;
            }
            SyncEvent::Error(err) => {
                error!(?err, "Background sync failed");
                self.state.set_status(format!("Sync failed: {err}"));
            }
        }
    }

    fn process_app_event(&mut self, maybe_event: Option<AppEvent>) -> bool {
        match maybe_event {
            Some(AppEvent::Input(event)) => {
                if self.name_prompt.is_some() {
                    if let Event::Key(key) = event {
                        if let Err(err) = self.handle_name_prompt_key(key) {
                            self.state.set_status(format!("Error: {err}"));
                        }
                    }
                } else if let Err(err) = self.handle_input(event) {
                    self.state.set_status(format!("Error: {err}"));
                }
                true
            }
            Some(AppEvent::Tick) => {
                self.handle_tick();
                true
            }
            Some(AppEvent::SessionLoaded(result)) => {
                self.pending_session = false;
                match result {
                    Ok(session) => {
                        info!(game_id = %session.info.id, title = %session.info.title, "Session loaded");
                        let saved_state = self.pending_save_state.take();
                        let base_session = session;
                        let play_state = if let Some(raw) = saved_state {
                            match from_value::<PlayState>(raw) {
                                Ok(mut state) => {
                                    state.session.info = base_session.info.clone();
                                    state.session.loaded_at = base_session.loaded_at;
                                    state
                                }
                                Err(err) => {
                                    error!(
                                        ?err,
                                        "Failed to restore saved play state; using fresh session"
                                    );
                                    PlayState::new(base_session)
                                }
                            }
                        } else {
                            PlayState::new(base_session)
                        };
                        let save_result = self.initialize_new_session_save(&play_state);
                        self.screen = Screen::Play;
                        self.play_state = Some(play_state);
                        match save_result {
                            Ok(Some(message)) => self.state.set_status(message),
                            Ok(None) => self.state.set_status("Session loaded".to_string()),
                            Err(err) => {
                                error!(?err, "Failed to prepare save for new session");
                                self.state
                                    .set_status(format!("Session started but save failed: {err}"));
                            }
                        }
                    }
                    Err(err) => {
                        error!(?err, "Session load failed");
                        self.screen = Screen::Browse;
                        self.state
                            .set_status(format!("Failed to load session: {err}"));
                    }
                }
                true
            }
            None => false,
        }
    }

    fn refresh_saves(&mut self) -> Result<()> {
        self.saves = self.save_manager.entries()?;
        Ok(())
    }

    fn load_save_entry(&mut self, entry: SaveEntry) -> Result<()> {
        let game_id = entry.game_id.clone();
        if !self.state.select_game(&game_id) {
            return Err(anyhow!("Saved game {} not available", game_id));
        }
        let payload = self.save_manager.load(&entry)?;
        self.pending_save_state = Some(payload.into_state());
        self.active_save = Some(entry);
        self.screen = Screen::Browse;
        self.start_session_load();
        Ok(())
    }

    fn initialize_new_session_save(&mut self, state: &PlayState) -> Result<Option<String>> {
        let Some(name) = self.pending_save_name.take() else {
            self.pending_game = None;
            self.pending_save_state = None;
            return Ok(None);
        };
        let game = self
            .pending_game
            .take()
            .unwrap_or_else(|| state.session.info.clone());
        let payload = to_value(state).context("serialize play state for save creation")?;
        let entry = self
            .save_manager
            .create_save(&game, Some(&name), payload)
            .with_context(|| format!("create save entry for {}", game.id))?;
        info!(game_id = %game.id, save_name = %entry.name, "New game save created");
        self.active_save = Some(entry.clone());
        self.refresh_saves()
            .context("refresh saves after creating new game")?;
        self.state.select_game(&game.id);
        self.pending_save_state = None;
        Ok(Some(format!(
            "Started {} as {}",
            state.session.info.title, entry.name
        )))
    }

    fn apply_history_step(&mut self, delta: isize) -> Result<()> {
        let Some(active) = self.active_save.clone() else {
            self.state
                .set_status("History unavailable: no save loaded".to_string());
            return Ok(());
        };
        let payload = self
            .save_manager
            .load(&active)
            .context("load save payload for history navigation")?;
        let total = payload.history_len();
        if total <= 1 {
            self.state
                .set_status("History unavailable for this save".to_string());
            return Ok(());
        }
        let current = payload.history_index() as isize;
        let target = current + delta;
        if target < 0 || target >= total as isize {
            let message = if delta < 0 {
                "Already at oldest history entry"
            } else {
                "Already at newest history entry"
            };
            self.state.set_status(message.to_string());
            return Ok(());
        }
        let (updated_entry, updated_payload) = self
            .save_manager
            .set_history_index(&active, target as usize)
            .context("update save history index")?;
        let state_value = updated_payload.state().clone();
        if state_value.is_null() {
            self.state
                .set_status("History entry has no recorded session state".to_string());
            return Ok(());
        }
        let play_state: PlayState =
            from_value(state_value).context("deserialize play state from history entry")?;
        self.active_save = Some(updated_entry.clone());
        if let Some(entry) = self
            .saves
            .iter_mut()
            .find(|entry| entry.path == updated_entry.path)
        {
            *entry = updated_entry;
            self.saves.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        }
        self.play_state = Some(play_state);
        let position = updated_payload.history_index() + 1;
        let message = if delta < 0 {
            format!("Undo applied ({position}/{total})")
        } else {
            format!("Redo applied ({position}/{total})")
        };
        self.state.set_status(message);
        Ok(())
    }

    fn persist_active_session(&mut self, state: &PlayState) -> Result<()> {
        let Some(active) = self.active_save.clone() else {
            return Ok(());
        };
        let payload = to_value(state).context("serialize play state for auto-save")?;
        let updated = self
            .save_manager
            .update_save(&active, payload)
            .context("failed to update save file")?;
        self.active_save = Some(updated.clone());
        if let Some(entry) = self
            .saves
            .iter_mut()
            .find(|entry| entry.path == updated.path)
        {
            *entry = updated;
            self.saves.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        }
        Ok(())
    }

    fn start_session_load(&mut self) {
        if self.pending_session {
            return;
        }
        let Some(game) = self.state.current_game().cloned() else {
            self.state.set_status("No game selected".to_string());
            return;
        };
        self.pending_game = Some(game.clone());
        let Some(sender) = self.event_tx.clone() else {
            self.state
                .set_status("Internal error: event channel unavailable".to_string());
            error!("event_channel_missing");
            return;
        };

        let display_name = game.display_name();
        let game_for_load = game.clone();
        self.pending_session = true;
        info!(game_id = %game.id, title = %display_name, "Loading session");
        self.state.set_status(format!("Loading {}…", display_name));
        let loader = self.session_loader.clone();
        spawn(async move {
            let result = loader.load(&game_for_load).await;
            let _ = sender.send(AppEvent::SessionLoaded(result)).await;
        });
    }

    fn begin_train_mode(&mut self, state: &mut PlayState) {
        let Some(corp_sym) = state.current_corporation().map(|corp| corp.sym.clone()) else {
            self.state.set_status("No corporation selected".to_string());
            return;
        };

        if state.enter_train_manage() {
            info!(sym = %corp_sym, "Entering train management");
            self.state
                .set_status(format!("Manage trains for {}", corp_sym));
        } else {
            self.state
                .set_status("No trains available to manage".to_string());
        }
    }

    fn begin_par_selection(&mut self, state: &mut PlayState) {
        let corp_sym = state.current_corporation().map(|corp| corp.sym.clone());
        debug!(?corp_sym, cursor = ?state.market_cursor(), "begin_par_selection invoked");
        if !state.enter_par_select() {
            debug!(?corp_sym, "enter_par_select returned false");
            self.state
                .set_status("No par spaces available for this market".to_string());
            state.exit_market();
            return;
        }
        if let Some(corp) = state.current_corporation() {
            debug!(
                sym = %corp.sym,
                cursor = ?state.market_cursor(),
                "Par selection mode activated"
            );
            info!(sym = %corp.sym, "Entering par selection");
            self.state
                .set_status(format!("Select par price for {}", corp.sym));
        } else {
            self.state.set_status("No corporation selected".to_string());
        }
    }

    fn begin_price_selection(&mut self, state: &mut PlayState) {
        state.enter_price_select();
        if let Some(corp) = state.current_corporation() {
            info!(sym = %corp.sym, "Entering stock price selection");
            self.state
                .set_status(format!("Select stock price for {}", corp.sym));
        } else {
            self.state.set_status("No corporation selected".to_string());
        }
    }

    fn apply_par_selection(&mut self, state: &mut PlayState) {
        let cursor = state.market_cursor();
        debug!(?cursor, "apply_par_selection triggered");
        if let Some(value) = state.apply_par_selection() {
            if let Some(corp) = state.current_corporation() {
                info!(sym = %corp.sym, value, "Par price updated");
                self.state
                    .set_status(format!("Par for {} set to ${}", corp.sym, value));
            }
        } else {
            debug!(?cursor, "apply_par_selection failed");
            self.state
                .set_status("Unable to set par price at current cell".to_string());
        }
    }

    fn apply_price_selection(&mut self, state: &mut PlayState) {
        if let Some(position) = state.apply_price_selection() {
            if let Some(corp) = state.current_corporation() {
                let price_display = sanitize_market_text(&position.raw);
                let price_display = if price_display.is_empty() {
                    position.raw.clone()
                } else {
                    price_display
                };
                info!(sym = %corp.sym, price = %price_display, "Stock price updated");
                self.state.set_status(format!(
                    "Stock price for {} set to {}",
                    corp.sym, price_display
                ));
            }
        } else {
            self.state
                .set_status("Unable to set stock price at current cell".to_string());
        }
    }

    fn apply_train_purchase(&mut self, state: &mut PlayState, selection: usize) {
        if state.current_corporation().is_none() {
            self.state.set_status("No corporation selected".to_string());
            return;
        }

        let Some(train) = state.purchase_available_train(selection) else {
            self.state
                .set_status("No train available for purchase".to_string());
            state.sync_pool_cursor();
            return;
        };

        let train_name = train.name.clone();
        let price = train.price.unwrap_or(0);

        let (corp_sym, new_owned_index) = {
            let Some(corp) = state.current_corporation_mut() else {
                // couldn't get mutable ref, restore train back to pool
                // since purchase_available_train already decremented, add back
                if let Some(entry) = state
                    .session
                    .train_pool
                    .iter_mut()
                    .find(|entry| entry.name == train_name)
                {
                    entry.remaining += 1;
                }
                self.state
                    .set_status("Unable to allocate train to corporation".to_string());
                return;
            };

            corp.trains.push(train);
            let idx = corp.trains.len().saturating_sub(1);
            PlayState::update_corporation_revenue(corp);
            let sym = corp.sym.clone();
            (sym, idx)
        };

        info!(sym = %corp_sym, train = %train_name, price, "Train purchased");
        self.state.set_status(format!(
            "{} buys {} train for ${}",
            corp_sym, train_name, price
        ));

        state.focus_owned();
        state.set_owned_cursor(new_owned_index);
        state.sync_pool_cursor();
    }

    fn handle_input(&mut self, event: Event) -> Result<()> {
        if let Event::Key(ref key) = event {
            if self.handle_global_shortcut(key)? {
                return Ok(());
            }
        }
        match self.screen {
            Screen::Menu => self.handle_menu_event(event)?,
            Screen::Browse => match event {
                Event::Key(key) => self.handle_key(key)?,
                Event::Resize(_, _) => {}
                Event::Mouse(_) => {}
                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            },
            Screen::Continue => self.handle_continue_event(event)?,
            Screen::Play => self.handle_play_event(event)?,
        }
        Ok(())
    }

    fn handle_global_shortcut(&mut self, key: &KeyEvent) -> Result<bool> {
        if key.modifiers.is_empty() {
            if let KeyCode::Char('u') = key.code {
                self.apply_history_step(-1)?;
                return Ok(true);
            }
        }
        if key.modifiers == KeyModifiers::CONTROL {
            if let KeyCode::Char('r') = key.code {
                self.apply_history_step(1)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_menu_event(&mut self, event: Event) -> Result<()> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.state.should_quit = true;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.state.move_menu_cursor(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.state.move_menu_cursor(-1);
                }
                KeyCode::Enter => match self.state.menu_cursor {
                    0 => {
                        self.screen = Screen::Browse;
                        self.state.mode = Mode::Browse;
                        self.state.set_status("Select a game to start".to_string());
                    }
                    1 => match self.refresh_saves() {
                        Ok(_) => {
                            self.screen = Screen::Continue;
                            self.state.move_continue_cursor(
                                0,
                                self.saves.len(),
                                self.state.list_height.max(1),
                            );
                            if self.saves.is_empty() {
                                self.state.set_status("No saves available".to_string());
                            } else {
                                self.state
                                    .set_status("Select a save to continue".to_string());
                            }
                        }
                        Err(err) => {
                            self.state
                                .set_status(format!("Failed to load saves: {err}"));
                        }
                    },
                    2 => {
                        self.state.should_quit = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_continue_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) => {
                let total = self.saves.len();
                let visible = self.state.list_height.max(1);
                match key.code {
                    KeyCode::Esc => {
                        self.screen = Screen::Menu;
                        self.state.set_status("Returned to main menu".to_string());
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.state.move_continue_cursor(1, total, visible);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.state.move_continue_cursor(-1, total, visible);
                    }
                    KeyCode::Enter => {
                        if total == 0 {
                            self.state.set_status("No saves available".to_string());
                        } else if let Some(entry) =
                            self.saves.get(self.state.continue_cursor).cloned()
                        {
                            if let Err(err) = self.load_save_entry(entry) {
                                self.state.set_status(format!("Failed to load save: {err}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Resize(_, _) => {}
            Event::Mouse(_) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
        }
        Ok(())
    }

    fn handle_train_run_key(&mut self, state: &mut PlayState, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('t') | KeyCode::Char('T') => {
                state.cancel_train_run();
                self.state.set_status("Train run cancelled".to_string());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') if key.modifiers.is_empty() => {
                self.state.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                state.train_run_move_cursor(1);
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                state.train_run_move_cursor(-1);
            }
            KeyCode::Char('+') => {
                if state.train_run_add_stop() {
                    if let Some(run) = state.train_run_state() {
                        self.state
                            .set_status(format!("Added stop; {} total stops", run.values.len()));
                    }
                } else {
                    self.state
                        .set_status("Stop limit reached for this train".to_string());
                }
            }
            KeyCode::Char('=') => {
                state.cancel_train_run();
                self.state.set_status("Run editing cancelled".to_string());
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                state.train_run_remove_stop();
                if let Some(run) = state.train_run_state() {
                    self.state
                        .set_status(format!("Removed stop; {} total stops", run.values.len()));
                }
            }
            KeyCode::Backspace => {
                state.train_run_backspace();
            }
            KeyCode::Char(' ') => {
                state.train_run_commit_input();
                state.train_run_move_cursor(1);
            }
            KeyCode::Enter => {
                state.train_run_commit_input();
                if let Some((corp_sym, train_name, total)) = state.apply_train_run() {
                    let summary = state.operating_round_summary();
                    self.state.set_status(format!(
                        "Run saved for {} {}: ${} ({summary})",
                        corp_sym, train_name, total
                    ));
                } else {
                    self.state
                        .set_status("Unable to save train run".to_string());
                }
            }
            KeyCode::Char(ch) if key.modifiers.is_empty() && ch.is_ascii_digit() => {
                state.train_run_append_digit(ch);
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                state.train_run_clear_current();
            }
            KeyCode::Tab => {
                state.train_run_move_cursor(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn prompt_new_game(&mut self) {
        if self.pending_session {
            self.state
                .set_status("A session is already loading".to_string());
            return;
        }
        if self.name_prompt.is_some() {
            return;
        }
        let Some(game) = self.state.current_game().cloned() else {
            self.state.set_status("No game selected".to_string());
            return;
        };
        let default_name = Self::default_save_name(&game);
        self.pending_game = Some(game.clone());
        self.pending_save_name = None;
        self.active_save = None;
        self.name_prompt = Some(NamePromptModal::new(game.clone(), default_name.clone()));
        self.state
            .set_status(format!("Enter save name for {}", game.title));
    }

    fn default_save_name(game: &GameInfo) -> String {
        let base = if game.id.trim().is_empty() {
            game.title.trim()
        } else {
            game.id.trim()
        };
        let date = Local::now().format("%Y-%m-%d");
        format!("{} {}", base, date)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.state.mode {
            Mode::Filter => self.handle_filter_key(key),
            Mode::Browse => self.handle_browse_key(key),
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.state.mode = Mode::Browse;
                self.state.set_status("Filter cancelled".to_string());
            }
            KeyCode::Enter => {
                self.state.mode = Mode::Browse;
                self.state
                    .set_status(format!("Filter applied: {}", self.state.filter));
            }
            KeyCode::Backspace => {
                self.state.filter.pop();
                self.state.apply_filter();
                self.state
                    .set_status(format!("Filter: {}", self.state.filter));
            }
            KeyCode::Char(c) => {
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                    self.state.filter.push(c);
                    self.state.apply_filter();
                    self.state
                        .set_status(format!("Filter: {}", self.state.filter));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') if key.modifiers.is_empty() => self.state.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.state.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.state.move_cursor(-1),
            KeyCode::Char('g') if key.modifiers.is_empty() => self.state.move_to(0),
            KeyCode::Char('G') if key.modifiers.is_empty() => self.state.move_to_end(),
            KeyCode::Home => self.state.move_to(0),
            KeyCode::End => self.state.move_to_end(),
            KeyCode::PageDown => self.state.page_down(),
            KeyCode::PageUp => self.state.page_up(),
            KeyCode::Char('/') => {
                self.state.mode = Mode::Filter;
                self.state.set_status("Enter filter text".to_string());
            }
            KeyCode::Char('b') if key.modifiers.is_empty() => {
                self.state.show_banner = !self.state.show_banner;
                let message = if self.state.show_banner {
                    "Banner enabled"
                } else {
                    "Banner hidden"
                };
                self.state.set_status(message.to_string());
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Err(err) = self.reload_games() {
                    self.state.set_status(format!("Reload failed: {err}"));
                } else {
                    if let Err(err) = self.refresh_saves() {
                        self.state
                            .set_status(format!("Reloaded but failed to read saves: {err}"));
                    } else {
                        self.state
                            .set_status(format!("Reloaded {} games", self.state.filtered.len()));
                    }
                }
            }
            KeyCode::Enter => {
                self.prompt_new_game();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_play_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) => self.handle_play_key(key)?,
            Event::Resize(_, _) => {}
            Event::Mouse(_) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
        }
        Ok(())
    }

    fn handle_name_prompt_key(&mut self, key: KeyEvent) -> Result<()> {
        let mut finalize: Option<(GameInfo, String)> = None;
        let mut cancel = false;
        if let Some(prompt) = self.name_prompt.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    cancel = true;
                }
                KeyCode::Enter => {
                    let name = prompt.value();
                    let game = prompt.game.clone();
                    finalize = Some((game, name));
                }
                KeyCode::Left => prompt.move_cursor(-1),
                KeyCode::Right => prompt.move_cursor(1),
                KeyCode::Home => prompt.move_home(),
                KeyCode::End => prompt.move_end(),
                KeyCode::Backspace => prompt.backspace(),
                KeyCode::Delete => prompt.delete(),
                KeyCode::Char(ch) => {
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                        prompt.insert(ch);
                    }
                }
                _ => {}
            }
        }

        if cancel {
            self.name_prompt = None;
            self.pending_game = None;
            self.pending_save_name = None;
            self.state.set_status("New game cancelled".to_string());
            return Ok(());
        }

        if let Some((game, name)) = finalize {
            if self.pending_session {
                self.state
                    .set_status("A session is already loading".to_string());
                self.name_prompt = None;
                self.pending_game = None;
                self.pending_save_name = None;
                return Ok(());
            }
            self.name_prompt = None;
            self.pending_game = Some(game.clone());
            self.pending_save_name = Some(name);
            self.active_save = None;
            if !self.state.select_game(&game.id) {
                self.pending_save_name = None;
                self.pending_game = None;
                self.state
                    .set_status("Selected game is no longer available".to_string());
                return Ok(());
            }
            self.start_session_load();
        }

        Ok(())
    }

    fn handle_play_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut state) = self.play_state.take() else {
            if matches!(key.code, KeyCode::Esc) {
                self.screen = Screen::Browse;
                self.pending_session = false;
            }
            return Ok(());
        };

        let mut result = match state.mode() {
            PlayMode::Idle => self.handle_play_idle_key(&mut state, key),
            PlayMode::ParSelect => self.handle_par_select_key(&mut state, key),
            PlayMode::PriceSelect => self.handle_price_select_key(&mut state, key),
            PlayMode::TrainManage => self.handle_train_manage_key(&mut state, key),
            PlayMode::TrainRun => self.handle_train_run_key(&mut state, key),
        };

        if self.screen == Screen::Play {
            if result.is_ok() {
                if let Err(err) = self.persist_active_session(&state) {
                    let err_msg = err.to_string();
                    error!(error = %err_msg, "Auto-save failed");
                    self.state
                        .set_status(format!("Auto-save failed: {err_msg}"));
                    result = Err(anyhow!(err_msg));
                }
            }
            self.play_state = Some(state);
        }

        result
    }

    fn handle_play_idle_key(&mut self, state: &mut PlayState, key: KeyEvent) -> Result<()> {
        let mut hide_banner = false;
        match key.code {
            KeyCode::Esc => {
                if let Err(err) = self.persist_active_session(state) {
                    let err_msg = err.to_string();
                    error!(error = %err_msg, "Auto-save failed on exit");
                    self.state
                        .set_status(format!("Auto-save failed: {err_msg}"));
                }
                self.screen = Screen::Browse;
                self.play_state = None;
                self.pending_session = false;
                self.state.mode = Mode::Browse;
                info!("Play session closed");
                self.state.set_status("Returned to game list".to_string());
            }
            KeyCode::Char('q') if key.modifiers.is_empty() => {
                self.state.should_quit = true;
                hide_banner = true;
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                if state.revenue_view_enabled() {
                    state.move_revenue_cursor(1, 0);
                    hide_banner = true;
                } else {
                    state.move_corporation(1);
                }
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                if state.revenue_view_enabled() {
                    state.move_revenue_cursor(-1, 0);
                    hide_banner = true;
                } else {
                    state.move_corporation(-1);
                }
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => {
                if state.revenue_view_enabled() {
                    state.move_revenue_cursor(0, -1);
                    hide_banner = true;
                }
            }
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => {
                if state.revenue_view_enabled() {
                    state.move_revenue_cursor(0, 1);
                    hide_banner = true;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::PageDown => {
                state.move_corporation(5)
            }
            KeyCode::PageUp => state.move_corporation(-5),
            KeyCode::Char('/') => {
                self.state
                    .set_status("Filtering not available in play screen".to_string());
                hide_banner = true;
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                let enabled = state.toggle_revenue_view();
                let message = if enabled {
                    "Revenue view enabled"
                } else {
                    "Stock market view enabled"
                };
                self.state.set_status(message.to_string());
                hide_banner = true;
            }
            KeyCode::Char('.') | KeyCode::Char('>') => {
                if state.advance_operating_round() {
                    let summary = state.operating_round_summary();
                    self.state.set_status(format!("Switched to {summary}"));
                } else {
                    self.state
                        .set_status("Already at final operating round".to_string());
                }
                hide_banner = true;
            }
            KeyCode::Char(',') | KeyCode::Char('<') => {
                if state.retreat_operating_round() {
                    let summary = state.operating_round_summary();
                    self.state.set_status(format!("Switched to {summary}"));
                } else {
                    self.state
                        .set_status("Already at first operating round".to_string());
                }
                hide_banner = true;
            }
            KeyCode::Char('[') => {
                if state.phase_count() == 0 {
                    self.state.set_status("No phase data available".to_string());
                } else {
                    let current_phase = state.current_phase_index();
                    state.move_phase(-1);
                    if state.current_phase_index() != current_phase {
                        self.state
                            .set_status(format!("Phase changed to {}", state.phase_label()));
                    } else {
                        self.state.set_status("Already at first phase".to_string());
                    }
                }
                hide_banner = true;
            }
            KeyCode::Char(']') => {
                if state.phase_count() == 0 {
                    self.state.set_status("No phase data available".to_string());
                } else {
                    let current_phase = state.current_phase_index();
                    state.move_phase(1);
                    if state.current_phase_index() != current_phase {
                        self.state
                            .set_status(format!("Phase changed to {}", state.phase_label()));
                    } else {
                        self.state.set_status("Already at final phase".to_string());
                    }
                }
                hide_banner = true;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if state.session.corporations.is_empty() {
                    self.state
                        .set_status("No corporations available for operating round".to_string());
                } else {
                    state.add_operating_round();
                    let label = format!("OR{}", state.revenue_cursor_or + 1);
                    self.state
                        .set_status(format!("Added operating round {label}"));
                }
                hide_banner = true;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if state.revenue_view_enabled() {
                    state.adjust_current_revenue_value(10);
                    if let Some((corp, or_idx)) = state.current_revenue_context() {
                        let value = state.current_revenue_value().unwrap_or_default();
                        self.state.set_status(format!(
                            "{} {} payout increased to {}",
                            corp.sym,
                            format!("OR{}", or_idx + 1),
                            format_currency(value)
                        ));
                    }
                    hide_banner = true;
                }
            }
            KeyCode::Char('-') => {
                if state.revenue_view_enabled() {
                    state.adjust_current_revenue_value(-10);
                    if let Some((corp, or_idx)) = state.current_revenue_context() {
                        let value = state.current_revenue_value().unwrap_or_default();
                        self.state.set_status(format!(
                            "{} {} payout reduced to {}",
                            corp.sym,
                            format!("OR{}", or_idx + 1),
                            format_currency(value)
                        ));
                    }
                    hide_banner = true;
                }
            }
            KeyCode::Char('0') => {
                if state.revenue_view_enabled() {
                    state.set_current_revenue_value(0);
                    if let Some((corp, or_idx)) = state.current_revenue_context() {
                        self.state.set_status(format!(
                            "{} {} payout cleared",
                            corp.sym,
                            format!("OR{}", or_idx + 1)
                        ));
                    }
                    hide_banner = true;
                }
            }
            KeyCode::Char(c) if ('1'..='6').contains(&c) => {
                if state.revenue_view_enabled() {
                    let percent = (c as u8 - b'0') as i32 * 10;
                    if let Some(base) = state.current_revenue_base() {
                        let value = base * percent / 100;
                        state.set_current_revenue_value(value);
                        if let Some((corp, or_idx)) = state.current_revenue_context() {
                            self.state.set_status(format!(
                                "{} {} payout set to {} ({}%)",
                                corp.sym,
                                format!("OR{}", or_idx + 1),
                                format_currency(value),
                                percent
                            ));
                        }
                    }
                    hide_banner = true;
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.begin_par_selection(state);
                hide_banner = true;
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.begin_train_mode(state);
                hide_banner = true;
            }
            KeyCode::Enter => {
                if let Some(corp) = state.current_corporation() {
                    if corp.par_value.is_some() {
                        self.begin_price_selection(state);
                    } else {
                        self.begin_par_selection(state);
                    }
                } else {
                    self.state.set_status("No corporation selected".to_string());
                }
                hide_banner = true;
            }
            _ => {}
        }
        if hide_banner {
            state.consume_title_banner();
        }
        Ok(())
    }

    fn handle_par_select_key(&mut self, state: &mut PlayState, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                debug!(cursor = ?state.market_cursor(), "Par selection cancelled");
                state.exit_market();
                self.state.set_status("Par selection cancelled".to_string());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') if key.modifiers.is_empty() => {
                self.state.should_quit = true;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.apply_par_selection(state);
            }
            KeyCode::Enter => {
                let corp_data = state
                    .current_corporation()
                    .map(|corp| (corp.sym.clone(), corp.par_value));
                let (corp_sym, par_set) = match corp_data {
                    Some((sym, value)) => (Some(sym), value.is_some()),
                    None => (None, false),
                };
                debug!(
                    ?corp_sym,
                    cursor = ?state.market_cursor(),
                    par_set,
                    "Par selection enter pressed"
                );
                if par_set {
                    debug!(?corp_sym, "Par already set; prompting for update");
                    self.state
                        .set_status("Par already set; press 'p' to update".to_string());
                } else {
                    self.apply_par_selection(state);
                }
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                state.move_market_cursor(1, 0)
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                state.move_market_cursor(-1, 0)
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => {
                state.move_market_cursor(0, -1)
            }
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => {
                state.move_market_cursor(0, 1)
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_price_select_key(&mut self, state: &mut PlayState, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                state.exit_market();
                self.state
                    .set_status("Stock price selection cancelled".to_string());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') if key.modifiers.is_empty() => {
                self.state.should_quit = true;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.begin_par_selection(state);
            }
            KeyCode::Enter => {
                self.apply_price_selection(state);
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                state.move_market_cursor(1, 0)
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                state.move_market_cursor(-1, 0)
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => {
                state.move_market_cursor(0, -1)
            }
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => {
                state.move_market_cursor(0, 1)
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_train_manage_key(&mut self, state: &mut PlayState, key: KeyEvent) -> Result<()> {
        if state.is_purchase_modal_active() {
            return self.handle_train_purchase_modal_key(state, key);
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('t') | KeyCode::Char('T') => {
                state.exit_train_manage();
                self.state.set_status("Train management closed".to_string());
            }
            KeyCode::Char('q') | KeyCode::Char('Q') if key.modifiers.is_empty() => {
                self.state.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                state.move_train_selection(1);
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                state.move_train_selection(-1);
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => {
                state.focus_owned();
            }
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => {
                state.focus_pool();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(train) = state.rust_selected_train() {
                    let corp = state
                        .current_corporation()
                        .map(|corp| corp.sym.clone())
                        .unwrap_or_default();
                    self.state
                        .set_status(format!("{} rusts {} train", corp, train.name));
                } else {
                    self.state
                        .set_status("No owned train selected to rust".to_string());
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                match state.apply_revenue_action(RevenueAction::Dividend) {
                    Ok(outcome) => {
                        let payouts = share_payout_line(outcome.total);
                        let verb = match outcome.action {
                            RevenueAction::Dividend => "pays",
                            RevenueAction::Withhold => "withholds",
                        };
                        let movement = if outcome.moved {
                            "price moved"
                        } else {
                            "price unchanged"
                        };
                        self.state.set_status(format!(
                            "{} {} {} dividend - price {} ({}) | {}",
                            outcome.corp_sym,
                            verb,
                            format_currency(outcome.total),
                            outcome.price_label,
                            movement,
                            payouts
                        ));
                    }
                    Err(err) => {
                        self.state.set_status(err.to_string());
                    }
                }
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                match state.apply_revenue_action(RevenueAction::Withhold) {
                    Ok(outcome) => {
                        let movement = if outcome.moved {
                            "price moved"
                        } else {
                            "price unchanged"
                        };
                        self.state.set_status(format!(
                            "{} withholds {} - price {} ({})",
                            outcome.corp_sym,
                            format_currency(outcome.total),
                            outcome.price_label,
                            movement
                        ));
                    }
                    Err(err) => {
                        self.state.set_status(err.to_string());
                    }
                }
            }
            KeyCode::Char('b') | KeyCode::Char('B') => {
                if state.available_trains().is_empty() {
                    self.state
                        .set_status("No train available for purchase".to_string());
                } else {
                    state.open_train_purchase_modal();
                    if state.is_purchase_modal_active() {
                        self.state.set_status(
                            "Select train to purchase (Enter confirm, Esc cancel)".to_string(),
                        );
                    }
                }
            }
            KeyCode::Enter => match state.train_focus() {
                TrainFocus::Owned => {
                    if state.current_owned_train().is_some() && state.start_train_run() {
                        if let Some((corp, train, _)) = state.train_run_context() {
                            self.state
                                .set_status(format!("Editing run for {} {}", corp.sym, train.name));
                        } else {
                            self.state.set_status("Editing train run".to_string());
                        }
                    } else if state.available_trains().is_empty() {
                        self.state
                            .set_status("No train available for purchase".to_string());
                    } else {
                        state.open_train_purchase_modal();
                        if state.is_purchase_modal_active() {
                            self.state.set_status(
                                "Select train to purchase (Enter confirm, Esc cancel)".to_string(),
                            );
                        }
                    }
                }
                TrainFocus::Pool => {
                    if state.available_trains().is_empty() {
                        self.state
                            .set_status("No train available for purchase".to_string());
                    } else {
                        state.open_train_purchase_modal();
                        if state.is_purchase_modal_active() {
                            self.state.set_status(
                                "Select train to purchase (Enter confirm, Esc cancel)".to_string(),
                            );
                        }
                    }
                }
            },
            KeyCode::Tab => {
                state.toggle_train_focus();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_train_purchase_modal_key(
        &mut self,
        state: &mut PlayState,
        key: KeyEvent,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') | KeyCode::Char('T') => {
                state.close_train_purchase_modal();
                self.state
                    .set_status("Train purchase cancelled".to_string());
            }
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                state.move_purchase_modal_cursor(1);
            }
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                state.move_purchase_modal_cursor(-1);
            }
            KeyCode::Enter => {
                if let Some(selection_idx) = state
                    .train_purchase_modal
                    .as_ref()
                    .map(|modal| modal.cursor)
                {
                    state.close_train_purchase_modal();
                    self.apply_train_purchase(state, selection_idx);
                } else {
                    self.state
                        .set_status("No train available for purchase".to_string());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        match self.screen {
            Screen::Menu => self.draw_menu(frame),
            Screen::Browse => self.draw_browse(frame),
            Screen::Continue => self.draw_continue(frame),
            Screen::Play => self.draw_play(frame),
        }
        if let Some(prompt) = &self.name_prompt {
            self.render_name_prompt(frame, prompt);
        }
    }

    fn draw_menu(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let banner_lines = block_font::render("18TUI");
        let banner_height = banner_lines.len() as u16;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length((banner_height + 2).min(area.height)),
                Constraint::Min(3),
            ])
            .split(area);

        let banner_content: Vec<Line> = banner_lines
            .into_iter()
            .map(|line| {
                Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(self.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect();
        let banner = Paragraph::new(banner_content).alignment(Alignment::Center);
        frame.render_widget(banner, layout[0]);

        let menu_items = ["New Game", "Continue", "Quit"];
        let menu_height = (menu_items.len() as u16)
            .saturating_mul(2)
            .saturating_add(2)
            .min(layout[1].height);
        let menu_width = 28.min(layout[1].width.max(1));
        let menu_area = centered_rect(menu_width, menu_height, layout[1]);

        let menu_lines: Vec<Line> = menu_items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                if idx == self.state.menu_cursor {
                    Line::from(Span::styled(
                        format!("▶ {item}"),
                        Style::default()
                            .fg(self.theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("  {item}"),
                        Style::default().fg(self.theme.primary_fg),
                    ))
                }
            })
            .collect();

        let menu = Paragraph::new(menu_lines)
            .block(Block::default().borders(Borders::ALL).title("Menu"))
            .alignment(Alignment::Center);
        frame.render_widget(menu, menu_area);
    }

    fn draw_browse(&mut self, frame: &mut Frame) {
        let size = frame.size();
        self.state.list_height = size.height.saturating_sub(5) as usize;

        let banner_lines = if self.state.show_banner {
            self.state
                .current_game()
                .map(|game| block_font::render(&game.title))
        } else {
            None
        };

        let mut constraints = Vec::new();
        if let Some(lines) = &banner_lines {
            constraints.push(Constraint::Length(lines.len() as u16 + 2));
        }
        constraints.push(Constraint::Min(8));
        constraints.push(Constraint::Length(3));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_iter = chunks.iter();
        let banner_chunk = if banner_lines.is_some() {
            chunk_iter.next()
        } else {
            None
        };
        let body_chunk = chunk_iter.next().copied().unwrap_or(size);
        let status_chunk = chunk_iter.next().copied().unwrap_or(size);

        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(body_chunk);

        self.render_game_list(frame, body_chunks[0]);
        self.render_game_info(frame, body_chunks[1]);
        self.render_status(frame, status_chunk);
        if let (Some(lines), Some(area)) = (banner_lines.as_ref(), banner_chunk.copied()) {
            self.render_banner(frame, area, lines);
        }
    }

    fn draw_continue(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(3)])
            .split(area);
        let list_area = chunks[0];
        let status_area = chunks[1];

        let list_height = list_area.height.saturating_sub(2) as usize;
        self.state.list_height = list_height;
        let total = self.saves.len();
        let visible = list_height.max(1);
        self.state.move_continue_cursor(0, total, visible); // clamp offsets

        let mut list_state = ListState::default();
        if total > 0 {
            list_state.select(Some(self.state.continue_cursor.min(total - 1)));
        }

        let items: Vec<ListItem> = if total == 0 {
            vec![ListItem::new(Line::from("  No saves found"))]
        } else {
            let mut entries = Vec::new();
            let end = cmp::min(self.state.continue_offset + visible, total);
            for (idx, entry) in self.saves[self.state.continue_offset..end]
                .iter()
                .enumerate()
            {
                let absolute_idx = self.state.continue_offset + idx;
                let marker = if absolute_idx == self.state.continue_cursor {
                    Span::styled("▶ ", Style::default().fg(self.theme.accent))
                } else {
                    Span::raw("  ")
                };
                let timestamp = entry.updated_at.format("%Y-%m-%d %H:%M");
                entries.push(ListItem::new(Line::from(vec![
                    marker,
                    Span::raw(format!("{}  [{}]", entry.name, timestamp)),
                ])));
            }
            entries
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Continue Game");
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(self.theme.selection_bg));

        frame.render_stateful_widget(list, list_area, &mut list_state);
        self.render_status(frame, status_area);
    }

    fn draw_play(&mut self, frame: &mut Frame) {
        let area = frame.size();
        if self.play_state.is_some() {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(60),
                    Constraint::Percentage(35),
                    Constraint::Length(3),
                ])
                .split(area);

            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(32), Constraint::Min(20)])
                .split(rows[0]);
            let bottom = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(rows[1]);

            if let Some(state) = self.play_state.as_mut() {
                Self::render_play_market(&self.theme, frame, top[1], state);
            }

            if let Some(state) = self.play_state.as_ref() {
                self.render_play_corporations(frame, top[0], state);
                self.render_play_info(frame, bottom[0], state);
                self.render_play_help(frame, bottom[1], state);
            }
            self.render_status(frame, rows[2]);

            if let Some(state) = self.play_state.as_mut() {
                if state.is_purchase_modal_active() {
                    Self::render_train_purchase_modal(&self.theme, frame, area, state);
                }
            }
        } else {
            let block = Block::default().borders(Borders::ALL).title("Play Mode");
            let message = if self.pending_session {
                "Loading session…".to_string()
            } else {
                "No session loaded".to_string()
            };
            let paragraph = Paragraph::new(vec![Line::from(message)])
                .block(block)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
        }
    }

    fn render_name_prompt(&self, frame: &mut Frame, prompt: &NamePromptModal) {
        let frame_area = frame.size();
        let mut width = cmp::min(60_u16, frame_area.width.saturating_sub(4));
        width = cmp::max(width, 24_u16);
        let height = 7_u16.min(frame_area.height.saturating_sub(2)).max(5_u16);
        let x = frame_area.x + (frame_area.width.saturating_sub(width)) / 2;
        let y = frame_area.y + (frame_area.height.saturating_sub(height)) / 2;
        let area = Rect::new(x, y, width, height);

        frame.render_widget(Clear, area);

        let title = format!("New Game - {}", prompt.game.title);
        let instruction = format!("Save name for {}", prompt.game.title);
        let input_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(self.theme.accent)),
            Span::raw(prompt.input.clone()),
        ]);
        let helper = Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" start  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" cancel"),
        ]);
        let default_hint = Line::from(format!("Default: {}", prompt.default));

        let paragraph = Paragraph::new(vec![
            Line::from(instruction),
            input_line,
            Line::from(""),
            helper,
            default_hint,
        ])
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: true });

        frame.render_widget(paragraph, area);

        let cursor_x =
            (area.x + 2 + prompt.cursor as u16).min(area.x + area.width.saturating_sub(2));
        let cursor_y = area.y + 1;
        frame.set_cursor(cursor_x, cursor_y);
    }

    fn render_game_list(&mut self, frame: &mut Frame, area: Rect) {
        self.state.list_height = area.height.saturating_sub(2) as usize;
        self.state.clamp_cursor();
        self.state.ensure_cursor_visible();

        let mut list_state = ListState::default();
        let height = area.height.saturating_sub(2) as usize;
        let games = self.state.visible_games(height);
        if !games.is_empty() {
            let selected = self
                .state
                .cursor
                .saturating_sub(self.state.offset)
                .min(games.len().saturating_sub(1));
            list_state.select(Some(selected));
        }
        let items: Vec<ListItem> = games
            .iter()
            .enumerate()
            .map(|(idx, game)| {
                let global_index = self.state.offset + idx;
                let is_selected = self.state.cursor == global_index;
                let marker = if is_selected {
                    Span::styled(
                        "▶ ",
                        Style::default()
                            .fg(self.theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("  ")
                };
                let title = Span::styled(
                    game.title.clone(),
                    Style::default()
                        .fg(self.theme.primary_fg)
                        .add_modifier(Modifier::BOLD),
                );
                let subtitle = game.subtitle.as_ref().map(|s| {
                    Span::styled(
                        format!(" · {}", s),
                        Style::default().fg(self.theme.muted),
                    )
                });
                let mut line = vec![marker, title];
                if let Some(sub) = subtitle {
                    line.push(sub);
                }
                ListItem::new(Line::from(line))
            })
            .collect();

        let block = Block::default().borders(Borders::ALL).title("Games");
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(self.theme.selection_bg));
        frame.render_stateful_widget(list, area, &mut list_state);
    }

    fn render_game_info(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title("Game Details");
        if let Some(game) = self.state.current_game() {
            let mut lines = Vec::new();
            lines.push(Line::from(vec![Span::styled(
                game.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            if let Some(subtitle) = &game.subtitle {
                lines.push(Line::from(Span::styled(
                    subtitle.clone(),
                    Style::default().fg(self.theme.muted),
                )));
            }
            if let Some(designer) = &game.designer {
                lines.push(Line::from(format!("Designer: {designer}")));
            }
            if let Some(location) = &game.location {
                lines.push(Line::from(format!("Location: {location}")));
            }
            if let Some(url) = &game.rules_url {
                lines.push(Line::from(format!("Rules: {url}")));
            }
            if let Some(commit) = &self.metadata.commit {
                let short = commit.chars().take(7).collect::<String>();
                lines.push(Line::from(format!("Commit: {}", short)));
            }
            if let Some(updated) = &self.metadata.updated_at {
                lines.push(Line::from(format!(
                    "Updated: {}",
                    updated.format("%Y-%m-%d %H:%M UTC")
                )));
            }
            if lines.is_empty() {
                lines.push(Line::from("No metadata available"));
            }
            let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
        } else {
            let paragraph = Paragraph::new("No games available").block(block);
            frame.render_widget(paragraph, area);
        }
    }

    fn render_play_corporations(&self, frame: &mut Frame, area: Rect, state: &PlayState) {
        let block = Block::default().borders(Borders::ALL).title("Corporations");
        let items: Vec<ListItem> = state
            .session
            .corporations
            .iter()
            .map(|corp| {
                let par_text = corp
                    .par_value
                    .map(|value| format!("${value}"))
                    .unwrap_or_else(|| "--".to_string());
                let market_text = corp
                    .market_position
                    .as_ref()
                    .map(|pos| {
                        let sanitized = sanitize_market_text(&pos.raw);
                        if sanitized.is_empty() {
                            pos.raw.clone()
                        } else {
                            sanitized
                        }
                    })
                    .unwrap_or_else(|| "--".to_string());
                let mut spans = vec![Span::styled(
                    format!("{:>3}", corp.sym),
                    Style::default()
                        .fg(self.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )];
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    corp.name.clone(),
                    Style::default().fg(self.theme.primary_fg),
                ));
                spans.push(Span::raw(format!("  P:{par_text:<4}")));
                spans.push(Span::raw(format!(" M:{market_text:<4}")));
                ListItem::new(Line::from(spans))
            })
            .collect();

        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(state.corporation_index.min(items.len() - 1)));
        }

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(self.theme.selection_bg))
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut list_state);
    }

    fn render_play_market(theme: &Theme, frame: &mut Frame, area: Rect, state: &mut PlayState) {
        if state.should_show_title_banner() {
            Self::render_play_title_banner(theme, frame, area, state);
            return;
        }

        if state.revenue_view_enabled() {
            Self::render_revenue_panel(theme, frame, area, state);
            return;
        }

        let block = Block::default().borders(Borders::ALL).title("Stock Market");
        let cell_width = state
            .session
            .market
            .iter()
            .flat_map(|row| row.iter())
            .map(|value| value.len())
            .max()
            .unwrap_or(1);
        let cell_width = cmp::max(4, cell_width + 2);
        let inner_height = area.height.saturating_sub(2) as usize;
        let inner_width = area.width.saturating_sub(2) as usize;
        let effective_col_width = cmp::max(1, cell_width);
        let view_cols = if inner_width == 0 {
            1
        } else {
            let total_cols = state.max_market_columns().max(1);
            let automatic_cols = cmp::max(
                1,
                (inner_width + effective_col_width - 1) / effective_col_width,
            );
            cmp::min(total_cols, automatic_cols)
        };
        let view_rows = cmp::max(1, inner_height);
        state.set_market_view(view_rows, view_cols);

        let cursor = state.market_cursor();
        let play_mode = state.mode();
        let corp_position = state
            .current_corporation()
            .and_then(|corp| corp.market_position.clone());
        // Precompute which corporations have tokens in each cell so we can overlay them while rendering.
        let mut token_map: HashMap<(usize, usize), Vec<&Corporation>> = HashMap::new();
        for corp in &state.session.corporations {
            if let Some(pos) = &corp.market_position {
                token_map.entry((pos.row, pos.col)).or_default().push(corp);
            }
        }

        let row_offset = state.market_row_offset();
        let col_offset = state.market_col_offset();
        let view_rows = state.market_view_rows();
        let view_cols = state.market_view_cols();
        let total_rows = state.session.market.len();

        let mut lines = Vec::new();
        let row_end = cmp::min(total_rows, row_offset + view_rows);
        for row_idx in row_offset..row_end {
            let row = &state.session.market[row_idx];
            let mut spans = Vec::new();
            if row.len() <= col_offset {
                spans.push(Span::raw(" ".repeat(cell_width * view_cols)));
            } else {
                let col_end = cmp::min(row.len(), col_offset + view_cols);
                for col_idx in col_offset..col_end {
                    let raw = &row[col_idx];
                    if raw.trim().is_empty() {
                        spans.push(Span::raw(" ".repeat(cell_width)));
                        continue;
                    }
                    let is_par_cell = state.is_par_cell(row_idx, col_idx);
                    let mut style = Style::default().fg(market_color(raw, theme));
                    if play_mode == PlayMode::ParSelect && !is_par_cell {
                        style = style.add_modifier(Modifier::DIM);
                    }
                    if let Some(pos) = &corp_position {
                        if pos.row == row_idx && pos.col == col_idx {
                            style = style.fg(theme.success).add_modifier(Modifier::BOLD);
                        }
                    }
                    if is_par_cell && play_mode == PlayMode::ParSelect {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if play_mode != PlayMode::Idle && cursor == (row_idx, col_idx) {
                        style = style
                            .fg(theme.on_accent)
                            .bg(theme.accent)
                            .add_modifier(Modifier::BOLD);
                    }

                    let sanitized = sanitize_market_text(raw);
                    let display = if sanitized.is_empty() {
                        raw.trim().to_string()
                    } else {
                        sanitized
                    };
                    let padded = format!("{text:^width$}", text = display, width = cell_width);
                    let mut cell_spans = vec![Span::styled(padded.clone(), style)];

                    if let Some(tokens) = token_map.get(&(row_idx, col_idx)) {
                        let mut base_chars: Vec<char> = padded.chars().collect();
                        let max_tokens = cmp::min(tokens.len(), base_chars.len());
                        for (idx, corp) in tokens.iter().take(max_tokens).enumerate() {
                            let glyph = token_glyph(corp);
                            if let Some(ch) = glyph.chars().next() {
                                let replace_index = idx.min(base_chars.len().saturating_sub(1));
                                base_chars[replace_index] = ch;
                            }
                        }
                        let updated = base_chars.iter().collect::<String>();
                        cell_spans[0] = Span::styled(updated, style);
                    }

                    spans.extend(cell_spans);
                }

                let displayed = col_end.saturating_sub(col_offset);
                if displayed < view_cols {
                    for _ in displayed..view_cols {
                        spans.push(Span::raw(" ".repeat(cell_width)));
                    }
                }
            }
            if spans.is_empty() {
                spans.push(Span::raw(""));
            }
            lines.push(Line::from(spans));
        }

        while lines.len() < view_rows {
            lines.push(Line::from(Span::raw(" ".repeat(cell_width * view_cols))));
        }

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_revenue_panel(
        theme: &Theme,
        frame: &mut Frame,
        area: Rect,
        state: &mut PlayState,
    ) {
        let phase_label = state.phase_label();
        state.ensure_phase_round_capacity(state.current_phase_index());
        let summary = state.operating_round_summary();
        let title = format!("Revenue by OR - {phase_label} ({summary})");

        let inner_height = area.height.saturating_sub(2) as usize;
        let inner_width = area.width.saturating_sub(2) as usize;
        let (total_rows, total_cols) = state.revenue_dimensions();

        if total_rows == 0 {
            let block = Block::default().borders(Borders::ALL).title(title);
            let paragraph = Paragraph::new(vec![
                Line::from("No corporations loaded"),
                Line::from("Press 'm' to return to market view"),
            ])
            .block(block)
            .alignment(Alignment::Center);
            frame.render_widget(paragraph, area);
            return;
        }

        if total_cols == 0 {
            let block = Block::default().borders(Borders::ALL).title(title);
            let mut lines = vec![Line::from("No operating rounds configured for this phase.")];
            lines.push(Line::from("Press 'a' to add an operating round."));
            lines.push(Line::from("Press 'm' to return to the stock market view."));
            let paragraph = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center);
            frame.render_widget(paragraph, area);
            return;
        }

        let corp_label_width = state
            .session
            .corporations
            .iter()
            .map(|corp| cmp::max(corp.sym.len(), corp.name.len()))
            .max()
            .unwrap_or(4);
        let corp_col_width = cmp::max(6, cmp::min(inner_width.max(1), corp_label_width + 2));

        let header_labels: Vec<String> = (0..total_cols)
            .map(|idx| format!("OR{}", idx + 1))
            .collect();
        let max_header_len = header_labels
            .iter()
            .map(|label| label.len())
            .max()
            .unwrap_or(2);
        let max_value_len = state
            .current_phase_rounds()
            .iter()
            .flat_map(|round| round.revenues.iter())
            .map(|value| value.to_string().len())
            .max()
            .unwrap_or(2);
        let mut col_width = cmp::max(max_header_len, max_value_len);
        col_width = cmp::max(
            4,
            cmp::min(
                inner_width.saturating_sub(corp_col_width).max(1),
                col_width + 2,
            ),
        );

        let available_width = inner_width.saturating_sub(corp_col_width);
        let view_cols = if available_width == 0 {
            0
        } else {
            let approx = (available_width + col_width - 1) / col_width;
            cmp::max(1, cmp::min(total_cols, approx))
        };
        let view_rows = cmp::max(1, inner_height.saturating_sub(1));
        state.set_revenue_view_dims(view_rows, view_cols.max(1));

        let row_offset = state.revenue_row_offset;
        let col_offset = state.revenue_col_offset;
        let row_end = cmp::min(total_rows, row_offset + state.revenue_view_rows);
        let col_end = cmp::min(total_cols, col_offset + state.revenue_view_cols);

        let phase_rounds = state.current_phase_rounds();
        let mut lines = Vec::new();
        let mut header_spans = Vec::new();
        header_spans.push(Span::styled(
            format!("{:<width$}", "Corp", width = corp_col_width),
            Style::default().add_modifier(Modifier::BOLD),
        ));

        for idx in col_offset..col_end {
            let label = header_labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("OR{}", idx + 1));
            let cell = format!("{:^width$}", label, width = col_width);
            header_spans.push(Span::styled(
                cell,
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(header_spans));

        for row_idx in row_offset..row_end {
            let corporation = &state.session.corporations[row_idx];
            let is_active_row = row_idx == state.revenue_cursor_corp;
            let mut spans = Vec::new();
            let mut corp_style = Style::default().add_modifier(Modifier::BOLD);
            if is_active_row {
                corp_style = corp_style.fg(theme.accent);
            }
            spans.push(Span::styled(
                format!(
                    "{:<width$}",
                    corporation.sym.clone(),
                    width = corp_col_width
                ),
                corp_style,
            ));

            for col_idx in col_offset..col_end {
                let value = phase_rounds
                    .get(col_idx)
                    .and_then(|round| round.revenues.get(row_idx))
                    .copied()
                    .unwrap_or_default();
                let text = if value == 0 {
                    "-".to_string()
                } else {
                    value.to_string()
                };
                let mut style = Style::default();
                if row_idx == state.revenue_cursor_corp && col_idx == state.revenue_cursor_or {
                    style = style
                        .bg(theme.accent)
                        .fg(theme.on_accent)
                        .add_modifier(Modifier::BOLD);
                } else if row_idx == state.revenue_cursor_corp {
                    style = style.fg(theme.accent);
                }
                let cell = format!("{:^width$}", text, width = col_width);
                spans.push(Span::styled(cell, style));
            }

            let displayed = col_end.saturating_sub(col_offset);
            if displayed < state.revenue_view_cols {
                for _ in displayed..state.revenue_view_cols {
                    spans.push(Span::raw(" ".repeat(col_width)));
                }
            }

            lines.push(Line::from(spans));
        }

        let active_value = state.current_revenue_value().unwrap_or_default();
        let base_value = state.current_revenue_base().unwrap_or_default();
        let dividend_spans = {
            let mut spans = Vec::with_capacity(14);
            spans.push(Span::styled(
                "Dividends ",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            for step in 1..=6 {
                let percent = step * 10;
                let computed = base_value * percent / 100;
                let label = if base_value == 0 {
                    format!("{percent:>2}%:-")
                } else {
                    format!("{percent:>2}%:{}", format_currency(computed))
                };
                let mut style = Style::default();
                if base_value != 0 && active_value == computed {
                    style = style
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                }
                spans.push(Span::styled(label, style));
                if step != 6 {
                    spans.push(Span::raw("  "));
                }
            }
            spans
        };
        let dividend_line = Line::from(dividend_spans);

        let controls_line = Line::from(vec![
            Span::styled("hjkl", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" move  "),
            Span::styled("1-6", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" set $10-$60  "),
            Span::styled("+/-", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" adjust  "),
            Span::styled("a", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" add OR  "),
            Span::styled(", .", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" switch OR  "),
            Span::styled("[ / ]", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" change phase  "),
            Span::styled("m", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" stock market"),
        ]);

        lines.push(Line::from(""));
        lines.push(dividend_line);
        lines.push(Line::from(""));
        lines.push(controls_line);

        let block = Block::default().borders(Borders::ALL).title(title);
        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn render_play_title_banner(theme: &Theme, frame: &mut Frame, area: Rect, state: &PlayState) {
        let block = Block::default().borders(Borders::ALL).title("Stock Market");
        let banner_lines = block_font::render(&state.session.info.title);
        let styled_lines: Vec<Line> = banner_lines
            .into_iter()
            .map(|line| {
                Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect();

        let inner_height = area.height.saturating_sub(2) as usize;
        let mut content: Vec<Line> = Vec::new();
        if inner_height > 0 {
            let padding = inner_height.saturating_sub(styled_lines.len());
            let top_padding = padding / 2;
            for _ in 0..top_padding {
                content.push(Line::from(String::new()));
            }
            content.extend(styled_lines.iter().cloned());
            while content.len() < inner_height {
                content.push(Line::from(String::new()));
            }
        } else {
            content = styled_lines;
        }

        let paragraph = Paragraph::new(content)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_play_info(&self, frame: &mut Frame, area: Rect, state: &PlayState) {
        match state.mode() {
            PlayMode::TrainManage | PlayMode::TrainRun => {
                self.render_train_manage_panel(frame, area, state);
                return;
            }
            _ => {}
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Corporation Info");
        let mut lines = Vec::new();
        if let Some(corp) = state.current_corporation() {
            lines.push(Line::from(vec![Span::styled(
                format!("{} ({})", corp.name, corp.sym),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            if let Some(par) = corp.par_value {
                lines.push(Line::from(format!("Par Value: ${par}")));
            } else {
                lines.push(Line::from("Par Value: unset"));
            }
            if let Some(position) = &corp.market_position {
                let value = sanitize_market_text(&position.raw);
                let value = if value.is_empty() {
                    position.raw.clone()
                } else {
                    value
                };
                lines.push(Line::from(format!(
                    "Market Position: {} ({},{})",
                    value, position.row, position.col
                )));
            } else {
                lines.push(Line::from("Market Position: --"));
            }
            lines.push(Line::from(format!("Last Revenue: ${}", corp.last_revenue)));
            if corp.trains.is_empty() {
                lines.push(Line::from("Owned Trains: none"));
            } else {
                lines.push(Line::from("Owned Trains:"));
                for owned in &corp.trains {
                    let price_text = owned
                        .price
                        .map(|value| format!("cost=${value}"))
                        .unwrap_or_else(|| "cost=?".to_string());
                    let stops_render = if owned.revenue_stops.is_empty() {
                        "[--]".to_string()
                    } else {
                        owned
                            .revenue_stops
                            .iter()
                            .enumerate()
                            .map(|(idx, value)| {
                                if idx == 0 {
                                    format!("[{}]", value)
                                } else {
                                    format!(" + [{}]", value)
                                }
                            })
                            .collect::<String>()
                    };
                    let stop_count = owned.revenue_stops.len();
                    let limit = state.train_stop_limit_for(&owned.name);
                    let usage_text = if let Some(limit) = limit {
                        format!("  used {stop_count}/{limit}")
                    } else {
                        format!("  used {stop_count}")
                    };
                    let over_limit = limit.map(|limit| stop_count > limit).unwrap_or(false);
                    let usage_style = if over_limit {
                        Style::default().fg(self.theme.warning)
                    } else {
                        Style::default()
                    };
                    let usage_span = Span::styled(usage_text, usage_style);
                    lines.push(Line::from(vec![
                        Span::raw(format!(
                            "  {}  dist={}  last=${}  {}  stops: {}",
                            owned.name,
                            format_distance(&owned.distance),
                            owned.last_revenue,
                            price_text,
                            stops_render
                        )),
                        usage_span,
                    ]));
                }
            }
        } else {
            lines.push(Line::from("No corporation selected"));
        }

        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_train_purchase_modal(
        theme: &Theme,
        frame: &mut Frame,
        area: Rect,
        state: &mut PlayState,
    ) {
        let available_entries = state.available_trains();
        if available_entries.is_empty() {
            return;
        }
        let available_strings: Vec<String> = available_entries
            .iter()
            .map(|(_, ty, remaining)| {
                format!(
                    "{}  dist={}  price=${}  ({} left)",
                    ty.name,
                    format_distance(&ty.distance),
                    ty.price.unwrap_or(0),
                    remaining
                )
            })
            .collect();
        let len = available_strings.len();
        drop(available_entries);
        let Some(modal) = state.train_purchase_modal.as_mut() else {
            return;
        };

        let max_text_width = available_strings.iter().map(|s| s.len()).max().unwrap_or(0);

        let modal_width = cmp::min(
            area.width.saturating_sub(2) as usize,
            (max_text_width + 6).max(40),
        ) as u16;
        let header_lines = 4usize;
        let max_height = area.height.saturating_sub(2) as usize;
        let mut modal_height = header_lines + available_strings.len();
        if max_height > 0 {
            modal_height = modal_height.min(max_height).max(header_lines + 1);
        }
        let popup = centered_rect(modal_width, modal_height as u16, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Purchase Train");
        let mut lines = Vec::new();
        lines.push(Line::from("Select a train to buy"));
        lines.push(Line::from("Enter confirm · Esc cancel"));
        lines.push(Line::from("j/k move cursor"));
        lines.push(Line::from(""));
        let visible = modal_height.saturating_sub(header_lines).max(1);
        if modal.cursor >= len {
            modal.cursor = len.saturating_sub(1);
        }
        if len <= visible {
            modal.offset = 0;
        } else {
            if modal.offset + visible > len {
                modal.offset = len - visible;
            }
            if modal.cursor < modal.offset {
                modal.offset = modal.cursor;
            } else if modal.cursor >= modal.offset + visible {
                modal.offset = modal.cursor + 1 - visible;
            }
        }
        let end = cmp::min(modal.offset + visible, len);
        for idx in modal.offset..end {
            let pointer = if idx == modal.cursor {
                Span::styled("▶ ", Style::default().fg(theme.accent))
            } else {
                Span::raw("  ")
            };
            let text = &available_strings[idx];
            lines.push(Line::from(vec![pointer, Span::raw(text.clone())]));
        }
        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, popup);
    }

    fn render_play_help(&self, frame: &mut Frame, area: Rect, state: &PlayState) {
        let block = Block::default().borders(Borders::ALL).title("Commands");
        let lines = match state.mode() {
            PlayMode::Idle => {
                let mut lines = vec![
                    Line::from("Esc   return to game list"),
                    Line::from("q     quit application"),
                    Line::from("j/k   select corporation"),
                    Line::from("Enter open market / set price"),
                    Line::from("p     set or update par price"),
                    Line::from("t     manage trains"),
                    Line::from("Auto-save enabled"),
                    Line::from("u     undo (history)"),
                    Line::from("Ctrl+R redo history"),
                ];
                if state.revenue_view_enabled() {
                    lines.push(Line::from("m     show stock market"));
                    lines.push(Line::from("hjkl move payout cursor"));
                    lines.push(Line::from("1-6  set $10-$60 payout"));
                    lines.push(Line::from("+/-  adjust payout by $10"));
                    lines.push(Line::from("0     clear payout"));
                    lines.push(Line::from("a     add operating round"));
                    lines.push(Line::from("[ ]   change phase"));
                } else {
                    lines.push(Line::from("m     show revenue by OR"));
                    lines.push(Line::from("[ ]   change phase"));
                    lines.push(Line::from("a     add operating round"));
                }
                lines
            }
            PlayMode::ParSelect => vec![
                Line::from("Esc   cancel par selection"),
                Line::from("hjkl move cursor"),
                Line::from("p     confirm par price"),
                Line::from("Enter confirm par (first set)"),
                Line::from("u     undo (history)"),
                Line::from("Ctrl+R redo history"),
            ],
            PlayMode::PriceSelect => vec![
                Line::from("Esc   cancel stock selection"),
                Line::from("hjkl move cursor"),
                Line::from("Enter set stock price"),
                Line::from("p     adjust par price"),
                Line::from("u     undo (history)"),
                Line::from("Ctrl+R redo history"),
            ],
            PlayMode::TrainManage => vec![
                Line::from("Esc/t exit train manager"),
                Line::from("hl   switch section (arrows ok)"),
                Line::from("jk   move selection"),
                Line::from("Enter edit selected run"),
                Line::from("b     buy train (modal)"),
                Line::from("d     pay dividend"),
                Line::from("w     withhold earnings"),
                Line::from("r     rust selected train"),
                Line::from("u     undo (history)"),
                Line::from("Ctrl+R redo history"),
            ],
            PlayMode::TrainRun => vec![
                Line::from("Esc/t cancel run editor"),
                Line::from("jk   move stop cursor"),
                Line::from("0-9  edit stop value"),
                Line::from("Enter commit value"),
                Line::from("Backspace delete digit"),
                Line::from("+/-  add or remove stop"),
                Line::from("Space commit and move next"),
                Line::from("Ctrl+Enter save run"),
                Line::from("u     undo (history)"),
                Line::from("Ctrl+R redo history"),
            ],
        };
        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_train_manage_panel(&self, frame: &mut Frame, area: Rect, state: &PlayState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Train Management");
        let mut lines = Vec::new();
        if let Some(corp) = state.current_corporation() {
            lines.push(Line::from(vec![Span::styled(
                format!("{} ({})", corp.name, corp.sym),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from("Owned:"));
            let owned_focus = state.train_focus() == TrainFocus::Owned;
            if corp.trains.is_empty() {
                let marker = if owned_focus { "▶ " } else { "  " };
                lines.push(Line::from(format!("{marker}none")));
            } else {
                for (idx, owned) in corp.trains.iter().enumerate() {
                    let is_selected = owned_focus && idx == state.owned_train_cursor();
                    let marker = if is_selected {
                        Span::styled("▶ ", Style::default().fg(self.theme.accent))
                    } else {
                        Span::raw("  ")
                    };
                    let price = owned
                        .price
                        .map(|value| format!("cost=${value}"))
                        .unwrap_or_else(|| "cost=?".to_string());

                    let (display_values, active_cursor, pending_input) =
                        if let Some(run) = state.train_run_state() {
                            if run.train_index == idx {
                                let pending = if run.has_pending_input() {
                                    Some(run.pending_input().to_string())
                                } else {
                                    None
                                };
                                (run.values.clone(), Some(run.cursor), pending)
                            } else {
                                (owned.revenue_stops.clone(), None, None)
                            }
                        } else {
                            (owned.revenue_stops.clone(), None, None)
                        };

                    let mut spans = vec![
                        marker,
                        Span::raw(format!(
                            "{} dist={} last=${} {}",
                            owned.name,
                            format_distance(&owned.distance),
                            owned.last_revenue,
                            price
                        )),
                        Span::raw("  stops: "),
                    ];

                    if display_values.is_empty() {
                        spans.push(Span::raw("[--]"));
                    } else {
                        for (sidx, value) in display_values.iter().enumerate() {
                            if sidx > 0 {
                                spans.push(Span::raw(" + "));
                            }
                            let label = if Some(sidx) == active_cursor {
                                pending_input
                                    .as_deref()
                                    .map(|pending| pending.to_string())
                                    .unwrap_or_else(|| value.to_string())
                            } else {
                                value.to_string()
                            };
                            let text = format!("[{}]", label);
                            let span = if Some(sidx) == active_cursor {
                                Span::styled(text, Style::default().fg(self.theme.accent))
                            } else {
                                Span::raw(text)
                            };
                            spans.push(span);
                        }
                    }

                    let stop_count = display_values.len();
                    let limit = state.train_stop_limit_for(&owned.name);
                    let usage_text = if let Some(limit) = limit {
                        format!("  used {stop_count}/{limit}")
                    } else {
                        format!("  used {stop_count}")
                    };
                    let over_limit = limit.map(|limit| stop_count > limit).unwrap_or(false);
                    let usage_style = if over_limit {
                        Style::default().fg(self.theme.warning)
                    } else {
                        Style::default()
                    };
                    spans.push(Span::styled(usage_text, usage_style));

                    lines.push(Line::from(spans));
                }
            }
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from("No corporation selected"));
        }

        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title("Status");
        let primary = if self.state.mode == Mode::Filter {
            format!("Filter: {}", self.state.filter)
        } else {
            self.state.status.clone()
        };
        let secondary = format!("Saves tracked: {}  (auto-save enabled)", self.saves.len());
        let paragraph = Paragraph::new(vec![Line::from(primary), Line::from(secondary)])
            .block(block)
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_banner(&self, frame: &mut Frame, area: Rect, lines: &[String]) {
        let content: Vec<Line> = lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default().fg(self.theme.accent),
                ))
            })
            .collect();
        let paragraph = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title("Game Title"))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor()?;
    Ok(())
}

fn spawn_input_thread(sender: mpsc::Sender<AppEvent>) {
    thread::spawn(move || loop {
        match event::poll(TICK_RATE) {
            Ok(true) => match event::read() {
                Ok(evt) => {
                    if sender.blocking_send(AppEvent::Input(evt)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {
                if sender.blocking_send(AppEvent::Tick).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    });
}

struct UiState {
    all_games: Vec<GameInfo>,
    filtered: Vec<GameInfo>,
    cursor: usize,
    offset: usize,
    list_height: usize,
    filter: String,
    status: String,
    show_banner: bool,
    mode: Mode,
    should_quit: bool,
    menu_cursor: usize,
    continue_cursor: usize,
    continue_offset: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            all_games: Vec::new(),
            filtered: Vec::new(),
            cursor: 0,
            offset: 0,
            list_height: 1,
            filter: String::new(),
            status: "Ready".to_string(),
            show_banner: true,
            mode: Mode::Browse,
            should_quit: false,
            menu_cursor: 0,
            continue_cursor: 0,
            continue_offset: 0,
        }
    }
}

impl UiState {
    fn set_games(&mut self, games: Vec<GameInfo>) {
        self.all_games = games;
    }

    fn apply_filter(&mut self) {
        if self.filter.trim().is_empty() {
            self.filtered = self.all_games.clone();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered = self
                .all_games
                .iter()
                .filter(|game| game_matches(game, &needle))
                .cloned()
                .collect();
        }
        self.cursor = 0;
        self.offset = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let mut idx = self.cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.cursor = idx as usize;
        self.ensure_cursor_visible();
    }

    fn move_to(&mut self, index: usize) {
        if self.filtered.is_empty() {
            return;
        }
        self.cursor = index.min(self.filtered.len() - 1);
        self.ensure_cursor_visible();
    }

    fn move_to_end(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.cursor = self.filtered.len() - 1;
        self.ensure_cursor_visible();
    }

    fn page_down(&mut self) {
        if self.filtered.is_empty() || self.list_height == 0 {
            return;
        }
        let delta = self.list_height.min(self.filtered.len());
        self.move_cursor(delta as isize);
    }

    fn page_up(&mut self) {
        if self.filtered.is_empty() || self.list_height == 0 {
            return;
        }
        let delta = self.list_height.min(self.filtered.len());
        self.move_cursor(-(delta as isize));
    }

    fn visible_games(&self, height: usize) -> &[GameInfo] {
        if self.filtered.is_empty() {
            return &[];
        }
        let end = (self.offset + height).min(self.filtered.len());
        &self.filtered[self.offset..end]
    }

    fn current_game(&self) -> Option<&GameInfo> {
        self.filtered.get(self.cursor)
    }

    fn select_game(&mut self, game_id: &str) -> bool {
        if self.all_games.is_empty() {
            return false;
        }

        if let Some(pos) = self.filtered.iter().position(|game| game.id == game_id) {
            self.cursor = pos;
            self.ensure_cursor_visible();
            return true;
        }

        if let Some(pos) = self.all_games.iter().position(|game| game.id == game_id) {
            self.filter.clear();
            self.apply_filter();
            if let Some(filtered_pos) = self.filtered.iter().position(|game| game.id == game_id) {
                self.cursor = filtered_pos;
                self.ensure_cursor_visible();
                return true;
            }
            // fallback: set cursor near original position based on full list
            self.cursor = pos.min(self.filtered.len().saturating_sub(1));
            self.ensure_cursor_visible();
        }

        false
    }

    fn set_status(&mut self, message: String) {
        self.status = message;
    }

    fn clamp_cursor(&mut self) {
        if self.filtered.is_empty() {
            self.cursor = 0;
            self.offset = 0;
        } else if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len() - 1;
        }
    }

    fn ensure_cursor_visible(&mut self) {
        if self.filtered.is_empty() || self.list_height == 0 {
            self.offset = 0;
            return;
        }
        let height = self.list_height;
        let max_offset = self.filtered.len().saturating_sub(height);

        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }

        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }

    fn move_menu_cursor(&mut self, delta: isize) {
        let options = 3isize;
        let mut idx = self.menu_cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= options {
            idx = options - 1;
        }
        self.menu_cursor = idx as usize;
    }

    fn move_continue_cursor(&mut self, delta: isize, total: usize, visible: usize) {
        if total == 0 {
            self.continue_cursor = 0;
            self.continue_offset = 0;
            return;
        }
        let mut idx = self.continue_cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= total as isize {
            idx = (total as isize) - 1;
        }
        self.continue_cursor = idx as usize;
        let visible = visible.max(1);
        if self.continue_cursor < self.continue_offset {
            self.continue_offset = self.continue_cursor;
        } else if self.continue_cursor >= self.continue_offset + visible {
            self.continue_offset = self.continue_cursor + 1 - visible;
        }
        let max_offset = total.saturating_sub(visible);
        if self.continue_offset > max_offset {
            self.continue_offset = max_offset;
        }
    }
}

fn game_matches(game: &GameInfo, needle: &str) -> bool {
    let candidates = [
        game.id.to_lowercase(),
        game.title.to_lowercase(),
        game.subtitle.clone().unwrap_or_default().to_lowercase(),
        game.designer.clone().unwrap_or_default().to_lowercase(),
        game.location.clone().unwrap_or_default().to_lowercase(),
    ];
    candidates.iter().any(|value| value.contains(needle))
}

#[derive(Clone, Serialize, Deserialize)]
struct PlayState {
    session: GameSession,
    corporation_index: usize,
    market_cursor: (usize, usize),
    mode: PlayMode,
    #[serde(default)]
    title_banner_visible: bool,
    train_focus: TrainFocus,
    train_pool_cursor: usize,
    train_owned_cursor: usize,
    train_run: Option<TrainRunState>,
    market_row_offset: usize,
    market_col_offset: usize,
    market_view_rows: usize,
    market_view_cols: usize,
    train_purchase_modal: Option<TrainPurchaseModal>,
    phases: Vec<PhaseInfo>,
    phase_index: usize,
    phase_rounds: Vec<Vec<OperatingRound>>,
    revenue_view: bool,
    revenue_cursor_corp: usize,
    revenue_cursor_or: usize,
    revenue_row_offset: usize,
    revenue_col_offset: usize,
    revenue_view_rows: usize,
    revenue_view_cols: usize,
    revenue_input: Option<String>,
}

impl PlayState {
    fn new(session: GameSession) -> Self {
        let market_cursor = default_market_cursor(&session);
        let phases = if session.phases.is_empty() {
            vec![PhaseInfo {
                name: "Phase".to_string(),
                operating_rounds: 2,
                raw: Value::Null,
            }]
        } else {
            session
                .phases
                .iter()
                .map(|value| PhaseInfo::from_value(value))
                .collect()
        };
        let corp_count = session.corporations.len();
        let mut phase_rounds = Vec::new();
        for phase in &phases {
            let count = phase.operating_rounds.max(1);
            let mut rounds = Vec::new();
            for _ in 0..count {
                rounds.push(OperatingRound {
                    revenues: vec![0; corp_count],
                });
            }
            phase_rounds.push(rounds);
        }
        let mut state = Self {
            session,
            corporation_index: 0,
            market_cursor,
            mode: PlayMode::Idle,
            title_banner_visible: true,
            train_focus: TrainFocus::Pool,
            train_pool_cursor: 0,
            train_owned_cursor: 0,
            train_run: None,
            market_row_offset: 0,
            market_col_offset: 0,
            market_view_rows: 0,
            market_view_cols: 0,
            train_purchase_modal: None,
            phases,
            phase_index: 0,
            phase_rounds,
            revenue_view: false,
            revenue_cursor_corp: 0,
            revenue_cursor_or: 0,
            revenue_row_offset: 0,
            revenue_col_offset: 0,
            revenue_view_rows: 1,
            revenue_view_cols: 1,
            revenue_input: None,
        };
        state.bootstrap_revenue_from_corporations();
        state
    }

    fn phase_count(&self) -> usize {
        self.phases.len()
    }

    fn current_phase_index(&self) -> usize {
        self.phase_index.min(self.phases.len().saturating_sub(1))
    }

    fn current_phase(&self) -> Option<&PhaseInfo> {
        self.phases.get(self.current_phase_index())
    }

    fn phase_label(&self) -> String {
        self.current_phase()
            .map(|phase| phase.name.clone())
            .unwrap_or_else(|| "Phase".to_string())
    }

    fn set_phase_index(&mut self, index: usize) {
        if self.phases.is_empty() {
            self.phase_index = 0;
            return;
        }
        let clamped = index.min(self.phases.len() - 1);
        if clamped != self.phase_index {
            self.phase_index = clamped;
            self.ensure_phase_round_capacity(self.phase_index);
            self.revenue_cursor_or = self
                .revenue_cursor_or
                .min(self.current_phase_rounds().len().saturating_sub(1));
            if self.current_phase_rounds().is_empty() {
                self.revenue_cursor_or = 0;
            }
            self.revenue_row_offset = 0;
            self.revenue_col_offset = 0;
            self.ensure_revenue_cursor_visible();
        }
    }

    fn move_phase(&mut self, delta: isize) {
        if self.phases.is_empty() {
            return;
        }
        let len = self.phases.len() as isize;
        let mut idx = self.phase_index as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.set_phase_index(idx as usize);
    }

    fn ensure_phase_round_capacity(&mut self, phase_idx: usize) {
        while self.phase_rounds.len() <= phase_idx {
            self.phase_rounds.push(Vec::new());
        }
        let corp_count = self.session.corporations.len();
        let desired = self
            .phases
            .get(phase_idx)
            .map(|phase| phase.operating_rounds.max(1))
            .unwrap_or(1);
        let rounds = &mut self.phase_rounds[phase_idx];
        if rounds.is_empty() {
            rounds.extend((0..desired).map(|_| OperatingRound::new(corp_count)));
        } else if rounds.len() < desired {
            for _ in rounds.len()..desired {
                rounds.push(OperatingRound::new(corp_count));
            }
        }
        for round in rounds.iter_mut() {
            if round.revenues.len() < corp_count {
                round.revenues.resize(corp_count, 0);
            }
        }
    }

    fn bootstrap_revenue_from_corporations(&mut self) {
        if self.session.corporations.is_empty() {
            return;
        }
        let phase_idx = self.current_phase_index();
        self.ensure_phase_round_capacity(phase_idx);
        if self.current_phase_rounds().is_empty() {
            return;
        }
        let max_index = self.current_phase_rounds().len().saturating_sub(1);
        let target_or = self.revenue_cursor_or.min(max_index);
        self.revenue_cursor_or = target_or;
        let revenues: Vec<i32> = self
            .session
            .corporations
            .iter()
            .map(|corp| corp.last_revenue)
            .collect();
        for (index, revenue) in revenues.into_iter().enumerate() {
            self.set_revenue_value(index, target_or, revenue);
        }
        self.ensure_revenue_cursor_visible();
    }

    fn current_phase_rounds(&self) -> &[OperatingRound] {
        let idx = self.current_phase_index();
        self.phase_rounds
            .get(idx)
            .map(|rounds| rounds.as_slice())
            .unwrap_or(&[])
    }

    fn current_phase_rounds_mut(&mut self) -> &mut Vec<OperatingRound> {
        let idx = self.current_phase_index();
        self.ensure_phase_round_capacity(idx);
        &mut self.phase_rounds[idx]
    }

    fn add_operating_round(&mut self) {
        let corp_count = self.session.corporations.len();
        if corp_count == 0 {
            return;
        }
        let rounds = self.current_phase_rounds_mut();
        rounds.push(OperatingRound::new(corp_count));
        self.revenue_cursor_or = rounds.len().saturating_sub(1);
        self.ensure_revenue_cursor_visible();
    }

    fn revenue_dimensions(&self) -> (usize, usize) {
        let rows = self.session.corporations.len();
        let cols = self.current_phase_rounds().len();
        (rows, cols)
    }

    fn revenue_view_enabled(&self) -> bool {
        self.revenue_view
    }

    fn toggle_revenue_view(&mut self) -> bool {
        self.title_banner_visible = false;
        self.revenue_view = !self.revenue_view;
        if self.revenue_view {
            self.ensure_phase_round_capacity(self.current_phase_index());
            self.sync_revenue_cursor_with_corp();
            self.revenue_row_offset = 0;
            self.revenue_col_offset = 0;
            if self.current_phase_rounds().is_empty() {
                self.revenue_cursor_or = 0;
            } else {
                self.revenue_cursor_or = self
                    .revenue_cursor_or
                    .min(self.current_phase_rounds().len().saturating_sub(1));
            }
            if self.revenue_view_rows == 0 || self.revenue_view_cols == 0 {
                self.revenue_view_rows = 1;
                self.revenue_view_cols = 1;
            }
            self.ensure_revenue_cursor_visible();
        }
        self.revenue_view
    }

    fn sync_revenue_cursor_with_corp(&mut self) {
        let corp_index = self
            .corporation_index
            .min(self.session.corporations.len().saturating_sub(1));
        self.revenue_cursor_corp = corp_index;
    }

    fn ensure_revenue_cursor_visible(&mut self) {
        let (total_rows, total_cols) = self.revenue_dimensions();
        if total_rows == 0 || total_cols == 0 {
            self.revenue_cursor_corp = 0;
            self.revenue_cursor_or = 0;
            self.revenue_row_offset = 0;
            self.revenue_col_offset = 0;
            return;
        }

        if self.revenue_cursor_corp >= total_rows {
            self.revenue_cursor_corp = total_rows - 1;
        }
        if self.revenue_cursor_or >= total_cols {
            self.revenue_cursor_or = total_cols - 1;
        }

        if self.revenue_cursor_corp < self.revenue_row_offset {
            self.revenue_row_offset = self.revenue_cursor_corp;
        } else if self.revenue_cursor_corp
            >= self.revenue_row_offset + self.revenue_view_rows.max(1)
        {
            self.revenue_row_offset = self.revenue_cursor_corp + 1 - self.revenue_view_rows.max(1);
        }

        if self.revenue_cursor_or < self.revenue_col_offset {
            self.revenue_col_offset = self.revenue_cursor_or;
        } else if self.revenue_cursor_or >= self.revenue_col_offset + self.revenue_view_cols.max(1)
        {
            self.revenue_col_offset = self.revenue_cursor_or + 1 - self.revenue_view_cols.max(1);
        }

        let max_row_offset = total_rows.saturating_sub(self.revenue_view_rows.max(1));
        self.revenue_row_offset = self.revenue_row_offset.min(max_row_offset);

        let max_col_offset = total_cols.saturating_sub(self.revenue_view_cols.max(1));
        self.revenue_col_offset = self.revenue_col_offset.min(max_col_offset);
    }

    fn set_revenue_view_dims(&mut self, rows: usize, cols: usize) {
        self.revenue_view_rows = rows.max(1);
        self.revenue_view_cols = cols.max(1);
        self.ensure_revenue_cursor_visible();
    }

    fn revenue_cursor(&self) -> (usize, usize) {
        (self.revenue_cursor_corp, self.revenue_cursor_or)
    }

    fn move_revenue_cursor(&mut self, delta_row: isize, delta_col: isize) {
        self.ensure_phase_round_capacity(self.current_phase_index());
        let (row_count, col_count) = self.revenue_dimensions();
        if row_count == 0 || col_count == 0 {
            return;
        }

        let mut row = self.revenue_cursor_corp as isize + delta_row;
        if row < 0 {
            row = 0;
        } else if row >= row_count as isize {
            row = row_count as isize - 1;
        }

        let mut col = self.revenue_cursor_or as isize + delta_col;
        if col < 0 {
            col = 0;
        } else if col >= col_count as isize {
            col = col_count as isize - 1;
        }

        self.revenue_cursor_corp = row as usize;
        self.revenue_cursor_or = col as usize;
        self.corporation_index = self.revenue_cursor_corp;
        self.ensure_revenue_cursor_visible();
    }

    fn advance_operating_round(&mut self) -> bool {
        let phase_idx = self.current_phase_index();
        self.ensure_phase_round_capacity(phase_idx);
        let total = self.current_phase_rounds().len();
        if total == 0 || self.revenue_cursor_or + 1 >= total {
            return false;
        }
        self.revenue_cursor_or += 1;
        self.ensure_revenue_cursor_visible();
        true
    }

    fn retreat_operating_round(&mut self) -> bool {
        let phase_idx = self.current_phase_index();
        self.ensure_phase_round_capacity(phase_idx);
        if self.revenue_cursor_or == 0 {
            return false;
        }
        self.revenue_cursor_or -= 1;
        self.ensure_revenue_cursor_visible();
        true
    }

    fn operating_round_summary(&self) -> String {
        let total = self.current_phase_rounds().len();
        if total == 0 {
            return "OR 0 of 0".to_string();
        }
        let current = self.revenue_cursor_or.min(total - 1) + 1;
        format!("OR {} of {}", current, total)
    }

    fn set_revenue_value(&mut self, row: usize, col: usize, value: i32) {
        let corp_count = self.session.corporations.len();
        if corp_count == 0 || row >= corp_count {
            return;
        }
        let phase_idx = self.current_phase_index();
        self.ensure_phase_round_capacity(phase_idx);
        if self.phase_rounds.len() <= phase_idx {
            return;
        }
        let rounds = &mut self.phase_rounds[phase_idx];
        if col >= rounds.len() {
            let missing = col + 1 - rounds.len();
            for _ in 0..missing {
                rounds.push(OperatingRound::new(corp_count));
            }
        }
        if let Some(round) = rounds.get_mut(col) {
            if round.revenues.len() < corp_count {
                round.revenues.resize(corp_count, 0);
            }
            round.revenues[row] = value;
        }
    }

    fn current_revenue_value(&self) -> Option<i32> {
        let (row, col) = self.revenue_cursor();
        let phase_idx = self.current_phase_index();
        self.phase_rounds
            .get(phase_idx)
            .and_then(|rounds| rounds.get(col))
            .and_then(|round| round.revenues.get(row))
            .copied()
    }

    fn current_revenue_base(&self) -> Option<i32> {
        self.current_corporation().map(|corp| corp.last_revenue)
    }

    fn set_current_revenue_value(&mut self, value: i32) {
        let (row, col) = self.revenue_cursor();
        self.set_revenue_value(row, col, value.max(0));
    }

    fn adjust_current_revenue_value(&mut self, delta: i32) {
        let current = self.current_revenue_value().unwrap_or(0);
        let updated = (current + delta).max(0);
        self.set_current_revenue_value(updated);
    }

    fn current_revenue_context(&self) -> Option<(&Corporation, usize)> {
        let corp = self.current_corporation()?;
        Some((corp, self.revenue_cursor_or))
    }

    fn mode(&self) -> PlayMode {
        self.mode
    }

    fn should_show_title_banner(&self) -> bool {
        self.title_banner_visible && !self.revenue_view_enabled()
    }

    fn consume_title_banner(&mut self) {
        self.title_banner_visible = false;
    }

    fn current_corporation(&self) -> Option<&Corporation> {
        self.session.corporations.get(self.corporation_index)
    }

    fn current_corporation_mut(&mut self) -> Option<&mut Corporation> {
        self.session.corporations.get_mut(self.corporation_index)
    }

    fn move_corporation(&mut self, delta: isize) {
        let len = self.session.corporations.len();
        if len == 0 {
            return;
        }
        let len = len as isize;
        let mut idx = self.corporation_index as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.corporation_index = idx as usize;
        self.sync_revenue_cursor_with_corp();
        self.ensure_revenue_cursor_visible();
    }

    fn market_cursor(&self) -> (usize, usize) {
        self.market_cursor
    }

    fn current_market_cell(&self) -> Option<&MarketCell> {
        let (row, col) = self.market_cursor;
        self.session.market_cell(row, col)
    }

    fn move_market_cursor(&mut self, row_delta: isize, col_delta: isize) {
        if row_delta == 0 && col_delta == 0 {
            return;
        }
        if self.session.market.is_empty() {
            return;
        }

        let row_count = self.session.market.len();
        if row_count == 1 {
            let row_len = self.session.market[0].len();
            if row_len == 0 {
                return;
            }
            if col_delta != 0 {
                let len = row_len as isize;
                let mut col = self.market_cursor.1 as isize + col_delta;
                col = ((col % len) + len) % len;
                self.market_cursor = (0, col as usize);
                self.ensure_market_cursor_visible();
            }
            return;
        }

        let mut row = self.market_cursor.0 as isize;
        let mut col = self.market_cursor.1 as isize;
        let mut attempts = 0usize;
        let max_attempts = row_count
            .saturating_mul(self.max_market_columns().max(1))
            .max(1);
        loop {
            row += row_delta;
            col += col_delta;
            attempts += 1;
            if row < 0 {
                break;
            }
            let row_usize = row as usize;
            if row_usize >= row_count {
                break;
            }
            let row_len = self.session.market[row_usize].len();
            if row_len == 0 {
                continue;
            }
            if col as usize >= row_len {
                if row_delta == 0 {
                    break;
                } else {
                    col = (row_len - 1) as isize;
                }
            }
            if col < 0 {
                if row_delta == 0 {
                    break;
                } else {
                    col = 0;
                }
            }
            let col_usize = col as usize;
            if let Some(cell) = self.session.market_cell(row_usize, col_usize) {
                if self.mode == PlayMode::ParSelect && !self.is_par_cell(cell.row, cell.col) {
                    continue;
                }
                self.market_cursor = (cell.row, cell.col);
                self.ensure_market_cursor_visible();
                break;
            }
            if attempts > max_attempts {
                break;
            }
        }
    }

    fn enter_par_select(&mut self) -> bool {
        let corp_sym = self.current_corporation().map(|corp| corp.sym.clone());
        debug!(
            ?corp_sym,
            current_cursor = ?self.market_cursor,
            "enter_par_select start"
        );
        if let Some(corp) = self.current_corporation() {
            if let Some(pos) = &corp.market_position {
                if self.is_par_cell(pos.row, pos.col) {
                    self.market_cursor = (pos.row, pos.col);
                }
            }
        }

        if !self.is_par_cell(self.market_cursor.0, self.market_cursor.1) {
            if let Some(cell) = self.session.par_cells.first() {
                self.market_cursor = (cell.row, cell.col);
            } else if let Some(cell) = self.session.market_cells.first() {
                self.market_cursor = (cell.row, cell.col);
            } else {
                debug!(?corp_sym, "enter_par_select: no market cells available");
                return false;
            }
        }

        self.mode = PlayMode::ParSelect;
        debug!(
            ?corp_sym,
            cursor = ?self.market_cursor,
            "enter_par_select success"
        );
        self.ensure_market_cursor_visible();
        true
    }

    fn enter_price_select(&mut self) {
        debug!(
            cursor = ?self.market_cursor,
            "enter_price_select start"
        );
        if let Some(corp) = self.current_corporation() {
            if let Some(pos) = &corp.market_position {
                self.market_cursor = (pos.row, pos.col);
            }
        }
        self.mode = PlayMode::PriceSelect;
        debug!(
            cursor = ?self.market_cursor,
            "enter_price_select success"
        );
        self.ensure_market_cursor_visible();
    }

    fn exit_market(&mut self) {
        debug!(previous_mode = ?self.mode, "exit_market called");
        self.mode = PlayMode::Idle;
    }

    fn apply_par_selection(&mut self) -> Option<i32> {
        let cell = self.current_market_cell()?.clone();
        debug!(
            cursor = ?self.market_cursor,
            row = cell.row,
            col = cell.col,
            raw = %cell.raw,
            "apply_par_selection start"
        );
        let value = cell.value.unwrap_or(0);
        {
            let corp = self.current_corporation_mut()?;
            corp.par_value = Some(value);
            corp.market_position = Some(cell_to_position(&cell));
            debug!(sym = %corp.sym, value, "apply_par_selection updated corporation");
        }
        self.mode = PlayMode::Idle;
        self.ensure_market_cursor_visible();
        debug!(
            row = cell.row,
            col = cell.col,
            value,
            "apply_par_selection completed"
        );
        Some(value)
    }

    fn apply_price_selection(&mut self) -> Option<MarketPosition> {
        let cell = self.current_market_cell()?.clone();
        let position = cell_to_position(&cell);
        {
            let corp = self.current_corporation_mut()?;
            corp.market_position = Some(position.clone());
        }
        self.mode = PlayMode::Idle;
        self.ensure_market_cursor_visible();
        Some(position)
    }

    fn is_par_cell(&self, row: usize, col: usize) -> bool {
        self.session
            .par_cells
            .iter()
            .any(|cell| cell.row == row && cell.col == col)
    }

    fn enter_train_manage(&mut self) -> bool {
        let corp_has_trains = self
            .current_corporation()
            .map(|corp| !corp.trains.is_empty())
            .unwrap_or(false);
        let pool_has_trains = !self.available_trains().is_empty();

        if !corp_has_trains && !pool_has_trains {
            return false;
        }

        self.mode = PlayMode::TrainManage;
        self.train_run = None;
        self.train_owned_cursor = 0;
        self.sync_pool_cursor();
        if corp_has_trains {
            self.train_focus = TrainFocus::Owned;
        } else {
            self.train_focus = TrainFocus::Pool;
        }
        true
    }

    fn exit_train_manage(&mut self) {
        self.mode = PlayMode::Idle;
        self.train_focus = TrainFocus::Pool;
        self.train_run = None;
    }

    fn train_focus(&self) -> TrainFocus {
        self.train_focus
    }

    fn toggle_train_focus(&mut self) {
        match self.train_focus {
            TrainFocus::Owned => {
                if !self.focus_pool_internal() {
                    self.focus_owned_internal();
                }
            }
            TrainFocus::Pool => {
                if !self.focus_owned_internal() {
                    self.focus_pool_internal();
                }
            }
        }
    }

    fn focus_owned(&mut self) {
        if !self.focus_owned_internal() {
            self.focus_pool_internal();
        }
    }

    fn focus_pool(&mut self) {
        if !self.focus_pool_internal() {
            self.focus_owned_internal();
        }
    }

    fn focus_owned_internal(&mut self) -> bool {
        if self
            .current_corporation()
            .map(|corp| !corp.trains.is_empty())
            .unwrap_or(false)
        {
            self.train_focus = TrainFocus::Owned;
            let len = self
                .current_corporation()
                .map(|corp| corp.trains.len())
                .unwrap_or(0);
            if len > 0 {
                self.train_owned_cursor = self.train_owned_cursor.min(len - 1);
            } else {
                self.train_owned_cursor = 0;
            }
            true
        } else {
            false
        }
    }

    fn focus_pool_internal(&mut self) -> bool {
        if self.available_trains().is_empty() {
            false
        } else {
            self.train_focus = TrainFocus::Pool;
            self.sync_pool_cursor();
            true
        }
    }

    fn set_market_view(&mut self, rows: usize, cols: usize) {
        let total_rows = self.session.market.len();
        if total_rows > 0 && total_rows <= rows {
            self.market_view_rows = total_rows.max(1);
            self.market_row_offset = 0;
        } else {
            self.market_view_rows = rows.max(1);
        }

        let total_cols = self.max_market_columns();
        if total_cols > 0 && total_cols <= cols {
            self.market_view_cols = total_cols.max(1);
            self.market_col_offset = 0;
        } else {
            self.market_view_cols = cols.max(1);
        }

        self.clamp_market_offsets();
        self.ensure_market_cursor_visible();
    }

    fn market_row_offset(&self) -> usize {
        self.market_row_offset
    }

    fn market_col_offset(&self) -> usize {
        self.market_col_offset
    }

    fn market_view_rows(&self) -> usize {
        self.market_view_rows.max(1)
    }

    fn market_view_cols(&self) -> usize {
        self.market_view_cols.max(1)
    }

    fn is_purchase_modal_active(&self) -> bool {
        self.train_purchase_modal.is_some()
    }

    fn open_train_purchase_modal(&mut self) {
        if self.available_trains().is_empty() {
            self.train_purchase_modal = None;
        } else {
            let cursor = self
                .available_trains()
                .iter()
                .position(|(idx, _, _)| *idx == self.pool_train_cursor())
                .unwrap_or(0);
            self.train_purchase_modal = Some(TrainPurchaseModal { cursor, offset: 0 });
        }
    }

    fn close_train_purchase_modal(&mut self) {
        self.train_purchase_modal = None;
    }

    fn move_purchase_modal_cursor(&mut self, delta: isize) {
        let len = self.available_trains().len();
        if len == 0 {
            if let Some(modal) = &mut self.train_purchase_modal {
                modal.cursor = 0;
                modal.offset = 0;
            }
            return;
        }
        if let Some(modal) = &mut self.train_purchase_modal {
            let len = len as isize;
            let mut idx = modal.cursor as isize + delta;
            if idx < 0 {
                idx = 0;
            } else if idx >= len {
                idx = len - 1;
            }
            modal.cursor = idx as usize;
            if modal.cursor < modal.offset {
                modal.offset = modal.cursor;
            }
        }
    }

    fn train_type_for(&self, name: &str) -> Option<&TrainType> {
        self.session.train_types.iter().find(|ty| ty.name == name)
    }

    fn train_stop_limit_for(&self, name: &str) -> Option<usize> {
        let train = self.train_type_for(name)?;
        stop_limit_from_distance(&train.distance)
    }

    fn owned_train_cursor(&self) -> usize {
        self.train_owned_cursor
    }

    fn set_owned_cursor(&mut self, index: usize) {
        if let Some(len) = self
            .current_corporation()
            .map(|corp| corp.trains.len())
            .filter(|len| *len > 0)
        {
            self.train_owned_cursor = index.min(len - 1);
        } else {
            self.train_owned_cursor = 0;
        }
    }

    fn pool_train_cursor(&self) -> usize {
        self.train_pool_cursor
    }

    fn sync_pool_cursor(&mut self) {
        let len = self.available_trains().len();
        if len == 0 {
            self.train_pool_cursor = 0;
        } else if self.train_pool_cursor >= len {
            self.train_pool_cursor = len - 1;
        }
    }

    fn move_train_selection(&mut self, delta: isize) {
        match self.train_focus {
            TrainFocus::Owned => self.move_owned_cursor(delta),
            TrainFocus::Pool => self.move_pool_cursor(delta),
        }
    }

    fn move_owned_cursor(&mut self, delta: isize) {
        let len = self
            .current_corporation()
            .map(|corp| corp.trains.len())
            .unwrap_or(0);
        if len == 0 {
            self.train_owned_cursor = 0;
            return;
        }
        let len = len as isize;
        let mut idx = self.train_owned_cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.train_owned_cursor = idx as usize;
    }

    fn move_pool_cursor(&mut self, delta: isize) {
        let len = self.available_trains().len();
        if len == 0 {
            self.train_pool_cursor = 0;
            return;
        }
        let len = len as isize;
        let mut idx = self.train_pool_cursor as isize + delta;
        if idx < 0 {
            idx = 0;
        } else if idx >= len {
            idx = len - 1;
        }
        self.train_pool_cursor = idx as usize;
    }

    fn available_trains(&self) -> Vec<(usize, &TrainType, i64)> {
        self.session
            .train_types
            .iter()
            .enumerate()
            .filter_map(|(idx, ty)| {
                let remaining = self
                    .session
                    .train_pool
                    .get(idx)
                    .map(|entry| entry.remaining)
                    .unwrap_or(0);
                if remaining > 0 {
                    Some((idx, ty, remaining))
                } else {
                    None
                }
            })
            .collect()
    }

    fn purchase_available_train(&mut self, selection: usize) -> Option<CorporationTrain> {
        let available = self.available_trains();
        let (idx, _, _) = *available.get(selection)?;
        drop(available);

        let ty = self.session.train_types.get(idx)?.clone();
        let pool_entry = self.session.train_pool.get_mut(idx)?;
        if pool_entry.remaining <= 0 {
            return None;
        }
        pool_entry.remaining -= 1;
        Some(CorporationTrain {
            name: ty.name.clone(),
            distance: ty.distance.clone(),
            price: ty.price,
            revenue_stops: Vec::new(),
            last_revenue: 0,
        })
    }

    fn current_owned_train(&self) -> Option<&CorporationTrain> {
        let corp = self.current_corporation()?;
        corp.trains.get(self.train_owned_cursor)
    }

    fn rust_selected_train(&mut self) -> Option<CorporationTrain> {
        if self.train_focus != TrainFocus::Owned {
            return None;
        }
        let cursor = self.train_owned_cursor;
        let remaining: usize;
        let removed = {
            let corp = self.current_corporation_mut()?;
            if corp.trains.is_empty() {
                return None;
            }
            let idx = cursor.min(corp.trains.len() - 1);
            let removed = corp.trains.remove(idx);
            Self::update_corporation_revenue(corp);
            remaining = corp.trains.len();
            removed
        };

        if remaining == 0 {
            self.train_owned_cursor = 0;
            self.focus_pool_internal();
        } else if self.train_owned_cursor >= remaining {
            self.train_owned_cursor = remaining - 1;
        }
        Some(removed)
    }

    fn update_corporation_revenue(corp: &mut Corporation) {
        corp.last_revenue = corp.trains.iter().map(|train| train.last_revenue).sum();
    }

    fn train_run_state(&self) -> Option<&TrainRunState> {
        self.train_run.as_ref()
    }

    fn train_run_state_mut(&mut self) -> Option<&mut TrainRunState> {
        self.train_run.as_mut()
    }

    fn train_run_move_cursor(&mut self, delta: isize) {
        if let Some(run) = self.train_run_state_mut() {
            run.move_cursor(delta);
        }
    }

    fn train_run_append_digit(&mut self, ch: char) {
        if let Some(run) = self.train_run_state_mut() {
            run.append_digit(ch);
        }
    }

    fn train_run_backspace(&mut self) {
        if let Some(run) = self.train_run_state_mut() {
            run.backspace();
        }
    }

    fn train_run_commit_input(&mut self) {
        if let Some(run) = self.train_run_state_mut() {
            run.commit_input();
        }
    }

    fn train_run_add_stop(&mut self) -> bool {
        let Some(train_name) = self.train_run_state().map(|run| run.train_name.clone()) else {
            return false;
        };
        let limit = self.train_stop_limit_for(&train_name);
        if let Some(run) = self.train_run_state_mut() {
            if limit
                .map(|limit| run.values.len() >= limit)
                .unwrap_or(false)
            {
                return false;
            }
            run.add_stop();
            true
        } else {
            false
        }
    }

    fn train_run_remove_stop(&mut self) {
        if let Some(run) = self.train_run_state_mut() {
            run.remove_stop();
        }
    }

    fn train_run_clear_current(&mut self) {
        if let Some(run) = self.train_run_state_mut() {
            run.clear_current();
        }
    }

    fn start_train_run(&mut self) -> bool {
        let Some((train_index, train_name, stops)) = self.current_corporation().and_then(|corp| {
            if corp.trains.is_empty() {
                None
            } else {
                let index = self.train_owned_cursor.min(corp.trains.len() - 1);
                let train = &corp.trains[index];
                Some((index, train.name.clone(), train.revenue_stops.clone()))
            }
        }) else {
            return false;
        };
        self.train_focus = TrainFocus::Owned;
        self.train_run = Some(TrainRunState::new(train_index, train_name, stops));
        self.mode = PlayMode::TrainRun;
        true
    }

    fn cancel_train_run(&mut self) {
        if let Some(run) = self.train_run_state_mut() {
            run.commit_input();
        }
        self.train_run = None;
        self.mode = PlayMode::TrainManage;
        self.focus_owned_internal();
    }

    fn apply_train_run(&mut self) -> Option<(String, String, i32)> {
        let mut run_state = self.train_run.take()?;
        run_state.commit_input();
        let total = run_state.total();
        let train_name = run_state.train_name.clone();
        let (corp_sym, corp_trains_len, corp_last_revenue) = {
            let corp = self.current_corporation_mut()?;
            if run_state.train_index >= corp.trains.len() {
                self.mode = PlayMode::TrainManage;
                return None;
            }
            let corp_sym = corp.sym.clone();
            if let Some(train) = corp.trains.get_mut(run_state.train_index) {
                train.revenue_stops = run_state.values.clone();
                train.last_revenue = total;
            }
            Self::update_corporation_revenue(corp);
            let corp_last_revenue = corp.last_revenue;
            let corp_trains_len = corp.trains.len();
            (corp_sym, corp_trains_len, corp_last_revenue)
        };
        let corp_count = self.session.corporations.len();
        let corp_index = self.corporation_index.min(corp_count.saturating_sub(1));
        self.train_owned_cursor = run_state.train_index.min(corp_trains_len.saturating_sub(1));
        self.mode = PlayMode::TrainManage;
        self.focus_owned_internal();
        self.set_revenue_value(corp_index, self.revenue_cursor_or, corp_last_revenue);
        self.revenue_cursor_corp = corp_index;
        self.ensure_revenue_cursor_visible();
        Some((corp_sym, train_name, total))
    }

    fn apply_revenue_action(
        &mut self,
        action: RevenueAction,
    ) -> Result<RevenueOutcome, RevenueError> {
        let (corp_sym, total, current_position) = {
            let corp = self
                .current_corporation()
                .ok_or(RevenueError::NoCorporation)?;
            let position = corp
                .market_position
                .clone()
                .ok_or(RevenueError::NoMarketPosition)?;
            (corp.sym.clone(), corp.last_revenue, position)
        };

        let desired_position = match action {
            RevenueAction::Dividend => self
                .offset_market_position(&current_position, (0, 1))
                .or_else(|| self.offset_market_position(&current_position, (-1, 0))),
            RevenueAction::Withhold => self
                .offset_market_position(&current_position, (0, -1))
                .or_else(|| self.offset_market_position(&current_position, (1, 0))),
        };
        let moved = desired_position.is_some();

        if let Some(new_pos) = desired_position.clone() {
            if let Some(corp) = self.current_corporation_mut() {
                corp.market_position = Some(new_pos);
            }
        }

        let position = self
            .current_corporation()
            .and_then(|corp| corp.market_position.clone())
            .unwrap_or(current_position);
        let price_label = display_price_label(&position.raw);

        debug!(
            sym = %corp_sym,
            total,
            action = ?action,
            price = %price_label,
            moved,
            "apply_revenue_action outcome"
        );

        Ok(RevenueOutcome {
            corp_sym,
            total,
            price_label,
            moved,
            action,
        })
    }

    fn offset_market_position(
        &self,
        position: &MarketPosition,
        delta: (isize, isize),
    ) -> Option<MarketPosition> {
        let row = position.row as isize + delta.0;
        let col = position.col as isize + delta.1;
        if row < 0 || col < 0 {
            return None;
        }
        let row = row as usize;
        let col = col as usize;
        self.session
            .market_cell(row, col)
            .map(|cell| cell_to_position(cell))
    }

    fn clamp_market_offsets(&mut self) {
        let total_rows = self.session.market.len();
        if total_rows <= 1 {
            self.market_row_offset = 0;
        } else if self.market_view_rows > 0 {
            let max_offset = total_rows.saturating_sub(self.market_view_rows);
            if self.market_row_offset > max_offset {
                self.market_row_offset = max_offset;
            }
        }

        let max_cols = self.max_market_columns();
        if max_cols == 0 {
            self.market_col_offset = 0;
        } else if self.market_view_cols > 0 {
            let max_offset = max_cols.saturating_sub(self.market_view_cols);
            if self.market_col_offset > max_offset {
                self.market_col_offset = max_offset;
            }
        }
    }

    fn ensure_market_cursor_visible(&mut self) {
        let total_rows = self.session.market.len();
        if total_rows == 0 {
            self.market_row_offset = 0;
            self.market_col_offset = 0;
            return;
        }

        if total_rows == 1 {
            self.market_row_offset = 0;
        } else if self.market_view_rows > 0 {
            if self.market_cursor.0 < self.market_row_offset {
                self.market_row_offset = self.market_cursor.0;
            } else if self.market_cursor.0 >= self.market_row_offset + self.market_view_rows {
                self.market_row_offset = self.market_cursor.0 + 1 - self.market_view_rows;
            }
            let max_offset = total_rows.saturating_sub(self.market_view_rows);
            if self.market_row_offset > max_offset {
                self.market_row_offset = max_offset;
            }
        }

        let max_cols = self.max_market_columns();
        if max_cols == 0 {
            self.market_col_offset = 0;
        } else if self.market_view_cols > 0 {
            if self.market_cursor.1 < self.market_col_offset {
                self.market_col_offset = self.market_cursor.1;
            } else if self.market_cursor.1 >= self.market_col_offset + self.market_view_cols {
                self.market_col_offset = self.market_cursor.1 + 1 - self.market_view_cols;
            }
            let max_offset = max_cols.saturating_sub(self.market_view_cols);
            if self.market_col_offset > max_offset {
                self.market_col_offset = max_offset;
            }
        }
    }

    fn max_market_columns(&self) -> usize {
        self.session
            .market
            .iter()
            .map(|row| row.len())
            .max()
            .unwrap_or(0)
    }

    fn train_run_context(&self) -> Option<(&Corporation, &CorporationTrain, &TrainRunState)> {
        let run = self.train_run.as_ref()?;
        let corp = self.current_corporation()?;
        let train = corp.trains.get(run.train_index)?;
        Some((corp, train, run))
    }
}

fn default_market_cursor(session: &GameSession) -> (usize, usize) {
    if let Some(cell) = session
        .par_cells
        .first()
        .or_else(|| session.market_cells.first())
    {
        return (cell.row, cell.col);
    }
    (0, 0)
}

fn cell_to_position(cell: &MarketCell) -> MarketPosition {
    MarketPosition {
        row: cell.row,
        col: cell.col,
        value: cell.value,
        raw: cell.raw.clone(),
    }
}

fn market_color(raw: &str, theme: &Theme) -> Color {
    let code = raw
        .chars()
        .find(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase());
    match code {
        Some('y') => theme.warning,
        Some('o') => theme.accent_alt,
        Some('b') => theme.success,
        Some('p') => theme.muted,
        _ => theme.primary_fg,
    }
}

fn sanitize_market_text(raw: &str) -> String {
    let filtered: String = raw.chars().filter(|c| !c.is_ascii_alphabetic()).collect();
    filtered.trim().to_string()
}

fn token_glyph(corp: &Corporation) -> String {
    let ch = corp.sym.chars().next().unwrap_or('?');
    to_subscript(ch)
}

fn display_price_label(raw: &str) -> String {
    let sanitized = sanitize_market_text(raw);
    if sanitized.is_empty() {
        raw.trim().to_string()
    } else {
        sanitized
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn format_currency(value: i32) -> String {
    format!("${value}")
}

fn to_subscript(ch: char) -> String {
    match ch.to_ascii_lowercase() {
        'a' => "ₐ".to_string(),
        'e' => "ₑ".to_string(),
        'h' => "ₕ".to_string(),
        'i' => "ᵢ".to_string(),
        'j' => "ⱼ".to_string(),
        'k' => "ₖ".to_string(),
        'l' => "ₗ".to_string(),
        'm' => "ₘ".to_string(),
        'n' => "ₙ".to_string(),
        'o' => "ₒ".to_string(),
        'p' => "ₚ".to_string(),
        'r' => "ᵣ".to_string(),
        's' => "ₛ".to_string(),
        't' => "ₜ".to_string(),
        'u' => "ᵤ".to_string(),
        'v' => "ᵥ".to_string(),
        'x' => "ₓ".to_string(),
        'y' => "ᵧ".to_string(),
        '0' => "₀".to_string(),
        '1' => "₁".to_string(),
        '2' => "₂".to_string(),
        '3' => "₃".to_string(),
        '4' => "₄".to_string(),
        '5' => "₅".to_string(),
        '6' => "₆".to_string(),
        '7' => "₇".to_string(),
        '8' => "₈".to_string(),
        '9' => "₉".to_string(),
        other => other.to_lowercase().collect(),
    }
}

fn format_distance(value: &Value) -> String {
    match value {
        Value::Null => "?".to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(format_distance)
            .collect::<Vec<_>>()
            .join("/"),
        Value::Object(_) => "map".to_string(),
        Value::Bool(flag) => flag.to_string(),
    }
}

fn stop_limit_from_distance(value: &Value) -> Option<usize> {
    match value {
        Value::Number(num) => num.as_u64().map(|v| v as usize),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        Value::Array(items) => Some(items.len()),
        Value::Object(map) => map.values().filter_map(stop_limit_from_distance).max(),
        _ => None,
    }
}

fn share_payout_line(total: i32) -> String {
    if total <= 0 {
        return "Dividends: $0".to_string();
    }
    let values = (10..=60)
        .step_by(10)
        .map(|pct| {
            let amount = ((total as f64) * (pct as f64) / 100.0).round() as i32;
            format!("{pct}% {}", format_currency(amount))
        })
        .collect::<Vec<_>>();
    format!("Dividends: {}", values.join(" | "))
}
