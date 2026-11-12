use crate::ai::{
    ChatMessage, CompletionItem, compute_heuristic_ghost_completion, compute_local_completions,
    request_chat_assistant, request_inline_completion,
};
use crate::config::{
    AiProvider, AppConfig, ConnectionProfile, Environment, QueryHistoryEntry, SavedSnippet, SslMode,
};
use crate::db::{ActiveSession, DatabaseSchema, PendingCellEdit, QueryResult};
use crate::er_diagram::ErCanvasState;
use crate::export::{
    generate_mock_data_sql, parse_csv_to_batch_insert, save_string_to_path, to_csv_string,
    to_json_string, to_markdown_table, to_sql_inserts,
};
use crate::schema_diff::{diff_schemas, SchemaDiffReport};
use crate::syntax::{
    extract_statement_at_cursor, format_sql_pretty, highlight_sql, is_potentially_destructive,
    is_write_query,
};
use crate::table_designer::{TableDesignerAction, TableDraft};
use chrono::Local;
use egui::{Color32, Key, Modifiers, RichText, Stroke, Ui};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Bar, BarChart, Line, Plot, PlotPoints};
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
        req_id: u64,
        result: Result<String, String>,
    },
    ChatAiReply(Result<ChatMessage, String>),
    ServerAdminLoaded {
        status: Result<HashMap<String, String>, String>,
        processes: Result<QueryResult, String>,
    },
    QueryKilled(Result<u64, String>),
    TestConnectionResult(Result<String, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultViewMode {
    Grid,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Bar,
    Line,
}

#[derive(Clone)]
pub struct QueryTab {
    pub id: String,
    pub title: String,
    pub sql: String,
    pub cursor_index: usize,
    pub selected_sql: Option<String>,
    pub last_edit_instant: Option<std::time::Instant>,
    pub ghost_completion: Option<String>,
    pub local_suggestions: Vec<CompletionItem>,
    pub current_token_prefix: String,
    pub inline_req_id: u64,
    pub is_running: bool,
    pub task_abort_handle: Option<tokio::task::AbortHandle>,
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub row_filter: String,
    pub current_page: usize,
    pub page_size: usize,
    pub sort_column: Option<(usize, bool)>, // (col_idx, ascending)
    pub editing_cell: Option<(usize, usize, String)>, // (row_idx, col_idx, buffer)
    pub pending_edits: Vec<PendingCellEdit>,
    pub view_mode: ResultViewMode,
    pub chart_type: ChartType,
    pub chart_x_col: usize,
    pub chart_y_col: usize,
    pub file_path: Option<std::path::PathBuf>,
    pub is_dirty: bool,
}

impl QueryTab {
    pub fn new(title: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            sql: sql.into(),
            cursor_index: 0,
            selected_sql: None,
            last_edit_instant: None,
            ghost_completion: None,
            local_suggestions: Vec::new(),
            current_token_prefix: String::new(),
            inline_req_id: 0,
            is_running: false,
            task_abort_handle: None,
            last_result: None,
            last_error: None,
            row_filter: String::new(),
            current_page: 0,
            page_size: 100,
            sort_column: None,
            editing_cell: None,
            pending_edits: Vec::new(),
            view_mode: ResultViewMode::Grid,
            chart_type: ChartType::Bar,
            chart_x_col: 0,
            chart_y_col: 1,
            file_path: None,
            is_dirty: false,
        }
    }

    pub fn display_title(&self) -> String {
        if self.is_dirty {
            format!("{} *", self.title)
        } else {
            self.title.clone()
        }
    }
}

#[derive(PartialEq, Eq)]
enum SidebarTab {
    Schema,
    Connections,
    SavedQueries,
    History,
}

#[derive(PartialEq, Eq)]
enum BottomTab {
    ResultsGrid,
    ErRelationships,
    ExecutionPlanAndLog,
    ServerAdmin,
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
    table_designer_draft: Option<TableDraft>,
    er_canvas_state: ErCanvasState,
    server_status_metrics: HashMap<String, String>,
    server_processes: Option<QueryResult>,
    is_server_admin_loading: bool,

    chat_messages: Vec<ChatMessage>,
    chat_input: String,
    is_chat_loading: bool,
    is_inline_ai_loading: bool,

    testing_connection: bool,
    test_connection_result: Option<Result<String, String>>,

    status_banner: String,

    // Safety & Destructive query protection
    show_safety_modal: bool,
    safety_warning_msg: String,
    pending_safe_sql: String,

    // Schema Diff & Sync modal
    show_schema_diff_modal: bool,
    diff_source_db: String,
    diff_target_db: String,
    current_diff_report: Option<SchemaDiffReport>,

    // Mock Data Generator modal
    show_mock_data_modal: bool,
    mock_data_target_table: Option<String>,
    mock_data_row_count: usize,

    // Saved Queries & History filter
    new_snippet_title: String,
    new_snippet_desc: String,
    new_snippet_db: String,
    show_save_snippet_modal: bool,
    snippet_search_filter: String,
    history_search_filter: String,

    // Live Server Monitoring History
    qps_history: Vec<[f64; 2]>,
    threads_history: Vec<[f64; 2]>,
    last_poll_instant: Option<std::time::Instant>,
    last_questions_count: Option<u64>,
}

impl MysqlRustyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: Arc<Runtime>) -> Self {
        configure_modern_dark_theme(&cc.egui_ctx);
        let (tx, rx) = unbounded_channel();
        let config = AppConfig::load();

        let initial_tab = QueryTab::new("Query 1", "");

