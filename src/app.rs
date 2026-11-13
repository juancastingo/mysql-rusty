use crate::ai::{
    ChatMessage, CompletionItem, compute_heuristic_ghost_completion, compute_local_completions,
    request_chat_assistant, request_inline_completion,
};
use crate::config::{
    AiProvider, AppConfig, ConnectionProfile, Environment, QueryHistoryEntry, SavedScratchpad,
    SavedSnippet, SslMode, TrashedConnection,
};
use crate::data_diff::{
    compute_data_diff, generate_data_diff_sync_sql, DataDiffReport, RowDiffStatus,
};
use crate::db::{ActiveSession, DatabaseSchema, PendingCellEdit, QueryResult};
use crate::er_diagram::ErCanvasState;
use crate::export::{
    format_column_as_js_array, format_column_as_sql_in, format_rows_as_json,
    format_rows_as_sql_insert, format_rows_as_sql_update, format_table_create_ddl,
    generate_csv_import_sql, generate_mock_data_sql, generate_table_dump_sql, inspect_csv_content,
    parse_csv_to_batch_insert, save_string_to_path, to_csv_string, to_json_string,
    to_markdown_table, to_sql_inserts,
};
use crate::schema_diff::{diff_schemas, SchemaDiffReport};
use crate::syntax::{
    apply_limit_to_sql, detect_current_limit, expand_sql_macro_with_cursor, extract_sql_parameters,
    extract_statement_at_cursor, format_sql_pretty, highlight_sql, is_potentially_destructive,
    is_write_query, replace_at_cursor, split_sql_at_cursor, split_sql_statements,
    substitute_sql_parameters,
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
    MultiQueryFinished {
        tab_id: String,
        results: Vec<Result<QueryResult, String>>,
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
    ServerVariablesLoaded(Result<Vec<(String, String)>, String>),
    CopilotDeviceCodeReady(Result<crate::ai::CopilotDeviceCodeResponse, String>),
    CopilotPollFinished(Result<String, String>),
    FkLookupLoaded {
        key: String,
        result: Result<QueryResult, String>,
    },
    DataDiffLoaded {
        left: Result<QueryResult, String>,
        right: Result<QueryResult, String>,
        key_col_idx: usize,
    },
    SqlDumpLoaded {
        sql: Result<String, String>,
        suggested_name: String,
    },
    SqlDumpImportFinished(Result<usize, String>),
    RowDeletesApplied {
        tab_id: String,
        deleted_count: Result<u64, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultViewMode {
    Grid,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiResultLayout {
    #[default]
    Tabs,
    Stacked,
    SideBySide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Bar,
    Line,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualFilterOp {
    Contains,
    Equals,
    NotEquals,
    StartsWith,
    GreaterThan,
    LessThan,
    IsNull,
    IsNotNull,
}

impl VisualFilterOp {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Equals => "= (exact)",
            Self::NotEquals => "!= (not equal)",
            Self::StartsWith => "starts with",
            Self::GreaterThan => "> (greater)",
            Self::LessThan => "< (less)",
            Self::IsNull => "IS NULL / empty",
            Self::IsNotNull => "IS NOT NULL",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CellInspectorState {
    pub table_name: Option<String>,
    pub column_name: String,
    pub row_index: usize,
    pub raw_value: String,
    pub formatted_json: Option<String>,
}

pub struct CsvImportWizard {
    pub file_path: Option<std::path::PathBuf>,
    pub file_name: String,
    pub headers: Vec<String>,
    pub sample_rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub target_table: String,
    pub new_table_name: String,
    pub create_new_table: bool,
    pub column_mappings: Vec<(String, String)>,
    pub is_importing: bool,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum ServerAdminSubtab {
    OverviewAndMetrics,
    ServerVariables,
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
    pub selected_suggestion_idx: usize,
    pub current_token_prefix: String,
    pub inline_req_id: u64,
    pub is_running: bool,
    pub task_abort_handle: Option<tokio::task::AbortHandle>,
    pub last_result: Option<QueryResult>,
    pub multi_results: Vec<QueryResult>,
    pub active_result_idx: usize,
    pub multi_result_layout: MultiResultLayout,
    pub multi_result_heights: HashMap<usize, f32>,
    pub multi_result_collapsed: HashMap<usize, bool>,
    pub multi_result_columns: Option<usize>,
    pub last_error: Option<String>,
    pub query_params: HashMap<String, String>,
    pub row_filter: String,
    pub visual_filter_col: Option<usize>,
    pub visual_filter_op: VisualFilterOp,
    pub visual_filter_val: String,
    pub show_visual_filter: bool,
    pub current_page: usize,
    pub page_size: usize,
    pub sort_column: Option<(usize, bool)>, // (col_idx, ascending)
    pub editing_cell: Option<(usize, usize, String)>, // (row_idx, col_idx, buffer)
    pub pending_edits: Vec<PendingCellEdit>,
    pub selected_rows: std::collections::HashSet<usize>,
    pub pending_row_deletes: Vec<(usize, String, String)>, // (row_idx, pk_col, pk_val)
    pub show_row_inspector: bool,
    pub row_inspector_row: Option<usize>,
    pub row_inspector_buffers: Vec<String>,
    pub baseline_result: Option<QueryResult>,
    pub show_baseline_compare: bool,
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
            selected_suggestion_idx: 0,
            current_token_prefix: String::new(),
            inline_req_id: 0,
            is_running: false,
            task_abort_handle: None,
            last_result: None,
            multi_results: Vec::new(),
            active_result_idx: 0,
            multi_result_layout: MultiResultLayout::Tabs,
            multi_result_heights: HashMap::new(),
            multi_result_collapsed: HashMap::new(),
            multi_result_columns: None,
            last_error: None,
            query_params: HashMap::new(),
            row_filter: String::new(),
            visual_filter_col: None,
            visual_filter_op: VisualFilterOp::Contains,
            visual_filter_val: String::new(),
            show_visual_filter: false,
            current_page: 0,
            page_size: 50,
            sort_column: None,
            editing_cell: None,
            pending_edits: Vec::new(),
            selected_rows: std::collections::HashSet::new(),
            pending_row_deletes: Vec::new(),
            show_row_inspector: false,
            row_inspector_row: None,
            row_inspector_buffers: Vec::new(),
            baseline_result: None,
            show_baseline_compare: false,
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
    renaming_tab_idx: Option<usize>,
    rename_tab_buffer: String,
    renaming_snippet_id: Option<String>,
    rename_snippet_buffer: String,

    // Live Server Monitoring History
    qps_history: Vec<[f64; 2]>,
    threads_history: Vec<[f64; 2]>,
    last_poll_instant: Option<std::time::Instant>,
    last_questions_count: Option<u64>,

    // Transaction Management
    in_transaction: bool,
    uncommitted_mutations: usize,

    // Cell Inspector
    cell_inspector: Option<CellInspectorState>,

    // CSV Import Wizard
    csv_wizard: Option<CsvImportWizard>,

    // Server Variables
    server_variables: Vec<(String, String)>,
    server_variables_filter: String,
    is_loading_variables: bool,
    server_admin_subtab: ServerAdminSubtab,

    // Command Palette (Ctrl+K / Ctrl+P)
    show_command_palette: bool,
    command_palette_query: String,
    command_palette_selected: usize,
    pending_delete_connection: Option<(String, String)>,
    show_connection_trash_modal: bool,

    // Copilot Authentication
    pub copilot_device_code_data: Option<crate::ai::CopilotDeviceCodeResponse>,
    pub copilot_auth_status: String,
    pub is_copilot_authenticating: bool,

    // Scratchpad auto-save
    last_scratchpad_save: std::time::Instant,

    // Multi-connection workspaces (Workbench-style top tabs)
    open_workspaces: Vec<OpenConnectionWorkspace>,
    active_workspace_idx: Option<usize>,

    // FK Lookup Dropdown cache (key: "ref_table.ref_col")
    fk_lookup_cache: HashMap<String, Vec<(String, String)>>,

    // Data Diff Modal (Table/Environment row comparison)
    show_data_diff_modal: bool,
    data_diff_source_db: String,
    data_diff_source_table: String,
    data_diff_target_db: String,
    data_diff_target_table: String,
    data_diff_key_col: usize,
    data_diff_report: Option<DataDiffReport>,
    data_diff_sync_sql: String,
    data_diff_loading: bool,

    // SQL Dump Export / Import Modal
    show_sql_dump_modal: bool,
    dump_include_drop: bool,
    dump_include_data: bool,
    dump_selected_tables: std::collections::HashSet<String>,
    dump_preview_sql: String,
    dump_is_busy: bool,

    // Schema Explorer Table Duplicate/Truncate & Table Info Inspector
    table_action_modal: Option<(String, String, String)>, // (db, table_name, new_name_buf)
    table_info_modal: Option<(String, crate::db::TableSchema, usize)>, // (db, schema, subtab)
}

#[derive(Clone)]
struct OpenConnectionWorkspace {
    profile_id: String,
    title: String,
    env_color: (u8, u8, u8),
    session: Arc<ActiveSession>,
    databases: Vec<String>,
    active_database: String,
    schemas: HashMap<String, DatabaseSchema>,
    tabs: Vec<QueryTab>,
    active_tab_idx: usize,
    in_transaction: bool,
    uncommitted_mutations: usize,
    er_state: ErCanvasState,
}

impl MysqlRustyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: Arc<Runtime>) -> Self {
        configure_modern_dark_theme(&cc.egui_ctx);
        let (tx, rx) = unbounded_channel();
        let config = AppConfig::load();

        let restored_tabs: Vec<QueryTab> = if !config.scratchpads.is_empty() {
            config
                .scratchpads
                .iter()
                .map(|sp| {
                    let mut t = QueryTab::new(&sp.title, &sp.sql);
                    if let Some(p) = &sp.file_path {
                        t.file_path = Some(std::path::PathBuf::from(p));
                    }
                    t
                })
                .collect()
        } else {
            vec![QueryTab::new("Query 1", "")]
        };

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
            tabs: restored_tabs,
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
            renaming_tab_idx: None,
            rename_tab_buffer: String::new(),
            renaming_snippet_id: None,
            rename_snippet_buffer: String::new(),

            qps_history: Vec::new(),
            threads_history: Vec::new(),
            last_poll_instant: None,
            last_questions_count: None,

            in_transaction: false,
            uncommitted_mutations: 0,
            cell_inspector: None,
            csv_wizard: None,
            server_variables: Vec::new(),
            server_variables_filter: String::new(),
            is_loading_variables: false,
            server_admin_subtab: ServerAdminSubtab::OverviewAndMetrics,
            show_command_palette: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,
            pending_delete_connection: None,
            show_connection_trash_modal: false,
            copilot_device_code_data: None,
            copilot_auth_status: String::new(),
            is_copilot_authenticating: false,
            last_scratchpad_save: std::time::Instant::now(),
            open_workspaces: Vec::new(),
            active_workspace_idx: None,
            fk_lookup_cache: HashMap::new(),
            show_data_diff_modal: false,
            data_diff_source_db: String::new(),
            data_diff_source_table: String::new(),
            data_diff_target_db: String::new(),
            data_diff_target_table: String::new(),
            data_diff_key_col: 0,
            data_diff_report: None,
            data_diff_sync_sql: String::new(),
            data_diff_loading: false,
            show_sql_dump_modal: false,
            dump_include_drop: true,
            dump_include_data: true,
            dump_selected_tables: std::collections::HashSet::new(),
            dump_preview_sql: String::new(),
            dump_is_busy: false,
            table_action_modal: None,
            table_info_modal: None,
        }
    }

    fn save_active_workspace_snapshot(&mut self) {
        let Some(idx) = self.active_workspace_idx else {
            return;
        };
        if self.active_session.is_none() {
            return;
        }
        if let Some(ws) = self.open_workspaces.get_mut(idx) {
            ws.databases = self.databases.clone();
            ws.active_database = self.active_database.clone();
            ws.schemas = self.schemas.clone();
            ws.tabs = self.tabs.clone();
            ws.active_tab_idx = self.active_tab_idx;
            ws.in_transaction = self.in_transaction;
            ws.uncommitted_mutations = self.uncommitted_mutations;
            ws.er_state = self.er_canvas_state.clone();
        }
    }

    fn switch_to_workspace(&mut self, target_idx: Option<usize>) {
        if self.active_workspace_idx == target_idx {
            return;
        }
        self.save_active_workspace_snapshot();
        match target_idx {
            None => {
                self.active_workspace_idx = None;
                self.active_session = None;
                self.active_profile_id = None;
                self.databases.clear();
                self.schemas.clear();
                self.active_database.clear();
                self.in_transaction = false;
                self.uncommitted_mutations = 0;
                self.sidebar_tab = SidebarTab::Connections;
            }
            Some(idx) => {
                if let Some(ws) = self.open_workspaces.get(idx).cloned() {
                    self.active_workspace_idx = Some(idx);
                    self.active_profile_id = Some(ws.profile_id);
                    self.active_session = Some(ws.session);
                    self.databases = ws.databases;
                    self.active_database = ws.active_database;
                    self.schemas = ws.schemas;
                    self.tabs = if ws.tabs.is_empty() {
                        vec![QueryTab::new("Query 1", "")]
                    } else {
                        ws.tabs
                    };
                    self.active_tab_idx = ws.active_tab_idx.min(self.tabs.len().saturating_sub(1));
                    self.in_transaction = ws.in_transaction;
                    self.uncommitted_mutations = ws.uncommitted_mutations;
                    self.er_canvas_state = ws.er_state;
                    self.sidebar_tab = SidebarTab::Schema;
                    self.status_banner = format!("Switched to connection `{}`.", ws.title);
                }
            }
        }
    }

    fn close_workspace(&mut self, idx: usize) {
        if idx >= self.open_workspaces.len() {
            return;
        }
        let closed_title = self.open_workspaces[idx].title.clone();
        self.open_workspaces.remove(idx);

        if self.open_workspaces.is_empty() {
            self.active_workspace_idx = None;
            self.active_session = None;
            self.active_profile_id = None;
            self.databases.clear();
            self.schemas.clear();
            self.active_database.clear();
            self.in_transaction = false;
            self.uncommitted_mutations = 0;
            self.sidebar_tab = SidebarTab::Connections;
            self.status_banner = format!("Disconnected from `{closed_title}`.");
            return;
        }

        match self.active_workspace_idx {
            Some(cur) if cur == idx => {
                let next_idx = idx.min(self.open_workspaces.len() - 1);
                self.active_workspace_idx = None; // prevent saving removed ws
                self.switch_to_workspace(Some(next_idx));
                self.status_banner = format!("Closed `{closed_title}`.");
            }
            Some(cur) if idx < cur => {
                self.active_workspace_idx = Some(cur - 1);
                self.status_banner = format!("Closed `{closed_title}`.");
            }
            _ => {
                self.status_banner = format!("Closed `{closed_title}`.");
            }
        }
    }

    fn disconnect(&mut self) {
        if let Some(idx) = self.active_workspace_idx {
            self.close_workspace(idx);
        } else {
            self.active_session = None;
            self.active_profile_id = None;
            self.databases.clear();
            self.schemas.clear();
            self.active_database.clear();
            self.sidebar_tab = SidebarTab::Connections;
            self.status_banner = "Disconnected. Select a connection to begin.".to_string();
        }
    }

    fn connect_to_profile(&mut self, profile: ConnectionProfile) {
        if let Some(existing_idx) = self
            .open_workspaces
            .iter()
            .position(|ws| ws.profile_id == profile.id)
        {
            self.switch_to_workspace(Some(existing_idx));
            return;
        }
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

    fn save_active_tab_direct(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        // Determine destination path: use existing file_path or auto-assign inside queries_dir
        let path = if let Some(existing_path) = &tab.file_path {
            existing_path.clone()
        } else {
            let safe_title = sanitize_filename(&tab.title);
            let base_name = if safe_title.is_empty() {
                "query".to_string()
            } else {
                safe_title
            };
            let mut candidate = AppConfig::queries_dir().join(format!("{base_name}.sql"));
            let mut counter = 1;
            while candidate.exists() {
                candidate = AppConfig::queries_dir().join(format!("{base_name}_{counter}.sql"));
                counter += 1;
            }
            tab.file_path = Some(candidate.clone());
            candidate
        };

        // Write SQL directly to file
        if let Err(e) = std::fs::write(&path, &tab.sql) {
            self.status_banner = format!("Error saving query: {e}");
            return;
        }

        tab.is_dirty = false;

        // Auto-sync into Saved Queries library (config.snippets)
        let tab_id = tab.id.clone();
        let tab_title = tab.title.clone();
        let tab_sql = tab.sql.clone();
        let active_db = if self.active_database.is_empty() {
            None
        } else {
            Some(self.active_database.clone())
        };
        let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

        if let Some(existing) = self.config.snippets.iter_mut().find(|s| s.id == tab_id) {
            existing.title = tab_title.clone();
            existing.sql = tab_sql;
            if existing.database.is_none() {
                existing.database = active_db;
            }
        } else {
            self.config.snippets.push(SavedSnippet {
                id: tab_id,
                title: tab_title.clone(),
                sql: tab_sql,
                database: active_db,
                description: String::new(),
                created_at: now_str,
            });
        }

        let _ = self.config.save();
        let display_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&tab_title);
        self.status_banner = format!("Saved '{display_name}' directly to {}", path.display());
    }

    fn rename_tab(&mut self, idx: usize, new_name: String) {
        let clean_name = new_name.trim();
        if clean_name.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let old_title = tab.title.clone();
        tab.title = clean_name.to_string();

        // If file_path was previously saved in queries_dir, rename the file too
        if let Some(old_path) = tab.file_path.take() {
            let safe = sanitize_filename(clean_name);
            let new_path = AppConfig::queries_dir().join(format!("{safe}.sql"));
            let _ = std::fs::rename(&old_path, &new_path);
            tab.file_path = Some(new_path);
        }

        // Also update in config.snippets if present
        let tab_id = tab.id.clone();
        if let Some(snip) = self
            .config
            .snippets
            .iter_mut()
            .find(|s| s.id == tab_id || s.title == old_title)
        {
            snip.title = clean_name.to_string();
            let _ = self.config.save();
        }

        self.status_banner = format!("Renamed query to '{clean_name}'");
    }

    fn rename_saved_query(&mut self, snippet_id: &str, new_title: &str) {
        let clean = new_title.trim();
        if clean.is_empty() {
            return;
        }
        if let Some(snip) = self.config.snippets.iter_mut().find(|s| s.id == snippet_id) {
            let old_title = snip.title.clone();
            snip.title = clean.to_string();

            for tab in &mut self.tabs {
                if tab.id == snippet_id || tab.title == old_title {
                    tab.title = clean.to_string();
                }
            }

            let old_file = AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(&old_title)));
            let new_file = AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(clean)));
            if old_file.exists() {
                let _ = std::fs::rename(old_file, new_file);
            }

            let _ = self.config.save();
            self.status_banner = format!("Renamed query to '{clean}'");
        }
    }

    fn save_active_tab_to_file(&mut self, save_as: bool) {
        if !save_as {
            self.save_active_tab_direct();
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };
        let default_name = format!("{}.sql", sanitize_filename(&tab.title));
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("SQL Files", &["sql", "txt"])
            .save_file()
        {
            if let Err(e) = std::fs::write(&path, &tab.sql) {
                self.status_banner = format!("Failed to export SQL file: {e}");
            } else {
                self.status_banner = format!("Exported query to {}", path.display());
            }
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
        let mut sql = match prefix_override {
            Some(prefix) => format!("{prefix} {}", raw_sql.trim()),
            None => raw_sql.trim().to_string(),
        };
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            if !tab.query_params.is_empty() {
                sql = substitute_sql_parameters(&sql, &tab.query_params);
            }
        }
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

        if self.in_transaction && is_write_query(&sql) {
            self.uncommitted_mutations += 1;
        }

        let tab_id = tab.id.clone();
        let db = self.active_database.clone();
        let schema = self.schemas.get(&db).cloned();
        let tx = self.tx.clone();

        self.status_banner = format!("Running query on `{db}`...");
        let statements = split_sql_statements(&sql);
        let join_handle = self.rt.spawn(async move {
            if statements.len() > 1 {
                let mut results = Vec::with_capacity(statements.len());
                for stmt in statements {
                    let res = session
                        .execute_query(&db, &stmt, schema.as_ref())
                        .await
                        .map_err(|e| format!("{e:#}"));
                    results.push(res);
                }
                let _ = tx.send(AsyncEvent::MultiQueryFinished { tab_id, results });
            } else {
                let res = session
                    .execute_query(&db, &sql, schema.as_ref())
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::QueryFinished {
                    tab_id,
                    result: res,
                });
            }
        });
        tab.task_abort_handle = Some(join_handle.abort_handle());
    }

    pub fn start_transaction(&mut self) {
        self.in_transaction = true;
        self.uncommitted_mutations = 0;
        self.status_banner = "Transaction started (Manual Commit Mode).".to_string();
        self.execute_sql_forced("START TRANSACTION;".to_string());
    }

    pub fn commit_transaction(&mut self) {
        self.in_transaction = false;
        self.uncommitted_mutations = 0;
        self.status_banner = "Transaction committed successfully.".to_string();
        self.execute_sql_forced("COMMIT;".to_string());
    }

    pub fn rollback_transaction(&mut self) {
        self.in_transaction = false;
        self.uncommitted_mutations = 0;
        self.status_banner = "Transaction rolled back.".to_string();
        self.execute_sql_forced("ROLLBACK;".to_string());
    }

    pub fn fetch_server_variables(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        self.is_loading_variables = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session.list_server_variables().await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ServerVariablesLoaded(res));
        });
    }

    pub fn open_csv_import_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select CSV file to import")
            .add_filter("CSV Files", &["csv"])
            .pick_file()
        {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let default_table = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    match inspect_csv_content(&content, 5) {
                        Ok((headers, sample_rows, total_rows)) => {
                            let mappings = headers.iter().map(|h| (h.clone(), h.clone())).collect();
                            self.csv_wizard = Some(CsvImportWizard {
                                file_path: Some(path),
                                file_name,
                                headers,
                                sample_rows,
                                total_rows,
                                target_table: default_table.clone(),
                                new_table_name: default_table,
                                create_new_table: false,
                                column_mappings: mappings,
                                is_importing: false,
                                status_message: None,
                                error_message: None,
                            });
                        }
                        Err(e) => {
                            self.status_banner = format!("Failed to inspect CSV: {e}");
                        }
                    }
                }
                Err(e) => {
                    self.status_banner = format!("Failed to read CSV file: {e}");
                }
            }
        }
    }

    pub fn execute_csv_import(&mut self) {
        let Some(wizard) = &mut self.csv_wizard else {
            return;
        };
        let Some(path) = &wizard.file_path else {
            return;
        };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                wizard.error_message = Some(format!("Failed to read file: {e}"));
                return;
            }
        };

        let target_name = if wizard.create_new_table {
            wizard.new_table_name.trim().to_string()
        } else {
            wizard.target_table.trim().to_string()
        };

        match generate_csv_import_sql(
            &target_name,
            &wizard.headers,
            &wizard.column_mappings,
            &content,
            wizard.create_new_table,
        ) {
            Ok(sql) => {
                let count = wizard.total_rows;
                self.tabs.push(QueryTab::new(format!("Import {target_name}"), sql));
                self.active_tab_idx = self.tabs.len() - 1;
                self.status_banner = format!("Generated import SQL for `{target_name}` ({count} rows).");
                self.csv_wizard = None;
            }
            Err(e) => {
                wizard.error_message = Some(format!("Error generating import SQL: {e}"));
            }
        }
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
        let (sql_prefix, sql_suffix) = {
            let (p, s) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
            (p.to_string(), s.to_string())
        };

        // Opción 1: Local Schema Native runs in 0ms without async network spawning
        if self.config.ai.provider == AiProvider::LocalSchema {
            let schema = self.schemas.get(&self.active_database);
            tab.ghost_completion = compute_heuristic_ghost_completion(&sql_prefix, schema);
            return;
        }

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
            let res = request_inline_completion(&ai_cfg, &sql_prefix, &sql_suffix, &ddl)
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
                    self.save_active_workspace_snapshot();

                    let (conn_title, env_color) = self
                        .config
                        .connections
                        .iter()
                        .find(|c| c.id == profile_id)
                        .map(|c| (c.name.clone(), c.environment.color_rgb()))
                        .unwrap_or_else(|| ("Connection".to_string(), (56, 189, 248)));

                    let mut schemas_map = HashMap::new();
                    let active_db = if let Some(schema) = initial_schema {
                        let name = schema.name.clone();
                        schemas_map.insert(name.clone(), schema);
                        name
                    } else {
                        databases.first().cloned().unwrap_or_default()
                    };

                    // Keep scratchpad tabs for the very first workspace; start fresh tab for extra connections
                    let initial_tabs = if self.open_workspaces.is_empty() && !self.tabs.is_empty() {
                        let mut tabs_clone = self.tabs.clone();
                        if let Some(tab) = tabs_clone.first_mut() {
                            if tab.sql.trim().is_empty() {
                                if let Some(schema) = schemas_map.get(&active_db) {
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
                        tabs_clone
                    } else {
                        let starter_sql = if let Some(schema) = schemas_map.get(&active_db) {
                            if let Some(first_tbl) = schema.tables.first() {
                                format!("SELECT * FROM `{}` LIMIT 100;", first_tbl.name)
                            } else {
                                "SHOW TABLES;".to_string()
                            }
                        } else {
                            "SHOW TABLES;".to_string()
                        };
                        vec![QueryTab::new("Query 1", starter_sql)]
                    };

                    let new_ws = OpenConnectionWorkspace {
                        profile_id: profile_id.clone(),
                        title: conn_title,
                        env_color,
                        session: session.clone(),
                        databases: databases.clone(),
                        active_database: active_db.clone(),
                        schemas: schemas_map.clone(),
                        tabs: initial_tabs.clone(),
                        active_tab_idx: 0,
                        in_transaction: false,
                        uncommitted_mutations: 0,
                        er_state: ErCanvasState::default(),
                    };

                    self.open_workspaces.push(new_ws);
                    let new_idx = self.open_workspaces.len() - 1;
                    self.active_workspace_idx = Some(new_idx);
                    self.active_profile_id = Some(profile_id);
                    self.active_session = Some(session);
                    self.databases = databases;
                    self.active_database = active_db;
                    self.schemas = schemas_map;
                    self.tabs = initial_tabs;
                    self.active_tab_idx = 0;
                    self.in_transaction = false;
                    self.uncommitted_mutations = 0;
                    self.er_canvas_state = ErCanvasState::default();

                    self.status_banner = format!(
                        "Connected to database `{}` ({} schemas discovered)",
                        self.active_database,
                        self.databases.len()
                    );
                    self.sidebar_tab = SidebarTab::Schema;

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
                                tab.multi_results.clear();
                                tab.active_result_idx = 0;
                                tab.last_error = None;
                                tab.current_page = 0;
                            }
                            Err(err) => {
                                self.status_banner = format!("Query failed: {err}");
                                tab.last_error = Some(err);
                                tab.last_result = None;
                                tab.multi_results.clear();
                            }
                        }
                    }
                }
                AsyncEvent::MultiQueryFinished { tab_id, results } => {
                    self.bottom_tab = BottomTab::ResultsGrid;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_running = false;
                        tab.task_abort_handle = None;
                        tab.multi_results.clear();
                        tab.active_result_idx = 0;
                        let mut first_err = None;
                        for r in results {
                            match r {
                                Ok(res) => tab.multi_results.push(res),
                                Err(e) => {
                                    if first_err.is_none() {
                                        first_err = Some(e);
                                    }
                                }
                            }
                        }
                        if let Some(first) = tab.multi_results.first().cloned() {
                            self.status_banner = format!(
                                "Executed {} statements ({} result sets)",
                                tab.multi_results.len(),
                                tab.multi_results.len()
                            );
                            tab.last_result = Some(first);
                            tab.last_error = first_err;
                            tab.current_page = 0;
                        } else if let Some(err) = first_err {
                            self.status_banner = format!("Multi-query failed: {err}");
                            tab.last_error = Some(err);
                            tab.last_result = None;
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
                                let (prefix, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                                tab.ghost_completion =
                                    compute_heuristic_ghost_completion(prefix, schema);
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
                AsyncEvent::ServerVariablesLoaded(res) => {
                    self.is_loading_variables = false;
                    match res {
                        Ok(vars) => {
                            self.server_variables = vars;
                            self.status_banner = format!("Loaded {} server variables.", self.server_variables.len());
                        }
                        Err(e) => {
                            self.status_banner = format!("Failed to load server variables: {e}");
                        }
                    }
                }
                AsyncEvent::CopilotDeviceCodeReady(res) => {
                    self.is_copilot_authenticating = false;
                    match res {
                        Ok(data) => {
                            self.copilot_auth_status = format!(
                                "One-time code generated: {}. Enter this code at https://github.com/login/device",
                                data.user_code
                            );
                            self.copilot_device_code_data = Some(data);
                        }
                        Err(e) => {
                            self.copilot_auth_status = format!("Failed to get device code: {e}");
                        }
                    }
                }
                AsyncEvent::CopilotPollFinished(res) => {
                    self.is_copilot_authenticating = false;
                    match res {
                        Ok(token) => {
                            self.config.ai.copilot_github_token = token;
                            self.copilot_device_code_data = None;
                            self.copilot_auth_status = "✅ Successfully connected to GitHub Copilot!".to_string();
                            let _ = self.config.save();
                            self.status_banner = "GitHub Copilot connected and saved.".to_string();
                        }
                        Err(e) => {
                            if e == "authorization_pending" {
                                self.copilot_auth_status = "Waiting for authorization in browser... Click Check again once approved.".to_string();
                            } else {
                                self.copilot_auth_status = format!("Authorization check error: {e}");
                            }
                        }
                    }
                }
                AsyncEvent::FkLookupLoaded { key, result } => {
                    if let Ok(res) = result {
                        let options: Vec<(String, String)> = res
                            .rows
                            .iter()
                            .take(100)
                            .map(|row| {
                                let pk = row.first().map(|c| c.display_text()).unwrap_or_default();
                                let preview = row
                                    .iter()
                                    .skip(1)
                                    .take(2)
                                    .map(|c| c.display_text())
                                    .filter(|s| !s.is_empty() && s != "NULL")
                                    .collect::<Vec<_>>()
                                    .join(" | ");
                                let label = if preview.is_empty() {
                                    pk.clone()
                                } else {
                                    format!("{pk} — {preview}")
                                };
                                (pk, label)
                            })
                            .collect();
                        self.fk_lookup_cache.insert(key, options);
                    }
                }
                AsyncEvent::DataDiffLoaded {
                    left,
                    right,
                    key_col_idx,
                } => {
                    self.data_diff_loading = false;
                    match (left, right) {
                        (Ok(l_res), Ok(r_res)) => {
                            let report = compute_data_diff(&l_res, &r_res, key_col_idx);
                            let sync_sql = generate_data_diff_sync_sql(
                                &self.data_diff_target_table,
                                &report,
                                key_col_idx,
                            );
                            self.status_banner = format!(
                                "Data Diff: {} modified, {} only in source, {} only in target",
                                report.modified_count,
                                report.left_only_count,
                                report.right_only_count
                            );
                            self.data_diff_report = Some(report);
                            self.data_diff_sync_sql = sync_sql;
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            self.status_banner = format!("Data Diff failed: {e}");
                        }
                    }
                }
                AsyncEvent::SqlDumpLoaded {
                    sql,
                    suggested_name,
                } => {
                    self.dump_is_busy = false;
                    match sql {
                        Ok(content) => {
                            self.dump_preview_sql = content;
                            self.status_banner = format!(
                                "Generated SQL dump ({} bytes) for `{suggested_name}`.",
                                self.dump_preview_sql.len()
                            );
                        }
                        Err(e) => {
                            self.status_banner = format!("SQL Dump generation failed: {e}");
                        }
                    }
                }
                AsyncEvent::SqlDumpImportFinished(res) => {
                    self.dump_is_busy = false;
                    match res {
                        Ok(count) => {
                            self.status_banner = format!(
                                "Successfully executed {count} SQL statements from dump file."
                            );
                            self.reload_active_schema();
                        }
                        Err(e) => {
                            self.status_banner = format!("SQL Dump import failed: {e}");
                        }
                    }
                }
                AsyncEvent::RowDeletesApplied {
                    tab_id,
                    deleted_count,
                } => match deleted_count {
                    Ok(n) => {
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.pending_row_deletes.clear();
                            tab.selected_rows.clear();
                        }
                        self.status_banner = format!("Deleted {n} row(s) successfully.");
                        self.execute_current_tab_query(None);
                    }
                    Err(e) => {
                        self.status_banner = format!("Failed to delete rows: {e}");
                    }
                },
            }
        }
    }

}

