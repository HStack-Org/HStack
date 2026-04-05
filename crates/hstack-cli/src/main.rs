#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(
    not(test),
    deny(
        clippy::panic,
        clippy::panic_in_result_fn,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hstack_agent::{
    manager::SimpleContextManager,
    memory::{HStackWorld, WorkingMemory},
    provider::{GeminiProvider, OpenAiProvider},
    workspace::{short_term_messages, workspace_runtime_snapshot, AllocationPlan, AppId},
    build_base_prompt, Agent, AgentControlSystem, AgentProgressUpdate, AgentPromptProfile,
};
use hstack_core::provider::{Message, ProviderConfig, ProviderKind, Role};
use hstack_core::stack_snapshot::StackSnapshot;
use hstack_core::sync::SyncAction;
use hstack_core::ticket::Ticket;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::stdout;
use std::time::Duration;
use tokio::sync::mpsc;

const SERVICE_NAME: &str = "hstack-llm-service";
const DESKTOP_BUNDLE_ACCOUNT: &str = "hstack-secure-store-v1";
const SETTINGS_FILE: &str = "settings.json";

fn get_app_identifier() -> String {
    std::env::var("HSTACK_APP_ID").unwrap_or_else(|_| "com.hstack.app".to_string())
}

fn load_provider_config() -> Result<(ProviderConfig, Option<String>)> {
    let data_dir = dirs::data_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| anyhow::anyhow!("Could not find data directory"))?;

    let app_id = get_app_identifier();
    let settings_path = data_dir.join(&app_id).join(SETTINGS_FILE);

    if !settings_path.exists() {
        return Err(anyhow::anyhow!(
            "No settings file found at {settings_path:?}. Set HSTACK_APP_ID env var if using a different app identifier."
        ));
    }

    let settings: hstack_core::settings::UserSettings = {
        let content = std::fs::read_to_string(&settings_path)
            .with_context(|| format!("Failed to read settings file: {settings_path:?}"))?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| "Failed to parse settings file as JSON")?;

        let settings_val = json.get("user_settings")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(hstack_core::settings::UserSettings::default()));

        serde_json::from_value(settings_val)
            .with_context(|| "Failed to parse user_settings from settings file")?
    };

    let saved_provider = settings
        .active_provider()
        .ok_or_else(|| anyhow::anyhow!("No active provider configured in settings"))?;

    let entry = keyring::Entry::new(SERVICE_NAME, DESKTOP_BUNDLE_ACCOUNT)
        .map_err(|e| anyhow::anyhow!("OS Keychain access error: {e}"))?;

    let raw = match entry.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => {
            return Err(anyhow::anyhow!(
                "No credentials found in keychain. Please set up hstack-open first."
            ))
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to retrieve from keychain: {e}")),
    };

    let entries: HashMap<String, String> = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse keychain data: {e}"))?;

    let api_key = entries.get(&saved_provider.id).cloned().unwrap_or_default();

    if api_key.is_empty() {
        return Err(anyhow::anyhow!(
            "No API key found in keychain for provider '{}'.",
            saved_provider.id
        ));
    }

    let provider_config = ProviderConfig {
        name: saved_provider.name.clone(),
        kind: saved_provider.kind.clone(),
        endpoint: saved_provider.endpoint.clone(),
        api_key,
        model_name: saved_provider.model_name.clone(),
        rate_limit: saved_provider.rate_limit.clone(),
    };

    let model = Some(provider_config.model_name.clone());

    Ok((provider_config, model))
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct FileBackedWorld {
    tickets: Vec<Ticket>,
}

#[async_trait::async_trait]
impl HStackWorld for FileBackedWorld {
    async fn get_stack_snapshot(&self) -> Result<StackSnapshot, String> {
        Ok(StackSnapshot::new(self.tickets.clone(), Vec::new()))
    }
}

struct CapturingControl {
    captured: std::sync::Mutex<Vec<SyncAction>>,
}