        Self {
            rt,
            tx,
            rx,
            config,
            active_profile_id: None,
            active_session: None,
            databases: Vec::new(),
            active_database: String::new(),
            schemas: HashMap::new(),
            schema_filter: String::new(),
            tabs: vec![initial_tab],
            active_tab_idx: 0,
            sidebar_tab: SidebarTab::Connections,
            bottom_tab: BottomTab::ResultsGrid,
            show_ai_panel: true,
            show_ai_settings_modal: false,
            show_connection_modal: false,
            editing_connection: ConnectionProfile::default(),
            table_designer_draft: None,
            er_canvas_state: ErCanvasState::default(),
            server_status_metrics: HashMap::new(),
            server_processes: None,
            is_server_admin_loading: false,
            chat_messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: "Welcome to **MySQL-Rusty Workbench**!\n\nSelect a server connection from the central dashboard or the 'Servers' sidebar tab to begin.".to_string(),
                extracted_sql: None,
            }],
            chat_input: String::new(),
            is_chat_loading: false,
            is_inline_ai_loading: false,
            testing_connection: false,
            test_connection_result: None,
            status_banner: "Ready — Select a connection to begin.".to_string(),

            show_safety_modal: false,
            safety_warning_msg: String::new(),
            pending_safe_sql: String::new(),

            show_schema_diff_modal: false,
            diff_source_db: String::new(),
            diff_target_db: String::new(),
            current_diff_report: None,

            show_mock_data_modal: false,
            mock_data_target_table: None,
            mock_data_row_count: 25,

            new_snippet_title: String::new(),
            new_snippet_desc: String::new(),
            new_snippet_db: String::new(),
            show_save_snippet_modal: false,
            snippet_search_filter: String::new(),
            history_search_filter: String::new(),

            qps_history: Vec::new(),
            threads_history: Vec::new(),
            last_poll_instant: None,
            last_questions_count: None,
        }
    }

    fn disconnect(&mut self) {
        self.active_session = None;
        self.active_profile_id = None;
        self.databases.clear();
        self.schemas.clear();
        self.active_database.clear();
        self.sidebar_tab = SidebarTab::Connections;
        self.status_banner = "Disconnected. Select a connection to begin.".to_string();
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

    fn save_active_tab_to_file(&mut self, save_as: bool) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };
        let path = if !save_as && tab.file_path.is_some() {
            tab.file_path.clone().unwrap()
        } else {
            let default_name = if !tab.title.is_empty() && !tab.title.contains(' ') {
                if tab.title.ends_with(".sql") {
                    tab.title.clone()
                } else {
                    format!("{}.sql", tab.title)
                }
            } else {
                "query.sql".to_string()
            };
            match rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter("SQL Files", &["sql", "txt"])
                .save_file()
            {
                Some(p) => p,
                None => return,
            }
        };

        if let Err(e) = std::fs::write(&path, &tab.sql) {
            self.status_banner = format!("Failed to save SQL file: {e}");
        } else {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("query.sql")
                .to_string();
            tab.title = filename;
            tab.file_path = Some(path.clone());
            tab.is_dirty = false;
            self.status_banner = format!("Saved query to {}", path.display());
        }
    }

    fn open_sql_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SQL Files", &["sql", "txt"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("query.sql")
                        .to_string();
                    let is_current_tab_empty = self
                        .tabs
                        .get(self.active_tab_idx)
                        .map(|t| t.sql.trim().is_empty())
                        .unwrap_or(false);

                    if is_current_tab_empty {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.title = filename;
                            tab.sql = content;
                            tab.file_path = Some(path.clone());
                            tab.is_dirty = false;
                        }
                    } else {
                        let mut new_tab = QueryTab::new(&filename, content);
                        new_tab.file_path = Some(path.clone());
                        self.tabs.push(new_tab);
                        self.active_tab_idx = self.tabs.len() - 1;
                    }
                    self.status_banner = format!("Opened file: {}", path.display());
                }
                Err(e) => {
                    self.status_banner = format!("Failed to read SQL file: {e}");
                }
            }
        }
    }

    fn open_save_query_modal(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            self.new_snippet_title = if !tab.title.is_empty() && !tab.title.starts_with("Query ") {
                tab.title.clone()
            } else {
                tab.sql
                    .lines()
                    .next()
                    .unwrap_or("My Query")
                    .trim_start_matches("--")
                    .trim()
                    .chars()
                    .take(45)
                    .collect::<String>()
            };
            if self.new_snippet_title.is_empty() {
                self.new_snippet_title = "My Query".to_string();
            }
            self.new_snippet_db = self.active_database.clone();
            self.new_snippet_desc.clear();
            self.show_save_snippet_modal = true;
        }
    }

    fn execute_sql_in_current_tab(&mut self, raw_sql: String, prefix_override: Option<&str>) {
        if self.active_session.is_none() {
            self.status_banner = "Not connected to any database session.".to_string();
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.last_error = Some("Not connected to any database. Please click 'Connect' on a profile in the Servers tab.".to_string());
            }
            return;
        }
        let sql = match prefix_override {
            Some(prefix) => format!("{prefix} {}", raw_sql.trim()),
            None => raw_sql.trim().to_string(),
        };
        if sql.is_empty() {
            return;
        }

        let trimmed_lower = sql.trim().to_lowercase();
        if trimmed_lower.starts_with("use ") {
            let target = sql.trim()[4..]
                .trim()
                .trim_matches(|c| c == ';' || c == '`' || c == ' ');
            if !target.is_empty() {
                self.active_database = target.to_string();
                if !self.databases.contains(&self.active_database) {
                    self.databases.push(self.active_database.clone());
                }
                self.reload_active_schema();
            }
        }

        // Production Guard & Read-Only Safety Checks
        if let Some(prof) = self
            .config
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
        {
            if prof.read_only && is_write_query(&sql) {
                self.status_banner = "Query blocked: connection is in Read-Only mode.".to_string();
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.last_error = Some("[BLOCKED] Connection is in Read-Only mode. Write operations (INSERT, UPDATE, DELETE, DROP, ALTER, TRUNCATE) are prohibited.".to_string());
                }
                return;
            }

            if let Some(reason) = is_potentially_destructive(&sql) {
                self.safety_warning_msg = format!("[WARNING] Destructive Query Detected:\n{reason}");
                self.pending_safe_sql = sql.clone();
                self.show_safety_modal = true;
                return;
            } else if prof.environment == Environment::Production && is_write_query(&sql) {
                self.safety_warning_msg = format!(
                    "[PRODUCTION GUARD]\nYou are connected to '{}' [PRODUCTION]. Confirm you want to execute this write query.",
                    prof.name
                );
                self.pending_safe_sql = sql.clone();
                self.show_safety_modal = true;
                return;
            }
        }

        self.execute_sql_forced(sql);
    }

    fn execute_sql_forced(&mut self, sql: String) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        tab.is_running = true;
        tab.last_error = None;
        tab.pending_edits.clear();
        tab.editing_cell = None;
        let tab_id = tab.id.clone();
        let db = self.active_database.clone();
        let schema = self.schemas.get(&db).cloned();
        let tx = self.tx.clone();

        self.status_banner = format!("Running query on `{db}`...");
        let join_handle = self.rt.spawn(async move {
            let res = session
                .execute_query(&db, &sql, schema.as_ref())
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::QueryFinished {
                tab_id,
                result: res,
            });
        });
        tab.task_abort_handle = Some(join_handle.abort_handle());
    }

    fn execute_current_tab_query(&mut self, prefix_override: Option<&str>) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let sql = if let Some(selected) = &tab.selected_sql {
            selected.clone()
        } else {
            tab.sql.clone()
        };
        self.execute_sql_in_current_tab(sql, prefix_override);
    }

    fn execute_current_statement(&mut self) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let (stmt, _) = extract_statement_at_cursor(&tab.sql, tab.cursor_index);
        if !stmt.trim().is_empty() {
            self.execute_sql_in_current_tab(stmt, None);
        }
    }

    fn trigger_inline_ai_completion(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };
        tab.inline_req_id += 1;
        let req_id = tab.inline_req_id;
        let tab_id = tab.id.clone();
        let sql_prefix = if tab.cursor_index > 0 {
            let chars: Vec<char> = tab.sql.chars().collect();
            let end_idx = tab.cursor_index.min(chars.len());
            chars[..end_idx].iter().collect::<String>()
        } else {
            tab.sql.clone()
        };
        let ddl = self
            .schemas
            .get(&self.active_database)
            .map(|s| s.to_pruned_ai_ddl_context(&sql_prefix))
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
                req_id,
                result: res,
            });
        });
    }

    pub fn refresh_server_admin(&mut self) {
        let Some(session) = self.active_session.clone() else {
            self.status_banner = "Not connected to any database session.".to_string();
            return;
        };
        self.is_server_admin_loading = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let status_res = session.server_status_summary().await.map_err(|e| format!("{e:#}"));
            let proc_res = session.list_processes().await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ServerAdminLoaded {
                status: status_res,
                processes: proc_res,
            });
        });
    }

    pub fn kill_server_query(&mut self, conn_id: u64) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.status_banner = format!("Killing query for connection #{conn_id}...");
        self.rt.spawn(async move {
            let res = session
                .kill_query(conn_id)
                .await
                .map(|_| conn_id)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::QueryKilled(res));
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
                    self.sidebar_tab = SidebarTab::Schema;

                    // Setup initial starter query if the active tab is empty
                    if let Some(tab) = self.tabs.first_mut() {
                        if tab.sql.trim().is_empty() {
                            if let Some(schema) = self.schemas.get(&self.active_database) {
                                if let Some(first_tbl) = schema.tables.first() {
                                    tab.sql = format!("SELECT * FROM `{}` LIMIT 100;", first_tbl.name);
                                } else {
                                    tab.sql = "SHOW TABLES;".to_string();
                                }
                            } else {
                                tab.sql = "SHOW TABLES;".to_string();
                            }
                        }
                    }

                    // Run default query automatically on first connect
                    if self
                        .tabs
                        .first()
                        .map(|t| !t.sql.trim().is_empty() && t.last_result.is_none())
                        .unwrap_or(false)
                    {
                        self.execute_current_tab_query(None);
                    }
                }
                AsyncEvent::ConnectionFailed(err) => {
                    self.status_banner = format!("Connection error: {err}");
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.last_error = Some(format!("Connection error: {err}"));
                    }
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
                    self.bottom_tab = BottomTab::ResultsGrid;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_running = false;
                        tab.task_abort_handle = None;
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
                                tab.last_result = None;
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
                AsyncEvent::InlineAiCompletion {
                    tab_id,
                    req_id,
                    result,
                } => {
                    self.is_inline_ai_loading = false;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        // Stale request check: discard if user continued typing or triggered a newer request
                        if tab.inline_req_id != req_id {
                            continue;
                        }
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
                                "**[AI Provider Error]** {err}\n\nTip: Check **AI Settings** in the top toolbar to configure Ollama, OpenAI, Anthropic, Gemini, or your local CLI (`agy`)."
                            ),
                            extracted_sql: None,
                        }),
                    }
                }
                AsyncEvent::ServerAdminLoaded { status, processes } => {
                    self.is_server_admin_loading = false;
                    match status {
                        Ok(metrics) => {
                            let now = std::time::Instant::now();
                            let current_questions = metrics
                                .get("Questions")
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            let current_threads = metrics
                                .get("Threads_connected")
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(1.0);

                            if let (Some(last_time), Some(last_q)) =
                                (self.last_poll_instant, self.last_questions_count)
                            {
                                let elapsed = now.duration_since(last_time).as_secs_f64();
                                if elapsed > 0.05 {
                                    let delta_q = current_questions.saturating_sub(last_q) as f64;
                                    let qps = delta_q / elapsed;
                                    let idx = self.qps_history.len() as f64;
                                    self.qps_history.push([idx, qps]);
                                    self.threads_history.push([idx, current_threads]);
                                    if self.qps_history.len() > 60 {
                                        self.qps_history.remove(0);
                                        self.threads_history.remove(0);
                                    }
                                }
                            } else {
                                self.qps_history.push([0.0, 0.0]);
                                self.threads_history.push([0.0, current_threads]);
                            }
                            self.last_poll_instant = Some(now);
                            self.last_questions_count = Some(current_questions);
                            self.server_status_metrics = metrics;
                        }
                        Err(e) => self.status_banner = format!("Server status error: {e}"),
                    }
                    match processes {
                        Ok(procs) => self.server_processes = Some(procs),
                        Err(e) => self.status_banner = format!("Process list error: {e}"),
                    }
                }
                AsyncEvent::QueryKilled(res) => {
                    match res {
                        Ok(id) => {
                            self.status_banner = format!("Successfully killed query for connection #{id}");
                            self.refresh_server_admin();
                        }
                        Err(e) => self.status_banner = format!("Failed to kill query: {e}"),
                    }
                }
                AsyncEvent::TestConnectionResult(res) => {
                    self.testing_connection = false;
                    self.test_connection_result = Some(res);
                }
            }
        }
    }
}

impl eframe::App for MysqlRustyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_async_events();

        // Global keyboard shortcuts (Ctrl+Enter, F5, Ctrl+Shift+Enter, Shift+F5)
        let is_modal_open = self.show_connection_modal || self.show_ai_settings_modal;
        if !is_modal_open {
            let mut global_run_stmt = false;
            let mut global_run_query = false;

            ui.ctx().input_mut(|i| {
                let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
                let is_shift = i.modifiers.shift;

                let has_enter_press = i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. }));

                if is_ctrl && is_shift && has_enter_press {
                    i.events.retain(|e| {
                        !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                    });
                    global_run_stmt = true;
                } else if is_ctrl && has_enter_press {
                    i.events.retain(|e| {
                        !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                    });
                    global_run_query = true;
                } else if is_shift && i.consume_key(Modifiers::NONE, Key::F5) {
                    global_run_stmt = true;
                } else if i.consume_key(Modifiers::NONE, Key::F5) {
                    global_run_query = true;
                }
            });

            if global_run_stmt {
                self.execute_current_statement();
            } else if global_run_query {
                self.execute_current_tab_query(None);
            }
        }

        let ai_triggered = ui.ctx().input_mut(|i| {
            let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
            let has_space_press = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { key: Key::Space, pressed: true, .. }));
            if is_ctrl && has_space_press {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Space, pressed: true, .. })
                });
                true
            } else {
                false
            }
        });
        if ai_triggered {
            self.trigger_inline_ai_completion();
        }

        // Debounced automatic inline AI completion
        if self.config.ai.enable_inline_autocomplete && !self.is_inline_ai_loading {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                if let Some(instant) = tab.last_edit_instant {
                    let debounce = std::time::Duration::from_millis(
                        self.config.ai.autocomplete_debounce_ms.max(50),
                    );
                    if instant.elapsed() >= debounce {
                        tab.last_edit_instant = None;
                        self.trigger_inline_ai_completion();
                    } else {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
                    }
                }
            }
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
        self.render_table_designer_modal(&ctx);
        self.render_safety_modal(&ctx);
        self.render_schema_diff_modal(&ctx);
        self.render_mock_data_modal(&ctx);
        self.render_save_snippet_modal(&ctx);
    }
}

impl MysqlRustyApp {
    fn render_top_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_toolbar")
            .exact_size(48.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    // Brand Logo with glowing accent
                    ui.label(
                        RichText::new("MySQL-Rusty")
                            .size(16.0)
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Connection selector & status indicator
                    let is_connected = self.active_session.is_some();
                    let active_conn_name = self
                        .config
                        .connections
                        .iter()
                        .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
                        .map(|c| c.name.as_str())
                        .unwrap_or("Select Connection...");

                    let status_dot = if is_connected {
                        RichText::new("●").color(Color32::from_rgb(16, 185, 129))
                    } else {
                        RichText::new("○").color(Color32::from_rgb(148, 163, 184))
                    };
                    ui.label(status_dot);

                    egui::ComboBox::from_id_salt("conn_selector")
                        .selected_text(RichText::new(active_conn_name).strong())
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            let profiles = self.config.connections.clone();
                            for prof in profiles {
                                let is_sel = Some(&prof.id) == self.active_profile_id.as_ref();
                                if ui.selectable_label(is_sel, &prof.name).clicked() {
                                    self.connect_to_profile(prof);
                                }
                            }
                        });

                    if is_connected {
                        if ui
                            .button(RichText::new("Disconnect").small().color(Color32::from_rgb(248, 113, 113)))
                            .on_hover_text("Disconnect from active database session")
                            .clicked()
                        {
                            self.disconnect();
                        }
                    } else if ui
                        .button(RichText::new("Servers Dashboard").small())
                        .on_hover_text("Open servers and connection profiles")
                        .clicked()
                    {
                        self.sidebar_tab = SidebarTab::Connections;
                    }

