use crate::ai::{
    ChatMessage, CompletionItem, compute_heuristic_ghost_completion, compute_local_completions,
    request_chat_assistant, request_inline_completion,
};
use crate::config::{AiProvider, AppConfig, ConnectionProfile, QueryHistoryEntry, SslMode};
use crate::db::{ActiveSession, DatabaseSchema, PendingCellEdit, QueryResult};
use crate::export::{save_string_to_path, to_csv_string, to_json_string, to_sql_inserts};
use crate::syntax::{format_sql_pretty, highlight_sql};
use chrono::Local;
use egui::{Color32, Key, KeyboardShortcut, Modifiers, RichText, Stroke, Ui};
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;

pub enum AsyncEvent {
    Connected {
        profile_id: String,
        session: Arc<ActiveSession>,
        databases: Vec<String>,
        initial_schema: Option<DatabaseSchema>,
    },
    ConnectionFailed(String),
    SchemaLoaded(DatabaseSchema),
    QueryFinished {
        tab_id: String,
        result: Result<QueryResult, String>,
    },
    CellEditsApplied {
        tab_id: String,
        updated_count: Result<u64, String>,
    },
    InlineAiCompletion {
        tab_id: String,
        result: Result<String, String>,
    },
    ChatAiReply(Result<ChatMessage, String>),
}

#[derive(Clone)]
pub struct QueryTab {
    pub id: String,
    pub title: String,
    pub sql: String,
    pub ghost_completion: Option<String>,
    pub local_suggestions: Vec<CompletionItem>,
    pub current_token_prefix: String,
    pub is_running: bool,
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub row_filter: String,
    pub current_page: usize,
    pub page_size: usize,
    pub sort_column: Option<(usize, bool)>, // (col_idx, ascending)
    pub editing_cell: Option<(usize, usize, String)>, // (row_idx, col_idx, buffer)
    pub pending_edits: Vec<PendingCellEdit>,
}

impl QueryTab {
    pub fn new(title: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            sql: sql.into(),
            ghost_completion: None,
            local_suggestions: Vec::new(),
            current_token_prefix: String::new(),
            is_running: false,
            last_result: None,
            last_error: None,
            row_filter: String::new(),
            current_page: 0,
            page_size: 100,
            sort_column: None,
            editing_cell: None,
            pending_edits: Vec::new(),
        }
    }
}

#[derive(PartialEq, Eq)]
enum SidebarTab {
    Schema,
    Connections,
    SnippetsAndHistory,
}

#[derive(PartialEq, Eq)]
enum BottomTab {
    ResultsGrid,
    ErRelationships,
    ExecutionPlanAndLog,
}

pub struct MysqlRustyApp {
    rt: Arc<Runtime>,
    tx: UnboundedSender<AsyncEvent>,
    rx: UnboundedReceiver<AsyncEvent>,

    config: AppConfig,
    active_profile_id: Option<String>,
    active_session: Option<Arc<ActiveSession>>,
    databases: Vec<String>,
    active_database: String,
    schemas: HashMap<String, DatabaseSchema>,
    schema_filter: String,

    tabs: Vec<QueryTab>,
    active_tab_idx: usize,

    sidebar_tab: SidebarTab,
    bottom_tab: BottomTab,
    show_ai_panel: bool,
    show_ai_settings_modal: bool,
    show_connection_modal: bool,
    editing_connection: ConnectionProfile,

    chat_messages: Vec<ChatMessage>,
    chat_input: String,
    is_chat_loading: bool,
    is_inline_ai_loading: bool,

    status_banner: String,
}

impl MysqlRustyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: Arc<Runtime>) -> Self {
        configure_modern_dark_theme(&cc.egui_ctx);
        let (tx, rx) = unbounded_channel();
        let config = AppConfig::load();

        let initial_tab = QueryTab::new(
            "Query 1",
            "SELECT \n    u.id,\n    u.full_name,\n    u.email,\n    u.country,\n    COUNT(o.id) AS total_orders,\n    ROUND(SUM(o.total_amount), 2) AS lifetime_value\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE o.status = 'paid'\nGROUP BY u.id, u.full_name, u.email, u.country\nORDER BY lifetime_value DESC\nLIMIT 25;",
        );