impl CapturingControl {
    fn new() -> Self {
        Self {
            captured: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl AgentControlSystem for CapturingControl {
    async fn validate_stack_action(&self, action: &SyncAction) -> Result<(), hstack_agent::error::Error> {
        if let Ok(mut guard) = self.captured.lock() {
            guard.push(action.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Panel {
    Stack,
    ShortTerm,
    Conversation,
    Trace,
}

struct AppState {
    messages: Vec<(String, String)>,
    working_memory: WorkingMemory,
    proposed_actions: Vec<SyncAction>,
    input_buffer: String,
    is_thinking: bool,
    runtime_status: Option<String>,
    world: FileBackedWorld,
    model_name: String,
    // Scroll states
    stack_state: ListState,
    short_term_state: ListState,
    trace_state: ListState,
    conv_scroll: u16,
    focused_panel: Panel,
}

impl AppState {
    fn projected_tickets(&self) -> Vec<Ticket> {
        StackSnapshot::new(self.world.tickets.clone(), Vec::new())
            .projected_agent_tickets(&self.working_memory.proposed_stack_actions)
    }

    fn short_term_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "policy: v1_last_user_goal_pinned".to_string(),
            "guarantee: latest user message remains mounted even under pressure".to_string(),
        ];

        for message in short_term_messages(&self.working_memory) {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };

            let content = message.content.unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }

            let mut content_lines = content.lines();
            if let Some(first) = content_lines.next() {
                lines.push(format!("{role}: {first}"));
            }
            for line in content_lines {
                lines.push(format!("  {line}"));
            }
        }

        if lines.len() == 2 {
            lines.push("- empty".to_string());
        }

        lines
    }

    fn allocation_plan(&self) -> AllocationPlan {
        let mut workspace = self.working_memory.workspace.clone();
        workspace.refresh_near_events(&self.world.tickets);
        workspace.materialize_allocation_plan()
    }

    fn runtime_snapshot_lines(&self) -> Vec<String> {
        let snapshot = workspace_runtime_snapshot(&self.working_memory);
        serde_json::to_string_pretty(&snapshot)
            .unwrap_or_else(|_| snapshot.to_string())
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn trace_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("runtime_status: {}", self.runtime_status.as_deref().unwrap_or("idle"))];

        lines.push(format!("sync_actions: {}", self.working_memory.proposed_stack_actions.len()));
        if self.working_memory.proposed_stack_actions.is_empty() {
            lines.push("  - none".to_string());
        } else {
            for action in &self.working_memory.proposed_stack_actions {
                let mut line = format!(
                    "  - {:?} {} {}",
                    action.r#type,
                    action.entity_type,
                    truncate_id(&action.entity_id)
                );
                if let Some(status) = &action.status {
                    line.push_str(&format!(" status={status:?}"));
                }
                lines.push(line);
            }
        }

        lines.push(String::new());
        lines.push(format!("technical_noise: {}", self.working_memory.technical_noise.len()));
        if self.working_memory.technical_noise.is_empty() {
            lines.push("  - none".to_string());
        } else {
            for (index, entry) in self.working_memory.technical_noise.iter().enumerate() {
                lines.push(format!("  [{index}]"));
                let pretty = serde_json::to_string_pretty(entry).unwrap_or_else(|_| entry.to_string());
                for trace_line in pretty.lines() {
                    lines.push(format!("    {trace_line}"));
                }
            }
        }

        lines
    }
}

#[derive(Debug, Clone, Copy)]
struct UiLayout {
    header: Rect,
    stack: Rect,
    short_term: Rect,
    dock: Rect,
    near_events: Rect,
    snapshot: Rect,
    apps: Rect,
    conversation: Rect,
    trace: Rect,
    input: Rect,
}

fn compute_layout(size: Rect) -> UiLayout {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(size);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(root[1]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(26),
            Constraint::Percentage(30),
            Constraint::Percentage(44),
        ])
        .split(body[0]);

    let workspace = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(18),
            Constraint::Percentage(18),
            Constraint::Percentage(24),
            Constraint::Min(8),
        ])
        .split(top[2]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[1]);

    UiLayout {
        header: root[0],
        stack: top[0],
        short_term: top[1],
        dock: workspace[0],
        near_events: workspace[1],
        snapshot: workspace[2],
        apps: workspace[3],
        conversation: bottom[0],
        trace: bottom[1],
        input: root[2],
    }
}