                    if ui.button("+ New").on_hover_text("Add new MySQL connection profile").clicked() {
                        self.editing_connection = ConnectionProfile::default();
                        self.show_connection_modal = true;
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Database Selector
                    ui.label(RichText::new("DB:").weak());
                    let current_db = if self.active_database.is_empty() {
                        "None"
                    } else {
                        &self.active_database
                    };
                    egui::ComboBox::from_id_salt("db_selector")
                        .selected_text(current_db)
                        .width(150.0)
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
                        .button("↻")
                        .on_hover_text("Refresh database schema metadata")
                        .clicked()
                    {
                        self.reload_active_schema();
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Query execution controls
                    let has_selection = self
                        .tabs
                        .get(self.active_tab_idx)
                        .and_then(|t| t.selected_sql.as_ref())
                        .is_some();
                    let run_label = if has_selection {
                        " ▶ Run Selection (Ctrl+Enter) "
                    } else {
                        " ▶ Run (Ctrl+Enter) "
                    };
                    let is_running = self
                        .tabs
                        .get(self.active_tab_idx)
                        .map(|t| t.is_running)
                        .unwrap_or(false);

                    let run_btn = egui::Button::new(
                        RichText::new(run_label)
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(16, 185, 129))
                    .corner_radius(egui::CornerRadius::same(6));

                    if ui.add(run_btn).clicked() {
                        self.execute_current_tab_query(None);
                    }

                    if is_running {
                        let stop_btn = egui::Button::new(
                            RichText::new(" ■ Stop ")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(Color32::from_rgb(225, 29, 72))
                        .corner_radius(egui::CornerRadius::same(6));

                        if ui
                            .add(stop_btn)
                            .on_hover_text("Abort running query execution")
                            .clicked()
                        {
                            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                                if let Some(handle) = tab.task_abort_handle.take() {
                                    handle.abort();
                                }
                                tab.is_running = false;
                                tab.last_error =
                                    Some("Query execution was cancelled by user.".to_string());
                            }
                            self.status_banner = "Query cancelled by user.".to_string();
                        }
                    }

                    if !has_selection {
                        let run_stmt_btn = egui::Button::new(
                            RichText::new(" ↳ Stmt ")
                                .color(Color32::from_rgb(209, 250, 229))
                                .size(12.0),
                        )
                        .fill(Color32::from_rgb(6, 78, 59))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(16, 185, 129)))
                        .corner_radius(egui::CornerRadius::same(6));

                        if ui
                            .add(run_stmt_btn)
                            .on_hover_text("Execute the SQL statement currently under cursor (Ctrl+Shift+Enter)")
                            .clicked()
                        {
                            self.execute_current_statement();
                        }
                    }

                    if ui
                        .button("Explain")
                        .on_hover_text("Inspect MySQL execution plan for current query")
                        .clicked()
                    {
                        self.execute_current_tab_query(Some("EXPLAIN"));
                    }