        let mut app = Self {
            rt,
            tx,
            rx,
            config,
            active_profile_id: None,
            active_session: None,
            databases: Vec::new(),
            active_database: "ecommerce_prod".to_string(),
            schemas: HashMap::new(),
            schema_filter: String::new(),
            tabs: vec![initial_tab],
            active_tab_idx: 0,
            sidebar_tab: SidebarTab::Schema,
            bottom_tab: BottomTab::ResultsGrid,
            show_ai_panel: true,
            show_ai_settings_modal: false,
            show_connection_modal: false,
            editing_connection: ConnectionProfile::default(),
            chat_messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: "Welcome to **MySQL-Rusty Workbench**!\n\nI have automatic context of your active database schema (`tables`, `columns`, `PK/FK relations`).\n- Press **`Ctrl+Space`** in the editor to trigger AI inline ghost completion (`Tab` to accept).\n- Ask me to generate complex `JOIN` queries, explain `EXPLAIN` plans, or suggest missing indexes.".to_string(),
                extracted_sql: None,
            }],
            chat_input: String::new(),
            is_chat_loading: false,
            is_inline_ai_loading: false,
            status_banner: "Ready — Connecting to Sandbox Demo (ecommerce_prod)...".to_string(),
        };

        // Automatically connect to the built-in Sandbox Demo profile on startup so UI is immediately interactive
        if let Some(demo) = app.config.connections.first().cloned() {
            app.connect_to_profile(demo);
        }

        app
    }

    fn connect_to_profile(&mut self, profile: ConnectionProfile) {
        self.status_banner = format!("Connecting to `{}` ({})...", profile.name, profile.host);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match ActiveSession::connect(profile.clone()).await {
                Ok(session) => {
                    let session = Arc::new(session);
                    let dbs = session.list_databases().await.unwrap_or_default();
                    let target_db = if !profile.default_database.is_empty() {
                        profile.default_database.clone()
                    } else {
                        dbs.first().cloned().unwrap_or_default()
                    };
                    let initial_schema = if !target_db.is_empty() {
                        session.introspect_database(&target_db).await.ok()
                    } else {
                        None
                    };
                    let _ = tx.send(AsyncEvent::Connected {
                        profile_id: profile.id,
                        session,
                        databases: dbs,
                        initial_schema,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AsyncEvent::ConnectionFailed(format!("{e:#}")));
                }
            }
        });
    }

    fn reload_active_schema(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let db = self.active_database.clone();
        if db.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Ok(schema) = session.introspect_database(&db).await {
                let _ = tx.send(AsyncEvent::SchemaLoaded(schema));
            }
        });
    }

    fn execute_current_tab_query(&mut self, prefix_override: Option<&str>) {
        let Some(session) = self.active_session.clone() else {
            self.status_banner = "Not connected to any database session.".to_string();
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        let sql = match prefix_override {
            Some(prefix) => format!("{prefix} {}", tab.sql.trim()),
            None => tab.sql.trim().to_string(),
        };
        if sql.is_empty() {
            return;
        }

        tab.is_running = true;
        tab.last_error = None;
        tab.pending_edits.clear();
        tab.editing_cell = None;
        let tab_id = tab.id.clone();
        let db = self.active_database.clone();
        let schema = self.schemas.get(&db).cloned();
        let tx = self.tx.clone();

        self.status_banner = format!("Running query on `{db}`...");
        self.rt.spawn(async move {
            let res = session
                .execute_query(&db, &sql, schema.as_ref())
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::QueryFinished {
                tab_id,
                result: res,
            });
        });
    }

    fn trigger_inline_ai_completion(&mut self) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let tab_id = tab.id.clone();
        let sql_prefix = tab.sql.clone();
        let ddl = self
            .schemas
            .get(&self.active_database)
            .map(|s| s.to_ai_ddl_context())
            .unwrap_or_default();
        let ai_cfg = self.config.ai.clone();
        let tx = self.tx.clone();

        self.is_inline_ai_loading = true;
        self.status_banner = format!(
            "Requesting inline SQL completion from {}...",
            ai_cfg.provider.label()
        );

        self.rt.spawn(async move {
            let res = request_inline_completion(&ai_cfg, &sql_prefix, &ddl)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::InlineAiCompletion {
                tab_id,
                result: res,
            });
        });
    }

    fn send_chat_prompt(&mut self, custom_prompt: Option<String>) {
        let prompt = custom_prompt.unwrap_or_else(|| self.chat_input.trim().to_string());
        if prompt.is_empty() {
            return;
        }
        self.chat_input.clear();
        self.chat_messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.clone(),
            extracted_sql: None,
        });
        self.is_chat_loading = true;

        let current_sql = self
            .tabs
            .get(self.active_tab_idx)
            .map(|t| t.sql.clone())
            .unwrap_or_default();
        let ddl = self
            .schemas
            .get(&self.active_database)
            .map(|s| s.to_ai_ddl_context())
            .unwrap_or_default();
        let ai_cfg = self.config.ai.clone();
        let tx = self.tx.clone();

        self.rt.spawn(async move {
            let res = request_chat_assistant(&ai_cfg, &prompt, &current_sql, &ddl)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ChatAiReply(res));
        });
    }

    fn process_async_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AsyncEvent::Connected {
                    profile_id,
                    session,
                    databases,
                    initial_schema,
                } => {
                    self.active_profile_id = Some(profile_id);
                    self.active_session = Some(session);
                    self.databases = databases;
                    if let Some(schema) = initial_schema {
                        self.active_database = schema.name.clone();
                        self.schemas.insert(schema.name.clone(), schema);
                    } else if let Some(first) = self.databases.first() {
                        self.active_database = first.clone();
                    }
                    self.status_banner = format!(
                        "Connected to database `{}` ({} schemas discovered)",
                        self.active_database,
                        self.databases.len()
                    );
                    // Run default query automatically on first connect
                    if self
                        .tabs
                        .first()
                        .map(|t| t.last_result.is_none())
                        .unwrap_or(false)
                    {
                        self.execute_current_tab_query(None);
                    }
                }
                AsyncEvent::ConnectionFailed(err) => {
                    self.status_banner = format!("Connection error: {err}");
                }
                AsyncEvent::SchemaLoaded(schema) => {
                    self.status_banner = format!(
                        "Loaded schema `{}` ({} tables)",
                        schema.name,
                        schema.tables.len()
                    );
                    self.schemas.insert(schema.name.clone(), schema);
                }
                AsyncEvent::QueryFinished { tab_id, result } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_running = false;
                        match result {
                            Ok(res) => {
                                self.status_banner = res.status_message.clone();
                                self.config.history.insert(
                                    0,
                                    QueryHistoryEntry {
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        database: res.database.clone(),
                                        sql: res.sql.clone(),
                                        duration_ms: res.duration_ms,
                                        row_count: res.rows.len(),
                                        success: true,
                                    },
                                );
                                self.config.history.truncate(50);
                                let _ = self.config.save();
                                tab.last_result = Some(res);
                                tab.last_error = None;
                                tab.current_page = 0;
                            }
                            Err(err) => {
                                self.status_banner = format!("Query failed: {err}");
                                tab.last_error = Some(err);
                            }
                        }
                    }
                }
                AsyncEvent::CellEditsApplied {
                    tab_id,
                    updated_count,
                } => match updated_count {
                    Ok(cnt) => {
                        self.status_banner =
                            format!("Successfully committed {cnt} inline cell update(s).");
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.pending_edits.clear();
                        }
                        self.execute_current_tab_query(None);
                    }
                    Err(e) => {
                        self.status_banner = format!("Failed to commit inline edits: {e}");
                    }
                },
                AsyncEvent::InlineAiCompletion { tab_id, result } => {
                    self.is_inline_ai_loading = false;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        match result {
                            Ok(completion) if !completion.is_empty() => {
                                tab.ghost_completion = Some(completion);
                                self.status_banner =
                                    "AI inline suggestion ready — Press Tab to accept.".to_string();
                            }
                            Ok(_) => {
                                self.status_banner =
                                    "AI returned an empty inline completion.".to_string();
                            }
                            Err(err) => {
                                // Fallback to heuristic completion so the user still gets a smart suggestion
                                let schema = self.schemas.get(&self.active_database);
                                tab.ghost_completion =
                                    compute_heuristic_ghost_completion(&tab.sql, schema);
                                self.status_banner = format!(
                                    "AI provider unreachable ({err}); using local schema heuristic."
                                );
                            }
                        }
                    }
                }
                AsyncEvent::ChatAiReply(res) => {
                    self.is_chat_loading = false;
                    match res {
                        Ok(msg) => self.chat_messages.push(msg),
                        Err(err) => self.chat_messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: format!(
                                "⚠️ **AI Provider Error:** {err}\n\nTip: Check **AI Settings** in the top toolbar to configure Ollama, OpenAI, Anthropic, Gemini, or your local CLI (`agy`)."
                            ),
                            extracted_sql: None,
                        }),
                    }
                }
            }
        }
    }
}