enum AgentResult {
    Progress {
        iteration: usize,
        phase: String,
        memory: WorkingMemory,
    },
    Success {
        answer: String,
        deltas: Vec<SyncAction>,
        memory: WorkingMemory,
        world: FileBackedWorld,
    },
    Error {
        error: String,
        memory: WorkingMemory,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let (provider_config, model) = load_provider_config()?;
    let world = FileBackedWorld::default();

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let model_name = model.unwrap_or_else(|| "unknown".to_string());

    let mut app = AppState {
        messages: Vec::new(),
        working_memory: WorkingMemory::new(),
        proposed_actions: Vec::new(),
        input_buffer: String::new(),
        is_thinking: false,
        runtime_status: None,
        world,
        model_name,
        stack_state: ListState::default(),
        short_term_state: ListState::default(),
        trace_state: ListState::default(),
        conv_scroll: 0,
        focused_panel: Panel::Conversation,
    };

    let res = run_app(&mut terminal, &mut app, provider_config).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    res
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    provider_config: ProviderConfig,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let (tx, mut rx) = mpsc::unbounded_channel();

    loop {
        terminal.draw(|f| render_ui(f, app))?;

        // Handle async agent responses with crash detection and memory preservation
        match rx.try_recv() {
            Ok(result) => {
                app.is_thinking = false;
                match result {
                    AgentResult::Progress { iteration, phase, memory } => {
                        app.runtime_status = Some(format!("iteration {iteration} • {phase}"));
                        app.working_memory = memory;
                        app.proposed_actions = app.working_memory.proposed_stack_actions.clone();
                    }
                    AgentResult::Success { answer, deltas, memory, world } => {
                        app.runtime_status = Some("completed".to_string());
                        app.messages.push(("Agent".to_string(), answer));
                        app.working_memory = memory;
                        app.proposed_actions = app.working_memory.proposed_stack_actions.clone();
                        app.world = world;
                        
                        for action in &deltas {
                            let _ = apply_action_to_world(&mut app.world, action);
                        }
                        
                        app.working_memory.proposed_stack_actions.clear();
                        app.proposed_actions.clear();

                        app.conv_scroll = u16::MAX;
                    }
                    AgentResult::Error { error, memory } => {
                        app.runtime_status = Some(format!("agent failure • {error}"));
                        app.working_memory = memory;
                        app.proposed_actions = app.working_memory.proposed_stack_actions.clone();
                        app.conv_scroll = u16::MAX;
                    }
                }
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                if app.is_thinking {
                    // The background thread died unexpectedly!
                    app.is_thinking = false;
                    app.runtime_status = Some("internal failure • background task crashed".to_string());
                    app.conv_scroll = u16::MAX;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {} // Do nothing, keep polling
        }

        // Handle UI events without blocking
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('c') | KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break;
                        }
                        KeyCode::Char(c) => {
                            app.input_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_buffer.pop();
                        }
                        KeyCode::Up => {
                            handle_scroll_up(app);
                        }
                        KeyCode::Down => {
                            handle_scroll_down(app);
                        }
                        KeyCode::PageUp => {
                            for _ in 0..5 {
                                handle_scroll_up(app);
                            }
                        }
                        KeyCode::PageDown => {
                            for _ in 0..5 {
                                handle_scroll_down(app);
                            }
                        }
                        KeyCode::Tab => {
                            app.focused_panel = match app.focused_panel {
                                Panel::Stack => Panel::ShortTerm,
                                Panel::ShortTerm => Panel::Conversation,
                                Panel::Conversation => Panel::Trace,
                                Panel::Trace => Panel::Stack,
                            };
                        }
                        KeyCode::Enter => {
                            let input = app.input_buffer.trim().to_string();
                            if !input.is_empty() && !app.is_thinking {
                                app.input_buffer.clear();
                                app.messages.push(("You".to_string(), input.clone()));
                                app.is_thinking = true;
                                app.runtime_status = Some("thinking".to_string());
                                app.conv_scroll = u16::MAX; // Jump to bottom instantly

                                app.working_memory.push_message(Message {
                                    role: Role::User,
                                    content: Some(input),
                                    tool_calls: None,
                                    tool_call_id: None,
                                    name: None,
                                });

                                let config_clone = provider_config.clone();
                                let mut memory_clone = app.working_memory.clone();
                                let world_clone = app.world.clone();
                                let tx_clone = tx.clone();

                                tokio::spawn(async move {
                                    let provider: Box<dyn hstack_agent::provider::LlmProvider> = match config_clone.kind {
                                        ProviderKind::Gemini => Box::new(GeminiProvider::new(config_clone, None)),
                                        ProviderKind::OpenAiCompatible => Box::new(OpenAiProvider::new(config_clone, None)),
                                    };

                                    let tools = match hstack_agent::tool::compose_tools(&[
                                        "identity",
                                        "follow_up",
                                        "search_stack",
                                        "create_ticket",
                                        "delete_ticket",
                                        "delete_all_tickets",
                                        "edit_ticket",
                                        "add_commute",
                                        "get_directions",
                                        "remove_commute",
                                        "start_live_directions",
                                        "create_countdown",
                                        "scratch_thought",
                                        "exa_search",
                                        "light_compute",
                                        "manage_app",
                                        "inspect_app",
                                        "scratchpad_search",
                                        "scratchpad_edit",
                                    ]) {
                                        Ok(tools) => tools,
                                        Err(e) => {
                                            let _ = tx_clone
                                                .send(AgentResult::Error {
                                                    error: format!("Tool configuration error: {e}"),
                                                    memory: memory_clone,
                                                });
                                            return;
                                        }
                                    };

                                    let agent = Agent {
                                        provider,
                                        manager: Box::new(SimpleContextManager),
                                        control: Box::new(CapturingControl::new()),
                                        tools,
                                        base_prompt: build_base_prompt(AgentPromptProfile::DebugInteractive),
                                    };

                                    let tx_progress = tx_clone.clone();
                                    match agent.run_with_progress(&world_clone, &mut memory_clone, move |update: AgentProgressUpdate| {
                                        let _ = tx_progress.send(AgentResult::Progress {
                                            iteration: update.iteration,
                                            phase: update.phase,
                                            memory: update.working_memory,
                                        });
                                    }).await {
                                        Ok((answer, deltas)) => {
                                            let _ = tx_clone.send(AgentResult::Success {
                                                answer,
                                                deltas,
                                                memory: memory_clone,
                                                world: world_clone,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AgentResult::Error {
                                                error: e.to_string(),
                                                memory: memory_clone, // Preserved trace!
                                            });
                                        }
                                    }
                                });
                            }
                        }
                        KeyCode::Esc => break,
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse_event(app, mouse, terminal.size()?.into());
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn handle_mouse_event(app: &mut AppState, mouse: MouseEvent, terminal_size: Rect) {
    match mouse.kind {
        MouseEventKind::ScrollUp => handle_scroll_up(app),
        MouseEventKind::ScrollDown => handle_scroll_down(app),
        MouseEventKind::Down(MouseButton::Left) => {
            handle_mouse_click(app, mouse.column, mouse.row, terminal_size);
        }
        _ => {}
    }
}

fn handle_mouse_click(app: &mut AppState, column: u16, row: u16, terminal_size: Rect) {
    let layout = compute_layout(terminal_size);

    if rect_contains(layout.stack, column, row) {
        app.focused_panel = Panel::Stack;
    } else if rect_contains(layout.short_term, column, row) {
        app.focused_panel = Panel::ShortTerm;
    } else if rect_contains(layout.conversation, column, row) {
        app.focused_panel = Panel::Conversation;
    } else if rect_contains(layout.trace, column, row) {
        app.focused_panel = Panel::Trace;
    }

    let focused_rect = match app.focused_panel {
        Panel::Stack => layout.stack,
        Panel::ShortTerm => layout.short_term,
        Panel::Conversation => layout.conversation,
        Panel::Trace => layout.trace,
    };

    if let Some(is_up_arrow) = hit_scrollbar_arrow(focused_rect, column, row) {
        if is_up_arrow {
            handle_scroll_up(app);
        } else {
            handle_scroll_down(app);
        }
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    let right = rect.x.saturating_add(rect.width);
    let bottom = rect.y.saturating_add(rect.height);
    column >= rect.x && column < right && row >= rect.y && row < bottom
}

fn hit_scrollbar_arrow(panel_rect: Rect, column: u16, row: u16) -> Option<bool> {
    let scroll_area = panel_rect.inner(Margin { vertical: 1, horizontal: 0 });
    if scroll_area.width == 0 || scroll_area.height == 0 {
        return None;
    }

    let rightmost = scroll_area.x.saturating_add(scroll_area.width).saturating_sub(1);
    if column != rightmost {
        return None;
    }

    let top = scroll_area.y;
    let bottom = scroll_area.y.saturating_add(scroll_area.height).saturating_sub(1);
    if row == top {
        Some(true)
    } else if row == bottom {
        Some(false)
    } else {
        None
    }
}

fn handle_scroll_up(app: &mut AppState) {
    match app.focused_panel {
        Panel::Stack => {
            let current = app.stack_state.selected().unwrap_or(0);
            if current > 0 {
                app.stack_state.select(Some(current.saturating_sub(1)));
            }
        }
        Panel::ShortTerm => {
            let current = app.short_term_state.selected().unwrap_or(0);
            if current > 0 {
                app.short_term_state.select(Some(current.saturating_sub(1)));
            }
        }
        Panel::Conversation => {
            app.conv_scroll = app.conv_scroll.saturating_sub(1);
        }
        Panel::Trace => {
            let current = app.trace_state.selected().unwrap_or(0);
            if current > 0 {
                app.trace_state.select(Some(current.saturating_sub(1)));
            }
        }
    }
}

fn handle_scroll_down(app: &mut AppState) {
    match app.focused_panel {
        Panel::Stack => {
            let max = app.world.tickets.len().saturating_sub(1);
            let current = app.stack_state.selected().unwrap_or(0);
            if current < max {
                app.stack_state.select(Some((current + 1).min(max)));
            }
        }
        Panel::ShortTerm => {
            let max = app.short_term_lines().len().saturating_sub(1);
            let current = app.short_term_state.selected().unwrap_or(0);
            if current < max {
                app.short_term_state.select(Some((current + 1).min(max)));
            }
        }
        Panel::Conversation => {
            app.conv_scroll = app.conv_scroll.saturating_add(1);
        }
        Panel::Trace => {
            let max = app.trace_lines().len().saturating_sub(1);
            let current = app.trace_state.selected().unwrap_or(0);
            if current < max {
                app.trace_state.select(Some((current + 1).min(max)));
            }
        }
    }
}

fn truncate_id(id: &str) -> &str {
    &id[..8.min(id.len())]
}

fn panel_name(panel: Panel) -> &'static str {
    match panel {
        Panel::Stack => "stack",
        Panel::ShortTerm => "short_term",
        Panel::Conversation => "conversation",
        Panel::Trace => "trace",
    }
}

fn render_text_panel(
    f: &mut ratatui::Frame,
    rect: Rect,
    title: &str,
    lines: &[String],
    border_style: Style,
) {
    let text = if lines.is_empty() {
        "- empty".to_string()
    } else {
        lines.join("\n")
    };

    let paragraph = Paragraph::new(text)
        .block(Block::default().title(title).borders(Borders::ALL).border_style(border_style))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, rect);
}

fn render_app_windows(
    f: &mut ratatui::Frame,
    rect: Rect,
    plan: &AllocationPlan,
    focused_app: AppId,
) {
    let apps = &plan.selected_app_contents;

    if apps.is_empty() {
        render_text_panel(
            f,
            rect,
            " APP WINDOWS ",
            &["- no mounted app viewport selected".to_string()],
            Style::default().fg(Color::DarkGray),
        );
        return;
    }

    let outer = Block::default()
        .title(" APP WINDOWS ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(rect);
    f.render_widget(outer, rect);

    if inner.width < 4 || inner.height < 4 {
        return;
    }

    let rects = match apps.len() {
        1 => vec![inner],
        2 => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner)
            .to_vec(),
        _ => {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let mut rects = Vec::new();
            rects.extend(
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(rows[0])
                    .to_vec(),
            );
            rects.extend(
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(rows[1])
                    .to_vec(),
            );
            rects
        }
    };

    for ((app_id, content), app_rect) in apps.iter().zip(rects.into_iter()) {
        let border = if *app_id == focused_app {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = format!(" {} ", app_id.label().to_uppercase());
        let body = content
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n");

        let paragraph = Paragraph::new(body)
            .block(Block::default().title(title).borders(Borders::ALL).border_style(border))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, app_rect);
    }
}

fn wrapped_line_count(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let mut total: u16 = 0;

    for segment in text.split('\n') {
        let chars = segment.chars().count();
        let visual = if chars == 0 {
            1
        } else {
            chars.div_ceil(width)
        };
        total = total.saturating_add(visual as u16);
    }

    total
}

fn estimate_conversation_height(app: &AppState, body_width: u16) -> u16 {
    let mut lines: u16 = 0;

    for (role, content) in &app.messages {
        let prefix = if role == "You" { "> You: " } else { "  Agent: " };
        let rendered = format!("{prefix}{content}");
        lines = lines.saturating_add(wrapped_line_count(&rendered, body_width));
    }

    if app.is_thinking {
        lines = lines.saturating_add(wrapped_line_count("  [Agent is thinking...]", body_width));
    }

    lines
}

fn render_ui(f: &mut ratatui::Frame, app: &mut AppState) {
    let size = f.area();
    let layout = compute_layout(size);
    let plan = app.allocation_plan();
    let projected_tickets = app.projected_tickets();
    let short_term_lines = app.short_term_lines();
    let trace_lines = app.trace_lines();
    let snapshot_lines = app.runtime_snapshot_lines();

    let border_style = |panel: Panel| -> Style {
        if app.focused_panel == panel {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };

    // Header
    let status = app.runtime_status.as_deref().unwrap_or("idle");
    let header_text = format!(
        " hstack-cli • {} • {} • ui_focus={} • dock_focus={} • mounted={} • ctrl+c: quit • Tab: switch panel ",
        app.model_name,
        status,
        panel_name(app.focused_panel),
        app.working_memory.workspace.dock.focused_app.label(),
        app.working_memory.workspace.dock.mounted_apps.len(),
    );
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(header, layout.header);

    // Stack
    let stack_items: Vec<ListItem> = projected_tickets.iter().map(|t| {
        let status_color = match t.status {
            hstack_core::ticket::TicketStatus::Completed => Color::Green,
            hstack_core::ticket::TicketStatus::InFocus => Color::Yellow,
            hstack_core::ticket::TicketStatus::Expired => Color::Red,
            _ => Color::White,
        };
        let line = Line::from(vec![
            Span::styled("• ", Style::default().fg(Color::DarkGray)),
            Span::styled(t.title.clone(), Style::default().fg(Color::White)),
            Span::styled(format!(" [{:?}]", t.r#type), Style::default().fg(Color::Magenta)),
            Span::styled(format!(" - {:?}", t.status), Style::default().fg(status_color)),
        ]);
        ListItem::new(line)
    }).collect();

    let stack_list = List::new(stack_items)
        .block(Block::default().title(" STACK ").borders(Borders::ALL).border_style(border_style(Panel::Stack)))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(stack_list, layout.stack, &mut app.stack_state);

    let mut stack_scroll_state = ScrollbarState::new(projected_tickets.len())
        .position(app.stack_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        layout.stack.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut stack_scroll_state,
    );

    // Short-term kernel
    let short_term_items: Vec<ListItem> = short_term_lines.iter().map(|line| {
        ListItem::new(Span::styled(line, Style::default().fg(Color::DarkGray)))
    }).collect();

    let short_term_list = List::new(short_term_items)
        .block(Block::default().title(" SHORT-TERM KERNEL ").borders(Borders::ALL).border_style(border_style(Panel::ShortTerm)))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(short_term_list, layout.short_term, &mut app.short_term_state);

    let mut short_term_scroll_state = ScrollbarState::new(short_term_lines.len())
        .position(app.short_term_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        layout.short_term.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut short_term_scroll_state,
    );

    // Workspace regions
    let dock_lines = plan
        .dock_content
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    render_text_panel(
        f,
        layout.dock,
        " DOCK ",
        &dock_lines,
        Style::default().fg(Color::DarkGray),
    );

    let near_event_lines = plan
        .near_event_content
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    render_text_panel(
        f,
        layout.near_events,
        " NEAR EVENTS ",
        &near_event_lines,
        Style::default().fg(Color::DarkGray),
    );

    render_text_panel(
        f,
        layout.snapshot,
        " WORKSPACE SNAPSHOT ",
        &snapshot_lines,
        Style::default().fg(Color::DarkGray),
    );

    render_app_windows(
        f,
        layout.apps,
        &plan,
        app.working_memory.workspace.dock.focused_app,
    );

    // Conversation
    let mut conv_text = Vec::new();
    for (role, content) in &app.messages {
        let (prefix, color) = if role == "You" { ("> You: ", Color::Green) } else { ("  Agent: ", Color::Blue) };
        conv_text.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(content, Style::default().fg(Color::White)),
        ]));
    }
    if app.is_thinking {
        conv_text.push(Line::from(Span::styled("  [Agent is thinking...]", Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC))));
    }

    // CLAMP SCROLLING LOGIC
    let conv_inner = layout.conversation.inner(Margin { vertical: 1, horizontal: 1 });
    let content_lines = estimate_conversation_height(app, conv_inner.width.max(1));
    let visible_height = conv_inner.height;
    let max_scroll = content_lines.saturating_sub(visible_height);
    
    // Ensure we don't scroll past the actual text bounds
    app.conv_scroll = app.conv_scroll.min(max_scroll);

    let conv_paragraph = Paragraph::new(conv_text.clone())
        .block(Block::default().title(" CONVERSATION ").borders(Borders::ALL).border_style(border_style(Panel::Conversation)))
        .wrap(Wrap { trim: true })
        .scroll((app.conv_scroll, 0));

    f.render_widget(conv_paragraph, layout.conversation);

    let mut conv_scroll_state = ScrollbarState::new(content_lines.into())
        .position(app.conv_scroll as usize);
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        layout.conversation.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut conv_scroll_state,
    );

    // Trace
    let trace_items: Vec<ListItem> = trace_lines.iter().map(|line| {
        ListItem::new(Span::styled(line, Style::default().fg(Color::DarkGray)))
    }).collect();

    let trace_list = List::new(trace_items)
        .block(Block::default().title(" TRACE ").borders(Borders::ALL).border_style(border_style(Panel::Trace)))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(trace_list, layout.trace, &mut app.trace_state);

    let mut trace_scroll_state = ScrollbarState::new(trace_lines.len())
        .position(app.trace_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        layout.trace.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut trace_scroll_state,
    );

    // Input Box
    let input_text = format!("> {}", app.input_buffer);
    let input = Paragraph::new(input_text).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::White)));
    f.render_widget(input, layout.input);
}

