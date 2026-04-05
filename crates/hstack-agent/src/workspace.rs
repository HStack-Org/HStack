use chrono::{DateTime, Duration, Utc, Local};
use hstack_core::provider::{Message, Role};
use hstack_core::sync::SyncAction;
use hstack_core::ticket::{Ticket, TicketPayload, TicketPriority, TicketType};
use hstack_core::settings::UserSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const SHORT_TERM_POLICY_VERSION: &str = "v1_last_user_goal_pinned";
fn short_term_message_cost(message: &Message) -> usize {
    let content = message.content.as_deref().unwrap_or_default();
    estimate_token_cost(content) + 16
}
const NEAR_EVENT_POLICY_VERSION: &str = "v1_72h_or_urgent";
const NEAR_EVENT_HORIZON_HOURS: i64 = 72;
const MAX_NEAR_EVENT_ITEMS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudget {
    pub usable_tokens: usize,
    pub prompt_budget: usize,
    pub short_term_budget: usize,
    pub near_event_budget: usize,
    pub dock_budget: usize,
    pub app_budget: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            usable_tokens: 24_000,
            prompt_budget: 4_000,
            short_term_budget: 4_000,
            near_event_budget: 3_000,
            dock_budget: 1_000,
            app_budget: 12_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppId {
    Scratchpad,
    WebSearch,
    StackSearch,
    Compute,
}

impl AppId {
    pub fn label(self) -> &'static str {
        match self {
            AppId::Scratchpad => "scratchpad",
            AppId::WebSearch => "websearch",
            AppId::StackSearch => "stack-search",
            AppId::Compute => "compute",
        }
    }
}