impl eframe::App for MysqlRustyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_async_events();

        // Periodic Scratchpad Auto-Save (every 5 seconds)
        if self.last_scratchpad_save.elapsed().as_secs() >= 5 {
            self.last_scratchpad_save = std::time::Instant::now();
            self.config.scratchpads = self
                .tabs
                .iter()
                .map(|t| SavedScratchpad {
                    title: t.title.clone(),
                    sql: t.sql.clone(),
                    file_path: t.file_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                })
                .collect();
            let _ = self.config.save();
        }

        // Global keyboard shortcuts (Ctrl+Enter, F5, Ctrl+Shift+Enter, Shift+F5, Ctrl+Shift+F, Ctrl+K, Ctrl+P)
        let is_modal_open = self.show_connection_modal || self.show_ai_settings_modal;
        if !is_modal_open {
            let mut global_run_stmt = false;
            let mut global_run_query = false;
            let mut global_format_sql = false;
            let mut toggle_palette = false;

            ui.ctx().input_mut(|i| {
                let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
                let is_shift = i.modifiers.shift;

                let has_enter_press = i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. }));

                if is_ctrl && (i.consume_key(Modifiers::NONE, Key::K) || i.consume_key(Modifiers::NONE, Key::P)) {
                    toggle_palette = true;
                } else if is_ctrl && is_shift && has_enter_press {
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
                } else if is_ctrl && is_shift && i.consume_key(Modifiers::NONE, Key::F) {
                    global_format_sql = true;
                }
            });

            if toggle_palette {
                self.show_command_palette = !self.show_command_palette;
                self.command_palette_query.clear();
                self.command_palette_selected = 0;
            } else if global_run_stmt {
                self.execute_current_statement();
            } else if global_run_query {
                self.execute_current_tab_query(None);
            } else if global_format_sql {
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.sql = format_sql_pretty(&tab.sql);
                    self.status_banner = "Formatted SQL query (Ctrl+Shift+F).".to_string();
                }
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

        // Debounced automatic inline AI completion (new edits supersede in-flight requests via inline_req_id)
        if self.config.ai.enable_inline_autocomplete {
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

        // #8 Environment Color Strip across the entire top of the window
        if let Some(prof) = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|c| &c.id == id))
        {
            let (r, g, b) = prof.environment.color_rgb();
            egui::Panel::top("env_color_strip")
                .exact_size(4.0)
                .frame(egui::Frame::NONE.fill(Color32::from_rgb(r, g, b)))
                .show(ui, |_ui| {});
        }

        self.render_open_connections_bar(ui);
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
        self.render_cell_inspector_modal(&ctx);
        self.render_csv_import_modal(&ctx);
        self.render_command_palette_modal(&ctx);
        self.render_delete_connection_confirm_modal(&ctx);
        self.render_connection_trash_modal(&ctx);
        self.render_data_diff_modal(&ctx);
        self.render_sql_dump_modal(&ctx);
        self.render_table_action_modal(&ctx);
        self.render_table_info_modal(&ctx);
    }
}