impl eframe::App for MysqlRustyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_async_events();

        // Global keyboard shortcuts
        let run_shortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
        let ai_complete_shortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Space);
        if ui.ctx().input_mut(|i| i.consume_shortcut(&run_shortcut)) {
            self.execute_current_tab_query(None);
        }
        if ui.ctx().input_mut(|i| i.consume_shortcut(&ai_complete_shortcut)) {
            self.trigger_inline_ai_completion();
        }

        self.render_top_toolbar(ui);
        self.render_bottom_status_bar(ui);
        self.render_left_sidebar(ui);
        if self.show_ai_panel {
            self.render_right_ai_panel(ui);
        }
        self.render_central_workspace(ui);

        let ctx = ui.ctx().clone();
        self.render_connection_modal(&ctx);
        self.render_ai_settings_modal(&ctx);
    }
}

impl MysqlRustyApp {
    fn render_top_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_toolbar")
            .exact_size(46.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.heading(
                        RichText::new("🦀 MySQL-Rusty")
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                    );
                    ui.separator();

                    // Active Connection Selector
                    let active_conn_name = self
                        .config
                        .connections
                        .iter()
                        .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
                        .map(|c| c.name.as_str())
                        .unwrap_or("Select Connection...");

                    egui::ComboBox::from_id_salt("conn_selector")
                        .selected_text(active_conn_name)
                        .width(230.0)
                        .show_ui(ui, |ui| {
                            let profiles = self.config.connections.clone();
                            for prof in profiles {
                                let is_sel = Some(&prof.id) == self.active_profile_id.as_ref();
                                if ui.selectable_label(is_sel, &prof.name).clicked() {
                                    self.connect_to_profile(prof);
                                }
                            }
                        });

                    if ui.button("➕ New Connection").clicked() {
                        self.editing_connection = ConnectionProfile::default();
                        self.show_connection_modal = true;
                    }

                    ui.separator();

                    // Database Selector
                    ui.label(RichText::new("DB:").weak());
                    let current_db = if self.active_database.is_empty() {
                        "None"
                    } else {
                        &self.active_database
                    };
                    egui::ComboBox::from_id_salt("db_selector")
                        .selected_text(current_db)
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            let dbs = self.databases.clone();
                            for db in dbs {
                                let selected = self.active_database == db;
                                if ui.selectable_label(selected, &db).clicked() {
                                    self.active_database = db;
                                    self.reload_active_schema();
                                }
                            }
                        });

                    if ui
                        .button("🔄")
                        .on_hover_text("Refresh database schema metadata")
                        .clicked()
                    {
                        self.reload_active_schema();
                    }

                    ui.separator();

                    // Query execution controls
                    let run_btn = egui::Button::new(
                        RichText::new("▶ Run (Ctrl+Enter)")
                            .color(Color32::from_rgb(16, 185, 129))
                            .strong(),
                    );
                    if ui.add(run_btn).clicked() {
                        self.execute_current_tab_query(None);
                    }

                    if ui
                        .button("🔍 EXPLAIN")
                        .on_hover_text("Inspect MySQL execution plan for current query")
                        .clicked()
                    {
                        self.execute_current_tab_query(Some("EXPLAIN"));
                    }

                    if ui
                        .button("✨ Format SQL")
                        .on_hover_text("Format SQL clauses cleanly")
                        .clicked()
                    {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.sql = format_sql_pretty(&tab.sql);
                        }
                    }

                    if ui
                        .button("🤖 AI Complete (Ctrl+Space)")
                        .on_hover_text("Trigger inline AI SQL completion at cursor")
                        .clicked()
                    {
                        self.trigger_inline_ai_completion();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .selectable_label(self.show_ai_panel, "💬 AI Copilot")
                            .clicked()
                        {
                            self.show_ai_panel = !self.show_ai_panel;
                        }
                        if ui
                            .button(format!("⚙ AI: {}", self.config.ai.provider.label()))
                            .clicked()
                        {
                            self.show_ai_settings_modal = true;
                        }
                    });
                });
            });
    }

    fn render_bottom_status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(26.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let dot_color = if self.active_session.is_some() {
                        Color32::from_rgb(16, 185, 129)
                    } else {
                        Color32::from_rgb(239, 68, 68)
                    };
                    ui.colored_label(dot_color, "●");
                    ui.label(&self.status_banner);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Tab: Accept Ghost SQL | Ctrl+Space: AI Autocomplete | Ctrl+Enter: Run")
                                .weak()
                                .small(),
                        );
                    });
                });
            });
    }

    fn render_left_sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_sidebar")
            .default_size(280.0)
            .size_range(220.0..=420.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.sidebar_tab, SidebarTab::Schema, "🗂 Schema");
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::Connections,
                        "🔌 Servers",
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::SnippetsAndHistory,
                        "📜 History",
                    );
                });
                ui.separator();

                match self.sidebar_tab {
                    SidebarTab::Schema => self.render_schema_tree(ui),
                    SidebarTab::Connections => self.render_connections_list(ui),
                    SidebarTab::SnippetsAndHistory => self.render_snippets_and_history(ui),
                }
            });
    }

    fn render_schema_tree(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("🔎");
            ui.add(
                egui::TextEdit::singleline(&mut self.schema_filter)
                    .hint_text("Filter tables or columns..."),
            );
        });
        ui.add_space(4.0);

        let Some(schema) = self.schemas.get(&self.active_database).cloned() else {
            ui.label(RichText::new("No schema loaded.").weak());
            return;
        };

        let filter_lower = self.schema_filter.to_lowercase();
        let mut pending_sql_action: Option<(String, bool)> = None; // (sql, auto_run)

        egui::ScrollArea::vertical().show(ui, |ui| {
            for table in &schema.tables {
                let matches_table = filter_lower.is_empty()
                    || table.name.to_lowercase().contains(&filter_lower)
                    || table
                        .columns
                        .iter()
                        .any(|c| c.name.to_lowercase().contains(&filter_lower));

                if !matches_table {
                    continue;
                }

                let header_label = format!("📊 {} (~{})", table.name, table.approx_rows);
                egui::CollapsingHeader::new(RichText::new(header_label).strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.small_button("SELECT *").clicked() {
                                pending_sql_action = Some((
                                    format!("SELECT * FROM `{}` LIMIT 100;", table.name),
                                    true,
                                ));
                            }
                            if ui.small_button("DDL").clicked() {
                                pending_sql_action = Some((
                                    format!("SHOW CREATE TABLE `{}`;", table.name),
                                    true,
                                ));
                            }
                            if ui.small_button("Insert Name").clicked() {
                                pending_sql_action = Some((table.name.clone(), false));
                            }
                        });

                        for col in &table.columns {
                            let icon = if col.is_primary_key { "🔑" } else { "  •" };
                            ui.horizontal(|ui| {
                                ui.label(format!("{icon} {}", col.name));
                                ui.label(
                                    RichText::new(&col.data_type)
                                        .color(Color32::from_rgb(100, 116, 139))
                                        .small(),
                                );
                            });
                        }

                        if !table.foreign_keys.is_empty() {
                            ui.collapsing(
                                RichText::new(format!(
                                    "🔗 Foreign Keys ({})",
                                    table.foreign_keys.len()
                                ))
                                .small(),
                                |ui| {
                                    for fk in &table.foreign_keys {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} -> {}.{}",
                                                fk.column_name,
                                                fk.referenced_table,
                                                fk.referenced_column
                                            ))
                                            .small(),
                                        );
                                    }
                                },
                            );
                        }
                    });
            }
        });

        if let Some((sql_snippet, run_immediately)) = pending_sql_action {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                if run_immediately {
                    tab.sql = sql_snippet;
                } else {
                    if !tab.sql.ends_with(' ') && !tab.sql.is_empty() {
                        tab.sql.push(' ');
                    }
                    tab.sql.push_str(&sql_snippet);
                }
            }
            if run_immediately {
                self.execute_current_tab_query(None);
            }
        }
    }

    fn render_connections_list(&mut self, ui: &mut Ui) {
        if ui.button("➕ Add MySQL / MariaDB Profile").clicked() {
            self.editing_connection = ConnectionProfile::default();
            self.show_connection_modal = true;
        }
        ui.add_space(6.0);

        let profiles = self.config.connections.clone();
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_delete = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for prof in profiles {
                let is_active = Some(&prof.id) == self.active_profile_id.as_ref();
                egui::Frame::group(ui.style())
                    .stroke(if is_active {
                        Stroke::new(1.5, Color32::from_rgb(16, 185, 129))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(51, 65, 85))
                    })
                    .show(ui, |ui| {
                        ui.label(RichText::new(&prof.name).strong());
                        ui.label(
                            RichText::new(format!("{}@{}:{}", prof.user, prof.host, prof.port))
                                .small()
                                .weak(),
                        );
                        if prof.ssh.enabled {
                            ui.label(
                                RichText::new(format!("🔒 SSH via {}", prof.ssh.ssh_host))
                                    .small()
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Connect").clicked() {
                                to_connect = Some(prof.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                to_edit = Some(prof.clone());
                            }
                            if !prof.is_demo && ui.small_button("Delete").clicked() {
                                to_delete = Some(prof.id.clone());
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        });

        if let Some(p) = to_connect {
            self.connect_to_profile(p);
        }
        if let Some(p) = to_edit {
            self.editing_connection = p;
            self.show_connection_modal = true;
        }
        if let Some(id) = to_delete {
            self.config.connections.retain(|c| c.id != id);
            let _ = self.config.save();
        }
    }

    fn render_snippets_and_history(&mut self, ui: &mut Ui) {
        let mut load_sql: Option<String> = None;

        ui.label(RichText::new("📌 Saved SQL Snippets").strong());
        for snip in &self.config.snippets {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(&snip.title).strong().small());
                if ui.small_button("Load into Editor").clicked() {
                    load_sql = Some(snip.sql.clone());
                }
            });
        }

        ui.separator();
        ui.label(RichText::new("🕘 Recent Query History").strong());
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &self.config.history {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&entry.timestamp).small().weak());
                        ui.label(
                            RichText::new(format!(
                                "{} rows • {} ms",
                                entry.row_count, entry.duration_ms
                            ))
                            .small()
                            .color(Color32::from_rgb(56, 189, 248)),
                        );
                    });
                    ui.label(RichText::new(&entry.sql).monospace().small());
                    if ui.small_button("Reuse SQL").clicked() {
                        load_sql = Some(entry.sql.clone());
                    }
                });
            }
        });

        if let Some(sql) = load_sql {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql;
            }
        }
    }

    fn render_right_ai_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("ai_assistant_panel")
            .default_size(340.0)
            .size_range(260.0..=520.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("🤖 AI SQL Copilot")
                            .color(Color32::from_rgb(192, 132, 252))
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("⚙ Configure").clicked() {
                            self.show_ai_settings_modal = true;
                        }
                    });
                });
                ui.label(
                    RichText::new(format!("Active Engine: {}", self.config.ai.provider.label()))
                        .small()
                        .weak(),
                );
                ui.checkbox(
                    &mut self.config.ai.include_schema_context,
                    "Include active schema DDL context",
                );
                ui.separator();

                // Quick AI actions
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("✨ Explain Current Query").clicked() {
                        self.send_chat_prompt(Some(
                            "Explain how the current SQL query works and whether any indexes can speed it up.".to_string(),
                        ));
                    }
                    if ui.small_button("🚀 Optimize & Suggest Indexes").clicked() {
                        self.send_chat_prompt(Some(
                            "Rewrite the current SQL query for maximum performance in MySQL 8.0 and provide CREATE INDEX statements if helpful.".to_string(),
                        ));
                    }
                    if ui.small_button("📊 Top Revenue Report").clicked() {
                        self.send_chat_prompt(Some(
                            "Write a MySQL query joining users, orders, and order_items to find the top customers and most profitable product categories.".to_string(),
                        ));
                    }
                });
                ui.separator();

                let mut sql_to_insert: Option<(String, bool)> = None;
                let available_height = (ui.available_height() - 95.0).max(120.0);

                egui::ScrollArea::vertical()
                    .max_height(available_height)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for msg in &self.chat_messages {
                            let is_user = msg.role == "user";
                            let bg = if is_user {
                                Color32::from_rgb(30, 41, 59)
                            } else {
                                Color32::from_rgb(15, 23, 42)
                            };
                            egui::Frame::group(ui.style()).fill(bg).show(ui, |ui| {
                                ui.label(
                                    RichText::new(if is_user { "You" } else { "AI Copilot" })
                                        .strong()
                                        .small()
                                        .color(if is_user {
                                            Color32::from_rgb(56, 189, 248)
                                        } else {
                                            Color32::from_rgb(192, 132, 252)
                                        }),
                                );
                                ui.label(&msg.content);
                                if let Some(sql_block) = &msg.extracted_sql {
                                    ui.horizontal(|ui| {
                                        if ui.small_button("📋 Insert into Editor").clicked() {
                                            sql_to_insert = Some((sql_block.clone(), false));
                                        }
                                        if ui.small_button("▶ Insert & Run").clicked() {
                                            sql_to_insert = Some((sql_block.clone(), true));
                                        }
                                    });
                                }
                            });
                            ui.add_space(4.0);
                        }
                        if self.is_chat_loading {
                            ui.spinner();
                            ui.label(RichText::new("Generating SQL response...").italics());
                        }
                    });

                if let Some((sql, run_now)) = sql_to_insert {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.sql = sql;
                    }
                    if run_now {
                        self.execute_current_tab_query(None);
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::multiline(&mut self.chat_input)
                            .desired_rows(2)
                            .desired_width(ui.available_width() - 62.0)
                            .hint_text("Ask in natural language (e.g. 'Monthly revenue by country')..."),
                    );
                    if (resp.lost_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift))
                        || ui.button("Send").clicked()
                    {
                        self.send_chat_prompt(None);
                    }
                });
            });
    }

    fn render_central_workspace(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            // Query Tabs Bar
            ui.horizontal(|ui| {
                let mut close_idx = None;
                for idx in 0..self.tabs.len() {
                    let is_active = idx == self.active_tab_idx;
                    let title = self.tabs[idx].title.clone();
                    if ui.selectable_label(is_active, &title).clicked() {
                        self.active_tab_idx = idx;
                    }
                    if self.tabs.len() > 1 && ui.small_button("×").clicked() {
                        close_idx = Some(idx);
                    }
                    ui.separator();
                }
                if let Some(idx) = close_idx {
                    self.tabs.remove(idx);
                    if self.active_tab_idx >= self.tabs.len() {
                        self.active_tab_idx = self.tabs.len().saturating_sub(1);
                    }
                }
                if ui.small_button("➕ New Tab").clicked() {
                    let n = self.tabs.len() + 1;
                    self.tabs
                        .push(QueryTab::new(format!("Query {n}"), "SELECT * FROM users LIMIT 50;"));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
            });
            ui.separator();

            // SQL Editor Section
            let schema = self.schemas.get(&self.active_database).cloned();
            let enable_ghost = self.config.ai.enable_inline_autocomplete;
            let mut request_fix_with_ai: Option<String> = None;

            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                // Accept ghost completion on Tab key
                if tab.ghost_completion.is_some()
                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
                {
                    if let Some(ghost) = tab.ghost_completion.take() {
                        tab.sql.push_str(&ghost);
                    }
                }

                let ghost_clone = tab.ghost_completion.clone();
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = highlight_sql(buf.as_str(), ghost_clone.as_deref(), 14.0);
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                let editor_resp = egui::ScrollArea::vertical()
                    .id_salt("sql_editor_scroll")
                    .max_height(195.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut tab.sql)
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .desired_rows(8)
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter),
                        )
                    })
                    .inner;

                if editor_resp.changed() {
                    let (prefix, suggestions) =
                        compute_local_completions(&tab.sql, schema.as_ref(), 8);
                    tab.current_token_prefix = prefix;
                    tab.local_suggestions = suggestions;
                    if enable_ghost {
                        tab.ghost_completion =
                            compute_heuristic_ghost_completion(&tab.sql, schema.as_ref());
                    } else {
                        tab.ghost_completion = None;
                    }
                }

                // Inline Autocomplete Bar (Schema tokens + Ghost completion accept button)
                if !tab.local_suggestions.is_empty() || tab.ghost_completion.is_some() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("⚡ Suggestions:")
                                .small()
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                        if let Some(ghost) = tab.ghost_completion.clone() {
                            let preview = ghost.replace('\n', " ↵ ");
                            if ui
                                .small_button(
                                    RichText::new(format!("✨ [Tab] {preview}"))
                                        .color(Color32::from_rgb(125, 211, 252)),
                                )
                                .clicked()
                            {
                                tab.sql.push_str(&ghost);
                                tab.ghost_completion = None;
                            }
                        }

                        let suggestions = tab.local_suggestions.clone();
                        let prefix_len = tab.current_token_prefix.len();
                        for item in suggestions {
                            let label = format!("[{}] {}", item.kind.badge(), item.label);
                            if ui
                                .small_button(label)
                                .on_hover_text(&item.detail)
                                .clicked()
                            {
                                if prefix_len <= tab.sql.len() {
                                    let new_len = tab.sql.len() - prefix_len;
                                    tab.sql.truncate(new_len);
                                }
                                tab.sql.push_str(&item.insert_text);
                                tab.local_suggestions.clear();
                            }
                        }
                    });
                }

                if let Some(err) = &tab.last_error {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(69, 10, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    Color32::from_rgb(248, 113, 113),
                                    format!("❌ SQL Error: {err}"),
                                );
                                if ui.small_button("🔧 Fix Query with AI").clicked() {
                                    request_fix_with_ai = Some(err.clone());
                                }
                            });
                        });
                }
            }

            if let Some(err_msg) = request_fix_with_ai {
                self.show_ai_panel = true;
                self.send_chat_prompt(Some(format!(
                    "My SQL query failed with the following MySQL error:\n`{err_msg}`\nPlease fix the query and explain the cause."
                )));
            }

            ui.separator();

            // Bottom Workspace Tabs (Results Grid | ER Relationships | Execution Log)
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ResultsGrid,
                    "📋 Results Grid",
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ErRelationships,
                    "🕸 ER Diagram & Schema Relations",
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ExecutionPlanAndLog,
                    "⚡ Query Stats & DDL",
                );
            });
            ui.separator();

            match self.bottom_tab {
                BottomTab::ResultsGrid => self.render_results_grid(ui),
                BottomTab::ErRelationships => self.render_er_relationships(ui),
                BottomTab::ExecutionPlanAndLog => self.render_execution_stats(ui),
            }
        });
    }

    fn render_results_grid(&mut self, ui: &mut Ui) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        if tab.is_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Executing SQL query...");
            });
            return;
        }

        let Some(result) = tab.last_result.clone() else {
            ui.label(RichText::new("Run a query (Ctrl+Enter) to view results here.").weak());
            return;
        };

        // Results Action Bar (Filter, Pagination, Export, Inline Edit Commit)
        let mut trigger_commit_edits = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("🔎 Filter rows:");
            ui.add(
                egui::TextEdit::singleline(&mut tab.row_filter)
                    .desired_width(170.0)
                    .hint_text("Search any cell..."),
            );

            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{} rows ({} ms)",
                    result.rows.len(),
                    result.duration_ms
                ))
                .strong(),
            );

            if let (Some(tbl), Some(pk)) = (&result.editable_table, &result.primary_key_column) {
                ui.label(
                    RichText::new(format!("✏ Inline Editable (`{tbl}`, PK `{pk}`) — Double-click cell"))
                        .small()
                        .color(Color32::from_rgb(16, 185, 129)),
                );
            }

            if !tab.pending_edits.is_empty() {
                if ui
                    .button(
                        RichText::new(format!(
                            "💾 Apply {} Pending Edit(s)",
                            tab.pending_edits.len()
                        ))
                        .color(Color32::from_rgb(250, 204, 21))
                        .strong(),
                    )
                    .clicked()
                {
                    trigger_commit_edits = true;
                }
                if ui.small_button("Discard").clicked() {
                    tab.pending_edits.clear();
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("📄 Export CSV").clicked() {
                    if let Ok(csv_data) = to_csv_string(&result) {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.csv")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &csv_data);
                            self.status_banner = format!("Saved CSV to {}", path.display());
                        } else {
                            ui.ctx().copy_text(csv_data);
                            self.status_banner =
                                "Copied CSV export to system clipboard.".to_string();
                        }
                    }
                }
                if ui.small_button("📦 Export JSON").clicked() {
                    if let Ok(json_data) = to_json_string(&result) {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.json")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &json_data);
                            self.status_banner = format!("Saved JSON to {}", path.display());
                        } else {
                            ui.ctx().copy_text(json_data);
                            self.status_banner =
                                "Copied JSON export to system clipboard.".to_string();
                        }
                    }
                }
                if ui.small_button("📋 Copy SQL INSERTs").clicked() {
                    let tbl = result
                        .editable_table
                        .as_deref()
                        .unwrap_or("exported_rows");
                    let inserts = to_sql_inserts(&result, tbl);
                    ui.ctx().copy_text(inserts);
                    self.status_banner = "Copied SQL INSERT statements to clipboard.".to_string();
                }
            });
        });

        ui.add_space(4.0);

        // Filter & sort rows
        let filter_lower = tab.row_filter.to_lowercase();
        let mut filtered_indices: Vec<usize> = (0..result.rows.len())
            .filter(|&r_idx| {
                if filter_lower.is_empty() {
                    return true;
                }
                result.rows[r_idx]
                    .iter()
                    .any(|c| c.display_text().to_lowercase().contains(&filter_lower))
            })
            .collect();

        if let Some((sort_col, asc)) = tab.sort_column {
            filtered_indices.sort_by(|&a, &b| {
                let va = result.rows[a]
                    .get(sort_col)
                    .map(|c| c.display_text())
                    .unwrap_or_default();
                let vb = result.rows[b]
                    .get(sort_col)
                    .map(|c| c.display_text())
                    .unwrap_or_default();
                if asc { va.cmp(&vb) } else { vb.cmp(&va) }
            });
        }

        let pk_col_idx = result.primary_key_column.as_ref().and_then(|pk| {
            result
                .columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(pk))
        });

        // Render Virtualized Data Grid
        let num_cols = result.columns.len().max(1);
        let mut builder = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

        for _ in 0..num_cols {
            builder = builder.column(Column::initial(135.0).at_least(65.0).clip(true));
        }

        builder
            .header(24.0, |mut header| {
                for (col_idx, col_name) in result.columns.iter().enumerate() {
                    header.col(|ui| {
                        let arrow = match tab.sort_column {
                            Some((c, true)) if c == col_idx => " ▲",
                            Some((c, false)) if c == col_idx => " ▼",
                            _ => "",
                        };
                        if ui
                            .button(RichText::new(format!("{col_name}{arrow}")).strong())
                            .clicked()
                        {
                            let next_asc = match tab.sort_column {
                                Some((c, asc)) if c == col_idx => !asc,
                                _ => true,
                            };
                            tab.sort_column = Some((col_idx, next_asc));
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, filtered_indices.len(), |mut row| {
                    let actual_row_idx = filtered_indices[row.index()];
                    let row_cells = &result.rows[actual_row_idx];

                    for col_idx in 0..result.columns.len() {
                        row.col(|ui| {
                            let is_editing = matches!(
                                &tab.editing_cell,
                                Some((r, c, _)) if *r == actual_row_idx && *c == col_idx
                            );

                            if is_editing {
                                let (_, _, mut edit_buf) = tab.editing_cell.clone().unwrap();
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut edit_buf)
                                        .desired_width(120.0),
                                );
                                if resp.changed() {
                                    tab.editing_cell =
                                        Some((actual_row_idx, col_idx, edit_buf.clone()));
                                }
                                if resp.lost_focus()
                                    || ui.input(|i| i.key_pressed(Key::Enter))
                                {
                                    if let (Some(pk_idx), Some(pk_name)) =
                                        (pk_col_idx, &result.primary_key_column)
                                    {
                                        let pk_val = row_cells[pk_idx].display_text();
                                        let old_val = row_cells[col_idx].display_text();
                                        if edit_buf != old_val {
                                            tab.pending_edits.push(PendingCellEdit {
                                                row_idx: actual_row_idx,
                                                col_idx,
                                                column_name: result.columns[col_idx].clone(),
                                                pk_column: pk_name.clone(),
                                                pk_value: pk_val,
                                                old_value: old_val,
                                                new_value: edit_buf,
                                            });
                                        }
                                    }
                                    tab.editing_cell = None;
                                }
                            } else {
                                let pending = tab.pending_edits.iter().find(|e| {
                                    e.row_idx == actual_row_idx && e.col_idx == col_idx
                                });
                                let cell = &row_cells[col_idx];
                                let text = if let Some(p) = pending {
                                    RichText::new(&p.new_value)
                                        .color(Color32::from_rgb(250, 204, 21))
                                        .strong()
                                } else if cell.is_null() {
                                    RichText::new("NULL")
                                        .italics()
                                        .color(Color32::from_rgb(100, 116, 139))
                                } else {
                                    RichText::new(cell.display_text())
                                };

                                let label_resp =
                                    ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                                if label_resp.double_clicked() && pk_col_idx.is_some() {
                                    tab.editing_cell = Some((
                                        actual_row_idx,
                                        col_idx,
                                        cell.display_text(),
                                    ));
                                }
                            }
                        });
                    }
                });
            });

        if trigger_commit_edits {
            if let (Some(session), Some(table_name)) =
                (self.active_session.clone(), result.editable_table.clone())
            {
                let edits = tab.pending_edits.clone();
                let tab_id = tab.id.clone();
                let db = self.active_database.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = session
                        .apply_cell_edits(&db, &table_name, &edits)
                        .await
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(AsyncEvent::CellEditsApplied {
                        tab_id,
                        updated_count: res,
                    });
                });
            }
        }
    }

    fn render_er_relationships(&mut self, ui: &mut Ui) {
        let Some(schema) = self.schemas.get(&self.active_database) else {
            ui.label("No active schema loaded.");
            return;
        };

        ui.label(
            RichText::new(format!(
                "Entity-Relationship Overview for `{}` ({} tables)",
                schema.name,
                schema.tables.len()
            ))
            .strong(),
        );
        ui.add_space(6.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for table in &schema.tables {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(15, 23, 42))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(56, 189, 248)))
                        .show(ui, |ui| {
                            ui.set_width(240.0);
                            ui.label(
                                RichText::new(format!("📊 {}", table.name))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                            ui.separator();
                            for col in &table.columns {
                                let badge = if col.is_primary_key { "🔑 " } else { "   " };
                                ui.label(
                                    RichText::new(format!(
                                        "{badge}{} : {}",
                                        col.name, col.data_type
                                    ))
                                    .monospace()
                                    .small(),
                                );
                            }
                            if !table.foreign_keys.is_empty() {
                                ui.separator();
                                for fk in &table.foreign_keys {
                                    ui.label(
                                        RichText::new(format!(
                                            "🔗 {} ➡ {}.{}",
                                            fk.column_name,
                                            fk.referenced_table,
                                            fk.referenced_column
                                        ))
                                        .small()
                                        .color(Color32::from_rgb(192, 132, 252)),
                                    );
                                }
                            }
                        });
                }
            });
        });
    }

    fn render_execution_stats(&mut self, ui: &mut Ui) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        if let Some(res) = &tab.last_result {
            ui.label(RichText::new("Last Executed SQL Statement:").strong());
            ui.code(&res.sql);
            ui.add_space(6.0);
            ui.label(format!("• Target Database: `{}`", res.database));
            ui.label(format!("• Execution Time: `{} ms`", res.duration_ms));
            ui.label(format!("• Rows Returned: `{}`", res.rows.len()));
            ui.label(format!("• Rows Affected: `{}`", res.affected_rows));
            if let Some(last_id) = res.last_insert_id {
                ui.label(format!("• Last Insert ID: `{last_id}`"));
            }
        }
        if let Some(schema) = self.schemas.get(&self.active_database) {
            ui.separator();
            ui.label(RichText::new("AI Schema DDL Context Preview:").strong());
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.code(schema.to_ai_ddl_context());
            });
        }
    }

    fn render_connection_modal(&mut self, ctx: &egui::Context) {
        if !self.show_connection_modal {
            return;
        }
        let mut open = self.show_connection_modal;
        let mut save_and_connect = false;

        egui::Window::new("🔌 MySQL / MariaDB Connection Profile")
            .open(&mut open)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::Grid::new("conn_form_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Connection Name:");
                        ui.text_edit_singleline(&mut self.editing_connection.name);
                        ui.end_row();

                        ui.label("Host / IP:");
                        ui.text_edit_singleline(&mut self.editing_connection.host);
                        ui.end_row();

                        ui.label("Port:");
                        ui.add(egui::DragValue::new(&mut self.editing_connection.port));
                        ui.end_row();

                        ui.label("Username:");
                        ui.text_edit_singleline(&mut self.editing_connection.user);
                        ui.end_row();

                        ui.label("Password:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editing_connection.password)
                                .password(true),
                        );
                        ui.end_row();

                        ui.label("Default Database:");
                        ui.text_edit_singleline(&mut self.editing_connection.default_database);
                        ui.end_row();

                        ui.label("SSL Mode:");
                        egui::ComboBox::from_id_salt("ssl_mode_combo")
                            .selected_text(format!("{:?}", self.editing_connection.ssl_mode))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Disabled,
                                    "Disabled",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Preferred,
                                    "Preferred",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Required,
                                    "Required",
                                );
                            });
                        ui.end_row();
                    });

                ui.separator();
                ui.checkbox(
                    &mut self.editing_connection.ssh.enabled,
                    "Enable SSH Tunnel Port Forwarding (`ssh -N -L`)",
                );
                if self.editing_connection.ssh.enabled {
                    egui::Grid::new("ssh_form_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("SSH Host:");
                            ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_host);
                            ui.end_row();

                            ui.label("SSH Port:");
                            ui.add(egui::DragValue::new(
                                &mut self.editing_connection.ssh.ssh_port,
                            ));
                            ui.end_row();

                            ui.label("SSH User:");
                            ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_user);
                            ui.end_row();

                            ui.label("SSH Private Key:");
                            ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_key_path);
                            ui.end_row();
                        });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save & Connect").clicked() {
                        save_and_connect = true;
                    }
                });
            });

        self.show_connection_modal = open;
        if save_and_connect {
            let prof = self.editing_connection.clone();
            if let Some(existing) = self
                .config
                .connections
                .iter_mut()
                .find(|c| c.id == prof.id)
            {
                *existing = prof.clone();
            } else {
                self.config.connections.push(prof.clone());
            }
            let _ = self.config.save();
            self.show_connection_modal = false;
            self.connect_to_profile(prof);
        }
    }

    fn render_ai_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_ai_settings_modal {
            return;
        }
        let mut open = self.show_ai_settings_modal;
        let mut should_save = false;

        egui::Window::new("🤖 AI Autocomplete & Provider Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label("Select AI Provider for Inline Autocomplete & SQL Chat:");
                egui::ComboBox::from_id_salt("ai_provider_combo")
                    .selected_text(self.config.ai.provider.label())
                    .width(340.0)
                    .show_ui(ui, |ui| {
                        let providers = [
                            AiProvider::Ollama,
                            AiProvider::OpenAiCompatible,
                            AiProvider::Anthropic,
                            AiProvider::Gemini,
                            AiProvider::CliCommand,
                        ];
                        for p in providers {
                            let label = p.label();
                            ui.selectable_value(&mut self.config.ai.provider, p, label);
                        }
                    });

                ui.checkbox(
                    &mut self.config.ai.enable_inline_autocomplete,
                    "Enable Copilot-style inline ghost SQL suggestions",
                );
                ui.separator();

                match self.config.ai.provider {
                    AiProvider::Ollama => {
                        ui.label("Ollama Endpoint URL:");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_endpoint);
                        ui.label("Model Name (e.g. `qwen2.5-coder:7b`, `codestral`, `llama3.2`):");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_model);
                    }
                    AiProvider::OpenAiCompatible => {
                        ui.label("Base URL (OpenAI, Groq, OpenRouter, vLLM):");
                        ui.text_edit_singleline(&mut self.config.ai.openai_base_url);
                        ui.label("API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.openai_api_key)
                                .password(true),
                        );
                        ui.label("Model Name:");
                        ui.text_edit_singleline(&mut self.config.ai.openai_model);
                    }
                    AiProvider::Anthropic => {
                        ui.label("Anthropic API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.anthropic_api_key)
                                .password(true),
                        );
                        ui.label("Claude Model:");
                        ui.text_edit_singleline(&mut self.config.ai.anthropic_model);
                    }
                    AiProvider::Gemini => {
                        ui.label("Google Gemini API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.gemini_api_key)
                                .password(true),
                        );
                        ui.label("Gemini Model:");
                        ui.text_edit_singleline(&mut self.config.ai.gemini_model);
                    }
                    AiProvider::CliCommand => {
                        ui.label(
                            "Local CLI Command (e.g. `agy`, `llm`, `ollama run qwen2.5-coder`):",
                        );
                        ui.text_edit_singleline(&mut self.config.ai.cli_command);
                        ui.checkbox(
                            &mut self.config.ai.cli_pass_via_stdin,
                            "Pass prompt via STDIN (uncheck to pass as final CLI argument)",
                        );
                    }
                }

                ui.separator();
                if ui.button("💾 Save AI Configuration").clicked() {
                    should_save = true;
                }
            });

        self.show_ai_settings_modal = open;
        if should_save {
            let _ = self.config.save();
            self.show_ai_settings_modal = false;
            self.status_banner = format!(
                "Saved AI configuration ({})",
                self.config.ai.provider.label()
            );
        }
    }
}

fn configure_modern_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::from_rgb(15, 23, 42); // Slate 900
    visuals.panel_fill = Color32::from_rgb(15, 23, 42);
    visuals.extreme_bg_color = Color32::from_rgb(2, 6, 23); // Slate 950 (Editor background)
    visuals.faint_bg_color = Color32::from_rgb(30, 41, 59); // Slate 800
    visuals.selection.bg_fill = Color32::from_rgb(3, 105, 161);
    ctx.set_visuals(visuals);
}