pub fn exa_search_is_available() -> bool {
    std::env::var("EXA_API_KEY")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub fn app_is_available(app_id: AppId) -> bool {
    match app_id {
        AppId::Scratchpad | AppId::StackSearch | AppId::Compute => true,
        AppId::WebSearch => exa_search_is_available(),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AppLifecycle {
    InstalledClosed,
    OpenUnmounted,
    OpenMounted,
    OpenMountedFocused,
}

impl AppLifecycle {
    pub fn is_open(self) -> bool {
        !matches!(self, AppLifecycle::InstalledClosed)
    }

    pub fn is_focused(self) -> bool {
        matches!(self, AppLifecycle::OpenMountedFocused)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AppPriorityClass {
    Foreground,
    Supporting,
    Background,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ViewportTier {
    Minimal,
    Preferred,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextViewport {
    pub start_line: usize,
    pub line_count: usize,
}

impl TextViewport {
    pub fn new(start_line: usize, line_count: usize) -> Self {
        Self {
            start_line,
            line_count,
        }
    }

    pub fn clamp(self, total_lines: usize) -> Self {
        let line_count = self.line_count.max(1);
        let start_line = if total_lines == 0 {
            0
        } else {
            self.start_line.min(total_lines.saturating_sub(1))
        };
        Self {
            start_line,
            line_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DockState {
    pub focused_app: AppId,
    pub mounted_apps: BTreeSet<AppId>,
}

impl Default for DockState {
    fn default() -> Self {
        let mut mounted_apps = BTreeSet::new();
        mounted_apps.insert(AppId::Scratchpad);
        Self {
            focused_app: AppId::Scratchpad,
            mounted_apps,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScratchpadApp {
    pub lifecycle: AppLifecycle,
    pub pinned: bool,
    pub viewport: TextViewport,
    pub document_lines: Vec<String>,
}

impl Default for ScratchpadApp {
    fn default() -> Self {
        Self {
            lifecycle: AppLifecycle::OpenMountedFocused,
            pinned: true,
            viewport: TextViewport::new(0, 16),
            document_lines: vec!["# Scratchpad".to_string()],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SearchAppKind {
    Web,
    Stack,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResultRecord {
    pub title: String,
    pub url: Option<String>,
    pub snippet: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchApp {
    pub app_id: AppId,
    pub kind: SearchAppKind,
    pub lifecycle: AppLifecycle,
    pub pinned: bool,
    pub viewport: TextViewport,
    pub query_history: Vec<String>,
    pub focused_query: Option<String>,
    pub results: Vec<SearchResultRecord>,
}

impl SearchApp {
    fn new(app_id: AppId, kind: SearchAppKind) -> Self {
        Self {
            app_id,
            kind,
            lifecycle: AppLifecycle::InstalledClosed,
            pinned: false,
            viewport: TextViewport::new(0, 6),
            query_history: Vec::new(),
            focused_query: None,
            results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComputeRecord {
    pub summary: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComputeApp {
    pub lifecycle: AppLifecycle,
    pub pinned: bool,
    pub viewport: TextViewport,
    pub history: Vec<ComputeRecord>,
}

impl Default for ComputeApp {
    fn default() -> Self {
        Self {
            lifecycle: AppLifecycle::InstalledClosed,
            pinned: false,
            viewport: TextViewport::new(0, 4),
            history: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NearEventItem {
    pub ticket_id: String,
    pub title: String,
    pub reason: String,
    pub timestamp_iso: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceState {
    pub budget: ContextBudget,
    pub dock: DockState,
    pub scratchpad: ScratchpadApp,
    pub web_search: SearchApp,
    pub stack_search: SearchApp,
    pub compute: ComputeApp,
    pub near_events: Vec<NearEventItem>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            budget: ContextBudget::default(),
            dock: DockState::default(),
            scratchpad: ScratchpadApp::default(),
            web_search: SearchApp::new(AppId::WebSearch, SearchAppKind::Web),
            stack_search: SearchApp::new(AppId::StackSearch, SearchAppKind::Stack),
            compute: ComputeApp::default(),
            near_events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkspaceDelta {
    ScratchpadAppend { thought: String, metadata: Value },
    ScratchpadPatch {
        start_line: usize,
        delete_count: usize,
        new_lines: Vec<String>,
    },
    PublishSearchResults {
        app_id: AppId,
        query: String,
        results: Vec<SearchResultRecord>,
    },
    RecordCompute {
        summary: String,
        payload: Value,
    },
    OpenApp(AppId),
    CloseApp(AppId),
    FocusApp(AppId),
    PinApp(AppId),
    UnpinApp(AppId),
    ScrollApp { app_id: AppId, delta: isize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportCandidate {
    pub app_id: AppId,
    pub tier: ViewportTier,
    pub mandatory: bool,
    pub utility: i64,
    pub cost_tokens: usize,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationPlan {
    pub dock_content: String,
    pub near_event_content: String,
    pub selected_app_contents: Vec<(AppId, String)>,
    pub mounted_apps: BTreeSet<AppId>,
}

impl WorkspaceState {
    pub fn apply_delta(&mut self, delta: WorkspaceDelta) -> Value {
        match delta {
            WorkspaceDelta::ScratchpadAppend { thought, metadata } => {
                self.focus_app(AppId::Scratchpad);
                let mut line = thought.clone();
                if !metadata.is_null() {
                    line.push_str(" :: ");
                    line.push_str(&serde_json::to_string(&metadata).unwrap_or_else(|_| "null".to_string()));
                }
                self.scratchpad.document_lines.push(line);
                serde_json::json!({
                    "scratchpad_append": {
                        "thought": thought,
                        "metadata": metadata,
                    }
                })
            }
            WorkspaceDelta::ScratchpadPatch {
                start_line,
                delete_count,
                new_lines,
            } => {
                self.focus_app(AppId::Scratchpad);
                let insert_at = start_line.min(self.scratchpad.document_lines.len());
                let delete_end = (insert_at + delete_count).min(self.scratchpad.document_lines.len());
                self.scratchpad
                    .document_lines
                    .splice(insert_at..delete_end, new_lines.clone());
                serde_json::json!({
                    "scratchpad_patch": {
                        "start_line": insert_at,
                        "delete_count": delete_count,
                        "new_lines": new_lines,
                    }
                })
            }
            WorkspaceDelta::PublishSearchResults { app_id, query, results } => {
                self.focus_app(app_id);
                let app = match app_id {
                    AppId::WebSearch => &mut self.web_search,
                    AppId::StackSearch => &mut self.stack_search,
                    _ => return serde_json::json!({ "workspace_error": { "reason": "invalid_search_app" } }),
                };
                app.focused_query = Some(query.clone());
                app.query_history.push(query.clone());
                app.results = results.clone();
                let key = match app_id {
                    AppId::WebSearch => format!("exa_search:{query}"),
                    AppId::StackSearch => format!("search_stack:{query}"),
                    _ => {
                        let label = app_id.label();
                        format!("{label}:{query}")
                    }
                };
                serde_json::json!({
                    key: {
                        "results": results,
                    }
                })
            }
            WorkspaceDelta::RecordCompute { summary, payload } => {
                self.focus_app(AppId::Compute);
                self.compute.history.push(ComputeRecord {
                    summary: summary.clone(),
                    payload: payload.clone(),
                });
                serde_json::json!({
                    "compute": {
                        "summary": summary,
                        "payload": payload,
                    }
                })
            }
            WorkspaceDelta::OpenApp(app_id) => {
                if !app_is_available(app_id) {
                    return serde_json::json!({ "workspace_error": { "reason": "unavailable_app", "app_id": app_id.label() } });
                }
                self.open_app(app_id);
                serde_json::json!({ "dock": { "opened": app_id.label() } })
            }
            WorkspaceDelta::CloseApp(app_id) => {
                self.close_app(app_id);
                serde_json::json!({ "dock": { "closed": app_id.label() } })
            }
            WorkspaceDelta::FocusApp(app_id) => {
                if !app_is_available(app_id) {
                    return serde_json::json!({ "workspace_error": { "reason": "unavailable_app", "app_id": app_id.label() } });
                }
                self.focus_app(app_id);
                serde_json::json!({ "dock": { "focused": app_id.label() } })
            }
            WorkspaceDelta::PinApp(app_id) => {
                if !app_is_available(app_id) {
                    return serde_json::json!({ "workspace_error": { "reason": "unavailable_app", "app_id": app_id.label() } });
                }
                self.set_pinned(app_id, true);
                serde_json::json!({ "dock": { "pinned": app_id.label() } })
            }
            WorkspaceDelta::UnpinApp(app_id) => {
                self.set_pinned(app_id, false);
                serde_json::json!({ "dock": { "unpinned": app_id.label() } })
            }
            WorkspaceDelta::ScrollApp { app_id, delta } => {
                if !app_is_available(app_id) {
                    return serde_json::json!({ "workspace_error": { "reason": "unavailable_app", "app_id": app_id.label() } });
                }
                self.scroll_app(app_id, delta);
                serde_json::json!({ "dock": { "scrolled": app_id.label(), "delta": delta } })
            }
        }
    }

    pub fn refresh_near_events(&mut self, tickets: &[Ticket]) {
        let now = Utc::now();
        let horizon = now + Duration::hours(NEAR_EVENT_HORIZON_HOURS);
        let mut items = Vec::new();
        for ticket in tickets {
            let mut maybe_item = scheduled_timestamp(ticket)
                .filter(|timestamp| *timestamp <= horizon)
                .map(|timestamp| NearEventItem {
                    ticket_id: ticket.id.clone(),
                    title: ticket.title.clone(),
                    reason: format!("scheduled_within_{NEAR_EVENT_HORIZON_HOURS}h"),
                    timestamp_iso: Some(timestamp.to_rfc3339()),
                });

            if maybe_item.is_none() && is_urgent(ticket) {
                maybe_item = Some(NearEventItem {
                    ticket_id: ticket.id.clone(),
                    title: ticket.title.clone(),
                    reason: "urgent_priority".to_string(),
                    timestamp_iso: None,
                });
            }

            if let Some(item) = maybe_item {
                items.push(item);
            }
        }

        items.sort_by(|left, right| left.timestamp_iso.cmp(&right.timestamp_iso).then(left.title.cmp(&right.title)));
        items.truncate(MAX_NEAR_EVENT_ITEMS);
        self.near_events = items;
    }

    pub fn build_allocation_plan(&self) -> AllocationPlan {
        let mut candidates = Vec::new();
        candidates.extend(self.candidates_for_scratchpad());
        candidates.extend(self.candidates_for_search(&self.web_search));
        candidates.extend(self.candidates_for_search(&self.stack_search));
        candidates.extend(self.candidates_for_compute());

        let selected = solve_viewport_selection(&candidates, self.budget.app_budget);
        let mounted_apps: BTreeSet<AppId> = selected.iter().map(|candidate| candidate.app_id).collect();
        let selected_app_contents = selected
            .into_iter()
            .map(|candidate| (candidate.app_id, candidate.content))
            .collect();

        AllocationPlan {
            dock_content: self.render_dock(&mounted_apps),
            near_event_content: self.render_near_events(),
            selected_app_contents,
            mounted_apps,
        }
    }

    pub fn materialize_allocation_plan(&mut self) -> AllocationPlan {
        self.normalize_unavailable_apps();
        let plan = self.build_allocation_plan();
        self.dock.mounted_apps = plan.mounted_apps.clone();
        self.sync_lifecycle_mount_state();
        plan
    }

    pub fn inspect_app(&self, app_id: AppId) -> Value {
        match app_id {
            AppId::Scratchpad => serde_json::json!({
                "app_id": app_id.label(),
                "lifecycle": format!("{:?}", self.scratchpad.lifecycle),
                "pinned": self.scratchpad.pinned,
                "viewport": self.scratchpad.viewport,
                "visible": self.visible_scratchpad_lines(),
            }),
            AppId::WebSearch => inspect_search_app(&self.web_search),
            AppId::StackSearch => inspect_search_app(&self.stack_search),
            AppId::Compute => serde_json::json!({
                "app_id": app_id.label(),
                "lifecycle": format!("{:?}", self.compute.lifecycle),
                "pinned": self.compute.pinned,
                "viewport": self.compute.viewport,
                "visible": self.visible_compute_records(),
            }),
        }
    }

    pub fn search_scratchpad(&self, query: &str) -> Vec<Value> {
        let needle = query.to_ascii_lowercase();
        self.scratchpad
            .document_lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                if line.to_ascii_lowercase().contains(&needle) {
                    Some(serde_json::json!({
                        "line": index,
                        "content": line,
                    }))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn visible_scratchpad_lines(&self) -> Vec<String> {
        visible_lines(&self.scratchpad.document_lines, self.scratchpad.viewport)
    }

    pub fn visible_compute_records(&self) -> Vec<String> {
        let lines: Vec<String> = self
            .compute
            .history
            .iter()
            .map(|record| {
                let summary = &record.summary;
                let payload = &record.payload;
                format!("{summary} :: {payload}")
            })
            .collect();
        visible_lines(&lines, self.compute.viewport)
    }

    pub fn render_short_term_messages(&self, messages: &[Message]) -> Vec<Message> {
        retain_short_term_messages(messages, self.budget.short_term_budget)
    }

    fn render_dock(&self, mounted_apps: &BTreeSet<AppId>) -> String {
        let entries = [
            self.render_dock_entry(AppId::Scratchpad, self.scratchpad.lifecycle, self.scratchpad.pinned, mounted_apps.contains(&AppId::Scratchpad)),
            self.render_dock_entry(AppId::WebSearch, self.web_search.lifecycle, self.web_search.pinned, mounted_apps.contains(&AppId::WebSearch)),
            self.render_dock_entry(AppId::StackSearch, self.stack_search.lifecycle, self.stack_search.pinned, mounted_apps.contains(&AppId::StackSearch)),
            self.render_dock_entry(AppId::Compute, self.compute.lifecycle, self.compute.pinned, mounted_apps.contains(&AppId::Compute)),
        ];

        let pinned_apps = self.collect_apps(|app_id| self.is_pinned(app_id));
        let open_apps = self.collect_apps(|app_id| self.lifecycle_for(app_id).is_open());
        let mounted_labels = mounted_apps.iter().map(|app_id| app_id.label()).collect::<Vec<_>>().join(", ");

        let mut content = format!(
            "DOCK\nfocused: {}\npinned: [{}]\nopen: [{}]\nmounted: [{}]\n",
            self.dock.focused_app.label(),
            pinned_apps,
            open_apps,
            mounted_labels,
        );
        for entry in entries.into_iter().flatten() {
            content.push_str("- ");
            content.push_str(&entry);
            content.push('\n');
        }
        clamp_text_to_budget(&content, self.budget.dock_budget)
    }

    fn render_near_events(&self) -> String {
        if self.near_events.is_empty() {
            return format!("NEAR HSTACK EVENTS\npolicy: {NEAR_EVENT_POLICY_VERSION}\n- none\n");
        }

        let mut content = format!("NEAR HSTACK EVENTS\npolicy: {NEAR_EVENT_POLICY_VERSION}\n");
        for item in &self.near_events {
            content.push_str("- ");
            content.push_str(&item.title);
            content.push_str(" [");
            content.push_str(&item.reason);
            content.push(']');
            if let Some(timestamp) = &item.timestamp_iso {
                content.push_str(" @ ");
                content.push_str(timestamp);
            }
            content.push('\n');
        }
        clamp_text_to_budget(&content, self.budget.near_event_budget)
    }

    fn render_dock_entry(&self, app_id: AppId, lifecycle: AppLifecycle, pinned: bool, mounted: bool) -> Option<String> {
        if !app_is_available(app_id) {
            return None;
        }
        let app_label = app_id.label();
        let mut label = format!("{app_label} :: {lifecycle:?}");
        if self.dock.focused_app == app_id {
            label.push_str(" :: focused");
        }
        if mounted {
            label.push_str(" :: mounted");
        } else if lifecycle.is_open() {
            label.push_str(" :: unmounted");
        }
        if pinned {
            label.push_str(" :: pinned");
        }
        Some(label)
    }

    fn lifecycle_for(&self, app_id: AppId) -> AppLifecycle {
        match app_id {
            AppId::Scratchpad => self.scratchpad.lifecycle,
            AppId::WebSearch => self.web_search.lifecycle,
            AppId::StackSearch => self.stack_search.lifecycle,
            AppId::Compute => self.compute.lifecycle,
        }
    }

    fn is_pinned(&self, app_id: AppId) -> bool {
        match app_id {
            AppId::Scratchpad => self.scratchpad.pinned,
            AppId::WebSearch => self.web_search.pinned,
            AppId::StackSearch => self.stack_search.pinned,
            AppId::Compute => self.compute.pinned,
        }
    }

    fn collect_apps<F>(&self, predicate: F) -> String
    where
        F: Fn(AppId) -> bool,
    {
        [AppId::Scratchpad, AppId::WebSearch, AppId::StackSearch, AppId::Compute]
            .into_iter()
            .filter(|app_id| app_is_available(*app_id))
            .filter(|app_id| predicate(*app_id))
            .map(|app_id| app_id.label())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn normalize_unavailable_apps(&mut self) {
        if !app_is_available(AppId::WebSearch) {
            self.web_search.lifecycle = AppLifecycle::InstalledClosed;
            self.web_search.pinned = false;
            self.web_search.focused_query = None;
            self.web_search.results.clear();
            self.dock.mounted_apps.remove(&AppId::WebSearch);
            if self.dock.focused_app == AppId::WebSearch {
                self.dock.focused_app = AppId::Scratchpad;
            }
        }
    }

    fn sync_lifecycle_mount_state(&mut self) {
        sync_app_mount_state(
            &mut self.scratchpad.lifecycle,
            AppId::Scratchpad,
            self.dock.focused_app,
            &self.dock.mounted_apps,
        );
        sync_app_mount_state(
            &mut self.web_search.lifecycle,
            AppId::WebSearch,
            self.dock.focused_app,
            &self.dock.mounted_apps,
        );
        sync_app_mount_state(
            &mut self.stack_search.lifecycle,
            AppId::StackSearch,
            self.dock.focused_app,
            &self.dock.mounted_apps,
        );
        sync_app_mount_state(
            &mut self.compute.lifecycle,
            AppId::Compute,
            self.dock.focused_app,
            &self.dock.mounted_apps,
        );
    }

    fn open_app(&mut self, app_id: AppId) {
        match app_id {
            AppId::Scratchpad => self.scratchpad.lifecycle = AppLifecycle::OpenMounted,
            AppId::WebSearch => self.web_search.lifecycle = AppLifecycle::OpenUnmounted,
            AppId::StackSearch => self.stack_search.lifecycle = AppLifecycle::OpenUnmounted,
            AppId::Compute => self.compute.lifecycle = AppLifecycle::OpenUnmounted,
        }
    }

    fn close_app(&mut self, app_id: AppId) {
        if app_id == AppId::Scratchpad {
            return;
        }

        match app_id {
            AppId::Scratchpad => {}
            AppId::WebSearch => self.web_search.lifecycle = AppLifecycle::InstalledClosed,
            AppId::StackSearch => self.stack_search.lifecycle = AppLifecycle::InstalledClosed,
            AppId::Compute => self.compute.lifecycle = AppLifecycle::InstalledClosed,
        }

        if self.dock.focused_app == app_id {
            self.focus_app(AppId::Scratchpad);
        }
    }

    fn focus_app(&mut self, app_id: AppId) {
        self.reset_focus();
        self.dock.focused_app = app_id;
        match app_id {
            AppId::Scratchpad => self.scratchpad.lifecycle = AppLifecycle::OpenMountedFocused,
            AppId::WebSearch => self.web_search.lifecycle = AppLifecycle::OpenMountedFocused,
            AppId::StackSearch => self.stack_search.lifecycle = AppLifecycle::OpenMountedFocused,
            AppId::Compute => self.compute.lifecycle = AppLifecycle::OpenMountedFocused,
        }
    }

    fn set_pinned(&mut self, app_id: AppId, pinned: bool) {
        match app_id {
            AppId::Scratchpad => self.scratchpad.pinned = pinned,
            AppId::WebSearch => self.web_search.pinned = pinned,
            AppId::StackSearch => self.stack_search.pinned = pinned,
            AppId::Compute => self.compute.pinned = pinned,
        }
    }

    fn scroll_app(&mut self, app_id: AppId, delta: isize) {
        match app_id {
            AppId::Scratchpad => {
                let total = self.scratchpad.document_lines.len();
                self.scratchpad.viewport = scroll_viewport(self.scratchpad.viewport, delta, total);
            }
            AppId::WebSearch => {
                let total = self.web_search.results.len().max(1);
                self.web_search.viewport = scroll_viewport(self.web_search.viewport, delta, total);
            }
            AppId::StackSearch => {
                let total = self.stack_search.results.len().max(1);
                self.stack_search.viewport = scroll_viewport(self.stack_search.viewport, delta, total);
            }
            AppId::Compute => {
                let total = self.compute.history.len().max(1);
                self.compute.viewport = scroll_viewport(self.compute.viewport, delta, total);
            }
        }
    }

    fn reset_focus(&mut self) {
        self.scratchpad.lifecycle = demote_focus(self.scratchpad.lifecycle);
        self.web_search.lifecycle = demote_focus(self.web_search.lifecycle);
        self.stack_search.lifecycle = demote_focus(self.stack_search.lifecycle);
        self.compute.lifecycle = demote_focus(self.compute.lifecycle);
    }

    fn candidates_for_scratchpad(&self) -> Vec<ViewportCandidate> {
        build_text_candidates(
            AppId::Scratchpad,
            self.scratchpad.lifecycle,
            self.scratchpad.pinned,
            &self.scratchpad.document_lines,
            self.scratchpad.viewport,
            "SCRATCHPAD",
            priority_class(self.dock.focused_app, AppId::Scratchpad),
        )
    }

    fn candidates_for_search(&self, app: &SearchApp) -> Vec<ViewportCandidate> {
        if !app.lifecycle.is_open() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        let app_label = app.app_id.label().to_uppercase();
        lines.push(format!("{app_label} APP"));
        if let Some(query) = &app.focused_query {
            lines.push(format!("query: {query}"));
        }
        for (index, result) in app.results.iter().enumerate() {
            let rank = index + 1;
            let title = &result.title;
            let mut line = format!("{rank}. {title}");
            if let Some(url) = &result.url {
                line.push_str(&format!(" <{url}>"));
            }
            if !result.snippet.is_empty() {
                let snippet = &result.snippet;
                line.push_str(&format!(" :: {snippet}"));
            }
            lines.push(line);
        }

        build_text_candidates(
            app.app_id,
            app.lifecycle,
            app.pinned,
            &lines,
            app.viewport,
            app.app_id.label().to_uppercase().as_str(),
            priority_class(self.dock.focused_app, app.app_id),
        )
    }

    fn candidates_for_compute(&self) -> Vec<ViewportCandidate> {
        if !self.compute.lifecycle.is_open() {
            return Vec::new();
        }

        let mut lines = vec!["COMPUTE APP".to_string()];
        for record in &self.compute.history {
            let summary = &record.summary;
            let payload = &record.payload;
            lines.push(format!("- {summary} :: {payload}"));
        }

        build_text_candidates(
            AppId::Compute,
            self.compute.lifecycle,
            self.compute.pinned,
            &lines,
            self.compute.viewport,
            "COMPUTE",
            priority_class(self.dock.focused_app, AppId::Compute),
        )
    }
}

pub fn compose_workspace_system_message(
    base_prompt: &str,
    memory: &crate::memory::WorkingMemory,
    tickets: &[Ticket],
    settings: &UserSettings,
    pending_actions: &[SyncAction],
) -> String {
    let now = Local::now();
    let local_time = now.format("%H:%M").to_string();
    let local_date = now.format("%Y-%m-%d").to_string();
    let weekday = now.format("%A").to_string();
    let offset = now.offset().to_string();

    let recent_actions_str = render_recent_actions(pending_actions);

    let saved_locations_str =
        hstack_core::location_utils::format_saved_locations_for_prompt(&settings.saved_locations);

    let mut content = base_prompt
        .replace("{recent_actions_str}", &recent_actions_str)
        .replace("{local_time}", &local_time)
        .replace("{local_date}", &local_date)
        .replace("{weekday}", &weekday)
        .replace("{offset}", &offset)
        .replace("{saved_locations_str}", &saved_locations_str);

    content.push_str("\n\n");
    content.push_str(&render_workspace_regions(memory, tickets));

    clamp_text_to_budget(
        &content,
        memory.workspace.budget.prompt_budget
            + memory.workspace.budget.dock_budget
            + memory.workspace.budget.near_event_budget
            + memory.workspace.budget.app_budget,
    )
}

fn render_recent_actions(pending_actions: &[SyncAction]) -> String {
    if pending_actions.is_empty() {
        return "- none".to_string();
    }

    pending_actions
        .iter()
        .rev()
        .take(5)
        .map(|a| {
            let title = a
                .payload
                .as_ref()
                .map(|p| p.get_title())
                .unwrap_or("Unknown");
            format!(
                "- {:?}: {} ({}) at {}",
                a.r#type, title, a.entity_type, a.timestamp
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_workspace_regions(memory: &crate::memory::WorkingMemory, tickets: &[Ticket]) -> String {
    let mut workspace = memory.workspace.clone();
    workspace.refresh_near_events(tickets);
    let plan = workspace.materialize_allocation_plan();

    let mut content = String::new();
    content.push_str(&render_projected_stack(tickets, memory.workspace.budget.app_budget / 3));
    content.push('\n');
    content.push_str(&plan.dock_content);
    content.push('\n');
    content.push_str(&plan.near_event_content);
    content.push('\n');
    for (_, block) in plan.selected_app_contents {
        content.push_str(&block);
        content.push('\n');
    }

    content
}

fn render_projected_stack(tickets: &[Ticket], budget_tokens: usize) -> String {
    let mut content = String::from("PROJECTED STACK\n");

    if tickets.is_empty() {
        content.push_str("- none\n");
        return content;
    }

    for ticket in tickets {
        content.push_str("- ");
        content.push_str(match ticket.r#type {
            TicketType::Task => "TASK",
            TicketType::Habit => "HABIT",
            TicketType::Event => "EVENT",
            TicketType::Commute => "COMMUTE",
            TicketType::Countdown => "COUNTDOWN",
        });
        content.push_str(" :: ");
        content.push_str(match ticket.status {
            hstack_core::ticket::TicketStatus::Idle => "idle",
            hstack_core::ticket::TicketStatus::InFocus => "in_focus",
            hstack_core::ticket::TicketStatus::Completed => "completed",
            hstack_core::ticket::TicketStatus::Expired => "expired",
        });
        content.push_str(" :: ");
        content.push_str(ticket.title.as_str());

        if let Some(schedule) = ticket.payload.shared_schedule() {
            if let Some(scheduled_time_iso) = schedule.scheduled_time_iso {
                content.push_str(" @ ");
                content.push_str(&scheduled_time_iso);
            }
        }

        content.push_str(" [");
        let shortened = ticket.id.chars().take(8).collect::<String>();
        content.push_str(&shortened);
        content.push_str("]\n");
    }

    clamp_text_to_budget(&content, budget_tokens.max(256))
}

pub fn render_workspace_projection(memory: &crate::memory::WorkingMemory, tickets: &[Ticket]) -> String {
    let mut content = String::new();
    content.push_str("SHORT-TERM KERNEL\n");
    content.push_str(&format!("policy: {SHORT_TERM_POLICY_VERSION}\n"));
    content.push_str("guarantee: latest user message remains mounted even under pressure\n");
    for message in short_term_messages(memory) {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let content_text = message.content.unwrap_or_default();
        if !content_text.trim().is_empty() {
            content.push_str("- ");
            content.push_str(role);
            content.push_str(": ");
            content.push_str(&content_text);
            content.push('\n');
        }
    }
    content.push('\n');
    content.push_str(&render_workspace_regions(memory, tickets));

    content
}

pub fn short_term_messages(memory: &crate::memory::WorkingMemory) -> Vec<Message> {
    memory.workspace.render_short_term_messages(&memory.messages)
}

pub fn retain_short_term_messages(messages: &[Message], budget: usize) -> Vec<Message> {
    let latest_user_index = messages.iter().rposition(|message| matches!(message.role, Role::User));
    let mut selected_indices = BTreeSet::new();
    let mut spent = 0usize;

    if let Some(index) = latest_user_index {
        selected_indices.insert(index);
        spent = spent.saturating_add(short_term_message_cost(&messages[index]));
    }

    for index in (0..messages.len()).rev() {
        if Some(index) == latest_user_index {
            continue;
        }

        let cost = short_term_message_cost(&messages[index]);
        if spent.saturating_add(cost) > budget {
            if !selected_indices.is_empty() {
                break;
            }
            continue;
        }

        selected_indices.insert(index);
        spent = spent.saturating_add(cost);
    }

    selected_indices
        .into_iter()
        .map(|index| messages[index].clone())
        .collect()
}

pub fn workspace_runtime_snapshot(memory: &crate::memory::WorkingMemory) -> Value {
    serde_json::json!({
        "policy": {
            "short_term": SHORT_TERM_POLICY_VERSION,
            "near_event": NEAR_EVENT_POLICY_VERSION,
        },
        "dock": {
            "focused_app": memory.workspace.dock.focused_app.label(),
            "mounted_apps": memory
                .workspace
                .dock
                .mounted_apps
                .iter()
                .map(|app_id| app_id.label())
                .collect::<Vec<_>>(),
        },
        "apps": {
            "scratchpad": {
                "lifecycle": format!("{:?}", memory.workspace.scratchpad.lifecycle),
                "lines": memory.workspace.scratchpad.document_lines.len(),
            },
            "web_search": {
                "lifecycle": format!("{:?}", memory.workspace.web_search.lifecycle),
                "results": memory.workspace.web_search.results.len(),
            },
            "stack_search": {
                "lifecycle": format!("{:?}", memory.workspace.stack_search.lifecycle),
                "results": memory.workspace.stack_search.results.len(),
            },
            "compute": {
                "lifecycle": format!("{:?}", memory.workspace.compute.lifecycle),
                "history": memory.workspace.compute.history.len(),
            },
        }
    })
}

fn build_text_candidates(
    app_id: AppId,
    lifecycle: AppLifecycle,
    pinned: bool,
    lines: &[String],
    viewport: TextViewport,
    header: &str,
    priority: AppPriorityClass,
) -> Vec<ViewportCandidate> {
    if !lifecycle.is_open() {
        return Vec::new();
    }

    let preferred_lines = viewport.line_count.max(6);
    let minimal_lines = preferred_lines.clamp(4, 8);

    let minimal = render_window(header, lines, viewport.start_line, minimal_lines);
    let preferred = render_window(header, lines, viewport.start_line, preferred_lines);
    let mandatory = pinned || lifecycle.is_focused();

    vec![
        ViewportCandidate {
            app_id,
            tier: ViewportTier::Minimal,
            mandatory,
            utility: base_utility(priority) + 20,
            cost_tokens: estimate_token_cost(&minimal),
            content: minimal,
        },
        ViewportCandidate {
            app_id,
            tier: ViewportTier::Preferred,
            mandatory: false,
            utility: base_utility(priority) + 50,
            cost_tokens: estimate_token_cost(&preferred),
            content: preferred,
        },
    ]
}

fn render_window(header: &str, lines: &[String], start_line: usize, line_count: usize) -> String {
    let mut content = String::new();
    content.push_str(header);
    content.push('\n');

    if lines.is_empty() {
        content.push_str("- empty\n");
        return content;
    }

    let clamped = TextViewport::new(start_line, line_count).clamp(lines.len());
    let end = (clamped.start_line + clamped.line_count).min(lines.len());
    for line in &lines[clamped.start_line..end] {
        content.push_str("- ");
        content.push_str(line);
        content.push('\n');
    }
    content
}

fn solve_viewport_selection(candidates: &[ViewportCandidate], budget: usize) -> Vec<ViewportCandidate> {
    let mut app_groups: Vec<Vec<ViewportCandidate>> = Vec::new();
    let mut min_mandatory_cost = 0;

    for app_id in [AppId::Scratchpad, AppId::WebSearch, AppId::StackSearch, AppId::Compute] {
        let app_candidates: Vec<ViewportCandidate> = candidates
            .iter()
            .filter(|candidate| candidate.app_id == app_id)
            .cloned()
            .collect();
            
        if !app_candidates.is_empty() {
            if let Some(mandatory) = app_candidates.iter().find(|c| c.mandatory) {
                min_mandatory_cost += mandatory.cost_tokens;
            }
            app_groups.push(app_candidates);
        }
    }

    if min_mandatory_cost >= budget {
        let mut fallback = Vec::new();
        for group in &app_groups {
            if let Some(mand) = group.iter().find(|c| c.mandatory) {
                fallback.push(mand.clone());
            }
        }
        return fallback;
    }

    let mut best_value = -1i64;
    let mut best_selection = Vec::new();
    let mut current_selection = Vec::new();

    solve_groups(
        &app_groups,
        0,
        budget,
        0,
        &mut current_selection,
        &mut best_selection,
        &mut best_value,
    );

    if best_selection.is_empty() {
        let mut fallback = Vec::new();
        for group in &app_groups {
            if let Some(mand) = group.iter().find(|c| c.mandatory) {
                fallback.push(mand.clone());
            }
        }
        return fallback;
    }

    best_selection
}

fn solve_groups(
    groups: &[Vec<ViewportCandidate>],
    index: usize,
    remaining_budget: usize,
    current_value: i64,
    current_selection: &mut Vec<ViewportCandidate>,
    best_selection: &mut Vec<ViewportCandidate>,
    best_value: &mut i64,
) {
    if index >= groups.len() {
        if current_value > *best_value {
            *best_value = current_value;
            *best_selection = current_selection.clone();
        }
        return;
    }

    let group = &groups[index];
    let has_mandatory = group.iter().any(|c| c.mandatory);

    if !has_mandatory {
        solve_groups(
            groups,
            index + 1,
            remaining_budget,
            current_value,
            current_selection,
            best_selection,
            best_value,
        );
    }

    for candidate in group {
        if candidate.cost_tokens > remaining_budget {
            continue;
        }
        current_selection.push(candidate.clone());
        solve_groups(
            groups,
            index + 1,
            remaining_budget - candidate.cost_tokens,
            current_value + candidate.utility,
            current_selection,
            best_selection,
            best_value,
        );
        current_selection.pop();
    }
}

fn visible_lines(lines: &[String], viewport: TextViewport) -> Vec<String> {
    let clamped = viewport.clamp(lines.len());
    let end = (clamped.start_line + clamped.line_count).min(lines.len());
    lines[clamped.start_line..end].to_vec()
}

fn inspect_search_app(app: &SearchApp) -> Value {
    let lines: Vec<String> = app
        .results
        .iter()
        .map(|result| {
            let mut line = result.title.clone();
            if !result.snippet.is_empty() {
                line.push_str(" :: ");
                line.push_str(&result.snippet);
            }
            line
        })
        .collect();
    serde_json::json!({
        "app_id": app.app_id.label(),
        "lifecycle": format!("{:?}", app.lifecycle),
        "pinned": app.pinned,
        "viewport": app.viewport,
        "focused_query": app.focused_query,
        "visible": visible_lines(&lines, app.viewport),
    })
}

fn estimate_token_cost(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

fn clamp_text_to_budget(text: &str, budget_tokens: usize) -> String {
    let max_chars = budget_tokens.saturating_mul(4);
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    text.chars().take(max_chars).collect()
}

fn priority_class(focused_app: AppId, app_id: AppId) -> AppPriorityClass {
    if focused_app == app_id {
        AppPriorityClass::Foreground
    } else {
        AppPriorityClass::Supporting
    }
}

fn base_utility(priority: AppPriorityClass) -> i64 {
    match priority {
        AppPriorityClass::Foreground => 300,
        AppPriorityClass::Supporting => 150,
        AppPriorityClass::Background => 50,
    }
}

fn demote_focus(lifecycle: AppLifecycle) -> AppLifecycle {
    match lifecycle {
        AppLifecycle::OpenMountedFocused => AppLifecycle::OpenMounted,
        _ => lifecycle,
    }
}

fn scroll_viewport(viewport: TextViewport, delta: isize, total_lines: usize) -> TextViewport {
    let moved = if delta.is_negative() {
        viewport.start_line.saturating_sub(delta.unsigned_abs())
    } else {
        viewport.start_line.saturating_add(delta as usize)
    };
    TextViewport::new(moved, viewport.line_count).clamp(total_lines)
}

fn scheduled_timestamp(ticket: &Ticket) -> Option<DateTime<Utc>> {
    ticket
        .payload
        .shared_schedule()
        .and_then(|schedule| schedule.scheduled_time_iso)
        .and_then(|timestamp| DateTime::parse_from_rfc3339(&timestamp).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| countdown_timestamp(ticket))
}

fn countdown_timestamp(ticket: &Ticket) -> Option<DateTime<Utc>> {
    match &ticket.payload {
        TicketPayload::Countdown { expires_at, .. } => expires_at
            .as_ref()
            .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        _ => None,
    }
}

fn is_urgent(ticket: &Ticket) -> bool {
    match &ticket.payload {
        TicketPayload::Commute { priority, .. }
        | TicketPayload::Countdown { priority, .. }
        | TicketPayload::Event { priority, .. }
        | TicketPayload::Habit { priority, .. }
        | TicketPayload::Task { priority, .. } => matches!(priority, Some(TicketPriority::Urgent)),
        TicketPayload::Generic(_) => false,
    }
}

fn sync_app_mount_state(
    lifecycle: &mut AppLifecycle,
    app_id: AppId,
    focused_app: AppId,
    mounted_apps: &BTreeSet<AppId>,
) {
    if !lifecycle.is_open() {
        return;
    }

    let mounted = mounted_apps.contains(&app_id);
    *lifecycle = if focused_app == app_id && mounted {
        AppLifecycle::OpenMountedFocused
    } else if mounted {
        AppLifecycle::OpenMounted
    } else {
        AppLifecycle::OpenUnmounted
    };
}