fn apply_action_to_world(world: &mut FileBackedWorld, action: &SyncAction) -> Result<()> {
    use hstack_core::sync::SyncActionType;
    use uuid::Uuid;

    match action.r#type {
        SyncActionType::Create => {
            let ticket = Ticket {
                id: action.entity_id.clone(),
                title: format!("{:?} - {}", action.entity_type, &action.entity_id[..8.min(action.entity_id.len())]),
                r#type: hstack_core::ticket::TicketType::Task,
                status: hstack_core::ticket::TicketStatus::Idle,
                payload: hstack_core::ticket::TicketPayload::Task {
                    title: format!("{:?} - {}", action.entity_type, &action.entity_id[..8.min(action.entity_id.len())]),
                    scheduled_time_iso: None,
                    rrule: None,
                    duration_minutes: None,
                    status: None,
                    priority: None,
                    completed: None,
                },
                notes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            world.tickets.push(ticket);
        }
        SyncActionType::Update => {
            if let Some(ticket) = world.tickets.iter_mut().find(|t| t.id == action.entity_id) {
                ticket.updated_at = Utc::now();
                if let Some(status) = &action.status {
                    ticket.status = status.clone();
                }
            }
        }
        SyncActionType::Delete => {
            world.tickets.retain(|t| t.id != action.entity_id);
        }
    }
    Ok(())
}