impl MysqlRustyApp {
    fn render_open_connections_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("open_connections_tab_bar")
            .exact_size(30.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    let is_home = self.active_workspace_idx.is_none();
                    let home_btn = egui::Button::new(
                        RichText::new("Home")
                            .size(12.0)
                            .strong()
                            .color(if is_home {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(148, 163, 184)
                            }),
                    )
                    .fill(if is_home {
                        Color32::from_rgb(30, 58, 138)
                    } else {
                        Color32::from_rgb(20, 27, 42)
                    })
                    .corner_radius(egui::CornerRadius::same(4));

                    if ui
                        .add(home_btn)
                        .on_hover_text("Connections Dashboard (keep open connections running)")
                        .clicked()
                    {
                        self.switch_to_workspace(None);
                    }

                    let mut switch_target: Option<usize> = None;
                    let mut close_target: Option<usize> = None;

                    for (idx, ws) in self.open_workspaces.iter().enumerate() {
                        let is_active = self.active_workspace_idx == Some(idx);
                        let bg = if is_active {
                            Color32::from_rgb(15, 39, 68)
                        } else {
                            Color32::from_rgb(19, 25, 38)
                        };
                        let border = if is_active {
                            Stroke::new(1.0, Color32::from_rgb(56, 189, 248))
                        } else {
                            Stroke::new(1.0, Color32::from_rgb(33, 43, 63))
                        };

                        egui::Frame::new()
                            .fill(bg)
                            .stroke(border)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (r, g, b) = ws.env_color;
                                    ui.label(
                                        RichText::new("●")
                                            .small()
                                            .color(Color32::from_rgb(r, g, b)),
                                    );
                                    let label_text = RichText::new(&ws.title).size(12.0).color(
                                        if is_active {
                                            Color32::WHITE
                                        } else {
                                            Color32::from_rgb(203, 213, 225)
                                        },
                                    );
                                    let label_text = if is_active {
                                        label_text.strong()
                                    } else {
                                        label_text
                                    };
                                    if ui.selectable_label(false, label_text).clicked() {
                                        switch_target = Some(idx);
                                    }
                                    if ui
                                        .small_button(
                                            RichText::new("×")
                                                .small()
                                                .color(Color32::from_rgb(248, 113, 113)),
                                        )
                                        .on_hover_text(format!("Close connection `{}`", ws.title))
                                        .clicked()
                                    {
                                        close_target = Some(idx);
                                    }
                                });
                            });
                    }

                    if ui
                        .small_button("+")
                        .on_hover_text("Open another connection from Home")
                        .clicked()
                    {
                        self.switch_to_workspace(None);
                    }

                    if let Some(idx) = close_target {
                        self.close_workspace(idx);
                    } else if let Some(idx) = switch_target {
                        self.switch_to_workspace(Some(idx));
                    }
                });
            });
    }

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

                    if is_connected {
                        if ui
                            .small_button("Data Diff")
                            .on_hover_text("Compare table rows across databases/tables")
                            .clicked()
                        {
                            self.show_data_diff_modal = true;
                            if self.data_diff_source_db.is_empty() {
                                self.data_diff_source_db = self.active_database.clone();
                                self.data_diff_target_db = self.active_database.clone();
                            }
                        }
                        if ui
                            .small_button("SQL Dump")
                            .on_hover_text("Export or import .sql database dumps")
                            .clicked()
                        {
                            self.show_sql_dump_modal = true;
                            if self.dump_selected_tables.is_empty() {
                                if let Some(schema) = self.schemas.get(&self.active_database) {
                                    for t in &schema.tables {
                                        self.dump_selected_tables.insert(t.name.clone());
                                    }
                                }
                            }
                        }
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
                        .on_hover_text("Directly save query to library and disk (Ctrl+S)")
                        .clicked()
                    {
                        self.save_active_tab_direct();
                    }

                    if ui
                        .button("Save As...")
                        .on_hover_text("Save query with custom title, tags, and description")
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
                            RichText::new("Ctrl+K: Palette | Tab: Macro/Ghost | Ctrl+Space: AI | Ctrl+Shift+F: Format | Ctrl+Enter: Run")
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
                .small_button("📥 Import CSV")
                .on_hover_text("Import a CSV file into a new or existing table with preview and mapping")
                .clicked()
            {
                self.open_csv_import_dialog();
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

        let dbs_list = self.databases.clone();
        let schemas_snapshot = self.schemas.clone();
        let filter_lower = self.schema_filter.to_lowercase();
        let mut pending_sql_action: Option<(String, bool)> = None; // (sql, auto_run)
        let mut toggle_favorite_table: Option<String> = None;
        let mut activate_db: Option<String> = None;
        let mut load_db_schema: Option<String> = None;

        let fav_tables: Vec<String> = self
            .config
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
            .map(|c| c.favorite_tables.clone())
            .unwrap_or_default();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for db_name in &dbs_list {
                let is_active_db = self.active_database == *db_name;
                let db_header_text = if is_active_db {
                    RichText::new(format!("{db_name} (active)"))
                        .strong()
                        .color(Color32::from_rgb(56, 189, 248))
                } else {
                    RichText::new(db_name.as_str()).strong()
                };

                let db_resp = egui::CollapsingHeader::new(db_header_text)
                    .id_salt(("sidebar_db_tree", db_name))
                    .default_open(is_active_db)
                    .show(ui, |ui| {
                        if !is_active_db {
                            if ui
                                .small_button("Set Active DB")
                                .on_hover_text(format!("Switch active database to `{db_name}`"))
                                .clicked()
                            {
                                activate_db = Some(db_name.clone());
                            }
                        }

                        let Some(schema) = schemas_snapshot.get(db_name) else {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Schema not loaded").small().weak());
                                if ui.small_button("Load").clicked() {
                                    load_db_schema = Some(db_name.clone());
                                }
                            });
                            return;
                        };

                        let mut sorted_tables: Vec<&crate::db::TableSchema> =
                            schema.tables.iter().collect();
                        sorted_tables.sort_by(|a, b| {
                            let fa = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&a.name));
                            let fb = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&b.name));
                            fb.cmp(&fa).then_with(|| a.name.cmp(&b.name))
                        });

                        for table in sorted_tables {
                            let matches_table = filter_lower.is_empty()
                                || table.name.to_lowercase().contains(&filter_lower)
                                || table
                                    .columns
                                    .iter()
                                    .any(|c| c.name.to_lowercase().contains(&filter_lower));

                            if !matches_table {
                                continue;
                            }

                            let is_fav = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&table.name));
                            let star_prefix = if is_fav { "★ " } else { "" };
                            let header_label =
                                format!("{star_prefix}{} (~{})", table.name, table.approx_rows);
                            let header_text = if is_fav {
                                RichText::new(header_label)
                                    .strong()
                                    .color(Color32::from_rgb(250, 204, 21))
                            } else {
                                RichText::new(header_label).strong()
                            };
                            egui::CollapsingHeader::new(header_text)
                                .id_salt(("sidebar_tbl_tree", db_name, &table.name))
                                .default_open(is_active_db)
                                .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let pin_txt = if is_fav { "★ Unpin" } else { "☆ Pin" };
                            if ui
                                .small_button(pin_txt)
                                .on_hover_text("Pin/Unpin favorite table to top of Schema Explorer")
                                .clicked()
                            {
                                toggle_favorite_table = Some(table.name.clone());
                            }
                            if ui.small_button("SELECT *").clicked() {
                                pending_sql_action = Some((
                                    format!("SELECT * FROM `{}`.`{}` LIMIT 100;", db_name, table.name),
                                    true,
                                ));
                            }
                            if ui.small_button("DDL").clicked() {
                                pending_sql_action = Some((
                                    format!("SHOW CREATE TABLE `{}`.`{}`;", db_name, table.name),
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
                                .small_button("Info")
                                .on_hover_text("Inspect Columns, Indexes, Foreign Keys, and DDL")
                                .clicked()
                            {
                                self.table_info_modal = Some((db_name.clone(), (*table).clone(), 0));
                            }
                            if ui
                                .small_button("Dup / Trunc")
                                .on_hover_text("Duplicate structure/data or truncate this table")
                                .clicked()
                            {
                                self.table_action_modal = Some((
                                    db_name.clone(),
                                    table.name.clone(),
                                    format!("{}_copy", table.name),
                                ));
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

                if db_resp.header_response.clicked() && !is_active_db {
                    activate_db = Some(db_name.clone());
                }
            }
        });

        if let Some(db) = activate_db {
            self.active_database = db;
            self.reload_active_schema();
        } else if let Some(db) = load_db_schema {
            if let Some(session) = self.active_session.clone() {
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    if let Ok(schema) = session.introspect_database(&db).await {
                        let _ = tx.send(AsyncEvent::SchemaLoaded(schema));
                    }
                });
            }
        }

        if let Some(tbl_name) = toggle_favorite_table {
            if let Some(prof) = self
                .config
                .connections
                .iter_mut()
                .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
            {
                if let Some(pos) = prof
                    .favorite_tables
                    .iter()
                    .position(|t| t.eq_ignore_ascii_case(&tbl_name))
                {
                    prof.favorite_tables.remove(pos);
                } else {
                    prof.favorite_tables.push(tbl_name);
                }
                let _ = self.config.save();
            }
        }

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

        if !self.config.trashed_connections.is_empty() {
            ui.horizontal(|ui| {
                if ui
                    .small_button(format!(
                        "Recycle Bin ({})",
                        self.config.trashed_connections.len()
                    ))
                    .on_hover_text("View and restore deleted connections")
                    .clicked()
                {
                    self.show_connection_trash_modal = true;
                }
            });
            ui.add_space(4.0);
        }

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
            let name = self
                .config
                .connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "this connection".to_string());
            self.pending_delete_connection = Some((id, name));
        }
    }

    fn render_saved_queries_panel(&mut self, ui: &mut Ui) {
        let mut load_sql: Option<String> = None;
        let mut run_sql_now: Option<String> = None;
        let mut new_tab_entry: Option<(String, String)> = None;
        let mut delete_snippet_id: Option<String> = None;
        let mut export_snippet_sql: Option<(String, String)> = None;
        let mut rename_snippet: Option<(String, String)> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Saved Queries Library").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Save Active").on_hover_text("Directly save active query to library and disk (Ctrl+S)").clicked() {
                    self.save_active_tab_direct();
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
                    self.save_active_tab_direct();
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
                        let is_renaming = self.renaming_snippet_id.as_deref() == Some(&snip.id);
                        ui.horizontal(|ui| {
                            if is_renaming {
                                let edit_res = ui.add(
                                    egui::TextEdit::singleline(&mut self.rename_snippet_buffer)
                                        .desired_width(120.0),
                                );
                                if edit_res.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                                    rename_snippet = Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                                }
                                if ui.small_button("OK").clicked() {
                                    rename_snippet = Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                                }
                                if ui.small_button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                                    self.renaming_snippet_id = None;
                                }
                            } else {
                                let label_res = ui.label(RichText::new(&snip.title).strong().size(13.0));
                                if label_res.double_clicked() {
                                    self.renaming_snippet_id = Some(snip.id.clone());
                                    self.rename_snippet_buffer = snip.title.clone();
                                }
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
                                    if ui.small_button("Rename").on_hover_text("Rename query (or double-click title)").clicked() {
                                        self.renaming_snippet_id = Some(snip.id.clone());
                                        self.rename_snippet_buffer = snip.title.clone();
                                    }
                                });
                            }
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

        if let Some((id, new_title)) = rename_snippet {
            self.rename_saved_query(&id, &new_title);
            self.renaming_snippet_id = None;
        }

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
        let mut save_snippet_from_history: Option<SavedSnippet> = None;

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
                            if ui.small_button("Save to Library").on_hover_text("Save query into Library").clicked() {
                                let title = entry
                                    .sql
                                    .lines()
                                    .next()
                                    .unwrap_or("Historic Query")
                                    .trim()
                                    .chars()
                                    .take(40)
                                    .collect::<String>();
                                let clean_title = if title.is_empty() { "Historic Query".to_string() } else { title };
                                let new_id = uuid::Uuid::new_v4().to_string();
                                save_snippet_from_history = Some(SavedSnippet {
                                    id: new_id,
                                    title: clean_title,
                                    sql: entry.sql.clone(),
                                    database: if entry.database.is_empty() { None } else { Some(entry.database.clone()) },
                                    description: format!("Executed on {}", entry.timestamp),
                                    created_at: entry.timestamp.clone(),
                                });
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        });

        if let Some(snip) = save_snippet_from_history {
            let title = snip.title.clone();
            self.config.snippets.push(snip);
            let _ = self.config.save();
            self.status_banner = format!("Saved '{title}' to library.");
        }

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
                    if !self.config.trashed_connections.is_empty() {
                        ui.add_space(8.0);
                        if ui
                            .button(format!(
                                "Recycle Bin ({})",
                                self.config.trashed_connections.len()
                            ))
                            .on_hover_text("Restore or permanently delete trashed connections")
                            .clicked()
                        {
                            self.show_connection_trash_modal = true;
                        }
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
            let name = self
                .config
                .connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "this connection".to_string());
            self.pending_delete_connection = Some((id, name));
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
                let mut tab_to_rename: Option<(usize, String)> = None;

                for idx in 0..self.tabs.len() {
                    let is_active = idx == self.active_tab_idx;
                    let is_renaming = self.renaming_tab_idx == Some(idx);
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
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if is_renaming {
                                    let edit_resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.rename_tab_buffer)
                                            .desired_width(100.0),
                                    );
                                    if edit_resp.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                                        tab_to_rename = Some((idx, self.rename_tab_buffer.clone()));
                                    }
                                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                                        self.renaming_tab_idx = None;
                                    }
                                } else {
                                    let label_color = if is_active {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(148, 163, 184)
                                    };
                                    let label = RichText::new(&title).size(12.5).color(label_color);
                                    let resp = ui.selectable_label(is_active, label);
                                    if resp.clicked() {
                                        self.active_tab_idx = idx;
                                    }
                                    if resp.double_clicked() {
                                        self.renaming_tab_idx = Some(idx);
                                        self.rename_tab_buffer = self.tabs[idx].title.clone();
                                    }
                                    resp.context_menu(|ui| {
                                        if ui.button("Rename").clicked() {
                                            self.renaming_tab_idx = Some(idx);
                                            self.rename_tab_buffer = self.tabs[idx].title.clone();
                                            ui.close();
                                        }
                                        if ui.button("Save (Direct)").clicked() {
                                            self.active_tab_idx = idx;
                                            self.save_active_tab_direct();
                                            ui.close();
                                        }
                                        if self.tabs.len() > 1 && ui.button("Close Tab").clicked() {
                                            close_idx = Some(idx);
                                            ui.close();
                                        }
                                    });
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
                if let Some((idx, new_title)) = tab_to_rename {
                    self.rename_tab(idx, new_title);
                    self.renaming_tab_idx = None;
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
                    .small_button("Rename Tab")
                    .on_hover_text("Rename active query tab (or double-click tab name)")
                    .clicked()
                {
                    self.renaming_tab_idx = Some(self.active_tab_idx);
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        self.rename_tab_buffer = tab.title.clone();
                    }
                }
                if ui
                    .small_button("Save (Ctrl+S)")
                    .on_hover_text("Directly save query to disk and library")
                    .clicked()
                {
                    self.save_active_tab_direct();
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
            let in_tx = self.in_transaction;
            let uncommitted_muts = self.uncommitted_mutations;
            let mut trigger_tx_action: Option<u8> = None;

            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                // Quick LIMIT Presets Bar
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Limit:")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    let active_sql_target = tab.selected_sql.as_deref().unwrap_or(&tab.sql);
                    let current_limit = detect_current_limit(active_sql_target);

                    let presets = [
                        (Some(10), "10"),
                        (Some(50), "50"),
                        (Some(100), "100"),
                        (Some(250), "250"),
                        (Some(500), "500"),
                        (Some(1000), "1000"),
                        (Some(5000), "5000"),
                        (None, "All"),
                    ];

                    for (preset_val, label) in presets {
                        let is_selected = current_limit == preset_val;
                        let btn = egui::Button::new(
                            RichText::new(label)
                                .size(11.0)
                                .strong()
                                .color(if is_selected {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(203, 213, 225)
                                }),
                        )
                        .fill(if is_selected {
                            Color32::from_rgb(14, 116, 144)
                        } else {
                            Color32::from_rgb(20, 27, 41)
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if is_selected {
                                Color32::from_rgb(56, 189, 248)
                            } else {
                                Color32::from_rgb(33, 43, 63)
                            },
                        ))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(btn)
                            .on_hover_text(if let Some(n) = preset_val {
                                format!("Set LIMIT {n} in current SQL query")
                            } else {
                                "Remove LIMIT clause (return all rows)".to_string()
                            })
                            .clicked()
                        {
                            if let Some(sel) = tab.selected_sql.clone() {
                                let updated_sel = apply_limit_to_sql(&sel, preset_val);
                                tab.sql = tab.sql.replace(&sel, &updated_sel);
                                tab.selected_sql = Some(updated_sel);
                            } else {
                                tab.sql = apply_limit_to_sql(&tab.sql, preset_val);
                            }
                            tab.is_dirty = true;
                        }
                    }

                    egui::ComboBox::from_id_salt("limit_more_presets_combo")
                        .selected_text(if let Some(lim) = current_limit {
                            format!("{lim} rows")
                        } else {
                            "No Limit".to_string()
                        })
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for val in [5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 5000, 10000] {
                                let is_sel = current_limit == Some(val);
                                if ui.selectable_label(is_sel, format!("LIMIT {val}")).clicked() {
                                    if let Some(sel) = tab.selected_sql.clone() {
                                        let updated_sel = apply_limit_to_sql(&sel, Some(val));
                                        tab.sql = tab.sql.replace(&sel, &updated_sel);
                                        tab.selected_sql = Some(updated_sel);
                                    } else {
                                        tab.sql = apply_limit_to_sql(&tab.sql, Some(val));
                                    }
                                    tab.is_dirty = true;
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(current_limit.is_none(), "No Limit (All)").clicked() {
                                if let Some(sel) = tab.selected_sql.clone() {
                                    let updated_sel = apply_limit_to_sql(&sel, None);
                                    tab.sql = tab.sql.replace(&sel, &updated_sel);
                                    tab.selected_sql = Some(updated_sel);
                                } else {
                                    tab.sql = apply_limit_to_sql(&tab.sql, None);
                                }
                                tab.is_dirty = true;
                            }
                        });

                    ui.separator();
                    if in_tx {
                        ui.label(
                            RichText::new(format!(
                                "⚠ IN TX ({} edit{})",
                                uncommitted_muts,
                                if uncommitted_muts == 1 { "" } else { "s" }
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(250, 204, 21))
                            .strong(),
                        );
                        let commit_btn = egui::Button::new(
                            RichText::new("Commit")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(220, 252, 231)),
                        )
                        .fill(Color32::from_rgb(22, 101, 52))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(34, 197, 94)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(commit_btn)
                            .on_hover_text("Commit current open transaction (COMMIT)")
                            .clicked()
                        {
                            trigger_tx_action = Some(1);
                        }

                        let rb_btn = egui::Button::new(
                            RichText::new("Rollback")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(254, 202, 202)),
                        )
                        .fill(Color32::from_rgb(153, 27, 27))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(rb_btn)
                            .on_hover_text("Rollback current open transaction (ROLLBACK)")
                            .clicked()
                        {
                            trigger_tx_action = Some(2);
                        }
                    } else {
                        let begin_btn = egui::Button::new(
                            RichText::new("Begin Tx")
                                .size(11.0)
                                .color(Color32::from_rgb(203, 213, 225)),
                        )
                        .fill(Color32::from_rgb(20, 27, 41))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(51, 65, 85)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(begin_btn)
                            .on_hover_text("Start a manual transaction (START TRANSACTION)")
                            .clicked()
                        {
                            trigger_tx_action = Some(3);
                        }
                    }
                });
                ui.add_space(2.0);

                // Dynamic Template Parameters (:param_name) Bar
                let detected_params = extract_sql_parameters(&tab.sql);
                if !detected_params.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Params:")
                                .size(12.0)
                                .strong()
                                .color(Color32::from_rgb(192, 132, 252)),
                        );
                        for p in &detected_params {
                            let val_entry = tab.query_params.entry(p.clone()).or_default();
                            ui.label(RichText::new(format!(":{p} =")).monospace().small());
                            ui.add(
                                egui::TextEdit::singleline(val_entry)
                                    .desired_width(95.0)
                                    .hint_text("value or NULL"),
                            );
                        }
                    });
                    ui.add_space(2.0);
                }

                let editor_id = ui.make_persistent_id(("sql_editor", &tab.id));
                let mut move_cursor_to_char: Option<usize> = None;
                let mut keep_editor_focus = false;

                // Accept or dismiss suggestions via keyboard: Esc, Alt+Up/Down, Tab, Alt+1..8
                let mut selected_suggestion: Option<CompletionItem> = None;
                let has_suggestions_or_ghost =
                    tab.ghost_completion.is_some() || !tab.local_suggestions.is_empty();

                if has_suggestions_or_ghost
                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                {
                    tab.ghost_completion = None;
                    tab.local_suggestions.clear();
                    tab.selected_suggestion_idx = 0;
                    keep_editor_focus = true;
                } else if !tab.local_suggestions.is_empty()
                    && ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown))
                {
                    tab.selected_suggestion_idx =
                        (tab.selected_suggestion_idx + 1) % tab.local_suggestions.len();
                    keep_editor_focus = true;
                } else if !tab.local_suggestions.is_empty()
                    && ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp))
                {
                    tab.selected_suggestion_idx = tab
                        .selected_suggestion_idx
                        .checked_sub(1)
                        .unwrap_or(tab.local_suggestions.len() - 1);
                    keep_editor_focus = true;
                } else {
                    let last_word = {
                        let (prefix_before_cursor, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                        prefix_before_cursor
                            .split_whitespace()
                            .last()
                            .unwrap_or("")
                            .trim()
                            .to_string()
                    };
                    let has_any_tab_completion = expand_sql_macro_with_cursor(&last_word).is_some()
                        || tab.ghost_completion.is_some()
                        || !tab.local_suggestions.is_empty();

                    if has_any_tab_completion
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
                    {
                        if let Some((expanded, cursor_offset)) =
                            expand_sql_macro_with_cursor(&last_word)
                        {
                            let new_cursor = replace_at_cursor(
                                &mut tab.sql,
                                tab.cursor_index,
                                last_word.len(),
                                expanded,
                            );
                            let base_chars = new_cursor.saturating_sub(expanded.chars().count());
                            tab.is_dirty = true;
                            move_cursor_to_char = Some(base_chars + cursor_offset);
                            keep_editor_focus = true;
                        } else if let Some(ghost) = tab.ghost_completion.take() {
                            let new_cursor =
                                replace_at_cursor(&mut tab.sql, tab.cursor_index, 0, &ghost);
                            tab.is_dirty = true;
                            move_cursor_to_char = Some(new_cursor);
                            keep_editor_focus = true;
                        } else if !tab.local_suggestions.is_empty() {
                            let idx = tab
                                .selected_suggestion_idx
                                .min(tab.local_suggestions.len() - 1);
                            selected_suggestion = Some(tab.local_suggestions[idx].clone());
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
                }

                if let Some(item) = selected_suggestion {
                    let new_cursor = replace_at_cursor(
                        &mut tab.sql,
                        tab.cursor_index,
                        tab.current_token_prefix.len(),
                        &item.insert_text,
                    );
                    tab.local_suggestions.clear();
                    tab.selected_suggestion_idx = 0;
                    tab.ghost_completion = None;
                    tab.is_dirty = true;
                    move_cursor_to_char = Some(new_cursor);
                    keep_editor_focus = true;
                }

                if let Some(target_char_idx) = move_cursor_to_char {
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                        let ccursor = egui::text::CCursor::new(target_char_idx);
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                        state.store(ui.ctx(), editor_id);
                    }
                    tab.cursor_index = target_char_idx;
                }

                let ghost_clone = tab.ghost_completion.clone();
                let cursor_idx_snapshot = tab.cursor_index;
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = highlight_sql(
                        buf.as_str(),
                        ghost_clone.as_deref(),
                        Some(cursor_idx_snapshot),
                        14.0,
                    );
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                let output = egui::ScrollArea::vertical()
                    .id_salt("sql_editor_scroll")
                    .max_height(195.0)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut tab.sql)
                            .id(editor_id)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner;

                if keep_editor_focus {
                    output.response.request_focus();
                }

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
                    let (sql_prefix, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                    let (prefix, suggestions) =
                        compute_local_completions(sql_prefix, schema.as_ref(), 8);
                    tab.current_token_prefix = prefix;
                    tab.local_suggestions = suggestions;
                    tab.selected_suggestion_idx = 0;
                    if enable_ghost {
                        tab.ghost_completion =
                            compute_heuristic_ghost_completion(sql_prefix, schema.as_ref());
                    } else {
                        tab.ghost_completion = None;
                    }
                }

                // Inline Autocomplete Bar (Schema tokens + Ghost completion accept button)
                if !tab.local_suggestions.is_empty() || tab.ghost_completion.is_some() {
                    let mut clicked_new_cursor: Option<usize> = None;
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
                                .on_hover_text("Insert ghost completion at cursor (Tab)")
                                .clicked()
                            {
                                let new_cursor =
                                    replace_at_cursor(&mut tab.sql, tab.cursor_index, 0, &ghost);
                                tab.ghost_completion = None;
                                tab.is_dirty = true;
                                clicked_new_cursor = Some(new_cursor);
                            }
                        }

                        let suggestions = tab.local_suggestions.clone();
                        let prefix_len = tab.current_token_prefix.len();
                        let has_ghost = tab.ghost_completion.is_some();
                        let active_idx = tab
                            .selected_suggestion_idx
                            .min(suggestions.len().saturating_sub(1));
                        for (idx, item) in suggestions.into_iter().enumerate() {
                            let is_selected = idx == active_idx;
                            let badge = if is_selected && !has_ghost {
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
                            let text = if is_selected {
                                RichText::new(label)
                                    .strong()
                                    .color(Color32::from_rgb(250, 204, 21))
                            } else {
                                RichText::new(label)
                            };
                            if ui
                                .small_button(text)
                                .on_hover_text(format!(
                                    "{}\nShortcut: {} (Alt+↑/↓ to select, Esc to dismiss)",
                                    item.detail,
                                    if is_selected && !has_ghost {
                                        "Tab"
                                    } else if idx < 8 {
                                        "Alt+1..8"
                                    } else {
                                        "Click"
                                    }
                                ))
                                .clicked()
                            {
                                let new_cursor = replace_at_cursor(
                                    &mut tab.sql,
                                    tab.cursor_index,
                                    prefix_len,
                                    &item.insert_text,
                                );
                                tab.local_suggestions.clear();
                                tab.selected_suggestion_idx = 0;
                                tab.ghost_completion = None;
                                tab.is_dirty = true;
                                clicked_new_cursor = Some(new_cursor);
                            }
                        }

                        if ui
                            .small_button(
                                RichText::new("[Esc] ✕")
                                    .small()
                                    .color(Color32::from_rgb(148, 163, 184)),
                            )
                            .on_hover_text("Dismiss suggestions (Esc)")
                            .clicked()
                        {
                            tab.local_suggestions.clear();
                            tab.selected_suggestion_idx = 0;
                            tab.ghost_completion = None;
                            editor_resp.request_focus();
                        }
                    });
                    if let Some(target_char_idx) = clicked_new_cursor {
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                            let ccursor = egui::text::CCursor::new(target_char_idx);
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                            state.store(ui.ctx(), editor_id);
                        }
                        tab.cursor_index = target_char_idx;
                        editor_resp.request_focus();
                    }
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

            match trigger_tx_action {
                Some(1) => self.commit_transaction(),
                Some(2) => self.rollback_transaction(),
                Some(3) => self.start_transaction(),
                _ => {}
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

        let mut result = self.tabs[self.active_tab_idx].last_result.clone().unwrap();
        let active_schema_opt = self.schemas.get(&self.active_database).cloned();
        let mut trigger_commit_edits = false;
        let mut trigger_commit_deletes = false;
        let mut trigger_fk_lookup: Option<(String, String, String)> = None; // (key, ref_table, ref_col)
        let mut trigger_ai_prompt: Option<String> = None;
        let mut cell_inspector_to_open: Option<CellInspectorState> = None;
        let mut open_related_query: Option<(String, String)> = None;
        let mut duplicate_insert_tab: Option<(String, String)> = None;
        let mut copy_banner_msg: Option<String> = None;
        let fk_cache_snapshot = self.fk_lookup_cache.clone();
        let pending_edits;
        let pending_deletes;

        {
            let tab = &mut self.tabs[self.active_tab_idx];

            // Multi-Result Sub-Tabs (when a script executes multiple statements)
            if tab.multi_results.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Result Sets:")
                            .small()
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    if tab.multi_result_layout == MultiResultLayout::Tabs {
                        for (r_idx, r_item) in tab.multi_results.iter().enumerate() {
                            let is_sel = tab.active_result_idx == r_idx;
                            let lbl = format!("Result {} ({} rows)", r_idx + 1, r_item.rows.len());
                            if ui.selectable_label(is_sel, lbl).clicked() {
                                tab.active_result_idx = r_idx;
                                tab.last_result = Some(r_item.clone());
                                tab.current_page = 0;
                                tab.sort_column = None;
                                result = r_item.clone();
                            }
                        }
                    } else {
                        ui.label(
                            RichText::new(format!("Showing all {} results", tab.multi_results.len()))
                                .small()
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        if tab.multi_result_layout == MultiResultLayout::SideBySide {
                            ui.separator();
                            ui.label(RichText::new("Cols:").small().weak());
                            let current_cols_label = match tab.multi_result_columns {
                                None => "Auto".to_string(),
                                Some(c) => format!("{c}"),
                            };
                            egui::ComboBox::from_id_salt("multi_res_cols_combo")
                                .selected_text(current_cols_label)
                                .width(58.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut tab.multi_result_columns, None, "Auto");
                                    for c in [1usize, 2, 3, 4] {
                                        ui.selectable_value(
                                            &mut tab.multi_result_columns,
                                            Some(c),
                                            format!("{c}"),
                                        );
                                    }
                                });
                        }
                        ui.separator();
                        if ui.small_button("▾ Expand All").clicked() {
                            tab.multi_result_collapsed.clear();
                        }
                        if ui.small_button("▸ Collapse All").clicked() {
                            for i in 0..tab.multi_results.len() {
                                tab.multi_result_collapsed.insert(i, true);
                            }
                        }
                        if ui.small_button("↺ Reset Size").on_hover_text("Reset custom card heights").clicked() {
                            tab.multi_result_heights.clear();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::SideBySide,
                            "Side-by-Side",
                        );
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::Stacked,
                            "Stacked",
                        );
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::Tabs,
                            "Tabs",
                        );
                    });
                });
                ui.separator();

                if tab.multi_result_layout != MultiResultLayout::Tabs {
                    Self::render_multi_results_split_view(ui, tab);
                    return;
                }
            }

            // Results Action Bar (Filter, Pagination, Export, Inline Edit Commit)
            ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut tab.row_filter)
                    .desired_width(130.0)
                    .hint_text("Search text..."),
            );

            let filter_btn_txt = if tab.show_visual_filter { "Hide Filter ▴" } else { "Column Filter ▾" };
            if ui.small_button(filter_btn_txt).clicked() {
                tab.show_visual_filter = !tab.show_visual_filter;
            }

            ui.separator();
            egui::ComboBox::from_id_salt("results_page_size_combo")
                .selected_text(if tab.page_size == 0 { "All rows".to_string() } else { format!("{} / page", tab.page_size) })
                .width(85.0)
                .show_ui(ui, |ui| {
                    for ps in [25, 50, 100, 250, 500, 0] {
                        let label = if ps == 0 { "All".to_string() } else { format!("{ps}") };
                        if ui.selectable_label(tab.page_size == ps, label).clicked() {
                            tab.page_size = ps;
                            tab.current_page = 0;
                        }
                    }
                });

            ui.separator();
            ui.selectable_value(&mut tab.view_mode, ResultViewMode::Grid, "Table");
            ui.selectable_value(&mut tab.view_mode, ResultViewMode::Chart, "Chart");

            ui.separator();
            if ui
                .selectable_label(tab.show_row_inspector, "Row Inspector")
                .on_hover_text("Toggle vertical single-row inspector & FK lookup drawer")
                .clicked()
            {
                tab.show_row_inspector = !tab.show_row_inspector;
                if tab.show_row_inspector && tab.row_inspector_row.is_none() && !result.rows.is_empty() {
                    tab.row_inspector_row = Some(0);
                    tab.row_inspector_buffers = result.rows[0]
                        .iter()
                        .map(|c| c.display_text())
                        .collect();
                }
            }

            if ui
                .small_button("Pin Baseline")
                .on_hover_text("Save current result set as baseline for comparison")
                .clicked()
            {
                tab.baseline_result = Some(result.clone());
                tab.show_baseline_compare = false;
                copy_banner_msg = Some("Pinned current result as Baseline.".to_string());
            }
            if tab.baseline_result.is_some() {
                let cmp_lbl = if tab.show_baseline_compare {
                    "Exit Compare"
                } else {
                    "Compare vs Baseline"
                };
                if ui.selectable_label(tab.show_baseline_compare, cmp_lbl).clicked() {
                    tab.show_baseline_compare = !tab.show_baseline_compare;
                }
            }

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

        ui.add_space(2.0);

        // Visual Column Filter Bar
        if tab.show_visual_filter {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Where:").size(12.0).strong().color(Color32::from_rgb(56, 189, 248)));
                let selected_col_name = tab.visual_filter_col
                    .and_then(|idx| result.columns.get(idx))
                    .cloned()
                    .unwrap_or_else(|| "Any Column".to_string());

                egui::ComboBox::from_id_salt("results_vis_filter_col_combo")
                    .selected_text(selected_col_name)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(tab.visual_filter_col.is_none(), "Any Column").clicked() {
                            tab.visual_filter_col = None;
                            tab.current_page = 0;
                        }
                        for (i, col) in result.columns.iter().enumerate() {
                            if ui.selectable_label(tab.visual_filter_col == Some(i), col).clicked() {
                                tab.visual_filter_col = Some(i);
                                tab.current_page = 0;
                            }
                        }
                    });

                egui::ComboBox::from_id_salt("results_vis_filter_op_combo")
                    .selected_text(tab.visual_filter_op.label())
                    .width(115.0)
                    .show_ui(ui, |ui| {
                        for op in [
                            VisualFilterOp::Contains,
                            VisualFilterOp::Equals,
                            VisualFilterOp::NotEquals,
                            VisualFilterOp::StartsWith,
                            VisualFilterOp::GreaterThan,
                            VisualFilterOp::LessThan,
                            VisualFilterOp::IsNull,
                            VisualFilterOp::IsNotNull,
                        ] {
                            if ui.selectable_label(tab.visual_filter_op == op, op.label()).clicked() {
                                tab.visual_filter_op = op;
                                tab.current_page = 0;
                            }
                        }
                    });

                if tab.visual_filter_op != VisualFilterOp::IsNull && tab.visual_filter_op != VisualFilterOp::IsNotNull {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut tab.visual_filter_val)
                            .desired_width(160.0)
                            .hint_text("Value to match..."),
                    );
                    if resp.changed() {
                        tab.current_page = 0;
                    }
                }

                if (!tab.visual_filter_val.is_empty() || tab.visual_filter_col.is_some()) && ui.small_button("Clear").clicked() {
                    tab.visual_filter_val.clear();
                    tab.visual_filter_col = None;
                    tab.current_page = 0;
                }
            });
            ui.add_space(2.0);
        }

        if tab.view_mode == ResultViewMode::Chart {
            Self::draw_results_chart(ui, tab, &result);
            return;
        }

        // #10 Result Snapshot Compare vs Baseline
        if tab.show_baseline_compare {
            if let Some(baseline) = &tab.baseline_result {
                let key_idx = result
                    .primary_key_column
                    .as_ref()
                    .and_then(|pk| result.columns.iter().position(|c| c.eq_ignore_ascii_case(pk)))
                    .unwrap_or(0);
                let report = compute_data_diff(baseline, &result, key_idx);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Baseline Compare: {} modified | {} added | {} removed | {} unchanged",
                            report.modified_count,
                            report.right_only_count,
                            report.left_only_count,
                            report.same_count
                        ))
                        .strong()
                        .color(Color32::from_rgb(56, 189, 248)),
                    );
                });
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    for entry in &report.entries {
                        if entry.status == RowDiffStatus::Same {
                            continue;
                        }
                        let (badge, color) = match entry.status {
                            RowDiffStatus::OnlyInRight => ("+ ADDED", Color32::from_rgb(52, 211, 153)),
                            RowDiffStatus::OnlyInLeft => ("- REMOVED", Color32::from_rgb(248, 113, 113)),
                            RowDiffStatus::Modified => ("~ MODIFIED", Color32::from_rgb(250, 204, 21)),
                            RowDiffStatus::Same => ("SAME", Color32::GRAY),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(badge).small().strong().color(color));
                            ui.label(RichText::new(format!("Key: {}", entry.key_display)).strong());
                            if let Some(r) = &entry.right_row {
                                ui.label(RichText::new(format!("New: {}", r.join(" | "))).small());
                            }
                            if entry.status == RowDiffStatus::Modified {
                                if let Some(l) = &entry.left_row {
                                    ui.label(RichText::new(format!("(Was: {})", l.join(" | "))).small().weak());
                                }
                            }
                        });
                    }
                });
                return;
            }
        }

        // #4 & #5 Multi-Row Selection & Pending Deletes Action Bar
        if !tab.selected_rows.is_empty() || !tab.pending_row_deletes.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if !tab.selected_rows.is_empty() {
                    let mut sel_indices: Vec<usize> = tab.selected_rows.iter().copied().collect();
                    sel_indices.sort_unstable();
                    ui.label(
                        RichText::new(format!("{} row(s) selected:", sel_indices.len()))
                            .small()
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    if ui.small_button("Copy JSON").clicked() {
                        ui.ctx().copy_text(format_rows_as_json(&result, &sel_indices));
                        copy_banner_msg = Some(format!("Copied {} row(s) as JSON.", sel_indices.len()));
                    }
                    if ui.small_button("Copy INSERT").clicked() {
                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                        ui.ctx().copy_text(format_rows_as_sql_insert(tbl, &result, &sel_indices));
                        copy_banner_msg = Some(format!("Copied {} row(s) as SQL INSERT.", sel_indices.len()));
                    }
                    if let Some(pk_col) = &result.primary_key_column {
                        if ui.small_button("Copy UPDATE").clicked() {
                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                            ui.ctx().copy_text(format_rows_as_sql_update(tbl, pk_col, &result, &sel_indices));
                            copy_banner_msg = Some(format!("Copied {} row(s) as SQL UPDATE.", sel_indices.len()));
                        }
                    }
                    if ui.small_button("Duplicate Row(s)").clicked() {
                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                        let sql = format_rows_as_sql_insert(tbl, &result, &sel_indices);
                        duplicate_insert_tab = Some((format!("Duplicate {tbl}"), sql));
                    }
                    if let (Some(_tbl), Some(pk_col)) = (&result.editable_table, &result.primary_key_column) {
                        if let Some(pk_idx) = result.columns.iter().position(|c| c.eq_ignore_ascii_case(pk_col)) {
                            if ui.small_button("Mark for Delete").clicked() {
                                for &r_idx in &sel_indices {
                                    if !tab.pending_row_deletes.iter().any(|(idx, _, _)| *idx == r_idx) {
                                        if let Some(pk_val) = result.rows.get(r_idx).and_then(|r| r.get(pk_idx)) {
                                            tab.pending_row_deletes.push((r_idx, pk_col.clone(), pk_val.display_text()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if ui.small_button("Clear Sel").clicked() {
                        tab.selected_rows.clear();
                    }
                }

                if !tab.pending_row_deletes.is_empty() {
                    ui.separator();
                    let del_btn = egui::Button::new(
                        RichText::new(format!("Commit Deletes ({})", tab.pending_row_deletes.len()))
                            .small()
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(185, 28, 28));
                    if ui.add(del_btn).clicked() {
                        trigger_commit_deletes = true;
                    }
                    if ui.small_button("Discard Deletes").clicked() {
                        tab.pending_row_deletes.clear();
                    }
                }
            });
            ui.add_space(2.0);
        }

        // Filter & sort rows
        let filter_lower = tab.row_filter.to_lowercase();
        let val_filter_lower = tab.visual_filter_val.to_lowercase();
        let mut filtered_indices: Vec<usize> = (0..result.rows.len())
            .filter(|&r_idx| {
                let row = &result.rows[r_idx];
                if !filter_lower.is_empty() && !row.iter().any(|c| c.display_text().to_lowercase().contains(&filter_lower)) {
                    return false;
                }
                if let Some(col_idx) = tab.visual_filter_col {
                    if let Some(cell) = row.get(col_idx) {
                        let text = cell.display_text();
                        let text_lower = text.to_lowercase();
                        match tab.visual_filter_op {
                            VisualFilterOp::Contains => text_lower.contains(&val_filter_lower),
                            VisualFilterOp::Equals => text.eq_ignore_ascii_case(&tab.visual_filter_val),
                            VisualFilterOp::NotEquals => !text.eq_ignore_ascii_case(&tab.visual_filter_val),
                            VisualFilterOp::StartsWith => text_lower.starts_with(&val_filter_lower),
                            VisualFilterOp::GreaterThan => {
                                if let (Ok(n1), Ok(n2)) = (text.parse::<f64>(), tab.visual_filter_val.parse::<f64>()) {
                                    n1 > n2
                                } else {
                                    text > tab.visual_filter_val
                                }
                            }
                            VisualFilterOp::LessThan => {
                                if let (Ok(n1), Ok(n2)) = (text.parse::<f64>(), tab.visual_filter_val.parse::<f64>()) {
                                    n1 < n2
                                } else {
                                    text < tab.visual_filter_val
                                }
                            }
                            VisualFilterOp::IsNull => text.is_empty() || text == "NULL",
                            VisualFilterOp::IsNotNull => !text.is_empty() && text != "NULL",
                        }
                    } else {
                        false
                    }
                } else if !val_filter_lower.is_empty() {
                    match tab.visual_filter_op {
                        VisualFilterOp::Contains => row.iter().any(|c| c.display_text().to_lowercase().contains(&val_filter_lower)),
                        VisualFilterOp::Equals => row.iter().any(|c| c.display_text().eq_ignore_ascii_case(&tab.visual_filter_val)),
                        VisualFilterOp::NotEquals => !row.iter().any(|c| c.display_text().eq_ignore_ascii_case(&tab.visual_filter_val)),
                        VisualFilterOp::StartsWith => row.iter().any(|c| c.display_text().to_lowercase().starts_with(&val_filter_lower)),
                        _ => true,
                    }
                } else {
                    true
                }
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

        let total_matching = filtered_indices.len();
        let total_pages = if tab.page_size == 0 {
            1
        } else {
            (total_matching + tab.page_size - 1) / tab.page_size
        }.max(1);

        if tab.current_page >= total_pages {
            tab.current_page = total_pages.saturating_sub(1);
        }

        let (display_start, display_end) = if tab.page_size == 0 {
            (0, total_matching)
        } else {
            let start = tab.current_page * tab.page_size;
            let end = (start + tab.page_size).min(total_matching);
            (start, end)
        };
        let page_indices: Vec<usize> = if total_matching > 0 && display_start < total_matching {
            filtered_indices[display_start..display_end].to_vec()
        } else {
            Vec::new()
        };

        // Render pagination navigation if pages > 1
        if tab.page_size > 0 && total_pages > 1 {
            ui.horizontal(|ui| {
                if ui.small_button("◀ Prev").clicked() && tab.current_page > 0 {
                    tab.current_page -= 1;
                }
                ui.label(RichText::new(format!(
                    "Showing rows {}-{} of {} (Page {} of {})",
                    if total_matching == 0 { 0 } else { display_start + 1 },
                    display_end,
                    total_matching,
                    tab.current_page + 1,
                    total_pages
                )).small().weak());
                if ui.small_button("Next ▶").clicked() && tab.current_page + 1 < total_pages {
                    tab.current_page += 1;
                }
            });
            ui.add_space(2.0);
        }

        let pk_col_idx = result.primary_key_column.as_ref().and_then(|pk| {
            result
                .columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(pk))
        });

        // #1 & #2 Vertical Row Inspector Drawer (Right side inside ResultsGrid)
        if tab.show_row_inspector {
            let max_row = result.rows.len().saturating_sub(1);
            let cur_row_idx = tab.row_inspector_row.unwrap_or(0).min(max_row);
            if tab.row_inspector_buffers.len() != result.columns.len() {
                if let Some(row_data) = result.rows.get(cur_row_idx) {
                    tab.row_inspector_buffers =
                        row_data.iter().map(|c| c.display_text()).collect();
                    tab.row_inspector_row = Some(cur_row_idx);
                }
            }

            egui::Panel::right("row_inspector_drawer")
                .resizable(true)
                .default_size(300.0)
                .min_size(220.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Row #{}", cur_row_idx + 1))
                                .strong()
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                        if ui.small_button("◀").clicked() && cur_row_idx > 0 {
                            let next_r = cur_row_idx - 1;
                            tab.row_inspector_row = Some(next_r);
                            if let Some(rd) = result.rows.get(next_r) {
                                tab.row_inspector_buffers =
                                    rd.iter().map(|c| c.display_text()).collect();
                            }
                        }
                        if ui.small_button("▶").clicked() && cur_row_idx < max_row {
                            let next_r = cur_row_idx + 1;
                            tab.row_inspector_row = Some(next_r);
                            if let Some(rd) = result.rows.get(next_r) {
                                tab.row_inspector_buffers =
                                    rd.iter().map(|c| c.display_text()).collect();
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("×").clicked() {
                                tab.show_row_inspector = false;
                            }
                        });
                    });
                    ui.separator();

                    let can_edit = pk_col_idx.is_some() && result.editable_table.is_some();
                    let table_fks: Vec<crate::db::ForeignKeyInfo> = result
                        .editable_table
                        .as_ref()
                        .and_then(|tname| {
                            active_schema_opt.as_ref().and_then(|s| {
                                s.tables
                                    .iter()
                                    .find(|t| t.name.eq_ignore_ascii_case(tname))
                                    .map(|t| t.foreign_keys.clone())
                            })
                        })
                        .unwrap_or_default();

                    egui::ScrollArea::vertical()
                        .id_salt("row_inspector_scroll")
                        .show(ui, |ui| {
                            for (c_idx, col_name) in result.columns.iter().enumerate() {
                                let is_pk = pk_col_idx == Some(c_idx);
                                let fk_opt = table_fks
                                    .iter()
                                    .find(|fk| fk.column_name.eq_ignore_ascii_case(col_name));

                                ui.horizontal(|ui| {
                                    let badge = if is_pk {
                                        " [PK]"
                                    } else if fk_opt.is_some() {
                                        " [FK]"
                                    } else {
                                        ""
                                    };
                                    ui.label(RichText::new(format!("{col_name}{badge}")).small().strong());
                                    if let Some(fk) = fk_opt {
                                        let fk_key = format!("{}.{}", fk.referenced_table, fk.referenced_column);
                                        if let Some(opts) = fk_cache_snapshot.get(&fk_key) {
                                            egui::ComboBox::from_id_salt(("fk_lookup_combo", c_idx))
                                                .selected_text("FK Lookup")
                                                .width(110.0)
                                                .show_ui(ui, |ui| {
                                                    for (val, lbl) in opts {
                                                        if ui.selectable_label(false, lbl).clicked() {
                                                            if let Some(buf) = tab.row_inspector_buffers.get_mut(c_idx) {
                                                                *buf = val.clone();
                                                            }
                                                        }
                                                    }
                                                });
                                        } else if ui
                                            .small_button("Lookup")
                                            .on_hover_text(format!("Load values from `{}`", fk.referenced_table))
                                            .clicked()
                                        {
                                            trigger_fk_lookup = Some((
                                                fk_key,
                                                fk.referenced_table.clone(),
                                                fk.referenced_column.clone(),
                                            ));
                                        }
                                    }
                                });

                                if let Some(buf) = tab.row_inspector_buffers.get_mut(c_idx) {
                                    if can_edit && !is_pk {
                                        ui.add(
                                            egui::TextEdit::singleline(buf)
                                                .desired_width(f32::INFINITY),
                                        );
                                    } else {
                                        ui.add_enabled(
                                            false,
                                            egui::TextEdit::singleline(buf)
                                                .desired_width(f32::INFINITY),
                                        );
                                    }
                                }
                                ui.add_space(4.0);
                            }
                        });

                    if can_edit {
                        ui.separator();
                        if ui.button("Stage Edits").clicked() {
                            if let (Some(pk_idx), Some(pk_name), Some(row_cells)) = (
                                pk_col_idx,
                                &result.primary_key_column,
                                result.rows.get(cur_row_idx),
                            ) {
                                let pk_val = row_cells[pk_idx].display_text();
                                for (c_idx, col_name) in result.columns.iter().enumerate() {
                                    if c_idx == pk_idx {
                                        continue;
                                    }
                                    if let Some(new_val) = tab.row_inspector_buffers.get(c_idx) {
                                        let old_val = row_cells
                                            .get(c_idx)
                                            .map(|c| c.display_text())
                                            .unwrap_or_default();
                                        if new_val != &old_val {
                                            tab.pending_edits.push(PendingCellEdit {
                                                row_idx: cur_row_idx,
                                                col_idx: c_idx,
                                                column_name: col_name.clone(),
                                                pk_column: pk_name.clone(),
                                                pk_value: pk_val.clone(),
                                                new_value: new_val.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
        }

        // Render Virtualized Data Grid
        let num_cols = result.columns.len().max(1);
        let mut builder = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

        builder = builder.column(Column::exact(28.0)); // Selection checkbox column
        for _ in 0..num_cols {
            builder = builder.column(Column::initial(135.0).at_least(65.0).clip(true));
        }

        builder
            .header(24.0, |mut header| {
                header.col(|ui| {
                    let all_selected = !page_indices.is_empty()
                        && page_indices.iter().all(|r| tab.selected_rows.contains(r));
                    let mut checked = all_selected;
                    if ui
                        .checkbox(&mut checked, "")
                        .on_hover_text("Select/Deselect all rows on this page")
                        .changed()
                    {
                        for &r in &page_indices {
                            if checked {
                                tab.selected_rows.insert(r);
                            } else {
                                tab.selected_rows.remove(&r);
                            }
                        }
                    }
                });
                for (col_idx, col_name) in result.columns.iter().enumerate() {
                    header.col(|ui| {
                        let arrow = match tab.sort_column {
                            Some((c, true)) if c == col_idx => " ▲",
                            Some((c, false)) if c == col_idx => " ▼",
                            _ => "",
                        };
                        let hdr_resp = ui
                            .button(RichText::new(format!("{col_name}{arrow}")).strong())
                            .on_hover_text("Click to sort | Right-click to copy column as IN (...) or JS Array");
                        if hdr_resp.clicked() {
                            let next_asc = match tab.sort_column {
                                Some((c, asc)) if c == col_idx => !asc,
                                _ => true,
                            };
                            tab.sort_column = Some((col_idx, next_asc));
                        }
                        hdr_resp.context_menu(|ui| {
                            if ui.button("📋 Copy Column as SQL IN (...)").clicked() {
                                let in_clause = format_column_as_sql_in(&result, col_idx, &filtered_indices);
                                ui.ctx().copy_text(in_clause);
                                copy_banner_msg = Some(format!("Copied `{col_name}` as SQL IN (...) clause."));
                                ui.close();
                            }
                            if ui.button("📋 Copy Column as JS/TS Array").clicked() {
                                let js_arr = format_column_as_js_array(&result, col_idx, &filtered_indices);
                                ui.ctx().copy_text(js_arr);
                                copy_banner_msg = Some(format!("Copied `{col_name}` as JS/TS array."));
                                ui.close();
                            }
                        });
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, page_indices.len(), |mut row| {
                    let actual_row_idx = page_indices[row.index()];
                    let row_cells = &result.rows[actual_row_idx];
                    let is_marked_delete = tab
                        .pending_row_deletes
                        .iter()
                        .any(|(idx, _, _)| *idx == actual_row_idx);

                    row.col(|ui| {
                        let mut sel = tab.selected_rows.contains(&actual_row_idx);
                        if ui.checkbox(&mut sel, "").changed() {
                            if sel {
                                tab.selected_rows.insert(actual_row_idx);
                            } else {
                                tab.selected_rows.remove(&actual_row_idx);
                            }
                        }
                    });

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
                                if label_resp.double_clicked() {
                                    if pk_col_idx.is_some() {
                                        tab.editing_cell = Some((
                                            actual_row_idx,
                                            col_idx,
                                            cell.display_text(),
                                        ));
                                    } else {
                                        let raw_val = cell.display_text();
                                        let formatted_json = serde_json::from_str::<serde_json::Value>(&raw_val)
                                            .ok()
                                            .and_then(|v| serde_json::to_string_pretty(&v).ok());
                                        cell_inspector_to_open = Some(CellInspectorState {
                                            table_name: result.editable_table.clone(),
                                            column_name: result.columns[col_idx].clone(),
                                            row_index: actual_row_idx,
                                            raw_value: raw_val,
                                            formatted_json,
                                        });
                                    }
                                }

                                label_resp.context_menu(|ui| {
                                    if ui.button("🔍 Inspect Cell / JSON").clicked() {
                                        let raw_val = cell.display_text();
                                        let formatted_json = serde_json::from_str::<serde_json::Value>(&raw_val)
                                            .ok()
                                            .and_then(|v| serde_json::to_string_pretty(&v).ok());
                                        cell_inspector_to_open = Some(CellInspectorState {
                                            table_name: result.editable_table.clone(),
                                            column_name: result.columns[col_idx].clone(),
                                            row_index: actual_row_idx,
                                            raw_value: raw_val,
                                            formatted_json,
                                        });
                                        ui.close();
                                    }
                                    if ui.button("📋 Copy Value").clicked() {
                                        ui.ctx().copy_text(cell.display_text());
                                        ui.close();
                                    }
                                    if ui.button("Inspect Row (Vertical)").clicked() {
                                        tab.show_row_inspector = true;
                                        tab.row_inspector_row = Some(actual_row_idx);
                                        tab.row_inspector_buffers = row_cells
                                            .iter()
                                            .map(|c| c.display_text())
                                            .collect();
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Copy Row as JSON").clicked() {
                                        ui.ctx().copy_text(format_rows_as_json(&result, &[actual_row_idx]));
                                        copy_banner_msg = Some("Copied row as JSON.".to_string());
                                        ui.close();
                                    }
                                    if ui.button("Copy Row as SQL INSERT").clicked() {
                                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                        ui.ctx().copy_text(format_rows_as_sql_insert(tbl, &result, &[actual_row_idx]));
                                        copy_banner_msg = Some("Copied row as SQL INSERT.".to_string());
                                        ui.close();
                                    }
                                    if let Some(pk_col) = &result.primary_key_column {
                                        if ui.button("Copy Row as SQL UPDATE").clicked() {
                                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                            ui.ctx().copy_text(format_rows_as_sql_update(tbl, pk_col, &result, &[actual_row_idx]));
                                            copy_banner_msg = Some("Copied row as SQL UPDATE.".to_string());
                                            ui.close();
                                        }
                                    }
                                    if ui.button("Duplicate Row (SQL INSERT)").clicked() {
                                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                        let sql = format_rows_as_sql_insert(tbl, &result, &[actual_row_idx]);
                                        duplicate_insert_tab = Some((format!("Duplicate {tbl}"), sql));
                                        ui.close();
                                    }
                                    if let (Some(_tbl), Some(pk_col), Some(pk_idx)) = (
                                        &result.editable_table,
                                        &result.primary_key_column,
                                        pk_col_idx,
                                    ) {
                                        let delete_lbl = if is_marked_delete {
                                            "Unmark Row Delete"
                                        } else {
                                            "Mark Row for Delete"
                                        };
                                        if ui.button(delete_lbl).clicked() {
                                            if is_marked_delete {
                                                tab.pending_row_deletes
                                                    .retain(|(idx, _, _)| *idx != actual_row_idx);
                                            } else {
                                                let pk_val = row_cells[pk_idx].display_text();
                                                tab.pending_row_deletes.push((
                                                    actual_row_idx,
                                                    pk_col.clone(),
                                                    pk_val,
                                                ));
                                            }
                                            ui.close();
                                        }
                                    }

                                    // Browse Related FK Rows
                                    if !cell.is_null() {
                                        if let (Some(tbl_name), Some(schema)) = (&result.editable_table, &active_schema_opt) {
                                            let col_name = &result.columns[col_idx];
                                            let raw_val = cell.display_text();
                                            let formatted_val = if raw_val.parse::<i64>().is_ok() || raw_val.parse::<f64>().is_ok() {
                                                raw_val.clone()
                                            } else {
                                                format!("'{}'", raw_val.replace('\'', "''"))
                                            };

                                            // Outgoing FKs
                                            if let Some(tbl_schema) = schema.tables.iter().find(|t| t.name.eq_ignore_ascii_case(tbl_name)) {
                                                for fk in &tbl_schema.foreign_keys {
                                                    if fk.column_name.eq_ignore_ascii_case(col_name) {
                                                        let lbl = format!("🔗 Browse `{}` ({} = {})", fk.referenced_table, fk.referenced_column, raw_val);
                                                        if ui.button(lbl).clicked() {
                                                            let sql = format!("SELECT * FROM `{}` WHERE `{}` = {} LIMIT 100;", fk.referenced_table, fk.referenced_column, formatted_val);
                                                            open_related_query = Some((format!("{} ({})", fk.referenced_table, raw_val), sql));
                                                            ui.close();
                                                        }
                                                    }
                                                }
                                            }
                                            // Incoming FKs from other tables referencing this table/column
                                            for other_tbl in &schema.tables {
                                                for fk in &other_tbl.foreign_keys {
                                                    if fk.referenced_table.eq_ignore_ascii_case(tbl_name) && fk.referenced_column.eq_ignore_ascii_case(col_name) {
                                                        let lbl = format!("🔗 Browse `{}` ({} = {})", other_tbl.name, fk.column_name, raw_val);
                                                        if ui.button(lbl).clicked() {
                                                            let sql = format!("SELECT * FROM `{}` WHERE `{}` = {} LIMIT 100;", other_tbl.name, fk.column_name, formatted_val);
                                                            open_related_query = Some((format!("{} ({})", other_tbl.name, raw_val), sql));
                                                            ui.close();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
            });

            pending_edits = tab.pending_edits.clone();
            pending_deletes = tab.pending_row_deletes.clone();
        }

        if let Some(msg) = copy_banner_msg {
            self.status_banner = msg;
        }
        if let Some((title, sql)) = duplicate_insert_tab {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
        }
        if let Some((fk_key, ref_table, ref_col)) = trigger_fk_lookup {
            if let Some(session) = self.active_session.clone() {
                let db = self.active_database.clone();
                let tx = self.tx.clone();
                let sql = format!(
                    "SELECT * FROM `{ref_table}` ORDER BY `{ref_col}` LIMIT 100;"
                );
                self.rt.spawn(async move {
                    let res = session
                        .execute_query(&db, &sql, None)
                        .await
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(AsyncEvent::FkLookupLoaded {
                        key: fk_key,
                        result: res,
                    });
                });
            }
        }
        if trigger_commit_deletes {
            if let (Some(session), Some(table_name)) =
                (self.active_session.clone(), result.editable_table.clone())
            {
                let tab_id = self.tabs[self.active_tab_idx].id.clone();
                let db = self.active_database.clone();
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let mut total_deleted = 0u64;
                    let mut err_out: Option<String> = None;
                    for (_r_idx, pk_col, pk_val) in &pending_deletes {
                        let lit = if pk_val.parse::<i64>().is_ok() || pk_val.parse::<f64>().is_ok() {
                            pk_val.clone()
                        } else {
                            format!("'{}'", pk_val.replace('\'', "''"))
                        };
                        let sql = format!("DELETE FROM `{table_name}` WHERE `{pk_col}` = {lit} LIMIT 1;");
                        match session.execute_query(&db, &sql, None).await {
                            Ok(r) => total_deleted += r.affected_rows.max(1),
                            Err(e) => {
                                err_out = Some(format!("{e:#}"));
                                break;
                            }
                        }
                    }
                    let _ = tx.send(AsyncEvent::RowDeletesApplied {
                        tab_id,
                        deleted_count: match err_out {
                            Some(e) => Err(e),
                            None => Ok(total_deleted),
                        },
                    });
                });
            }
        }
        if let Some((title, sql)) = open_related_query {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(None);
        }
        if let Some(ci) = cell_inspector_to_open {
            self.cell_inspector = Some(ci);
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

    fn render_multi_results_split_view(ui: &mut Ui, tab: &mut QueryTab) {
        let mut focus_idx: Option<usize> = None;
        let results_snapshot = tab.multi_results.clone();
        let layout = tab.multi_result_layout;
        let default_height = match layout {
            MultiResultLayout::Stacked => 220.0,
            MultiResultLayout::SideBySide => 260.0,
            MultiResultLayout::Tabs => 240.0,
        };

        egui::ScrollArea::vertical()
            .id_salt("multi_results_split_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match layout {
                MultiResultLayout::Stacked => {
                    for (idx, res) in results_snapshot.iter().enumerate() {
                        let height = tab
                            .multi_result_heights
                            .entry(idx)
                            .or_insert(default_height);
                        let collapsed = tab
                            .multi_result_collapsed
                            .entry(idx)
                            .or_insert(false);
                        if let Some(f) =
                            Self::render_compact_result_card(ui, idx, res, height, collapsed)
                        {
                            focus_idx = Some(f);
                        }
                        ui.add_space(6.0);
                    }
                }
                MultiResultLayout::SideBySide => {
                    let cols_per_row = tab.multi_result_columns.unwrap_or_else(|| {
                        if results_snapshot.len() >= 3 && ui.available_width() >= 1100.0 {
                            3
                        } else {
                            2
                        }
                    }).max(1);
                    for chunk_start in (0..results_snapshot.len()).step_by(cols_per_row) {
                        let chunk_end = (chunk_start + cols_per_row).min(results_snapshot.len());
                        let count = chunk_end - chunk_start;
                        ui.columns(count, |columns| {
                            for offset in 0..count {
                                let idx = chunk_start + offset;
                                let res = &results_snapshot[idx];
                                let height = tab
                                    .multi_result_heights
                                    .entry(idx)
                                    .or_insert(default_height);
                                let collapsed = tab
                                    .multi_result_collapsed
                                    .entry(idx)
                                    .or_insert(false);
                                if let Some(f) = Self::render_compact_result_card(
                                    &mut columns[offset],
                                    idx,
                                    res,
                                    height,
                                    collapsed,
                                ) {
                                    focus_idx = Some(f);
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                MultiResultLayout::Tabs => {}
            });

        if let Some(idx) = focus_idx {
            if let Some(res) = tab.multi_results.get(idx).cloned() {
                tab.active_result_idx = idx;
                tab.last_result = Some(res);
                tab.current_page = 0;
                tab.sort_column = None;
                tab.multi_result_layout = MultiResultLayout::Tabs;
            }
        }
    }

    fn render_compact_result_card(
        ui: &mut Ui,
        idx: usize,
        res: &QueryResult,
        max_table_height: &mut f32,
        collapsed: &mut bool,
    ) -> Option<usize> {
        let mut focus_clicked = false;
        let frame = egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(15, 22, 34))
            .stroke(Stroke::new(1.0, Color32::from_rgb(38, 50, 72)))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8));

        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let toggle_icon = if *collapsed { "▸" } else { "▾" };
                if ui
                    .small_button(toggle_icon)
                    .on_hover_text(if *collapsed { "Expand result" } else { "Collapse result" })
                    .clicked()
                {
                    *collapsed = !*collapsed;
                }
                ui.label(
                    RichText::new(format!("Result #{}", idx + 1))
                        .strong()
                        .color(Color32::from_rgb(56, 189, 248)),
                );
                ui.label(
                    RichText::new(format!("{} rows • {} ms", res.rows.len(), res.duration_ms))
                        .small()
                        .color(Color32::from_rgb(148, 163, 184)),
                );
                let sql_preview: String = res.sql.split_whitespace().collect::<Vec<_>>().join(" ");
                let short_sql = if sql_preview.len() > 36 {
                    format!("{}...", &sql_preview[..36])
                } else {
                    sql_preview.clone()
                };
                if !short_sql.is_empty() {
                    ui.label(RichText::new(short_sql).small().monospace().weak())
                        .on_hover_text(&res.sql);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("🔍 Tab").on_hover_text("Focus this result in single-tab view").clicked() {
                        focus_clicked = true;
                    }
                    if !*collapsed {
                        let expand_label = if *max_table_height >= 420.0 { "Compact" } else { "Expand" };
                        if ui
                            .small_button(expand_label)
                            .on_hover_text("Toggle taller / compact height (or drag bottom edge)")
                            .clicked()
                        {
                            *max_table_height = if *max_table_height >= 420.0 {
                                240.0
                            } else {
                                500.0
                            };
                        }
                    }
                    if ui.small_button("📋 CSV").on_hover_text("Copy this result as CSV").clicked() {
                        if let Ok(csv_text) = to_csv_string(res) {
                            ui.ctx().copy_text(csv_text);
                        }
                    }
                });
            });

            if *collapsed {
                return;
            }

            ui.separator();

            if res.columns.is_empty() {
                ui.label(RichText::new(&res.status_message).small().color(Color32::from_rgb(52, 211, 153)));
                return;
            }

            let num_cols = res.columns.len();
            let col_width = (ui.available_width() / num_cols.max(1) as f32).clamp(90.0, 220.0);

            ui.push_id(("multi_res_card", idx), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt(("multi_res_hscroll", idx))
                    .show(ui, |ui| {
                        let table = TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                            .max_scroll_height(*max_table_height)
                            .columns(Column::initial(col_width).at_least(60.0), num_cols);

                        table
                            .header(22.0, |mut header| {
                                for col_name in &res.columns {
                                    header.col(|ui| {
                                        ui.label(
                                            RichText::new(col_name)
                                                .small()
                                                .strong()
                                                .color(Color32::from_rgb(203, 213, 225)),
                                        );
                                    });
                                }
                            })
                            .body(|body| {
                                body.rows(20.0, res.rows.len(), |mut row| {
                                    let r_idx = row.index();
                                    let row_data = &res.rows[r_idx];
                                    for c_idx in 0..num_cols {
                                        row.col(|ui| {
                                            let txt = row_data
                                                .get(c_idx)
                                                .map(|c| c.display_text())
                                                .unwrap_or_default();
                                            let is_null = row_data
                                                .get(c_idx)
                                                .map(|c| matches!(c, crate::db::CellValue::Null))
                                                .unwrap_or(false);
                                            let rich = if is_null {
                                                RichText::new("NULL").small().italics().color(Color32::GRAY)
                                            } else {
                                                RichText::new(&txt).small()
                                            };
                                            if ui.add(egui::Label::new(rich).truncate().sense(egui::Sense::click()))
                                                .on_hover_text("Click to copy value")
                                                .clicked()
                                            {
                                                ui.ctx().copy_text(txt);
                                            }
                                        });
                                    }
                                });
                            });
                    });
            });

            // Vertical resize drag handle at the bottom edge of the card
            let (handle_rect, handle_resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 7.0),
                egui::Sense::click_and_drag(),
            );
            let line_color = if handle_resp.hovered() || handle_resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                Color32::from_rgb(56, 189, 248)
            } else {
                Color32::from_rgb(51, 65, 85)
            };
            let center_y = handle_rect.center().y;
            let bar_half_w = 24.0_f32.min(handle_rect.width() * 0.25);
            ui.painter().line_segment(
                [
                    egui::pos2(handle_rect.center().x - bar_half_w, center_y),
                    egui::pos2(handle_rect.center().x + bar_half_w, center_y),
                ],
                Stroke::new(2.0, line_color),
            );
            if handle_resp.dragged() {
                *max_table_height = (*max_table_height + handle_resp.drag_delta().y).clamp(80.0, 900.0);
            }
        });

        if focus_clicked {
            Some(idx)
        } else {
            None
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

                // Index Advisor Recommendation
                let mut advise_index_actions: Vec<(String, String)> = Vec::new();
                for row in &res.rows {
                    let table_name = col_table_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let access_type = col_type_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let key_used = col_key_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let extra = col_extra_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();

                    if !table_name.is_empty()
                        && (access_type.eq_ignore_ascii_case("ALL")
                            || key_used == "NULL"
                            || extra.contains("Using filesort"))
                    {
                        let recommended_cols = if let Some(schema) =
                            self.schemas.get(&self.active_database)
                        {
                            if let Some(tbl) = schema
                                .tables
                                .iter()
                                .find(|t| t.name.eq_ignore_ascii_case(&table_name))
                            {
                                tbl.columns
                                    .iter()
                                    .filter(|c| !c.is_primary_key)
                                    .take(2)
                                    .map(|c| c.name.clone())
                                    .collect::<Vec<_>>()
                            } else {
                                vec!["created_at".to_string()]
                            }
                        } else {
                            vec!["status".to_string()]
                        };
                        let col_str = recommended_cols.join(", ");
                        let first_col = recommended_cols
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "lookup".into());
                        let idx_name = format!("idx_{}_{}", table_name, first_col);
                        let ddl =
                            format!("CREATE INDEX `{idx_name}` ON `{table_name}` ({col_str});");
                        if !advise_index_actions.iter().any(|(t, _)| t == &table_name) {
                            advise_index_actions.push((table_name, ddl));
                        }
                    }
                }

                if !advise_index_actions.is_empty() {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(30, 27, 75))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(129, 140, 248)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("💡 Index Advisor Recommendations")
                                        .strong()
                                        .color(Color32::from_rgb(165, 180, 252)),
                                );
                            });
                            ui.add_space(4.0);
                            for (tbl, ddl) in advise_index_actions {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("• For `{tbl}`:")).strong().small(),
                                    );
                                    ui.monospace(&ddl);
                                    if ui.small_button("📋 Copy").clicked() {
                                        ui.ctx().copy_text(ddl.clone());
                                        self.status_banner =
                                            format!("Copied index DDL for `{tbl}` to clipboard.");
                                    }
                                    if ui.small_button("⚡ Open in Tab").clicked() {
                                        self.tabs.push(QueryTab::new(format!("Index {tbl}"), ddl));
                                        self.active_tab_idx = self.tabs.len() - 1;
                                    }
                                });
                            }
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

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::OverviewAndMetrics,
                "📊 Metrics & Processes",
            );
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::ServerVariables,
                "⚙ Server Variables (SHOW VARIABLES)",
            );
        });
        ui.separator();

        if self.server_admin_subtab == ServerAdminSubtab::ServerVariables {
            if self.server_variables.is_empty() && !self.is_loading_variables {
                self.fetch_server_variables();
            }
            ui.horizontal(|ui| {
                ui.label("Filter variables:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_variables_filter)
                        .desired_width(220.0)
                        .hint_text("Search name or value..."),
                );
                if ui.button("Reload").clicked() {
                    self.fetch_server_variables();
                }
                if self.is_loading_variables {
                    ui.spinner();
                    ui.label("Loading variables from server...");
                }
            });
            ui.separator();

            let filter = self.server_variables_filter.to_lowercase();
            let filtered_vars: Vec<&(String, String)> = self
                .server_variables
                .iter()
                .filter(|(k, v)| {
                    filter.is_empty()
                        || k.to_lowercase().contains(&filter)
                        || v.to_lowercase().contains(&filter)
                })
                .collect();

            ui.label(
                RichText::new(format!(
                    "Showing {} of {} server variables",
                    filtered_vars.len(),
                    self.server_variables.len()
                ))
                .small()
                .weak(),
            );

            let mut copy_val: Option<String> = None;
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .column(Column::initial(240.0).at_least(140.0))
                .column(Column::remainder().at_least(180.0))
                .column(Column::exact(65.0))
                .header(22.0, |mut h| {
                    h.col(|ui| {
                        ui.strong("Variable Name");
                    });
                    h.col(|ui| {
                        ui.strong("Value");
                    });
                    h.col(|ui| {
                        ui.strong("Action");
                    });
                })
                .body(|body| {
                    body.rows(20.0, filtered_vars.len(), |mut row| {
                        let (k, v) = filtered_vars[row.index()];
                        row.col(|ui| {
                            ui.label(
                                RichText::new(k)
                                    .monospace()
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(v).monospace());
                        });
                        row.col(|ui| {
                            if ui
                                .small_button("📋")
                                .on_hover_text("Copy variable value")
                                .clicked()
                            {
                                copy_val = Some(v.clone());
                            }
                        });
                    });
                });

            if let Some(val) = copy_val {
                ui.ctx().copy_text(val);
                self.status_banner = "Copied variable value to clipboard.".to_string();
            }
            return;
        }

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
                            AiProvider::LocalSchema,
                            AiProvider::Ollama,
                            AiProvider::GitHubCopilot,
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
                    AiProvider::LocalSchema => {
                        ui.colored_label(
                            Color32::from_rgb(52, 211, 153),
                            "⚡ Option 1: Local Schema Native (0ms Latency, 100% Offline)",
                        );
                        ui.label(
                            "Runs instantly in Rust memory without network calls or cloud APIs. \
                            Generates context-aware ghost continuations for SELECT, JOIN, WHERE, \
                            ORDER BY, GROUP BY, INSERT, and UPDATE based on your database tables and foreign keys."
                        );
                        ui.small("Recommended for max typing speed, zero latency, and private offline schemas.");
                    }
                    AiProvider::Ollama => {
                        ui.colored_label(
                            Color32::from_rgb(167, 139, 250),
                            "💻 Option 2: Ollama FIM (Fast Local Code LLM)",
                        );
                        ui.label("Ollama Endpoint URL:");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_endpoint);
                        ui.label("Model Name (e.g. `qwen2.5-coder:1.5b` [recommended], `0.5b`, `7b`):");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_model);
                        ui.checkbox(
                            &mut self.config.ai.ollama_use_fim,
                            "⚡ Enable native FIM (Fill-In-The-Middle) prompt mode for code models",
                        );
                        ui.small("Tip: `qwen2.5-coder:1.5b` with FIM responds in ~80-150ms on local hardware.");
                    }
                    AiProvider::GitHubCopilot => {
                        ui.colored_label(
                            Color32::from_rgb(56, 189, 248),
                            "✨ Option 3: GitHub Copilot (Official Subscription)",
                        );

                        let is_logged_in = !self.config.ai.copilot_github_token.trim().is_empty();
                        if is_logged_in {
                            ui.horizontal(|ui| {
                                ui.colored_label(Color32::from_rgb(34, 197, 94), "✅ Logged In & Ready");
                                if ui.button("Disconnect").clicked() {
                                    self.config.ai.copilot_github_token.clear();
                                    self.config.ai.copilot_session_token.clear();
                                    self.config.ai.copilot_token_expires_at = 0;
                                    self.copilot_auth_status = "Disconnected from GitHub Copilot.".to_string();
                                }
                            });
                            ui.label("Copilot Model Engine:");
                            egui::ComboBox::from_id_salt("copilot_model_combo")
                                .selected_text(&self.config.ai.copilot_model)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "gpt-4o".to_string(), "gpt-4o (Smartest)");
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "gpt-3.5-turbo".to_string(), "gpt-3.5-turbo (Ultra-fast)");
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "claude-3.5-sonnet".to_string(), "claude-3.5-sonnet");
                                });
                        } else {
                            ui.label("Log in using your GitHub account with active Copilot subscription:");
                            if self.is_copilot_authenticating {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Connecting to GitHub...");
                                });
                            } else if let Some(dev) = &self.copilot_device_code_data {
                                ui.group(|ui| {
                                    ui.label(RichText::new("Step 1: Open verification page").strong());
                                    if ui.button("🌐 Open https://github.com/login/device in Browser").clicked() {
                                        ui.ctx().open_url(egui::OpenUrl::new_tab(&dev.verification_uri));
                                    }
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Step 2: Enter this one-time code:").strong());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(&dev.user_code)
                                                .size(20.0)
                                                .strong()
                                                .color(Color32::from_rgb(251, 191, 36)),
                                        );
                                        if ui.button("📋 Copy Code").clicked() {
                                            ui.ctx().copy_text(dev.user_code.clone());
                                            self.status_banner = "Copied device code to clipboard!".to_string();
                                        }
                                    });
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Step 3: Click once approved in browser:").strong());
                                    if ui.button("🔄 Check / Confirm Authorization").clicked() {
                                        let device_code = dev.device_code.clone();
                                        let tx = self.tx.clone();
                                        self.is_copilot_authenticating = true;
                                        self.rt.spawn(async move {
                                            let res = crate::ai::poll_copilot_device_token(&device_code).await;
                                            let outcome = match res {
                                                Ok(Ok(token)) => Ok(token),
                                                Ok(Err(err_msg)) => Err(err_msg),
                                                Err(e) => Err(format!("{e:#}")),
                                            };
                                            let _ = tx.send(AsyncEvent::CopilotPollFinished(outcome));
                                        });
                                    }
                                });
                            } else {
                                if ui.button("🔗 Connect with GitHub (Device Code)").clicked() {
                                    self.is_copilot_authenticating = true;
                                    let tx = self.tx.clone();
                                    self.rt.spawn(async move {
                                        let res = crate::ai::request_copilot_device_code()
                                            .await
                                            .map_err(|e| format!("{e:#}"));
                                        let _ = tx.send(AsyncEvent::CopilotDeviceCodeReady(res));
                                    });
                                }
                            }

                            ui.collapsing("Or paste GitHub Token manually", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.ai.copilot_github_token)
                                        .password(true)
                                        .hint_text("ghu_... or personal access token"),
                                );
                                ui.small("Accepts personal token or ghu_ OAuth token with Copilot access.");
                            });

                            if !self.copilot_auth_status.is_empty() {
                                ui.colored_label(Color32::from_rgb(234, 179, 8), &self.copilot_auth_status);
                            }
                        }
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
                        if ui.button("📄 Versioned Migration").clicked() {
                            open_in_tab_sql = Some(report.generate_migration_file_content());
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

    fn render_cell_inspector_modal(&mut self, ctx: &egui::Context) {
        let Some(inspector) = &self.cell_inspector else {
            return;
        };
        let mut close_modal = false;
        let mut copy_val: Option<String> = None;

        egui::Window::new("🔍 Cell Value & JSON Inspector")
            .collapsible(false)
            .resizable(true)
            .default_size([580.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Column: `{}`", inspector.column_name))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    if let Some(tbl) = &inspector.table_name {
                        ui.label(RichText::new(format!("Table: `{tbl}`")).weak());
                    }
                    ui.label(RichText::new(format!("Row: #{}", inspector.row_index + 1)).weak());
                    if inspector.formatted_json.is_some() {
                        ui.label(
                            RichText::new("[VALID JSON]")
                                .color(Color32::from_rgb(34, 197, 94))
                                .strong()
                                .small(),
                        );
                    }
                });
                ui.separator();

                let is_json = inspector.formatted_json.is_some();
                let display_text = if let Some(pretty) = &inspector.formatted_json {
                    pretty
                } else {
                    &inspector.raw_value
                };

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Length: {} chars ({} bytes){}",
                            display_text.len(),
                            display_text.as_bytes().len(),
                            if is_json { " (Prettified JSON)" } else { "" }
                        ))
                        .small()
                        .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("📋 Copy Value").clicked() {
                            copy_val = Some(display_text.clone());
                        }
                    });
                });
                ui.add_space(4.0);

                egui::ScrollArea::both()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut display_text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close_modal = true;
                    }
                });
            });

        if let Some(val) = copy_val {
            ctx.copy_text(val);
            self.status_banner = "Copied cell content to clipboard.".to_string();
        }
        if close_modal {
            self.cell_inspector = None;
        }
    }

    fn render_csv_import_modal(&mut self, ctx: &egui::Context) {
        let Some(wizard) = &mut self.csv_wizard else {
            return;
        };
        let mut close_modal = false;
        let mut trigger_import = false;

        egui::Window::new("📥 CSV Import Wizard")
            .collapsible(false)
            .resizable(true)
            .default_size([650.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("Import Data from CSV")
                            .color(Color32::from_rgb(56, 189, 248))
                            .size(16.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close_modal = true;
                        }
                    });
                });
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("File: `{}`", wizard.file_name)).strong());
                    ui.label(format!("({} total rows detected)", wizard.total_rows));
                });
                ui.add_space(6.0);

                // Target table configuration
                ui.horizontal(|ui| {
                    ui.radio_value(&mut wizard.create_new_table, false, "Existing Table:");
                    let current_target = &mut wizard.target_table;
                    let tables = self
                        .schemas
                        .get(&self.active_database)
                        .map(|s| s.tables.clone())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("csv_target_table_combo")
                        .selected_text(if current_target.is_empty() {
                            "Select a table..."
                        } else {
                            current_target.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for tbl in &tables {
                                ui.selectable_value(current_target, tbl.name.clone(), &tbl.name);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut wizard.create_new_table, true, "Create New Table:");
                    ui.add(egui::TextEdit::singleline(&mut wizard.new_table_name).hint_text("table_name"));
                });

                ui.separator();
                ui.label(RichText::new("Column Mapping:").strong());
                egui::ScrollArea::vertical()
                    .id_salt("csv_mapping_scroll")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (csv_col, db_col) in wizard.column_mappings.iter_mut() {
                            ui.horizontal(|ui| {
                                ui.set_min_width(180.0);
                                ui.label(RichText::new(format!("CSV `{csv_col}`")).monospace());
                                ui.label("➜");
                                ui.add(
                                    egui::TextEdit::singleline(db_col)
                                        .desired_width(160.0)
                                        .hint_text("db_column_name"),
                                );
                            });
                        }
                    });

                ui.separator();
                ui.label(RichText::new("Data Preview (First 5 Rows):").strong());
                egui::ScrollArea::horizontal()
                    .id_salt("csv_preview_scroll")
                    .max_height(100.0)
                    .show(ui, |ui| {
                        let num_cols = wizard.headers.len().max(1);
                        let mut builder = TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                        for _ in 0..num_cols {
                            builder = builder.column(Column::initial(100.0).at_least(60.0));
                        }
                        builder
                            .header(20.0, |mut h| {
                                for head in &wizard.headers {
                                    h.col(|ui| {
                                        ui.strong(head);
                                    });
                                }
                            })
                            .body(|mut b| {
                                for row in &wizard.sample_rows {
                                    b.row(18.0, |mut r| {
                                        for cell in row {
                                            r.col(|ui| {
                                                ui.label(cell);
                                            });
                                        }
                                    });
                                }
                            });
                    });

                if let Some(err) = &wizard.error_message {
                    ui.colored_label(Color32::from_rgb(239, 68, 68), err);
                }
                if let Some(msg) = &wizard.status_message {
                    ui.colored_label(Color32::from_rgb(34, 197, 94), msg);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let target_name = if wizard.create_new_table {
                        wizard.new_table_name.trim()
                    } else {
                        wizard.target_table.trim()
                    };
                    let can_import = !target_name.is_empty() && !wizard.is_importing;

                    let btn = egui::Button::new(
                        RichText::new("🚀 Import Now")
                            .strong()
                            .color(Color32::from_rgb(16, 185, 129)),
                    )
                    .fill(Color32::from_rgb(6, 78, 59));
                    if ui.add_enabled(can_import, btn).clicked() {
                        trigger_import = true;
                    }

                    if ui.button("Cancel").clicked() {
                        close_modal = true;
                    }
                });
            });

        if trigger_import {
            self.execute_csv_import();
        }
        if close_modal {
            self.csv_wizard = None;
        }
    }

    fn render_command_palette_modal(&mut self, ctx: &egui::Context) {
        if !self.show_command_palette {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_command_palette = false;
            return;
        }

        #[derive(Clone)]
        enum PaletteAction {
            NewTab,
            RunQuery,
            ExplainQuery,
            FormatSql,
            OpenCsvImport,
            NewTableDesigner,
            OpenSchemaDiff,
            OpenServerAdmin,
            StartTransaction,
            ToggleAiPanel,
            OpenTable(String),
            OpenSnippet { title: String, sql: String },
        }

        struct PaletteEntry {
            category: &'static str,
            label: String,
            detail: String,
            action: PaletteAction,
        }

        let mut entries = vec![
            PaletteEntry {
                category: "Action",
                label: "New Query Tab".to_string(),
                detail: "Open a blank SQL scratchpad tab".to_string(),
                action: PaletteAction::NewTab,
            },
            PaletteEntry {
                category: "Action",
                label: "Run Query".to_string(),
                detail: "Execute SQL in current tab (Ctrl+Enter)".to_string(),
                action: PaletteAction::RunQuery,
            },
            PaletteEntry {
                category: "Action",
                label: "Explain Query".to_string(),
                detail: "Run EXPLAIN on current query".to_string(),
                action: PaletteAction::ExplainQuery,
            },
            PaletteEntry {
                category: "Action",
                label: "Format SQL".to_string(),
                detail: "Pretty-print SQL in current tab".to_string(),
                action: PaletteAction::FormatSql,
            },
            PaletteEntry {
                category: "Action",
                label: "Import CSV...".to_string(),
                detail: "Open CSV Import Wizard".to_string(),
                action: PaletteAction::OpenCsvImport,
            },
            PaletteEntry {
                category: "Action",
                label: "New Table Designer".to_string(),
                detail: "Create a new table visually".to_string(),
                action: PaletteAction::NewTableDesigner,
            },
            PaletteEntry {
                category: "Action",
                label: "Schema Diff".to_string(),
                detail: "Compare schemas between databases".to_string(),
                action: PaletteAction::OpenSchemaDiff,
            },
            PaletteEntry {
                category: "Action",
                label: "Server Admin & Variables".to_string(),
                detail: "Inspect SHOW VARIABLES and status".to_string(),
                action: PaletteAction::OpenServerAdmin,
            },
            PaletteEntry {
                category: "Action",
                label: "Start Transaction".to_string(),
                detail: "BEGIN a manual transaction".to_string(),
                action: PaletteAction::StartTransaction,
            },
            PaletteEntry {
                category: "Action",
                label: "Toggle AI Assistant".to_string(),
                detail: "Show or hide AI SQL assistant panel".to_string(),
                action: PaletteAction::ToggleAiPanel,
            },
        ];

        if let Some(schema) = self.schemas.get(&self.active_database) {
            for t in &schema.tables {
                entries.push(PaletteEntry {
                    category: "Table",
                    label: t.name.clone(),
                    detail: format!("SELECT * FROM `{}` (~{} rows)", t.name, t.approx_rows),
                    action: PaletteAction::OpenTable(t.name.clone()),
                });
            }
        }

        for snip in &self.config.snippets {
            entries.push(PaletteEntry {
                category: "Snippet",
                label: snip.title.clone(),
                detail: snip.sql.lines().next().unwrap_or("").to_string(),
                action: PaletteAction::OpenSnippet {
                    title: snip.title.clone(),
                    sql: snip.sql.clone(),
                },
            });
        }

        let query_lower = self.command_palette_query.trim().to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();
        let filtered: Vec<PaletteEntry> = entries
            .into_iter()
            .filter(|e| {
                if terms.is_empty() {
                    return true;
                }
                let hay = format!("{} {} {}", e.category, e.label, e.detail).to_lowercase();
                terms.iter().all(|t| hay.contains(t))
            })
            .take(50)
            .collect();

        if filtered.is_empty() {
            self.command_palette_selected = 0;
        } else if self.command_palette_selected >= filtered.len() {
            self.command_palette_selected = filtered.len() - 1;
        }

        let (down_pressed, up_pressed, enter_pressed) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });

        if down_pressed && !filtered.is_empty() {
            self.command_palette_selected = (self.command_palette_selected + 1) % filtered.len();
        }
        if up_pressed && !filtered.is_empty() {
            self.command_palette_selected = if self.command_palette_selected == 0 {
                filtered.len() - 1
            } else {
                self.command_palette_selected - 1
            };
        }

        let mut chosen_action: Option<PaletteAction> = if enter_pressed && !filtered.is_empty() {
            Some(filtered[self.command_palette_selected].action.clone())
        } else {
            None
        };

        let mut open = self.show_command_palette;
        egui::Window::new("Command Palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .default_width(540.0)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.command_palette_query)
                        .hint_text("Type a command, table, or saved snippet... (Esc to close)")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                if resp.changed() {
                    self.command_palette_selected = 0;
                }

                ui.add_space(6.0);
                ui.separator();

                if filtered.is_empty() {
                    ui.label(
                        RichText::new("No matching commands, tables, or snippets.")
                            .color(Color32::GRAY)
                            .italics(),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (idx, item) in filtered.iter().enumerate() {
                                let is_selected = idx == self.command_palette_selected;
                                let bg = if is_selected {
                                    Color32::from_rgb(30, 58, 138)
                                } else {
                                    Color32::TRANSPARENT
                                };
                                let frame = egui::Frame::NONE
                                    .fill(bg)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 5));

                                let row_resp = frame
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let badge_color = match item.category {
                                                "Table" => Color32::from_rgb(56, 189, 248),
                                                "Snippet" => Color32::from_rgb(250, 204, 21),
                                                _ => Color32::from_rgb(52, 211, 153),
                                            };
                                            ui.label(
                                                RichText::new(format!("[{}]", item.category))
                                                    .small()
                                                    .strong()
                                                    .color(badge_color),
                                            );
                                            ui.label(RichText::new(&item.label).strong());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(&item.detail)
                                                            .small()
                                                            .color(Color32::from_rgb(148, 163, 184)),
                                                    );
                                                },
                                            );
                                        });
                                    })
                                    .response;

                                if row_resp.interact(egui::Sense::click()).clicked() {
                                    chosen_action = Some(item.action.clone());
                                }
                            }
                        });
                }
            });

        self.show_command_palette = open;

        if let Some(action) = chosen_action {
            self.show_command_palette = false;
            match action {
                PaletteAction::NewTab => {
                    let n = self.tabs.len() + 1;
                    self.tabs
                        .push(QueryTab::new(format!("Query {}", n), String::new()));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
                PaletteAction::RunQuery => {
                    self.execute_current_tab_query(None);
                }
                PaletteAction::ExplainQuery => {
                    self.execute_current_tab_query(Some("EXPLAIN"));
                }
                PaletteAction::FormatSql => {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.sql = format_sql_pretty(&tab.sql);
                    }
                }
                PaletteAction::OpenCsvImport => {
                    self.open_csv_import_dialog();
                }
                PaletteAction::NewTableDesigner => {
                    self.table_designer_draft = Some(TableDraft::new_empty("new_table"));
                }
                PaletteAction::OpenSchemaDiff => {
                    self.show_schema_diff_modal = true;
                }
                PaletteAction::OpenServerAdmin => {
                    self.bottom_tab = BottomTab::ServerAdmin;
                }
                PaletteAction::StartTransaction => {
                    self.start_transaction();
                }
                PaletteAction::ToggleAiPanel => {
                    self.show_ai_panel = !self.show_ai_panel;
                }
                PaletteAction::OpenTable(table_name) => {
                    let sql = format!("SELECT * FROM `{}` LIMIT 100;", table_name);
                    self.tabs.push(QueryTab::new(table_name, sql));
                    self.active_tab_idx = self.tabs.len() - 1;
                    self.execute_current_tab_query(None);
                }
                PaletteAction::OpenSnippet { title, sql } => {
                    self.tabs.push(QueryTab::new(title, sql));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
            }
        }
    }

    fn render_delete_connection_confirm_modal(&mut self, ctx: &egui::Context) {
        let Some((conn_id, conn_name)) = self.pending_delete_connection.clone() else {
            return;
        };

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_delete_connection = None;
            return;
        }

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("⚠ Move Connection to Recycle Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Move connection \"{conn_name}\" to the Recycle Bin?"
                    ))
                    .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("You can restore it later from the Recycle Bin.")
                        .small()
                        .color(Color32::from_rgb(148, 163, 184)),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let del_btn = egui::Button::new(
                        RichText::new("Move to Trash").strong().color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(185, 28, 28));
                    if ui.add(del_btn).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            if let Some(pos) = self.config.connections.iter().position(|c| c.id == conn_id) {
                let profile = self.config.connections.remove(pos);
                self.config.trashed_connections.insert(
                    0,
                    TrashedConnection {
                        profile,
                        deleted_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    },
                );
                let _ = self.config.save();
                self.status_banner = format!("Moved connection '{conn_name}' to Recycle Bin.");
            }
            self.pending_delete_connection = None;
        } else if !open || cancelled {
            self.pending_delete_connection = None;
        }
    }

    fn render_connection_trash_modal(&mut self, ctx: &egui::Context) {
        if !self.show_connection_trash_modal {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_connection_trash_modal = false;
            return;
        }

        let mut open = self.show_connection_trash_modal;
        let mut restore_idx: Option<usize> = None;
        let mut purge_idx: Option<usize> = None;
        let mut empty_all = false;

        egui::Window::new("Connection Recycle Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                if self.config.trashed_connections.is_empty() {
                    ui.label(
                        RichText::new("Recycle Bin is empty.")
                            .italics()
                            .color(Color32::GRAY),
                    );
                    return;
                }

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} deleted connection(s)",
                            self.config.trashed_connections.len()
                        ))
                        .small()
                        .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let empty_btn = egui::Button::new(
                            RichText::new("Empty Trash").small().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(153, 27, 27));
                        if ui.add(empty_btn).clicked() {
                            empty_all = true;
                        }
                    });
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (idx, item) in self.config.trashed_connections.iter().enumerate() {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(17, 24, 39))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(&item.profile.name).strong());
                                            ui.label(
                                                RichText::new(format!(
                                                    "{}@{}:{}/{} • deleted {}",
                                                    item.profile.user,
                                                    item.profile.host,
                                                    item.profile.port,
                                                    item.profile.default_database,
                                                    item.deleted_at
                                                ))
                                                .small()
                                                .color(Color32::from_rgb(148, 163, 184)),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.small_button("Delete Forever").clicked() {
                                                    purge_idx = Some(idx);
                                                }
                                                let restore_btn = egui::Button::new(
                                                    RichText::new("Restore")
                                                        .small()
                                                        .strong()
                                                        .color(Color32::WHITE),
                                                )
                                                .fill(Color32::from_rgb(16, 185, 129));
                                                if ui.add(restore_btn).clicked() {
                                                    restore_idx = Some(idx);
                                                }
                                            },
                                        );
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    });
            });

        self.show_connection_trash_modal = open;

        if empty_all {
            self.config.trashed_connections.clear();
            let _ = self.config.save();
            self.status_banner = "Emptied Connection Recycle Bin.".to_string();
        } else if let Some(idx) = restore_idx {
            if idx < self.config.trashed_connections.len() {
                let item = self.config.trashed_connections.remove(idx);
                let name = item.profile.name.clone();
                self.config.connections.push(item.profile);
                let _ = self.config.save();
                self.status_banner = format!("Restored connection '{name}'.");
            }
        } else if let Some(idx) = purge_idx {
            if idx < self.config.trashed_connections.len() {
                let item = self.config.trashed_connections.remove(idx);
                let _ = self.config.save();
                self.status_banner =
                    format!("Permanently deleted connection '{}'.", item.profile.name);
            }
        }
    }

    fn render_data_diff_modal(&mut self, ctx: &egui::Context) {
        if !self.show_data_diff_modal {
            return;
        }
        let mut open = self.show_data_diff_modal;
        let mut run_diff = false;
        let mut open_sync_in_tab = false;

        egui::Window::new("Data Diff (Row Comparison)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(780.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Source:");
                    egui::ComboBox::from_id_salt("dd_src_db")
                        .selected_text(&self.data_diff_source_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.data_diff_source_db, db.clone(), db);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.data_diff_source_table)
                            .desired_width(120.0)
                            .hint_text("source table"),
                    );

                    ui.label("Target:");
                    egui::ComboBox::from_id_salt("dd_tgt_db")
                        .selected_text(&self.data_diff_target_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.data_diff_target_db, db.clone(), db);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.data_diff_target_table)
                            .desired_width(120.0)
                            .hint_text("target table"),
                    );

                    ui.label("Key Col #:");
                    ui.add(egui::DragValue::new(&mut self.data_diff_key_col).range(0..=30));

                    if ui
                        .add_enabled(!self.data_diff_loading, egui::Button::new("Compare Rows"))
                        .clicked()
                    {
                        run_diff = true;
                    }
                });

                ui.separator();

                if self.data_diff_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading and comparing table rows...");
                    });
                } else if let Some(report) = &self.data_diff_report {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Same: {} | Modified: {} | Only in Source: {} | Only in Target: {}",
                                report.same_count,
                                report.modified_count,
                                report.left_only_count,
                                report.right_only_count
                            ))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Open Sync SQL in Tab").clicked() {
                                open_sync_in_tab = true;
                            }
                            if ui.small_button("Copy Sync SQL").clicked() {
                                ui.ctx().copy_text(self.data_diff_sync_sql.clone());
                            }
                        });
                    });
                    ui.separator();

                    egui::ScrollArea::both()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for entry in &report.entries {
                                if entry.status == RowDiffStatus::Same {
                                    continue;
                                }
                                let (badge, color) = match entry.status {
                                    RowDiffStatus::OnlyInLeft => {
                                        ("+ SOURCE ONLY", Color32::from_rgb(52, 211, 153))
                                    }
                                    RowDiffStatus::OnlyInRight => {
                                        ("- TARGET ONLY", Color32::from_rgb(248, 113, 113))
                                    }
                                    RowDiffStatus::Modified => {
                                        ("~ MODIFIED", Color32::from_rgb(250, 204, 21))
                                    }
                                    RowDiffStatus::Same => ("SAME", Color32::GRAY),
                                };
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(badge).small().strong().color(color));
                                    ui.label(RichText::new(&entry.key_display).strong());
                                    if let Some(l) = &entry.left_row {
                                        ui.label(
                                            RichText::new(format!("Src: {}", l.join(" | "))).small(),
                                        );
                                    }
                                    if let Some(r) = &entry.right_row {
                                        ui.label(
                                            RichText::new(format!("Tgt: {}", r.join(" | ")))
                                                .small()
                                                .weak(),
                                        );
                                    }
                                });
                            }
                        });

                    ui.separator();
                    ui.label(RichText::new("Synchronization SQL Preview:").small().strong());
                    egui::ScrollArea::vertical()
                        .max_height(110.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.data_diff_sync_sql)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                } else {
                    ui.label(
                        RichText::new("Select source & target databases and table names, then click Compare Rows.")
                            .weak(),
                    );
                }
            });

        self.show_data_diff_modal = open;

        if open_sync_in_tab {
            self.tabs.push(QueryTab::new(
                format!("DataSync {}", self.data_diff_target_table),
                self.data_diff_sync_sql.clone(),
            ));
            self.active_tab_idx = self.tabs.len() - 1;
            self.show_data_diff_modal = false;
        }

        if run_diff && !self.data_diff_source_table.trim().is_empty() && !self.data_diff_target_table.trim().is_empty() {
            if let Some(session) = self.active_session.clone() {
                self.data_diff_loading = true;
                let src_db = self.data_diff_source_db.clone();
                let src_tbl = self.data_diff_source_table.clone();
                let tgt_db = self.data_diff_target_db.clone();
                let tgt_tbl = self.data_diff_target_table.clone();
                let key_col = self.data_diff_key_col;
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let sql_l = format!("SELECT * FROM `{src_db}`.`{src_tbl}` LIMIT 500;");
                    let sql_r = format!("SELECT * FROM `{tgt_db}`.`{tgt_tbl}` LIMIT 500;");
                    let left = session.execute_query(&src_db, &sql_l, None).await.map_err(|e| format!("{e:#}"));
                    let right = session.execute_query(&tgt_db, &sql_r, None).await.map_err(|e| format!("{e:#}"));
                    let _ = tx.send(AsyncEvent::DataDiffLoaded {
                        left,
                        right,
                        key_col_idx: key_col,
                    });
                });
            }
        }
    }

    fn render_sql_dump_modal(&mut self, ctx: &egui::Context) {
        if !self.show_sql_dump_modal {
            return;
        }
        let mut open = self.show_sql_dump_modal;
        let mut trigger_export = false;
        let mut trigger_save_file = false;
        let mut trigger_import_file = false;

        egui::Window::new("SQL Dump Export & Import")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.dump_include_drop, "Include DROP TABLE IF EXISTS");
                    ui.checkbox(&mut self.dump_include_data, "Include Data (INSERTs)");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.dump_is_busy, egui::Button::new("Import / Run .sql File..."))
                            .clicked()
                        {
                            trigger_import_file = true;
                        }
                    });
                });
                ui.separator();

                if let Some(schema) = self.schemas.get(&self.active_database) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Select Tables:").strong());
                        if ui.small_button("All").clicked() {
                            for t in &schema.tables {
                                self.dump_selected_tables.insert(t.name.clone());
                            }
                        }
                        if ui.small_button("None").clicked() {
                            self.dump_selected_tables.clear();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("dump_tables_list")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for t in &schema.tables {
                                    let mut checked = self.dump_selected_tables.contains(&t.name);
                                    if ui.checkbox(&mut checked, &t.name).changed() {
                                        if checked {
                                            self.dump_selected_tables.insert(t.name.clone());
                                        } else {
                                            self.dump_selected_tables.remove(&t.name);
                                        }
                                    }
                                }
                            });
                        });
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.dump_is_busy, egui::Button::new("Generate Dump"))
                        .clicked()
                    {
                        trigger_export = true;
                    }
                    if !self.dump_preview_sql.is_empty() {
                        if ui.button("Save to .sql File...").clicked() {
                            trigger_save_file = true;
                        }
                        if ui.button("Copy All").clicked() {
                            ui.ctx().copy_text(self.dump_preview_sql.clone());
                        }
                    }
                    if self.dump_is_busy {
                        ui.spinner();
                        ui.label("Working...");
                    }
                });

                if !self.dump_preview_sql.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("dump_preview_scroll")
                        .max_height(210.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.dump_preview_sql)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                }
            });

        self.show_sql_dump_modal = open;

        if trigger_save_file {
            let default_name = format!("{}_dump.sql", self.active_database);
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .save_file()
            {
                let _ = save_string_to_path(&path, &self.dump_preview_sql);
                self.status_banner = format!("Saved SQL dump to {}", path.display());
            }
        }

        if trigger_export {
            if let (Some(session), Some(schema)) = (
                self.active_session.clone(),
                self.schemas.get(&self.active_database).cloned(),
            ) {
                self.dump_is_busy = true;
                let db = self.active_database.clone();
                let selected = self.dump_selected_tables.clone();
                let inc_drop = self.dump_include_drop;
                let inc_data = self.dump_include_data;
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let mut full_dump = format!("-- MySQL-Rusty SQL Dump for `{db}`\nUSE `{db}`;\n\n");
                    let mut err_msg = None;
                    for tbl in &schema.tables {
                        if !selected.contains(&tbl.name) {
                            continue;
                        }
                        let data_res = if inc_data {
                            let sql = format!("SELECT * FROM `{db}`.`{}` LIMIT 1000;", tbl.name);
                            match session.execute_query(&db, &sql, None).await {
                                Ok(r) => Some(r),
                                Err(e) => {
                                    err_msg = Some(format!("{e:#}"));
                                    break;
                                }
                            }
                        } else {
                            None
                        };
                        full_dump.push_str(&generate_table_dump_sql(
                            tbl,
                            data_res.as_ref(),
                            inc_drop,
                            inc_data,
                            100,
                        ));
                        full_dump.push('\n');
                    }
                    let _ = tx.send(AsyncEvent::SqlDumpLoaded {
                        sql: match err_msg {
                            Some(e) => Err(e),
                            None => Ok(full_dump),
                        },
                        suggested_name: db,
                    });
                });
            }
        }

        if trigger_import_file {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("SQL", &["sql", "txt"])
                .pick_file()
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(session) = self.active_session.clone() {
                        self.dump_is_busy = true;
                        let db = self.active_database.clone();
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            let stmts = split_sql_statements(&content);
                            let mut executed = 0usize;
                            let mut err_out = None;
                            for stmt in stmts {
                                if let Err(e) = session.execute_query(&db, &stmt, None).await {
                                    err_out = Some(format!("{e:#}"));
                                    break;
                                }
                                executed += 1;
                            }
                            let _ = tx.send(AsyncEvent::SqlDumpImportFinished(match err_out {
                                Some(e) => Err(e),
                                None => Ok(executed),
                            }));
                        });
                    }
                }
            }
        }
    }

    fn render_table_action_modal(&mut self, ctx: &egui::Context) {
        let Some((db_name, table_name, mut new_name)) = self.table_action_modal.clone() else {
            return;
        };
        let mut open = true;
        let mut sql_to_run: Option<String> = None;
        let mut close_now = false;

        egui::Window::new(format!("Table Actions: `{table_name}`"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Duplicate Table").strong());
                ui.horizontal(|ui| {
                    ui.label("New name:");
                    ui.text_edit_singleline(&mut new_name);
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Duplicate Structure Only").clicked() && !new_name.trim().is_empty() {
                        sql_to_run = Some(format!(
                            "CREATE TABLE `{db_name}`.`{}` LIKE `{db_name}`.`{table_name}`;",
                            new_name.trim()
                        ));
                        close_now = true;
                    }
                    if ui.button("Duplicate Structure + Data").clicked() && !new_name.trim().is_empty() {
                        let target = new_name.trim();
                        sql_to_run = Some(format!(
                            "CREATE TABLE `{db_name}`.`{target}` LIKE `{db_name}`.`{table_name}`;\nINSERT INTO `{db_name}`.`{target}` SELECT * FROM `{db_name}`.`{table_name}`;"
                        ));
                        close_now = true;
                    }
                });
                ui.separator();
                ui.label(RichText::new("Danger Zone").strong().color(Color32::from_rgb(248, 113, 113)));
                if ui
                    .button(RichText::new("Truncate Table").color(Color32::from_rgb(248, 113, 113)))
                    .clicked()
                {
                    sql_to_run = Some(format!("TRUNCATE TABLE `{db_name}`.`{table_name}`;"));
                    close_now = true;
                }
            });

        if close_now || !open {
            self.table_action_modal = None;
        } else {
            self.table_action_modal = Some((db_name, table_name.clone(), new_name));
        }

        if let Some(sql) = sql_to_run {
            self.tabs.push(QueryTab::new(format!("Action {table_name}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(None);
        }
    }

    fn render_table_info_modal(&mut self, ctx: &egui::Context) {
        let Some((db_name, tbl, mut subtab)) = self.table_info_modal.clone() else {
            return;
        };
        let mut open = true;

        egui::Window::new(format!("Table Info: `{db_name}`.`{}`", tbl.name))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(580.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut subtab, 0, format!("Columns ({})", tbl.columns.len()));
                    ui.selectable_value(&mut subtab, 1, format!("Foreign Keys ({})", tbl.foreign_keys.len()));
                    ui.selectable_value(&mut subtab, 2, "DDL");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("~{} rows", tbl.approx_rows))
                                .small()
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match subtab {
                    0 => {
                        for col in &tbl.columns {
                            ui.horizontal(|ui| {
                                let pk = if col.is_primary_key { "[PK] " } else { "" };
                                ui.label(RichText::new(format!("{pk}{}", col.name)).strong());
                                ui.label(RichText::new(&col.data_type).color(Color32::from_rgb(56, 189, 248)));
                                if !col.is_nullable {
                                    ui.label(RichText::new("NOT NULL").small().weak());
                                }
                                if let Some(def) = &col.default_value {
                                    ui.label(RichText::new(format!("DEFAULT {def}")).small().weak());
                                }
                            });
                        }
                    }
                    1 => {
                        if tbl.foreign_keys.is_empty() {
                            ui.label(RichText::new("No foreign keys defined.").weak());
                        } else {
                            for fk in &tbl.foreign_keys {
                                ui.label(format!(
                                    "{} -> {}.{}",
                                    fk.column_name, fk.referenced_table, fk.referenced_column
                                ));
                            }
                        }
                    }
                    _ => {
                        let mut ddl = format_table_create_ddl(&tbl);
                        ui.horizontal(|ui| {
                            if ui.small_button("Copy DDL").clicked() {
                                ui.ctx().copy_text(ddl.clone());
                            }
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut ddl)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            });

        if !open {
            self.table_info_modal = None;
        } else {
            self.table_info_modal = Some((db_name, tbl, subtab));
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

pub fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let without_ext = trimmed.strip_suffix(".sql").unwrap_or(trimmed);
    let mut safe = String::new();
    for c in without_ext.chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            safe.push(c);
        } else if c.is_whitespace() || c == '/' || c == '\\' || c == ':' || c == '*' || c == '?' || c == '"' || c == '<' || c == '>' || c == '|' {
            if !safe.ends_with('_') {
                safe.push('_');
            }
        }
    }
    let res = safe.trim_matches(|c| c == '_' || c == '.').to_string();
    if res.is_empty() {
        "query".to_string()
    } else {
        res
    }
}