                    if ui
                        .button("Format SQL")
                        .on_hover_text("Format SQL clauses cleanly")
                        .clicked()
                    {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.sql = format_sql_pretty(&tab.sql);
                        }
                    }

                    if ui
                        .button("Save Query")
                        .on_hover_text("Save query to library or file (Ctrl+S)")
                        .clicked()
                    {
                        self.open_save_query_modal();
                    }

                    if ui
                        .button("Open SQL...")
                        .on_hover_text("Open .sql script from disk (Ctrl+O)")
                        .clicked()
                    {
                        self.open_sql_file_dialog();
                    }

                    let ai_btn = egui::Button::new(
                        RichText::new(" AI Auto ")
                            .color(Color32::from_rgb(233, 213, 255))
                            .size(12.0),
                    )
                    .fill(Color32::from_rgb(88, 28, 135))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(168, 85, 247)))
                    .corner_radius(egui::CornerRadius::same(6));

                    if ui
                        .add(ai_btn)
                        .on_hover_text("Trigger inline AI SQL completion at cursor (Ctrl+Space)")
                        .clicked()
                    {
                        self.trigger_inline_ai_completion();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let copilot_text = if self.show_ai_panel {
                            RichText::new("AI Copilot").color(Color32::WHITE).strong()
                        } else {
                            RichText::new("AI Copilot").weak()
                        };
                        let copilot_btn = egui::Button::new(copilot_text)
                            .fill(if self.show_ai_panel {
                                Color32::from_rgb(126, 34, 206)
                            } else {
                                Color32::from_rgb(24, 32, 47)
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if self.show_ai_panel {
                                    Color32::from_rgb(168, 85, 247)
                                } else {
                                    Color32::from_rgb(45, 55, 75)
                                },
                            ))
                            .corner_radius(egui::CornerRadius::same(6));

                        if ui.add(copilot_btn).clicked() {
                            self.show_ai_panel = !self.show_ai_panel;
                        }

                        if ui
                            .button(format!("AI: {}", self.config.ai.provider.label()))
                            .on_hover_text("Configure AI autocomplete & chat provider")
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
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::Schema,
                        RichText::new("Schema").size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::Connections,
                        RichText::new("Servers").size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::SavedQueries,
                        RichText::new("Saved Queries").size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::History,
                        RichText::new("History").size(12.0),
                    );
                });
                ui.separator();

                match self.sidebar_tab {
                    SidebarTab::Schema => self.render_schema_tree(ui),
                    SidebarTab::Connections => self.render_connections_list(ui),
                    SidebarTab::SavedQueries => self.render_saved_queries_panel(ui),
                    SidebarTab::History => self.render_query_history_panel(ui),
                }
            });
    }

    fn render_schema_tree(&mut self, ui: &mut Ui) {
        if self.active_session.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.label(RichText::new("No database connected").color(Color32::from_rgb(148, 163, 184)));
                ui.add_space(8.0);
                if ui.button("Choose Connection...").clicked() {
                    self.sidebar_tab = SidebarTab::Connections;
                }
            });
            return;
        }

        // Database selector and quick switcher
        ui.horizontal(|ui| {
            ui.label(RichText::new("DB:").strong());
            let current_db = if self.active_database.is_empty() {
                "None"
            } else {
                &self.active_database
            };
            egui::ComboBox::from_id_salt("sidebar_schema_db_selector")
                .selected_text(current_db)
                .width(130.0)
                .show_ui(ui, |ui| {
                    let dbs = self.databases.clone();
                    for db in dbs {
                        let is_sel = self.active_database == db;
                        if ui.selectable_label(is_sel, &db).clicked() {
                            self.active_database = db;
                            self.reload_active_schema();
                        }
                    }
                });
            if ui.small_button("Refresh").on_hover_text("Refresh active database schema").clicked() {
                self.reload_active_schema();
            }
        });

        if self.databases.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                for db in self.databases.clone() {
                    let is_sel = self.active_database == db;
                    let text = if is_sel {
                        RichText::new(format!("[{db}]")).strong().color(Color32::from_rgb(56, 189, 248))
                    } else {
                        RichText::new(db.clone()).weak()
                    };
                    if ui.selectable_label(is_sel, text).clicked() {
                        self.active_database = db;
                        self.reload_active_schema();
                    }
                }
            });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.schema_filter)
                    .hint_text("Search tables or columns..."),
            );
            if ui
                .small_button("+ New Table")
                .on_hover_text("Visually design and create a new table")
                .clicked()
            {
                self.table_designer_draft = Some(TableDraft::new_empty("new_table"));
            }
            if ui
                .small_button("Diff & Sync")
                .on_hover_text("Compare two schemas and generate DDL synchronization script")
                .clicked()
            {
                self.show_schema_diff_modal = true;
                self.diff_source_db = self.active_database.clone();
                self.diff_target_db = self
                    .databases
                    .iter()
                    .find(|d| **d != self.active_database)
                    .cloned()
                    .unwrap_or_default();
                if !self.diff_source_db.is_empty() && !self.diff_target_db.is_empty() {
                    if let (Some(src), Some(tgt)) = (
                        self.schemas.get(&self.diff_source_db),
                        self.schemas.get(&self.diff_target_db),
                    ) {
                        self.current_diff_report = Some(diff_schemas(src, tgt));
                    }
                }
            }
        });
        ui.add_space(4.0);

        let Some(schema) = self.schemas.get(&self.active_database).cloned() else {
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Loading schema for `{}`...", self.active_database)).weak());
                if ui.button("Load Schema").clicked() {
                    self.reload_active_schema();
                }
            });
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

                let header_label = format!("{} (~{})", table.name, table.approx_rows);
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
                            if ui
                                .small_button("Design Table")
                                .on_hover_text("Visual table designer to alter columns, indexes, and FKs")
                                .clicked()
                            {
                                self.table_designer_draft = Some(TableDraft::from_table(table));
                            }
                            if ui
                                .small_button("Import CSV")
                                .on_hover_text("Import CSV file as batch INSERT statements into this table")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("CSV", &["csv", "txt"])
                                    .pick_file()
                                {
                                    if let Ok(content) = std::fs::read_to_string(&path) {
                                        match parse_csv_to_batch_insert(&table.name, &content) {
                                            Ok(batch_sql) => {
                                                self.tabs.push(QueryTab::new(
                                                    format!("Import {}", table.name),
                                                    batch_sql,
                                                ));
                                                self.active_tab_idx = self.tabs.len() - 1;
                                                self.status_banner = format!(
                                                    "Generated batch INSERT statements from `{}`",
                                                    path.file_name().unwrap_or_default().to_string_lossy()
                                                );
                                            }
                                            Err(err) => {
                                                self.status_banner = format!("Failed to parse CSV: {err}");
                                            }
                                        }
                                    }
                                }
                            }
                            if ui
                                .small_button("Mock Data")
                                .on_hover_text("Generate realistic random seed data for this table")
                                .clicked()
                            {
                                self.mock_data_target_table = Some(table.name.clone());
                                self.show_mock_data_modal = true;
                            }
                            if ui.small_button("Insert Name").clicked() {
                                pending_sql_action = Some((table.name.clone(), false));
                            }
                        });

                        for col in &table.columns {
                            let icon = if col.is_primary_key { "[PK] " } else { "     " };
                            ui.horizontal(|ui| {
                                ui.label(format!("{icon}{}", col.name));
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
                                    "[FK] Foreign Keys ({})",
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
        if ui.button("+ Add MySQL / MariaDB Profile").clicked() {
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
                egui::Frame::new()
                    .fill(Color32::from_rgb(17, 24, 39))
                    .stroke(if is_active {
                        Stroke::new(1.0, Color32::from_rgb(16, 185, 129))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(31, 41, 55))
                    })
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&prof.name).strong().size(13.0));
                            let (er, eg, eb) = prof.environment.color_rgb();
                            ui.colored_label(Color32::from_rgb(er, eg, eb), format!("[{}]", prof.environment.label()));
                            if prof.read_only {
                                ui.colored_label(Color32::from_rgb(251, 146, 60), "[Read-Only]");
                            }
                            if is_active {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(6, 78, 59))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(5, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ONLINE")
                                                .size(9.5)
                                                .color(Color32::from_rgb(52, 211, 153))
                                                .strong(),
                                        );
                                    });
                            }
                        });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("{}@{}:{}", prof.user, prof.host, prof.port))
                                .small()
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        if prof.ssh.enabled {
                            ui.label(
                                RichText::new(format!("SSH: {}", prof.ssh.ssh_host))
                                    .size(10.5)
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let conn_label = if is_active { "Reconnect" } else { "Connect" };
                            let btn = egui::Button::new(RichText::new(conn_label).size(11.5).strong())
                                .fill(if is_active { Color32::from_rgb(6, 78, 59) } else { Color32::from_rgb(2, 132, 199) })
                                .corner_radius(egui::CornerRadius::same(4));
                            if ui.add(btn).clicked() {
                                to_connect = Some(prof.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                to_edit = Some(prof.clone());
                            }
                            if ui.small_button("Delete").clicked() {
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

    fn render_saved_queries_panel(&mut self, ui: &mut Ui) {
        let mut load_sql: Option<String> = None;
        let mut run_sql_now: Option<String> = None;
        let mut new_tab_entry: Option<(String, String)> = None;
        let mut delete_snippet_id: Option<String> = None;
        let mut export_snippet_sql: Option<(String, String)> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Saved Queries Library").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Save Active").on_hover_text("Bookmark active query tab to library").clicked() {
                    self.open_save_query_modal();
                }
                if ui.button("Open File...").on_hover_text("Open .sql file into a tab (Ctrl+O)").clicked() {
                    self.open_sql_file_dialog();
                }
            });
        });
        ui.add_space(2.0);

        ui.add(
            egui::TextEdit::singleline(&mut self.snippet_search_filter)
                .hint_text("Search queries by name, tag, database, or SQL...")
                .desired_width(ui.available_width()),
        );
        ui.add_space(4.0);

        let filter = self.snippet_search_filter.to_lowercase();
        let total_count = self.config.snippets.len();

        if self.config.snippets.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("No saved queries yet.").weak());
                ui.add_space(6.0);
                if ui.button("Save Current Query Tab").clicked() {
                    self.open_save_query_modal();
                }
            });
            return;
        }

        ui.label(
            RichText::new(format!("Showing queries ({} total)", total_count))
                .small()
                .weak(),
        );
        ui.add_space(2.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for snip in &self.config.snippets {
                if !filter.is_empty()
                    && !snip.title.to_lowercase().contains(&filter)
                    && !snip.sql.to_lowercase().contains(&filter)
                    && !snip.description.to_lowercase().contains(&filter)
                    && !snip.database.as_deref().unwrap_or("").to_lowercase().contains(&filter)
                {
                    continue;
                }

                egui::Frame::new()
                    .fill(Color32::from_rgb(17, 24, 39))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(31, 41, 55)))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&snip.title).strong().size(13.0));
                            if let Some(db) = &snip.database {
                                ui.colored_label(
                                    Color32::from_rgb(56, 189, 248),
                                    format!("[{db}]"),
                                );
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Delete").on_hover_text("Delete saved query").clicked() {
                                    delete_snippet_id = Some(snip.id.clone());
                                }
                            });
                        });

                        if !snip.description.is_empty() {
                            ui.label(RichText::new(&snip.description).small().color(Color32::from_rgb(148, 163, 184)));
                        }

                        let preview_lines: Vec<&str> = snip.sql.lines().take(3).collect();
                        let preview = preview_lines.join("\n");
                        ui.label(RichText::new(preview).monospace().small().weak());

                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button("Open Tab").on_hover_text("Open this query in a new editor tab").clicked() {
                                new_tab_entry = Some((snip.title.clone(), snip.sql.clone()));
                            }
                            if ui.small_button("Run Now").on_hover_text("Load and execute immediately").clicked() {
                                run_sql_now = Some(snip.sql.clone());
                            }
                            if ui.small_button("Paste").on_hover_text("Paste into active tab").clicked() {
                                load_sql = Some(snip.sql.clone());
                            }
                            if ui.small_button("Copy").on_hover_text("Copy SQL to clipboard").clicked() {
                                ui.ctx().copy_text(snip.sql.clone());
                                self.status_banner = "Copied SQL to clipboard.".to_string();
                            }
                            if ui.small_button("Export .sql").on_hover_text("Save query as .sql file on disk").clicked() {
                                export_snippet_sql = Some((snip.title.clone(), snip.sql.clone()));
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        });

        if let Some(id) = delete_snippet_id {
            self.config.snippets.retain(|s| s.id != id);
            let _ = self.config.save();
            self.status_banner = "Deleted saved query.".to_string();
        }

        if let Some((title, sql)) = new_tab_entry {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
        }

        if let Some(sql) = load_sql {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql;
                tab.is_dirty = true;
            }
        }

        if let Some(sql) = run_sql_now {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql.clone();
            }
            self.execute_sql_in_current_tab(sql, None);
        }

        if let Some((title, sql)) = export_snippet_sql {
            let filename = format!("{}.sql", title.replace(' ', "_").replace('/', "_"));
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&filename)
                .add_filter("SQL Files", &["sql"])
                .save_file()
            {
                let _ = std::fs::write(&path, sql);
                self.status_banner = format!("Exported query to {}", path.display());
            }
        }
    }

    fn render_query_history_panel(&mut self, ui: &mut Ui) {
        let mut load_sql: Option<String> = None;
        let mut new_tab_sql: Option<String> = None;
        let mut clear_all_history = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Execution History").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.config.history.is_empty() {
                    if ui.small_button("Clear History").clicked() {
                        clear_all_history = true;
                    }
                }
            });
        });
        ui.add_space(2.0);

        ui.add(
            egui::TextEdit::singleline(&mut self.history_search_filter)
                .hint_text("Search execution history by SQL or database...")
                .desired_width(ui.available_width()),
        );
        ui.add_space(4.0);

        let filter = self.history_search_filter.to_lowercase();

        if self.config.history.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("No executed queries recorded yet.").weak());
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in self.config.history.iter().rev() {
                if !filter.is_empty()
                    && !entry.sql.to_lowercase().contains(&filter)
                    && !entry.database.to_lowercase().contains(&filter)
                {
                    continue;
                }
                egui::Frame::new()
                    .fill(Color32::from_rgb(15, 23, 42))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&entry.timestamp).small().weak());
                            if !entry.database.is_empty() {
                                ui.colored_label(
                                    Color32::from_rgb(56, 189, 248),
                                    format!("[{}]", entry.database),
                                );
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{} rows • {} ms",
                                    entry.row_count, entry.duration_ms
                                ))
                                .small()
                                .color(Color32::from_rgb(52, 211, 153)),
                            );
                        });
                        ui.label(RichText::new(&entry.sql).monospace().small());
                        ui.horizontal(|ui| {
                            if ui.small_button("Reuse SQL").clicked() {
                                load_sql = Some(entry.sql.clone());
                            }
                            if ui.small_button("Open in Tab").clicked() {
                                new_tab_sql = Some(entry.sql.clone());
                            }
                            if ui.small_button("Save to Library").clicked() {
                                self.new_snippet_title = entry
                                    .sql
                                    .lines()
                                    .next()
                                    .unwrap_or("Historic Query")
                                    .trim()
                                    .chars()
                                    .take(40)
                                    .collect();
                                self.new_snippet_db = entry.database.clone();
                                self.new_snippet_desc = format!("Executed on {}", entry.timestamp);
                                self.show_save_snippet_modal = true;
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        });

        if clear_all_history {
            self.config.history.clear();
            let _ = self.config.save();
            self.status_banner = "Query history cleared.".to_string();
        }

        if let Some(sql) = load_sql {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql;
                tab.is_dirty = true;
            }
        }

        if let Some(sql) = new_tab_sql {
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Query {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
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
                        RichText::new("AI SQL Copilot")
                            .color(Color32::from_rgb(192, 132, 252))
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Configure").clicked() {
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
                    if ui.small_button("Explain Current Query").clicked() {
                        self.send_chat_prompt(Some(
                            "Explain how the current SQL query works and whether any indexes can speed it up.".to_string(),
                        ));
                    }
                    if ui.small_button("Suggest Indexes").clicked() {
                        self.send_chat_prompt(Some(
                            "Rewrite the current SQL query for maximum performance in MySQL 8.0 and provide CREATE INDEX statements if helpful.".to_string(),
                        ));
                    }
                    if ui.small_button("Top Revenue Report").clicked() {
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
                                        if ui.small_button("Insert into Editor").clicked() {
                                            sql_to_insert = Some((sql_block.clone(), false));
                                        }
                                        if ui.small_button("Insert & Run").clicked() {
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

    fn parse_hex_color(&self, hex: &str) -> Color32 {
        let clean = hex.trim_start_matches('#');
        if clean.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&clean[0..2], 16),
                u8::from_str_radix(&clean[2..4], 16),
                u8::from_str_radix(&clean[4..6], 16),
            ) {
                return Color32::from_rgb(r, g, b);
            }
        }
        Color32::from_rgb(56, 189, 248)
    }

    fn render_connections_dashboard(&mut self, ui: &mut egui::Ui) {
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_delete = None;

        egui::ScrollArea::vertical()
            .id_salt("connections_dashboard_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(15, 23, 42))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("WORKBENCH CONNECTIONS")
                                    .size(11.0)
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                        });

                    ui.add_space(10.0);
                    ui.heading(
                        RichText::new("MySQL & MariaDB Servers")
                            .size(24.0)
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Select a server connection to open the SQL workspace, or configure a new remote profile.")
                            .size(13.5)
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.add_space(14.0);

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(" + New Connection Profile ")
                                    .size(13.5)
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(14, 116, 144))
                            .corner_radius(egui::CornerRadius::same(6)),
                        )
                        .clicked()
                    {
                        self.editing_connection = ConnectionProfile::default();
                        self.show_connection_modal = true;
                    }
                });
                ui.add_space(20.0);

                let max_w = 880.0f32.min(ui.available_width() - 48.0);
                let left_pad = ((ui.available_width() - max_w) / 2.0).max(24.0);

                if self.config.connections.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(
                            RichText::new("No connection profiles available.")
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(10.0);
                        if ui.button("Restore Sandbox Demo Profile").clicked() {
                            self.config.connections.push(ConnectionProfile::demo_profile());
                            let _ = self.config.save();
                        }
                    });
                } else {
                    let profiles = self.config.connections.clone();
                    for prof in profiles {
                        let is_demo = prof.is_demo;
                        let card_accent = if is_demo {
                            Color32::from_rgb(16, 185, 129) // Emerald
                        } else {
                            self.parse_hex_color(&prof.color_hex)
                        };

                        ui.horizontal(|ui| {
                            ui.add_space(left_pad);
                            ui.vertical(|ui| {
                                ui.set_width(max_w);
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(17, 24, 39)) // Dark elevated surface
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(31, 41, 55)))
                                    .corner_radius(egui::CornerRadius::same(10))
                                    .inner_margin(16.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            // Left color indicator bar
                                            let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 48.0), egui::Sense::hover());
                                            ui.painter().rect_filled(rect, egui::CornerRadius::same(2), card_accent);
                                            ui.add_space(10.0);

                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.heading(
                                                        RichText::new(&prof.name)
                                                            .size(17.0)
                                                            .color(Color32::WHITE)
                                                            .strong(),
                                                    );
                                                    if is_demo {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(6, 78, 59))
                                                            .corner_radius(egui::CornerRadius::same(4))
                                                            .inner_margin(egui::Margin::symmetric(6, 2))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("DEMO SANDBOX")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(52, 211, 153))
                                                                        .strong(),
                                                                );
                                                             });
                                                    }
                                                    if prof.ssh.enabled {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(8, 51, 68))
                                                            .corner_radius(egui::CornerRadius::same(4))
                                                            .inner_margin(egui::Margin::symmetric(6, 2))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("SSH TUNNEL")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(56, 189, 248))
                                                                        .strong(),
                                                                );
                                                            });
                                                    }
                                                    let (er, eg, eb) = prof.environment.color_rgb();
                                                    egui::Frame::new()
                                                        .fill(Color32::from_rgb(er / 5, eg / 5, eb / 5))
                                                        .stroke(Stroke::new(1.0, Color32::from_rgb(er, eg, eb)))
                                                        .corner_radius(egui::CornerRadius::same(4))
                                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(prof.environment.label().to_uppercase())
                                                                    .size(10.5)
                                                                    .color(Color32::from_rgb(er, eg, eb))
                                                                    .strong(),
                                                            );
                                                        });
                                                    if prof.read_only {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(67, 20, 7))
                                                            .stroke(Stroke::new(1.0, Color32::from_rgb(251, 146, 60)))
                                                            .corner_radius(egui::CornerRadius::same(4))
                                                            .inner_margin(egui::Margin::symmetric(6, 2))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("READ-ONLY")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(251, 146, 60))
                                                                        .strong(),
                                                                );
                                                            });
                                                    }
                                                });
                                                ui.add_space(4.0);
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.label(
                                                        RichText::new(format!("User: {}", prof.user))
                                                            .size(12.5)
                                                            .color(Color32::from_rgb(226, 232, 240)),
                                                    );
                                                    ui.label(RichText::new("•").weak());
                                                    ui.label(
                                                        RichText::new(format!("Host: {}:{}", prof.host, prof.port))
                                                            .size(12.5)
                                                            .color(Color32::from_rgb(148, 163, 184)),
                                                    );
                                                    ui.label(RichText::new("•").weak());
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "DB: {}",
                                                            if prof.default_database.is_empty() { "None" } else { &prof.default_database }
                                                        ))
                                                        .size(12.5)
                                                        .color(Color32::from_rgb(148, 163, 184)),
                                                    );
                                                });
                                                if prof.ssh.enabled {
                                                    ui.add_space(2.0);
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "SSH Bastion: {}@{} (Port: {})",
                                                            prof.ssh.ssh_user, prof.ssh.ssh_host, prof.ssh.ssh_port
                                                        ))
                                                        .size(11.5)
                                                        .color(Color32::from_rgb(56, 189, 248)),
                                                    );
                                                }
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new(" Connect ")
                                                                .size(13.5)
                                                                .color(Color32::WHITE)
                                                                .strong(),
                                                        )
                                                        .fill(if is_demo { Color32::from_rgb(16, 185, 129) } else { Color32::from_rgb(2, 132, 199) })
                                                        .corner_radius(egui::CornerRadius::same(6)),
                                                    )
                                                    .clicked()
                                                {
                                                    to_connect = Some(prof.clone());
                                                }
                                                if ui.button("Edit").clicked() {
                                                    to_edit = Some(prof.clone());
                                                }
                                                if ui.button("Delete").clicked() {
                                                    to_delete = Some(prof.id.clone());
                                                }
                                            });
                                        });
                                    });
                            });
                        });
                        ui.add_space(10.0);
                    }
                }
                ui.add_space(30.0);
            });

        if let Some(prof) = to_connect {
            self.connect_to_profile(prof);
        }
        if let Some(prof) = to_edit {
            self.editing_connection = prof;
            self.show_connection_modal = true;
        }
        if let Some(id) = to_delete {
            self.config.connections.retain(|c| c.id != id);
            let _ = self.config.save();
        }
    }

    fn render_central_workspace(&mut self, ui: &mut egui::Ui) {
        if self.active_session.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                self.render_connections_dashboard(ui);
            });
            return;
        }

        let trigger_save = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S) || i.consume_key(Modifiers::CTRL, Key::S));
        let trigger_open = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O) || i.consume_key(Modifiers::CTRL, Key::O));
        if trigger_save {
            self.save_active_tab_to_file(false);
        }
        if trigger_open {
            self.open_sql_file_dialog();
        }

        egui::CentralPanel::default().show(ui, |ui| {
            // Query Tabs Bar
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                let mut close_idx = None;
                for idx in 0..self.tabs.len() {
                    let is_active = idx == self.active_tab_idx;
                    let title = self.tabs[idx].display_title();

                    let tab_bg = if is_active {
                        Color32::from_rgb(13, 18, 28)
                    } else {
                        Color32::from_rgb(9, 13, 20)
                    };
                    let tab_stroke = if is_active {
                        Stroke::new(1.0, Color32::from_rgb(56, 189, 248))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(26, 35, 51))
                    };

                    egui::Frame::new()
                        .fill(tab_bg)
                        .stroke(tab_stroke)
                        .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                        .inner_margin(egui::Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label_color = if is_active {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(148, 163, 184)
                                };
                                let label = RichText::new(&title).size(12.5).color(label_color);
                                if ui.selectable_label(is_active, label).clicked() {
                                    self.active_tab_idx = idx;
                                }
                                if self.tabs.len() > 1 {
                                    if ui.button(RichText::new("×").size(11.0).weak()).clicked() {
                                        close_idx = Some(idx);
                                    }
                                }
                            });
                        });
                    ui.add_space(2.0);
                }
                if let Some(idx) = close_idx {
                    self.tabs.remove(idx);
                    if self.active_tab_idx >= self.tabs.len() {
                        self.active_tab_idx = self.tabs.len().saturating_sub(1);
                    }
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("+ New Tab").size(12.0).weak())
                            .fill(Color32::from_rgb(15, 21, 33))
                            .corner_radius(egui::CornerRadius::same(5)),
                    )
                    .clicked()
                {
                    let n = self.tabs.len() + 1;
                    self.tabs
                        .push(QueryTab::new(format!("Query {n}"), "SELECT * FROM users LIMIT 50;"));
                    self.active_tab_idx = self.tabs.len() - 1;
                }

                ui.add_space(4.0);
                if ui
                    .small_button("Save (Ctrl+S)")
                    .on_hover_text("Save query to file or bookmark in library")
                    .clicked()
                {
                    self.save_active_tab_to_file(false);
                }
                if ui
                    .small_button("Open (Ctrl+O)")
                    .on_hover_text("Open .sql file from disk into an editor tab")
                    .clicked()
                {
                    self.open_sql_file_dialog();
                }
            });
            ui.separator();

            // SQL Editor Section
            let schema = self.schemas.get(&self.active_database).cloned();
            let enable_ghost = self.config.ai.enable_inline_autocomplete;
            let mut request_fix_with_ai: Option<String> = None;
            let mut editor_run_query = false;
            let mut editor_run_stmt = false;

            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                // Accept suggestions via keyboard: Tab or Alt+1..8
                let mut selected_suggestion: Option<CompletionItem> = None;
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                    if let Some(ghost) = tab.ghost_completion.take() {
                        tab.sql.push_str(&ghost);
                    } else if let Some(first) = tab.local_suggestions.first() {
                        selected_suggestion = Some(first.clone());
                    }
                } else {
                    for (idx, item) in tab.local_suggestions.iter().enumerate().take(8) {
                        let num_key = match idx {
                            0 => Key::Num1,
                            1 => Key::Num2,
                            2 => Key::Num3,
                            3 => Key::Num4,
                            4 => Key::Num5,
                            5 => Key::Num6,
                            6 => Key::Num7,
                            7 => Key::Num8,
                            _ => break,
                        };
                        if ui.input_mut(|i| i.consume_key(Modifiers::ALT, num_key)) {
                            selected_suggestion = Some(item.clone());
                            break;
                        }
                    }
                }

                if let Some(item) = selected_suggestion {
                    let prefix_len = tab.current_token_prefix.len();
                    if prefix_len <= tab.sql.len() {
                        let new_len = tab.sql.len() - prefix_len;
                        tab.sql.truncate(new_len);
                    }
                    tab.sql.push_str(&item.insert_text);
                    tab.local_suggestions.clear();
                    tab.ghost_completion = None;
                }

                let ghost_clone = tab.ghost_completion.clone();
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = highlight_sql(buf.as_str(), ghost_clone.as_deref(), 14.0);
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                let output = egui::ScrollArea::vertical()
                    .id_salt("sql_editor_scroll")
                    .max_height(195.0)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut tab.sql)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner;

                if let Some(cursor_range) = output.state.cursor.char_range() {
                    let sorted = cursor_range.as_sorted_char_range();
                    tab.cursor_index = cursor_range.primary.index.into();
                    let s_start_raw: usize = sorted.start.into();
                    let s_end_raw: usize = sorted.end.into();
                    if s_start_raw != s_end_raw {
                        let chars: Vec<char> = tab.sql.chars().collect();
                        let s_start = s_start_raw.min(chars.len());
                        let s_end = s_end_raw.min(chars.len());
                        let selected: String = chars[s_start..s_end].iter().collect();
                        tab.selected_sql = if selected.trim().is_empty() {
                            None
                        } else {
                            Some(selected)
                        };
                    } else {
                        tab.selected_sql = None;
                    }
                }

                let editor_resp = output.response;

                if editor_resp.has_focus() {
                    ui.input_mut(|i| {
                        let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
                        let is_shift = i.modifiers.shift;

                        let has_enter_press = i.events.iter().any(|e| {
                            matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                        });

                        if is_ctrl && is_shift && has_enter_press {
                            i.events.retain(|e| {
                                !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                            });
                            editor_run_stmt = true;
                        } else if is_ctrl && has_enter_press {
                            i.events.retain(|e| {
                                !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                            });
                            editor_run_query = true;
                        } else if is_shift && i.consume_key(Modifiers::NONE, Key::F5) {
                            editor_run_stmt = true;
                        } else if i.consume_key(Modifiers::NONE, Key::F5) {
                            editor_run_query = true;
                        }
                    });
                }

                if editor_resp.changed() {
                    tab.is_dirty = true;
                    tab.selected_sql = None;
                    tab.last_edit_instant = Some(std::time::Instant::now());
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
                            RichText::new("Suggestions:")
                                .small()
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                        if let Some(ghost) = tab.ghost_completion.clone() {
                            let preview = ghost.replace('\n', " ↵ ");
                            if ui
                                .small_button(
                                    RichText::new(format!("[Tab] {preview}"))
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
                        let has_ghost = tab.ghost_completion.is_some();
                        for (idx, item) in suggestions.into_iter().enumerate() {
                            let badge = if idx == 0 && !has_ghost {
                                "[Tab]".to_string()
                            } else if idx < 8 {
                                format!("[Alt+{}]", idx + 1)
                            } else {
                                String::new()
                            };
                            let label = if badge.is_empty() {
                                format!("[{}] {}", item.kind.badge(), item.label)
                            } else {
                                format!("{badge} [{}] {}", item.kind.badge(), item.label)
                            };
                            if ui
                                .small_button(label)
                                .on_hover_text(format!(
                                    "{}\nShortcut: {}",
                                    item.detail,
                                    if idx == 0 && !has_ghost { "Tab or Alt+1" } else if idx < 8 { "Alt+1..8" } else { "Click" }
                                ))
                                .clicked()
                            {
                                if prefix_len <= tab.sql.len() {
                                    let new_len = tab.sql.len() - prefix_len;
                                    tab.sql.truncate(new_len);
                                }
                                tab.sql.push_str(&item.insert_text);
                                tab.local_suggestions.clear();
                                tab.ghost_completion = None;
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
                                    format!("SQL Error: {err}"),
                                );
                                if ui.small_button("Fix Query with AI").clicked() {
                                    request_fix_with_ai = Some(err.clone());
                                }
                            });
                        });
                }
            }

            if editor_run_stmt {
                self.execute_current_statement();
            } else if editor_run_query {
                self.execute_current_tab_query(None);
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
                    RichText::new("Results Grid").size(12.5),
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ErRelationships,
                    RichText::new("ER Diagram").size(12.5),
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ExecutionPlanAndLog,
                    RichText::new("Query Stats & DDL").size(12.5),
                );
                if ui
                    .selectable_value(
                        &mut self.bottom_tab,
                        BottomTab::ServerAdmin,
                        RichText::new("Server Monitor").size(12.5),
                    )
                    .clicked()
                {
                    if self.server_processes.is_none() && !self.is_server_admin_loading {
                        self.refresh_server_admin();
                    }
                }
            });
            ui.separator();

            match self.bottom_tab {
                BottomTab::ResultsGrid => self.render_results_grid(ui),
                BottomTab::ErRelationships => self.render_er_relationships(ui),
                BottomTab::ExecutionPlanAndLog => self.render_execution_stats(ui),
                BottomTab::ServerAdmin => self.render_server_admin(ui),
            }
        });
    }

    fn render_results_grid(&mut self, ui: &mut Ui) {
        let (is_running, has_result) = match self.tabs.get(self.active_tab_idx) {
            Some(t) => (t.is_running, t.last_result.is_some()),
            None => return,
        };

        if is_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Executing SQL query...");
            });
            return;
        }

        if !has_result {
            ui.label(RichText::new("Run a query (Ctrl+Enter) to view results here.").weak());
            return;
        }

        let result = self.tabs[self.active_tab_idx].last_result.clone().unwrap();
        let mut trigger_commit_edits = false;
        let mut trigger_ai_prompt: Option<String> = None;
        let pending_edits;

        {
            let tab = &mut self.tabs[self.active_tab_idx];

            // Results Action Bar (Filter, Pagination, Export, Inline Edit Commit)
            ui.horizontal_wrapped(|ui| {
            ui.label("Filter rows:");
            ui.add(
                egui::TextEdit::singleline(&mut tab.row_filter)
                    .desired_width(170.0)
                    .hint_text("Search any cell..."),
            );

            ui.separator();
            ui.selectable_value(&mut tab.view_mode, ResultViewMode::Grid, "Table");
            ui.selectable_value(&mut tab.view_mode, ResultViewMode::Chart, "Chart");

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
                    RichText::new(format!("Editable Table (`{tbl}`, PK `{pk}`) — Double-click cell"))
                        .small()
                        .color(Color32::from_rgb(16, 185, 129)),
                );
            }

            if !tab.pending_edits.is_empty() {
                if ui
                    .button(
                        RichText::new(format!(
                            "Apply {} Pending Edit(s)",
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
                if ui
                    .small_button(
                        RichText::new("AI Insights")
                            .color(Color32::from_rgb(192, 132, 252))
                            .strong(),
                    )
                    .on_hover_text("Analyze these query results with AI")
                    .clicked()
                {
                    let preview_rows: Vec<String> = result
                        .rows
                        .iter()
                        .take(25)
                        .map(|r| {
                            r.iter()
                                .map(|c| c.display_text())
                                .collect::<Vec<_>>()
                                .join(" | ")
                        })
                        .collect();
                    trigger_ai_prompt = Some(format!(
                        "Please analyze these database query results:\nQuery: `{}`\nColumns: {}\nSample Rows (first {}):\n{}\n\nProvide key metrics, patterns, summary statistics, and notable takeaways concisely.",
                        result.sql,
                        result.columns.join(", "),
                        preview_rows.len(),
                        preview_rows.join("\n")
                    ));
                }
                if ui.small_button("Copy Markdown").clicked() {
                    let md = to_markdown_table(&result);
                    ui.ctx().copy_text(md);
                }
                if ui.small_button("Export CSV").clicked() {
                    if let Ok(csv_data) = to_csv_string(&result) {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.csv")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &csv_data);
                        } else {
                            ui.ctx().copy_text(csv_data);
                        }
                    }
                }
                if ui.small_button("Export JSON").clicked() {
                    if let Ok(json_data) = to_json_string(&result) {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.json")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &json_data);
                        } else {
                            ui.ctx().copy_text(json_data);
                        }
                    }
                }
                if ui.small_button("Copy SQL INSERTs").clicked() {
                    let tbl = result
                        .editable_table
                        .as_deref()
                        .unwrap_or("exported_rows");
                    let inserts = to_sql_inserts(&result, tbl);
                    ui.ctx().copy_text(inserts);
                }
            });
        });

        ui.add_space(4.0);

        if tab.view_mode == ResultViewMode::Chart {
            Self::draw_results_chart(ui, tab, &result);
            return;
        }

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

            pending_edits = tab.pending_edits.clone();
        }

        if let Some(prompt) = trigger_ai_prompt {
            self.show_ai_panel = true;
            self.send_chat_prompt(Some(prompt));
        }

        if trigger_commit_edits {
            if let (Some(session), Some(table_name)) =
                (self.active_session.clone(), result.editable_table.clone())
            {
                let tab_id = self.tabs[self.active_tab_idx].id.clone();
                let db = self.active_database.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = session
                        .apply_cell_edits(&db, &table_name, &pending_edits)
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

    fn draw_results_chart(ui: &mut Ui, tab: &mut QueryTab, result: &QueryResult) {
        if result.columns.is_empty() || result.rows.is_empty() {
            ui.label("No query result data available to chart.");
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Chart Type:").strong());
            ui.selectable_value(&mut tab.chart_type, ChartType::Bar, "Bar");
            ui.selectable_value(&mut tab.chart_type, ChartType::Line, "Line");

            ui.separator();
            ui.label(RichText::new("X Axis (Label):").strong());
            let current_x = result
                .columns
                .get(tab.chart_x_col)
                .cloned()
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("chart_x_axis_cb")
                .selected_text(current_x)
                .show_ui(ui, |ui| {
                    for (i, col) in result.columns.iter().enumerate() {
                        ui.selectable_value(&mut tab.chart_x_col, i, col);
                    }
                });

            ui.separator();
            ui.label(RichText::new("Y Axis (Metric):").strong());
            let current_y = result
                .columns
                .get(tab.chart_y_col)
                .cloned()
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("chart_y_axis_cb")
                .selected_text(current_y)
                .show_ui(ui, |ui| {
                    for (i, col) in result.columns.iter().enumerate() {
                        ui.selectable_value(&mut tab.chart_y_col, i, col);
                    }
                });
        });
        ui.separator();

        let y_col = tab.chart_y_col;
        let x_col = tab.chart_x_col;
        let chart_type = tab.chart_type;
        let col_name = result.columns.get(y_col).cloned().unwrap_or_default();

        Plot::new("result_visual_chart")
            .legend(egui_plot::Legend::default())
            .height(ui.available_height().max(250.0))
            .show(ui, |plot_ui| {
                if chart_type == ChartType::Bar {
                    let mut bars = Vec::new();
                    for (i, row) in result.rows.iter().enumerate() {
                        if let Some(cell) = row.get(y_col) {
                            let val = match cell {
                                crate::db::CellValue::Integer(v) => *v as f64,
                                crate::db::CellValue::Float(v) => *v,
                                crate::db::CellValue::Text(s) => s.parse::<f64>().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            let label = row
                                .get(x_col)
                                .map(|c| c.display_text())
                                .unwrap_or_default();
                            bars.push(Bar::new(i as f64, val).width(0.65).name(label));
                        }
                    }
                    plot_ui.bar_chart(
                        BarChart::new(&col_name, bars)
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                } else {
                    let mut pts = Vec::new();
                    for (i, row) in result.rows.iter().enumerate() {
                        if let Some(cell) = row.get(y_col) {
                            let val = match cell {
                                crate::db::CellValue::Integer(v) => *v as f64,
                                crate::db::CellValue::Float(v) => *v,
                                crate::db::CellValue::Text(s) => s.parse::<f64>().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            pts.push([i as f64, val]);
                        }
                    }
                    plot_ui.line(
                        Line::new(&col_name, PlotPoints::new(pts))
                            .color(Color32::from_rgb(45, 212, 191))
                            .width(2.5),
                    );
                }
            });
    }

    fn render_er_relationships(&mut self, ui: &mut Ui) {
        let Some(schema) = self.schemas.get(&self.active_database).cloned() else {
            ui.label(RichText::new("No active schema loaded.").weak());
            return;
        };
        self.er_canvas_state.show(ui, &schema);
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

            // Visual EXPLAIN Tree
            let is_explain = res.sql.trim().to_uppercase().starts_with("EXPLAIN")
                || res
                    .columns
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("select_type"));
            if is_explain && !res.rows.is_empty() {
                ui.separator();
                ui.heading(
                    RichText::new("Visual Execution Plan (EXPLAIN Tree)")
                        .color(Color32::from_rgb(56, 189, 248))
                        .size(15.0),
                );
                ui.add_space(4.0);

                let col_table_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("table"));
                let col_type_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("type"));
                let col_key_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("key"));
                let col_rows_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("rows"));
                let col_extra_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("Extra"));

                let mut has_full_scan = false;

                for (idx, row) in res.rows.iter().enumerate() {
                    let table_name = col_table_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| format!("Node #{idx}"));
                    let access_type = col_type_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "ALL".to_string());
                    let key_used = col_key_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "NULL".to_string());
                    let rows_est = col_rows_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "—".to_string());
                    let extra = col_extra_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();

                    let (type_color, type_badge) = match access_type.to_uppercase().as_str() {
                        "ALL" => {
                            has_full_scan = true;
                            (Color32::from_rgb(239, 68, 68), "FULL TABLE SCAN [WARN]")
                        }
                        "INDEX" => (Color32::from_rgb(249, 115, 22), "FULL INDEX SCAN"),
                        "RANGE" => (Color32::from_rgb(234, 179, 8), "INDEX RANGE SCAN"),
                        "REF" | "EQ_REF" | "CONST" => {
                            (Color32::from_rgb(34, 197, 94), "INDEX LOOKUP [OK]")
                        }
                        _ => (Color32::from_rgb(148, 163, 184), "SCAN"),
                    };

                    egui::Frame::new()
                        .fill(Color32::from_rgb(17, 24, 39))
                        .stroke(Stroke::new(1.0, type_color))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Step {}: `{}`", idx + 1, table_name))
                                        .strong()
                                        .size(13.0),
                                );
                                ui.colored_label(type_color, format!("[{type_badge}]"));
                                ui.label(
                                    RichText::new(format!("Est. Rows: {rows_est}")).small().weak(),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Index Used: `{key_used}`"))
                                        .small()
                                        .color(Color32::from_rgb(56, 189, 248)),
                                );
                                if !extra.is_empty() {
                                    ui.label(
                                        RichText::new(format!("Extra: {extra}")).small().weak(),
                                    );
                                }
                            });
                        });
                    ui.add_space(4.0);
                }

                if has_full_scan {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(69, 10, 10))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    "[Optimization Tip] Table performs a FULL TABLE SCAN (`ALL`). Creating an index on the join or WHERE filter columns can drastically reduce query latency.",
                                )
                                .color(Color32::from_rgb(254, 202, 202))
                                .small(),
                            );
                        });
                }
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

    fn render_server_admin(&mut self, ui: &mut Ui) {
        let Some(_session) = self.active_session.as_ref() else {
            ui.label(RichText::new("Connect to a MySQL server or Demo Sandbox to inspect server administration.").weak());
            return;
        };

        let mut kill_query_id = None;

        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("Server Administration & Process Monitor")
                    .color(Color32::from_rgb(56, 189, 248))
                    .strong(),
            );
            ui.add_space(16.0);
            if ui.button("Refresh").clicked() {
                self.refresh_server_admin();
            }
            if self.is_server_admin_loading {
                ui.spinner();
                ui.label("Refreshing status...");
            }
        });
        ui.separator();

        // Server Metrics Cards
        let uptime_str = self.server_status_metrics.get("Uptime");
        let uptime_disp = format_uptime_secs(uptime_str);
        let threads_disp = self.server_status_metrics.get("Threads_connected").cloned().unwrap_or_else(|| "—".to_string());
        let queries_disp = self.server_status_metrics.get("Questions").cloned().unwrap_or_else(|| "—".to_string());
        let slow_disp = self.server_status_metrics.get("Slow_queries").cloned().unwrap_or_else(|| "0".to_string());
        let buffer_disp = format_buffer_pool_bytes(self.server_status_metrics.get("Innodb_buffer_pool_bytes_data"));

        ui.horizontal_wrapped(|ui| {
            let cards = [
                ("Uptime", &uptime_disp, Color32::from_rgb(56, 189, 248)),
                ("Connections", &threads_disp, Color32::from_rgb(52, 211, 153)),
                ("Total Queries", &queries_disp, Color32::from_rgb(192, 132, 252)),
                ("Slow Queries", &slow_disp, Color32::from_rgb(251, 146, 60)),
                ("InnoDB Buffer", &buffer_disp, Color32::from_rgb(45, 212, 191)),
            ];

            for (title, value, color) in cards {
                egui::Frame::group(ui.style())
                    .fill(Color32::from_rgb(15, 23, 42))
                    .stroke(Stroke::new(1.0, color))
                    .show(ui, |ui| {
                        ui.set_width(140.0);
                        ui.label(RichText::new(title).small().weak());
                        ui.label(RichText::new(value).strong().color(color).size(15.0));
                    });
            }
        });

        if !self.qps_history.is_empty() || !self.threads_history.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Live QPS (Queries / Sec)")
                            .strong()
                            .small()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    Plot::new("qps_history_plot")
                        .height(85.0)
                        .width(ui.available_width() * 0.49)
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("QPS", PlotPoints::new(self.qps_history.clone()))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .width(2.0),
                            );
                        });
                });
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Active Connections (Threads)")
                            .strong()
                            .small()
                            .color(Color32::from_rgb(52, 211, 153)),
                    );
                    Plot::new("threads_history_plot")
                        .height(85.0)
                        .width(ui.available_width())
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("Threads", PlotPoints::new(self.threads_history.clone()))
                                    .color(Color32::from_rgb(52, 211, 153))
                                    .width(2.0),
                            );
                        });
                });
            });
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Live Process List (SHOW FULL PROCESSLIST):").strong());
        ui.separator();

        if let Some(result) = &self.server_processes {
            let procs = result.clone();
            let num_rows = procs.rows.len();

            let text_height = egui::TextStyle::Body
                .resolve(ui.style())
                .size
                .max(14.0)
                + 8.0;

            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(60.0).at_least(40.0)) // Id
                .column(Column::initial(90.0).at_least(60.0)) // User
                .column(Column::initial(130.0).at_least(80.0)) // Host
                .column(Column::initial(110.0).at_least(60.0)) // DB
                .column(Column::initial(80.0).at_least(60.0)) // Command
                .column(Column::initial(60.0).at_least(40.0)) // Time
                .column(Column::initial(90.0).at_least(50.0)) // State
                .column(Column::remainder().at_least(180.0)) // Info
                .column(Column::exact(80.0)) // Action
                .header(24.0, |mut header| {
                    header.col(|ui| { ui.strong("Id"); });
                    header.col(|ui| { ui.strong("User"); });
                    header.col(|ui| { ui.strong("Host"); });
                    header.col(|ui| { ui.strong("DB"); });
                    header.col(|ui| { ui.strong("Command"); });
                    header.col(|ui| { ui.strong("Time (s)"); });
                    header.col(|ui| { ui.strong("State"); });
                    header.col(|ui| { ui.strong("Info"); });
                    header.col(|ui| { ui.strong("Action"); });
                })
                .body(|body| {
                    body.rows(text_height, num_rows, |mut row| {
                        let row_idx = row.index();
                        if let Some(r) = procs.rows.get(row_idx) {
                            let id_val = r.get(0).map(|c| c.display_text()).unwrap_or_default();
                            let user_val = r.get(1).map(|c| c.display_text()).unwrap_or_default();
                            let host_val = r.get(2).map(|c| c.display_text()).unwrap_or_default();
                            let db_val = r.get(3).map(|c| c.display_text()).unwrap_or_default();
                            let cmd_val = r.get(4).map(|c| c.display_text()).unwrap_or_default();
                            let time_val = r.get(5).map(|c| c.display_text()).unwrap_or_default();
                            let state_val = r.get(6).map(|c| c.display_text()).unwrap_or_default();
                            let info_val = r.get(7).map(|c| c.display_text()).unwrap_or_default();

                            row.col(|ui| { ui.monospace(&id_val); });
                            row.col(|ui| { ui.label(&user_val); });
                            row.col(|ui| { ui.label(&host_val); });
                            row.col(|ui| { ui.label(&db_val); });
                            row.col(|ui| { ui.label(&cmd_val); });
                            row.col(|ui| { ui.monospace(&time_val); });
                            row.col(|ui| { ui.label(&state_val); });
                            row.col(|ui| {
                                ui.add(egui::Label::new(RichText::new(&info_val).monospace().small()).truncate());
                            });
                            row.col(|ui| {
                                if let Ok(conn_id) = id_val.parse::<u64>() {
                                    if ui.small_button(RichText::new("Kill").color(Color32::from_rgb(239, 68, 68))).clicked() {
                                        kill_query_id = Some(conn_id);
                                    }
                                }
                            });
                        }
                    });
                });
        } else {
            ui.label(RichText::new("No process list loaded. Click 'Refresh' to inspect active connections.").weak());
        }

        if let Some(id) = kill_query_id {
            self.kill_server_query(id);
        }
    }

    fn render_connection_modal(&mut self, ctx: &egui::Context) {
        if !self.show_connection_modal {
            return;
        }
        let mut open = self.show_connection_modal;
        let mut save_and_connect = false;
        let mut save_only = false;

        egui::Window::new("MySQL / MariaDB Connection Profile")
            .open(&mut open)
            .resizable(false)
            .default_width(460.0)
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

                        ui.label("Environment Badge:");
                        egui::ComboBox::from_id_salt("env_badge_modal_combo")
                            .selected_text(self.editing_connection.environment.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Development,
                                    "Development (Dev)",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Staging,
                                    "Staging",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Production,
                                    "Production (Strict Guard)",
                                );
                            });
                        ui.end_row();

                        ui.label("Security Mode:");
                        ui.checkbox(
                            &mut self.editing_connection.read_only,
                            "Read-Only (Prevent write queries)",
                        )
                        .on_hover_text("When checked, blocks all INSERT, UPDATE, DELETE, DROP, and ALTER queries.");
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

                        if self.editing_connection.ssl_mode != SslMode::Disabled {
                            ui.label("Trust Certificates:");
                            ui.checkbox(
                                &mut self.editing_connection.ssl_accept_invalid_certs,
                                "Allow Self-Signed / Unknown CA",
                            )
                            .on_hover_text("Accept self-signed or internal CA SSL certificates (prevents UnknownIssuer error).");
                            ui.end_row();
                        }
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
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_key_path);
                                if ui.small_button("Browse...").on_hover_text("Browse SSH private key file").clicked() {
                                    let default_dir = dirs::home_dir().map(|h| h.join(".ssh")).unwrap_or_else(|| std::path::PathBuf::from("/"));
                                    if let Some(path) = rfd::FileDialog::new().set_directory(default_dir).pick_file() {
                                        self.editing_connection.ssh.ssh_key_path = path.to_string_lossy().to_string();
                                    }
                                }
                            });
                            ui.end_row();
                        });
                }

                // Test Connection Result feedback
                if self.testing_connection {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Testing connection (handshake, SSH tunnel & auth)...")
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                    });
                } else if let Some(res) = &self.test_connection_result {
                    ui.add_space(8.0);
                    match res {
                        Ok(msg) => {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(6, 78, 59))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(16, 185, 129)))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("[OK]")
                                                .color(Color32::from_rgb(16, 185, 129))
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(msg)
                                                .color(Color32::from_rgb(209, 250, 229))
                                                .small(),
                                        );
                                    });
                                });
                        }
                        Err(err) => {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(69, 10, 10))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new("[FAIL] Connection Failed")
                                                .color(Color32::from_rgb(239, 68, 68))
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(err)
                                                .color(Color32::from_rgb(254, 202, 202))
                                                .small(),
                                        );
                                        if err.contains("Permission denied") || err.contains("SSH") {
                                            ui.add_space(2.0);
                                            ui.label(
                                                RichText::new("Tip: For AWS EC2 / Amazon Linux bastion, SSH User is usually 'ec2-user' (not 'ubuntu'). Verify key file path and permissions.")
                                                    .color(Color32::from_rgb(253, 224, 71))
                                                    .small(),
                                            );
                                        }
                                    });
                                });
                        }
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.testing_connection, egui::Button::new("Test Connection"))
                        .on_hover_text("Test network, SSH tunnel, credentials and MySQL ping")
                        .clicked()
                    {
                        self.testing_connection = true;
                        self.test_connection_result = None;
                        let mut test_prof = self.editing_connection.clone();
                        if let Some((h, p)) = test_prof
                            .ssh
                            .ssh_host
                            .trim()
                            .split_once(':')
                            .map(|(h, p)| (h.trim().to_string(), p.trim().to_string()))
                        {
                            test_prof.ssh.ssh_host = h;
                            if let Ok(port) = p.parse::<u16>() {
                                test_prof.ssh.ssh_port = port;
                            }
                        }
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            let res = match ActiveSession::connect(test_prof).await {
                                Ok(session) => {
                                    match session.execute_query("", "SELECT VERSION(), CURRENT_USER();", None).await {
                                        Ok(qr) => {
                                            let v = qr.rows.first().and_then(|r| r.first()).map(|val| val.display_text()).unwrap_or_else(|| "Unknown".to_string());
                                            let u = qr.rows.first().and_then(|r| r.get(1)).map(|val| val.display_text()).unwrap_or_else(|| "Unknown".to_string());
                                            Ok(format!("Connection successful!\n• MySQL Version: {v}\n• Authenticated User: {u}"))
                                        }
                                        Err(_) => Ok("Connection successful (handshake & auth succeeded)!".to_string()),
                                    }
                                }
                                Err(e) => Err(format!("{e:#}")),
                            };
                            let _ = tx.send(AsyncEvent::TestConnectionResult(res));
                        });
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(
                                RichText::new("Save & Connect")
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            )
                            .clicked()
                        {
                            save_and_connect = true;
                        }
                        if ui.button("Save").clicked() {
                            save_only = true;
                        }
                    });
                });
            });

        self.show_connection_modal = open;
        if !open {
            self.test_connection_result = None;
            self.testing_connection = false;
        }

        if save_and_connect || save_only {
            let mut prof = self.editing_connection.clone();
            if let Some((h, p)) = prof
                .ssh
                .ssh_host
                .trim()
                .split_once(':')
                .map(|(h, p)| (h.trim().to_string(), p.trim().to_string()))
            {
                prof.ssh.ssh_host = h;
                if let Ok(port) = p.parse::<u16>() {
                    prof.ssh.ssh_port = port;
                }
            }

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
            self.test_connection_result = None;

            if save_and_connect {
                self.connect_to_profile(prof);
            }
        }
    }

    fn render_ai_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_ai_settings_modal {
            return;
        }
        let mut open = self.show_ai_settings_modal;
        let mut should_save = false;

        egui::Window::new("AI Autocomplete & Provider Settings")
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
                        ui.label("Model Name (e.g. `qwen2.5-coder:1.5b` [ultra-fast], `0.5b`, `7b`):");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_model);
                        ui.small("Tip: `qwen2.5-coder:1.5b` responds in ~100-200ms with 0 cloud cost.");
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
                ui.label(RichText::new("Performance & Inline Latency").strong());
                ui.horizontal(|ui| {
                    ui.label("Debounce Delay:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ai.autocomplete_debounce_ms, 50..=600)
                            .suffix(" ms"),
                    );
                });
                ui.small("150–200 ms recommended for fast, natural response while typing.");

                ui.horizontal(|ui| {
                    ui.label("Max Completion Tokens:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ai.autocomplete_max_tokens, 15..=100)
                            .suffix(" tokens"),
                    );
                });
                ui.small("Capping at 25–40 tokens cuts LLM inference time by ~75%.");

                ui.separator();
                if ui.button("Save Configuration").clicked() {
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

    fn render_table_designer_modal(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.table_designer_draft.take() else {
            return;
        };
        let mut keep_open = true;
        let mut action = TableDesignerAction::None;

        egui::Window::new("Visual Table Designer")
            .collapsible(false)
            .resizable(true)
            .default_size([740.0, 580.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                action = draft.render_editor(ui);
            });

        if !keep_open || action == TableDesignerAction::Close {
            self.table_designer_draft = None;
        } else {
            match action {
                TableDesignerAction::ApplyDdl(ddl) => {
                    self.table_designer_draft = None;
                    self.execute_sql_in_current_tab(ddl, None);
                    self.reload_active_schema();
                }
                TableDesignerAction::OpenInQueryTab(ddl) => {
                    self.table_designer_draft = None;
                    let n = self.tabs.len() + 1;
                    self.tabs.push(QueryTab::new(format!("DDL {n}"), ddl));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
                TableDesignerAction::None => {
                    self.table_designer_draft = Some(draft);
                }
                TableDesignerAction::Close => {
                    self.table_designer_draft = None;
                }
            }
        }
    }

    fn render_safety_modal(&mut self, ctx: &egui::Context) {
        if !self.show_safety_modal {
            return;
        }
        let mut open = self.show_safety_modal;
        let mut execute_confirmed = false;

        egui::Window::new("Safety Confirmation & Production Guard")
            .open(&mut open)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(69, 10, 10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&self.safety_warning_msg)
                                .color(Color32::from_rgb(254, 202, 202))
                                .strong(),
                        );
                    });

                ui.add_space(8.0);
                ui.label(RichText::new("Target Statement:").strong());
                egui::ScrollArea::vertical()
                    .max_height(110.0)
                    .show(ui, |ui| {
                        ui.code(&self.pending_safe_sql);
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Proceed & Execute Statement")
                                    .color(Color32::from_rgb(239, 68, 68))
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(127, 29, 29)),
                        )
                        .clicked()
                    {
                        execute_confirmed = true;
                    }

                    if ui.button("Cancel (Recommended)").clicked() {
                        self.show_safety_modal = false;
                        self.pending_safe_sql.clear();
                    }
                });
            });

        if execute_confirmed {
            self.show_safety_modal = false;
            let sql = std::mem::take(&mut self.pending_safe_sql);
            self.execute_sql_forced(sql);
        } else if !open {
            self.show_safety_modal = false;
            self.pending_safe_sql.clear();
        }
    }

    fn render_schema_diff_modal(&mut self, ctx: &egui::Context) {
        if !self.show_schema_diff_modal {
            return;
        }
        let mut open = self.show_schema_diff_modal;
        let mut open_in_tab_sql = None;

        egui::Window::new("Schema Diff & DDL Synchronization")
            .open(&mut open)
            .default_width(650.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Source DB:").strong());
                    egui::ComboBox::from_id_salt("diff_src_cb")
                        .selected_text(if self.diff_source_db.is_empty() {
                            "Select Source..."
                        } else {
                            &self.diff_source_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.diff_source_db, db.clone(), db);
                            }
                        });

                    ui.label(RichText::new("Target DB:").strong());
                    egui::ComboBox::from_id_salt("diff_tgt_cb")
                        .selected_text(if self.diff_target_db.is_empty() {
                            "Select Target..."
                        } else {
                            &self.diff_target_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.diff_target_db, db.clone(), db);
                            }
                        });

                    if ui.button("Compare").clicked() {
                        if let (Some(src), Some(tgt)) = (
                            self.schemas.get(&self.diff_source_db),
                            self.schemas.get(&self.diff_target_db),
                        ) {
                            self.current_diff_report = Some(diff_schemas(src, tgt));
                        } else {
                            self.status_banner =
                                "Please ensure both database schemas are loaded before comparing."
                                    .to_string();
                        }
                    }
                });
                ui.separator();

                if let Some(report) = &self.current_diff_report {
                    ui.label(
                        RichText::new(format!(
                            "Diff Summary ({} changes detected):",
                            report.summary.len()
                        ))
                        .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(130.0)
                        .show(ui, |ui| {
                            for item in &report.summary {
                                let color = if item.starts_with('+') {
                                    Color32::from_rgb(52, 211, 153)
                                } else if item.starts_with('~') {
                                    Color32::from_rgb(250, 204, 21)
                                } else if item.starts_with('-') {
                                    Color32::from_rgb(251, 146, 60)
                                } else {
                                    Color32::from_rgb(148, 163, 184)
                                };
                                ui.colored_label(color, item);
                            }
                        });

                    ui.separator();
                    ui.label(RichText::new("Generated DDL Migration SQL:").strong());
                    let mut sql_view = report.migration_sql.as_str();
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut sql_view)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY),
                            );
                        });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy Migration Script").clicked() {
                            ui.ctx().copy_text(report.migration_sql.clone());
                            self.status_banner = "Copied migration DDL to clipboard.".to_string();
                        }
                        if ui.button("▶ Open in Query Tab").clicked() {
                            open_in_tab_sql = Some(report.migration_sql.clone());
                        }
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(RichText::new("Select Source and Target databases above, then click 'Compare' to generate a synchronization DDL script.").weak());
                    });
                }
            });

        if let Some(sql) = open_in_tab_sql {
            self.show_schema_diff_modal = false;
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Migration Sync {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        } else if !open {
            self.show_schema_diff_modal = false;
        }
    }

    fn render_mock_data_modal(&mut self, ctx: &egui::Context) {
        if !self.show_mock_data_modal {
            return;
        }
        let mut open = self.show_mock_data_modal;
        let mut generate_to_tab = None;
        let mut insert_directly = None;

        let table_opt = self.mock_data_target_table.clone();
        let schema_opt = self.schemas.get(&self.active_database).cloned();

        egui::Window::new("Intelligent Mock Data Generator")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                if let (Some(tbl_name), Some(schema)) = (table_opt.as_ref(), schema_opt.as_ref()) {
                    if let Some(table) = schema.tables.iter().find(|t| t.name == *tbl_name) {
                        ui.label(
                            RichText::new(format!(
                                "Target Table: `{tbl_name}` ({} columns)",
                                table.columns.len()
                            ))
                            .strong(),
                        );
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label("Rows to generate:");
                            ui.add(
                                egui::DragValue::new(&mut self.mock_data_row_count).range(1..=1000),
                            );
                            if ui.small_button("10").clicked() {
                                self.mock_data_row_count = 10;
                            }
                            if ui.small_button("25").clicked() {
                                self.mock_data_row_count = 25;
                            }
                            if ui.small_button("50").clicked() {
                                self.mock_data_row_count = 50;
                            }
                            if ui.small_button("100").clicked() {
                                self.mock_data_row_count = 100;
                            }
                        });

                        ui.separator();
                        let preview_sql =
                            generate_mock_data_sql(table, self.mock_data_row_count.min(8));
                        ui.label(RichText::new("Data Preview (Sample rows):").strong());
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.code(preview_sql);
                            });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Open in Query Tab").clicked() {
                                let full_sql =
                                    generate_mock_data_sql(table, self.mock_data_row_count);
                                generate_to_tab = Some((tbl_name.clone(), full_sql));
                            }
                            if ui.button("▶ Insert Directly into DB").clicked() {
                                let full_sql =
                                    generate_mock_data_sql(table, self.mock_data_row_count);
                                insert_directly = Some(full_sql);
                            }
                            if ui.button("Cancel").clicked() {
                                self.show_mock_data_modal = false;
                            }
                        });
                    } else {
                        ui.label("Table not found in active database schema.");
                    }
                } else {
                    ui.label("No active database or table selected.");
                }
            });

        if let Some((tbl, sql)) = generate_to_tab {
            self.show_mock_data_modal = false;
            self.tabs.push(QueryTab::new(format!("Seed {tbl}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        } else if let Some(sql) = insert_directly {
            self.show_mock_data_modal = false;
            self.execute_sql_forced(sql);
        } else if !open {
            self.show_mock_data_modal = false;
        }
    }

    fn render_save_snippet_modal(&mut self, ctx: &egui::Context) {
        if !self.show_save_snippet_modal {
            return;
        }
        let mut open = self.show_save_snippet_modal;
        let mut save_to_library = false;
        let mut save_as_file = false;

        egui::Window::new("Save Query")
            .open(&mut open)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Query Name / Title:").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_snippet_title)
                        .hint_text("e.g. Monthly Revenue by Category")
                        .desired_width(ui.available_width()),
                );
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Database Tag:");
                    let current_db = if self.new_snippet_db.is_empty() {
                        "Any / None"
                    } else {
                        &self.new_snippet_db
                    };
                    egui::ComboBox::from_id_salt("save_query_db_combo")
                        .selected_text(current_db)
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.new_snippet_db.is_empty(), "Any / None").clicked() {
                                self.new_snippet_db.clear();
                            }
                            for db in &self.databases {
                                let is_sel = &self.new_snippet_db == db;
                                if ui.selectable_label(is_sel, db).clicked() {
                                    self.new_snippet_db = db.clone();
                                }
                            }
                        });
                });
                ui.add_space(4.0);

                ui.label("Description / Notes (Optional):");
                ui.add(
                    egui::TextEdit::multiline(&mut self.new_snippet_desc)
                        .desired_rows(2)
                        .hint_text("Add context, filters, or notes for this query...")
                        .desired_width(ui.available_width()),
                );

                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    ui.add_space(6.0);
                    ui.label(RichText::new("SQL Preview:").strong().small());
                    egui::ScrollArea::vertical()
                        .max_height(100.0)
                        .show(ui, |ui| {
                            ui.code(&tab.sql);
                        });
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save_btn = egui::Button::new(
                        RichText::new("Save to Library")
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(14, 116, 144));
                    if ui.add(save_btn).clicked() {
                        save_to_library = true;
                    }

                    if ui.button("Save as .sql File...").on_hover_text("Save query as a standalone .sql file on disk").clicked() {
                        save_as_file = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.show_save_snippet_modal = false;
                    }
                });
            });

        if save_to_library {
            self.show_save_snippet_modal = false;
            if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                let title = if self.new_snippet_title.trim().is_empty() {
                    "Saved Query".to_string()
                } else {
                    self.new_snippet_title.trim().to_string()
                };
                let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                self.config.snippets.push(SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title,
                    sql: tab.sql.clone(),
                    database: if self.new_snippet_db.trim().is_empty() { None } else { Some(self.new_snippet_db.trim().to_string()) },
                    description: self.new_snippet_desc.trim().to_string(),
                    created_at: now_str,
                });
                let _ = self.config.save();
                self.status_banner = "Saved query to app library successfully.".to_string();
            }
        } else if save_as_file {
            self.show_save_snippet_modal = false;
            self.save_active_tab_to_file(true);
        } else if !open {
            self.show_save_snippet_modal = false;
        }
    }
}

