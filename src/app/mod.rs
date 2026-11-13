mod admin;
mod editor;
mod modals;
mod results;
mod theme;

pub use admin::ServerAdminSubtab;

pub use results::{
    CellInspectorState, ChartType, MultiResultLayout, ResultViewMode, VisualFilterOp,
};

pub use theme::*;

use crate::ai::{
    ChatMessage, CompletionItem, compute_heuristic_ghost_completion, compute_local_completions,
    request_chat_assistant, request_inline_completion,
};
use crate::config::{
    AiProvider, AppConfig, ConnectionProfile, Environment, QueryHistoryEntry, SavedScratchpad,
    SavedSnippet, SslMode, TrashedConnection,
};
use crate::data_diff::{
    DataDiffReport, RowDiffStatus, compute_data_diff, generate_data_diff_sync_sql,
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
use crate::schema_diff::{SchemaDiffReport, diff_schemas};
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

            let old_file =
                AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(&old_title)));
            let new_file =
                AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(clean)));
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
        if let Some(tab) = self.tabs.get(self.active_tab_idx)
            && !tab.query_params.is_empty()
        {
            sql = substitute_sql_parameters(&sql, &tab.query_params);
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
                self.safety_warning_msg =
                    format!("[WARNING] Destructive Query Detected:\n{reason}");
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
        self.status_banner = "Starting transaction...".to_string();
        self.execute_sql_forced("START TRANSACTION;".to_string());
    }

    pub fn commit_transaction(&mut self) {
        self.status_banner = "Committing transaction...".to_string();
        self.execute_sql_forced("COMMIT;".to_string());
    }

    pub fn rollback_transaction(&mut self) {
        self.status_banner = "Rolling back transaction...".to_string();
        self.execute_sql_forced("ROLLBACK;".to_string());
    }

    pub fn open_csv_import_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select CSV file to import")
            .add_filter("CSV Files", &["csv"])
            .pick_file()
        {
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let default_table = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            match std::fs::read_to_string(&path) {
                Ok(content) => match inspect_csv_content(&content, 5) {
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
                },
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
                self.tabs
                    .push(QueryTab::new(format!("Import {target_name}"), sql));
                self.active_tab_idx = self.tabs.len() - 1;
                self.status_banner =
                    format!("Generated import SQL for `{target_name}` ({count} rows).");
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
                        if let Some(tab) = tabs_clone.first_mut()
                            && tab.sql.trim().is_empty()
                        {
                            if let Some(schema) = schemas_map.get(&active_db) {
                                if let Some(first_tbl) = schema.tables.first() {
                                    tab.sql =
                                        format!("SELECT * FROM `{}` LIMIT 100;", first_tbl.name);
                                } else {
                                    tab.sql = "SHOW TABLES;".to_string();
                                }
                            } else {
                                tab.sql = "SHOW TABLES;".to_string();
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
                                let upper_sql = res
                                    .sql
                                    .trim()
                                    .trim_end_matches(';')
                                    .trim()
                                    .to_ascii_uppercase();
                                match upper_sql.as_str() {
                                    "START TRANSACTION" | "BEGIN" | "BEGIN WORK" => {
                                        self.in_transaction = true;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner =
                                            "Transaction started (Manual Commit Mode).".to_string();
                                    }
                                    "COMMIT" | "COMMIT WORK" => {
                                        self.in_transaction = false;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner =
                                            "Transaction committed successfully.".to_string();
                                    }
                                    "ROLLBACK" | "ROLLBACK WORK" => {
                                        self.in_transaction = false;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner = "Transaction rolled back.".to_string();
                                    }
                                    _ => {
                                        self.status_banner = res.status_message.clone();
                                    }
                                }
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
                AsyncEvent::QueryKilled(res) => match res {
                    Ok(id) => {
                        self.status_banner =
                            format!("Successfully killed query for connection #{id}");
                        self.refresh_server_admin();
                    }
                    Err(e) => self.status_banner = format!("Failed to kill query: {e}"),
                },
                AsyncEvent::TestConnectionResult(res) => {
                    self.testing_connection = false;
                    self.test_connection_result = Some(res);
                }
                AsyncEvent::ServerVariablesLoaded(res) => {
                    self.is_loading_variables = false;
                    match res {
                        Ok(vars) => {
                            self.server_variables = vars;
                            self.status_banner =
                                format!("Loaded {} server variables.", self.server_variables.len());
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
                            self.copilot_auth_status =
                                "✅ Successfully connected to GitHub Copilot!".to_string();
                            let _ = self.config.save();
                            self.status_banner = "GitHub Copilot connected and saved.".to_string();
                        }
                        Err(e) => {
                            if e == "authorization_pending" {
                                self.copilot_auth_status = "Waiting for authorization in browser... Click Check again once approved.".to_string();
                            } else {
                                self.copilot_auth_status =
                                    format!("Authorization check error: {e}");
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
                    file_path: t
                        .file_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
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

                let has_enter_press = i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            ..
                        }
                    )
                });

                if is_ctrl
                    && (i.consume_key(Modifiers::NONE, Key::K)
                        || i.consume_key(Modifiers::NONE, Key::P))
                {
                    toggle_palette = true;
                } else if is_ctrl && is_shift && has_enter_press {
                    i.events.retain(|e| {
                        !matches!(
                            e,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
                    });
                    global_run_stmt = true;
                } else if is_ctrl && has_enter_press {
                    i.events.retain(|e| {
                        !matches!(
                            e,
                            egui::Event::Key {
                                key: Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
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
            } else if global_format_sql && let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = format_sql_pretty(&tab.sql);
                self.status_banner = "Formatted SQL query (Ctrl+Shift+F).".to_string();
            }
        }

        let ai_triggered = ui.ctx().input_mut(|i| {
            let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
            let has_space_press = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: Key::Space,
                        pressed: true,
                        ..
                    }
                )
            });
            if is_ctrl && has_space_press {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key {
                            key: Key::Space,
                            pressed: true,
                            ..
                        }
                    )
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
        if self.config.ai.enable_inline_autocomplete
            && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
            && let Some(instant) = tab.last_edit_instant
        {
            let debounce =
                std::time::Duration::from_millis(self.config.ai.autocomplete_debounce_ms.max(50));
            if instant.elapsed() >= debounce {
                tab.last_edit_instant = None;
                self.trigger_inline_ai_completion();
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(40));
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
                        RichText::new("Home").size(12.0).strong().color(if is_home {
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
                                    let label_text =
                                        RichText::new(&ws.title).size(12.0).color(if is_active {
                                            Color32::WHITE
                                        } else {
                                            Color32::from_rgb(203, 213, 225)
                                        });
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
                            if self.dump_selected_tables.is_empty()
                                && let Some(schema) = self.schemas.get(&self.active_database) {
                                    for t in &schema.tables {
                                        self.dump_selected_tables.insert(t.name.clone());
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
                        && let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.sql = format_sql_pretty(&tab.sql);
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
                ui.label(
                    RichText::new("No database connected").color(Color32::from_rgb(148, 163, 184)),
                );
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
            if ui
                .small_button("Refresh")
                .on_hover_text("Refresh active database schema")
                .clicked()
            {
                self.reload_active_schema();
            }
        });

        if self.databases.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                for db in self.databases.clone() {
                    let is_sel = self.active_database == db;
                    let text = if is_sel {
                        RichText::new(format!("[{db}]"))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248))
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
                .on_hover_text(
                    "Import a CSV file into a new or existing table with preview and mapping",
                )
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
                if !self.diff_source_db.is_empty()
                    && !self.diff_target_db.is_empty()
                    && let (Some(src), Some(tgt)) = (
                        self.schemas.get(&self.diff_source_db),
                        self.schemas.get(&self.diff_target_db),
                    )
                {
                    self.current_diff_report = Some(diff_schemas(src, tgt));
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
                        if !is_active_db
                            && ui
                                .small_button("Set Active DB")
                                .on_hover_text(format!("Switch active database to `{db_name}`"))
                                .clicked()
                            {
                                activate_db = Some(db_name.clone());
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
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("CSV", &["csv", "txt"])
                                    .pick_file()
                                    && let Ok(content) = std::fs::read_to_string(&path) {
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
        } else if let Some(db) = load_db_schema
            && let Some(session) = self.active_session.clone()
        {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Ok(schema) = session.introspect_database(&db).await {
                    let _ = tx.send(AsyncEvent::SchemaLoaded(schema));
                }
            });
        }

        if let Some(tbl_name) = toggle_favorite_table
            && let Some(prof) = self
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
                            ui.colored_label(
                                Color32::from_rgb(er, eg, eb),
                                format!("[{}]", prof.environment.label()),
                            );
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
                            let btn =
                                egui::Button::new(RichText::new(conn_label).size(11.5).strong())
                                    .fill(if is_active {
                                        Color32::from_rgb(6, 78, 59)
                                    } else {
                                        Color32::from_rgb(2, 132, 199)
                                    })
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
                if ui
                    .button("+ Save Active")
                    .on_hover_text("Directly save active query to library and disk (Ctrl+S)")
                    .clicked()
                {
                    self.save_active_tab_direct();
                }
                if ui
                    .button("Open File...")
                    .on_hover_text("Open .sql file into a tab (Ctrl+O)")
                    .clicked()
                {
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
                    && !snip
                        .database
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
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
                                if edit_res.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter))
                                {
                                    rename_snippet =
                                        Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                                }
                                if ui.small_button("OK").clicked() {
                                    rename_snippet =
                                        Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                                }
                                if ui.small_button("Cancel").clicked()
                                    || ui.input(|i| i.key_pressed(Key::Escape))
                                {
                                    self.renaming_snippet_id = None;
                                }
                            } else {
                                let label_res =
                                    ui.label(RichText::new(&snip.title).strong().size(13.0));
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
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("Delete")
                                            .on_hover_text("Delete saved query")
                                            .clicked()
                                        {
                                            delete_snippet_id = Some(snip.id.clone());
                                        }
                                        if ui
                                            .small_button("Rename")
                                            .on_hover_text("Rename query (or double-click title)")
                                            .clicked()
                                        {
                                            self.renaming_snippet_id = Some(snip.id.clone());
                                            self.rename_snippet_buffer = snip.title.clone();
                                        }
                                    },
                                );
                            }
                        });

                        if !snip.description.is_empty() {
                            ui.label(
                                RichText::new(&snip.description)
                                    .small()
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }

                        let preview_lines: Vec<&str> = snip.sql.lines().take(3).collect();
                        let preview = preview_lines.join("\n");
                        ui.label(RichText::new(preview).monospace().small().weak());

                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .small_button("Open Tab")
                                .on_hover_text("Open this query in a new editor tab")
                                .clicked()
                            {
                                new_tab_entry = Some((snip.title.clone(), snip.sql.clone()));
                            }
                            if ui
                                .small_button("Run Now")
                                .on_hover_text("Load and execute immediately")
                                .clicked()
                            {
                                run_sql_now = Some(snip.sql.clone());
                            }
                            if ui
                                .small_button("Paste")
                                .on_hover_text("Paste into active tab")
                                .clicked()
                            {
                                load_sql = Some(snip.sql.clone());
                            }
                            if ui
                                .small_button("Copy")
                                .on_hover_text("Copy SQL to clipboard")
                                .clicked()
                            {
                                ui.ctx().copy_text(snip.sql.clone());
                                self.status_banner = "Copied SQL to clipboard.".to_string();
                            }
                            if ui
                                .small_button("Export .sql")
                                .on_hover_text("Save query as .sql file on disk")
                                .clicked()
                            {
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

        if let Some(sql) = load_sql
            && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
        {
            tab.sql = sql;
            tab.is_dirty = true;
        }

        if let Some(sql) = run_sql_now {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql.clone();
            }
            self.execute_sql_in_current_tab(sql, None);
        }

        if let Some((title, sql)) = export_snippet_sql {
            let filename = format!("{}.sql", title.replace([' ', '/'], "_"));
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
                if !self.config.history.is_empty() && ui.small_button("Clear History").clicked() {
                    clear_all_history = true;
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
                            if ui
                                .small_button("Save to Library")
                                .on_hover_text("Save query into Library")
                                .clicked()
                            {
                                let title = entry
                                    .sql
                                    .lines()
                                    .next()
                                    .unwrap_or("Historic Query")
                                    .trim()
                                    .chars()
                                    .take(40)
                                    .collect::<String>();
                                let clean_title = if title.is_empty() {
                                    "Historic Query".to_string()
                                } else {
                                    title
                                };
                                let new_id = uuid::Uuid::new_v4().to_string();
                                save_snippet_from_history = Some(SavedSnippet {
                                    id: new_id,
                                    title: clean_title,
                                    sql: entry.sql.clone(),
                                    database: if entry.database.is_empty() {
                                        None
                                    } else {
                                        Some(entry.database.clone())
                                    },
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

        if let Some(sql) = load_sql
            && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
        {
            tab.sql = sql;
            tab.is_dirty = true;
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
}
