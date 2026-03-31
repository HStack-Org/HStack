#![deny(clippy::unwrap_used, clippy::expect_used)]

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hstack_agent::{
    manager::SimpleContextManager,
    memory::{HStackWorld, WorkingMemory},
    provider::{GeminiProvider, OpenAiProvider},
    Agent, AgentControlSystem,
};
use hstack_core::provider::{Message, ProviderConfig, ProviderKind, Role};
use hstack_core::sync::SyncAction;
use hstack_core::ticket::Ticket;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Margin},
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
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

const SERVICE_NAME: &str = "hstack-llm-service";
const DESKTOP_BUNDLE_ACCOUNT: &str = "hstack-secure-store-v1";
const WORLD_FILE: &str = "cli-world.json";
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
            "No settings file found at {:?}. Set HSTACK_APP_ID env var if using a different app identifier.",
            settings_path
        ));
    }

    let settings: hstack_core::settings::UserSettings = {
        let content = std::fs::read_to_string(&settings_path)
            .with_context(|| format!("Failed to read settings file: {:?}", settings_path))?;
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
        .map_err(|e| anyhow::anyhow!("OS Keychain access error: {}", e))?;

    let raw = match entry.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => {
            return Err(anyhow::anyhow!(
                "No credentials found in keychain. Please set up hstack-open first."
            ))
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to retrieve from keychain: {}", e)),
    };

    let entries: HashMap<String, String> = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse keychain data: {}", e))?;

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
    #[serde(skip)]
    file_path: Option<PathBuf>,
}

impl FileBackedWorld {
    fn load() -> Result<Self> {
        let path = Self::world_file_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read world file: {:?}", path))?;
            let mut world: Self = serde_json::from_str(&content)
                .with_context(|| "Failed to parse world file")?;
            world.file_path = Some(path);
            Ok(world)
        } else {
            let mut world = Self::default();
            world.file_path = Some(path);
            Ok(world)
        }
    }

    fn save(&self) -> Result<()> {
        if let Some(ref path) = self.file_path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = serde_json::to_string_pretty(&self)?;
            std::fs::write(path, content)?;
        }
        Ok(())
    }

    fn world_file_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow::anyhow!("Could not find data directory"))?;

        let app_id = get_app_identifier();
        Ok(data_dir.join(app_id).join(WORLD_FILE))
    }
}

#[async_trait::async_trait]
impl HStackWorld for FileBackedWorld {
    async fn get_tickets(&self) -> Result<Vec<Ticket>, String> {
        Ok(self.tickets.clone())
    }

    async fn search_tickets(&self, query: &str) -> Result<Vec<Ticket>, String> {
        let query = query.to_lowercase();
        Ok(self
            .tickets
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&query)
                    || t.notes
                        .as_ref()
                        .map(|n| n.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .cloned()
            .collect())
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
    Memory,
    Conversation,
    Actions,
}

struct AppState {
    messages: Vec<(String, String)>,
    working_memory: WorkingMemory,
    proposed_actions: Vec<SyncAction>,
    input_buffer: String,
    is_thinking: bool,
    world: FileBackedWorld,
    model_name: String,
    // Scroll states
    stack_state: ListState,
    memory_state: ListState,
    actions_state: ListState,
    conv_scroll: u16,
    focused_panel: Panel,
}

impl AppState {
    fn extract_memory_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for msg in &self.working_memory.messages {
            let content = msg.content.clone().unwrap_or_default();
            if !content.trim().is_empty() {
                lines.push(format!("[{:?}] {}", msg.role, content));
            }
        }
        for noise in &self.working_memory.technical_noise {
            lines.push(format!("[tool] {}", noise));
        }
        lines
    }
}

enum AgentResult {
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
    let world = FileBackedWorld::load()?;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let model_name = model.unwrap_or_else(|| "unknown".to_string());

    let mut app = AppState {
        messages: Vec::new(),
        working_memory: WorkingMemory::new(),
        proposed_actions: Vec::new(),
        input_buffer: String::new(),
        is_thinking: false,
        world,
        model_name,
        stack_state: ListState::default(),
        memory_state: ListState::default(),
        actions_state: ListState::default(),
        conv_scroll: 0,
        focused_panel: Panel::Conversation,
    };

    let res = run_app(&mut terminal, &mut app, provider_config).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = app.world.save() {
        eprintln!("Warning: failed to save world: {}", e);
    }

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
    let (tx, mut rx) = mpsc::channel(10);