fn configure_modern_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::from_rgb(11, 15, 23); // Obsidian 950
    visuals.panel_fill = Color32::from_rgb(13, 18, 28);  // Deep slate surface
    visuals.extreme_bg_color = Color32::from_rgb(8, 11, 17); // Editor background
    visuals.faint_bg_color = Color32::from_rgb(20, 26, 40); // Faint background
    visuals.selection.bg_fill = Color32::from_rgb(14, 116, 144); // Cyan selection
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(56, 189, 248));

    // Corner radii
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);

    // Widget styling (Buttons, inputs, dropdowns)
    let subtle_border = Stroke::new(1.0, Color32::from_rgb(33, 43, 63));
    let hover_border = Stroke::new(1.0, Color32::from_rgb(56, 189, 248));
    let active_border = Stroke::new(1.0, Color32::from_rgb(14, 165, 233));

    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(15, 21, 32);
    visuals.widgets.noninteractive.bg_stroke = subtle_border;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(148, 163, 184));

    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(20, 27, 41);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(16, 22, 34);
    visuals.widgets.inactive.bg_stroke = subtle_border;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(226, 232, 240));

    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(30, 41, 62);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(25, 34, 52);
    visuals.widgets.hovered.bg_stroke = hover_border;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.bg_fill = Color32::from_rgb(37, 51, 77);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(32, 44, 66);
    visuals.widgets.active.bg_stroke = active_border;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.open.bg_fill = Color32::from_rgb(23, 31, 47);
    visuals.widgets.open.bg_stroke = hover_border;

    ctx.set_visuals(visuals);

    // Spacing and ergonomic button sizes
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size = egui::vec2(28.0, 24.0);
        style.spacing.combo_width = 180.0;
    });

    let mut fonts = egui::FontDefinitions::default();
    let candidate_fonts = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];
    for path in candidate_fonts {
        if let Ok(font_bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_fallback".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("system_fallback".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("system_fallback".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn format_uptime_secs(uptime_str: Option<&String>) -> String {
    let Some(s) = uptime_str else {
        return "—".to_string();
    };
    let Ok(secs) = s.parse::<u64>() else {
        return s.clone();
    };
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let rem_secs = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m {rem_secs}s")
    } else {
        format!("{mins}m {rem_secs}s")
    }
}

fn format_buffer_pool_bytes(bytes_str: Option<&String>) -> String {
    let Some(s) = bytes_str else {
        return "—".to_string();
    };
    let Ok(bytes) = s.parse::<u64>() else {
        return s.clone();
    };
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