    loop {
        terminal.draw(|f| render_ui(f, app))?;

        // Handle async agent responses with crash detection and memory preservation
        match rx.try_recv() {
            Ok(result) => {
                app.is_thinking = false;
                match result {
                    AgentResult::Success { answer, deltas, memory, world } => {
                        app.messages.push(("Agent".to_string(), answer));
                        app.proposed_actions = deltas.clone();
                        app.working_memory = memory;
                        app.world = world;
                        
                        for action in &deltas {
                            let _ = apply_action_to_world(&mut app.world, action);
                        }
                        if !deltas.is_empty() {
                            let _ = app.world.save();
                        }
                        
                        app.conv_scroll = u16::MAX;
                    }
                    AgentResult::Error { error, memory } => {
                        app.messages.push(("System".to_string(), format!("API Error: {}", error)));
                        // Important: Save the trace so you can see where it looped/failed
                        app.working_memory = memory; 
                        app.conv_scroll = u16::MAX;
                    }
                }
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                if app.is_thinking {
                    // The background thread died unexpectedly!
                    app.is_thinking = false;
                    app.messages.push((
                        "System".to_string(), 
                        "ERROR: The background agent crashed or panicked.".to_string()
                    ));
                    app.conv_scroll = u16::MAX;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {} // Do nothing, keep polling
        }

        // Handle UI events without blocking
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
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
                                Panel::Stack => Panel::Memory,
                                Panel::Memory => Panel::Conversation,
                                Panel::Conversation => Panel::Actions,
                                Panel::Actions => Panel::Stack,
                            };
                        }
                        KeyCode::Enter => {
                            let input = app.input_buffer.trim().to_string();
                            if !input.is_empty() && !app.is_thinking {
                                app.input_buffer.clear();
                                app.messages.push(("You".to_string(), input.clone()));
                                app.is_thinking = true;
                                app.conv_scroll = u16::MAX; // Jump to bottom instantly

                                app.working_memory.messages.push(Message {
                                    role: Role::User,
                                    content: Some(input),
                                    tool_calls: None,
                                    tool_call_id: None,
                                    name: None,
                                });

                                let config_clone = provider_config.clone();
                                let mut memory_clone = app.working_memory.clone();
                                let mut world_clone = app.world.clone();
                                let tx_clone = tx.clone();

                                tokio::spawn(async move {
                                    let provider: Box<dyn hstack_agent::provider::LlmProvider> = match config_clone.kind {
                                        ProviderKind::Gemini => Box::new(GeminiProvider::new(config_clone, None)),
                                        ProviderKind::OpenAiCompatible => Box::new(OpenAiProvider::new(config_clone, None)),
                                    };

                                    let agent = Agent {
                                        provider,
                                        manager: Box::new(SimpleContextManager),
                                        control: Box::new(CapturingControl::new()),
                                        tools: vec![
                                            Box::new(hstack_agent::tool::IdentityTool),
                                            Box::new(hstack_agent::tool::SearchStack),
                                            Box::new(hstack_agent::tool::ScratchThought),
                                        ],
                                        base_prompt: "You are an AI assistant helping the user manage their tasks and habits. You have access to their stack (tickets/habits) and can search it or propose changes.\n\nWhen you have completed your task and are ready to respond to the user, you MUST call the `identity` tool with your final answer. Do not just write text - always use the `identity` tool to signal completion.".to_string(),
                                    };

                                    match agent.run(&mut world_clone, &mut memory_clone).await {
                                        Ok((answer, deltas)) => {
                                            let _ = tx_clone.send(AgentResult::Success {
                                                answer,
                                                deltas,
                                                memory: memory_clone,
                                                world: world_clone,
                                            }).await;
                                        }
                                        Err(e) => {
                                            let _ = tx_clone.send(AgentResult::Error {
                                                error: e.to_string(),
                                                memory: memory_clone, // Preserved trace!
                                            }).await;
                                        }
                                    }
                                });
                            }
                        }
                        KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn handle_scroll_up(app: &mut AppState) {
    match app.focused_panel {
        Panel::Stack => {
            let current = app.stack_state.selected().unwrap_or(0);
            if current > 0 {
                app.stack_state.select(Some(current.saturating_sub(1)));
            }
        }
        Panel::Memory => {
            let current = app.memory_state.selected().unwrap_or(0);
            if current > 0 {
                app.memory_state.select(Some(current.saturating_sub(1)));
            }
        }
        Panel::Conversation => {
            app.conv_scroll = app.conv_scroll.saturating_sub(1);
        }
        Panel::Actions => {
            let current = app.actions_state.selected().unwrap_or(0);
            if current > 0 {
                app.actions_state.select(Some(current.saturating_sub(1)));
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
        Panel::Memory => {
            let max = app.extract_memory_lines().len().saturating_sub(1);
            let current = app.memory_state.selected().unwrap_or(0);
            if current < max {
                app.memory_state.select(Some((current + 1).min(max)));
            }
        }
        Panel::Conversation => {
            app.conv_scroll = app.conv_scroll.saturating_add(1);
        }
        Panel::Actions => {
            let max = app.proposed_actions.len().saturating_sub(1);
            let current = app.actions_state.selected().unwrap_or(0);
            if current < max {
                app.actions_state.select(Some((current + 1).min(max)));
            }
        }
    }
}

fn render_ui(f: &mut ratatui::Frame, app: &mut AppState) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Percentage(25), // Stack
            Constraint::Percentage(25), // Working Memory
            Constraint::Min(0), // Conversation (Flexible)
            Constraint::Length(5), // Actions
            Constraint::Length(3), // Input
        ])
        .split(size);

    let border_style = |panel: Panel| -> Style {
        if app.focused_panel == panel {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };

    // Header
    let header_text = format!(" hstack-cli • {} • ctrl+c: quit • Tab: switch panel ", app.model_name);
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(header, chunks[0]);

    // Stack
    let stack_items: Vec<ListItem> = app.world.tickets.iter().map(|t| {
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

    f.render_stateful_widget(stack_list, chunks[1], &mut app.stack_state);

    let mut stack_scroll_state = ScrollbarState::new(app.world.tickets.len())
        .position(app.stack_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        chunks[1].inner(Margin { vertical: 1, horizontal: 0 }),
        &mut stack_scroll_state,
    );

    // Working Memory
    let memory_lines = app.extract_memory_lines();
    let memory_items: Vec<ListItem> = memory_lines.iter().map(|line| {
        ListItem::new(Span::styled(line, Style::default().fg(Color::DarkGray)))
    }).collect();

    let memory_list = List::new(memory_items)
        .block(Block::default().title(" WORKING MEMORY ").borders(Borders::ALL).border_style(border_style(Panel::Memory)))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(memory_list, chunks[2], &mut app.memory_state);

    let mut memory_scroll_state = ScrollbarState::new(memory_lines.len())
        .position(app.memory_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        chunks[2].inner(Margin { vertical: 1, horizontal: 0 }),
        &mut memory_scroll_state,
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
    let content_lines = conv_text.len() as u16;
    let visible_height = chunks[3].height.saturating_sub(2); // subtract top/bottom borders
    let max_scroll = content_lines.saturating_sub(visible_height);
    
    // Ensure we don't scroll past the actual text bounds
    app.conv_scroll = app.conv_scroll.min(max_scroll);

    let conv_paragraph = Paragraph::new(conv_text.clone())
        .block(Block::default().title(" CONVERSATION ").borders(Borders::ALL).border_style(border_style(Panel::Conversation)))
        .wrap(Wrap { trim: true })
        .scroll((app.conv_scroll, 0));

    f.render_widget(conv_paragraph, chunks[3]);

    let mut conv_scroll_state = ScrollbarState::new(content_lines.into())
        .position(app.conv_scroll as usize);
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        chunks[3].inner(Margin { vertical: 1, horizontal: 0 }),
        &mut conv_scroll_state,
    );

    // Proposed Actions
    let action_items: Vec<ListItem> = app.proposed_actions.iter().map(|a| {
        ListItem::new(Line::from(vec![
            Span::styled(format!("• {:?}", a.r#type), Style::default().fg(Color::Yellow)),
            Span::styled(format!(" {} ({})", &a.entity_id[..8.min(a.entity_id.len())], a.entity_type), Style::default().fg(Color::White)),
        ]))
    }).collect();

    let actions_list = List::new(action_items)
        .block(Block::default().title(" PROPOSED ACTIONS ").borders(Borders::ALL).border_style(border_style(Panel::Actions)))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(actions_list, chunks[4], &mut app.actions_state);

    let mut actions_scroll_state = ScrollbarState::new(app.proposed_actions.len())
        .position(app.actions_state.selected().unwrap_or(0));
    f.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight).begin_symbol(Some("↑")).end_symbol(Some("↓")),
        chunks[4].inner(Margin { vertical: 1, horizontal: 0 }),
        &mut actions_scroll_state,
    );

    // Input Box
    let input_text = format!("> {}", app.input_buffer);
    let input = Paragraph::new(input_text).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::White)));
    f.render_widget(input, chunks[5]);
}

fn apply_action_to_world(world: &mut FileBackedWorld, action: &SyncAction) -> Result<()> {
    use hstack_core::sync::SyncActionType;
    use uuid::Uuid;

    match action.r#type {
        SyncActionType::Create => {
            let ticket = Ticket {
                id: Uuid::new_v4().to_string(),
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