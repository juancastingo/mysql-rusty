mod actions;
mod admin;
mod ai_panel;
mod chrome;
mod dashboard;
mod editor;
mod events;
mod modals;
mod results;
mod sidebar;
mod state;
mod theme;

pub use state::*;

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
    AiProvider, AppConfig, ConnectionProfile, Environment, SavedScratchpad, SavedSnippet, SslMode,
    TrashedConnection,
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

    // Connection Keepalive & Auto-Reconnect (#13)
    connection_lost: Option<String>,
    last_keepalive_ping: std::time::Instant,
    pending_retry_query: Option<(String, String, Option<usize>)>,
    auto_retry_attempted: bool,

    // SQLite Query History (#15)
    history_store: Option<Arc<crate::history::HistoryStore>>,
    history_records: Vec<crate::history::HistoryRecord>,
    history_filter_current_conn: bool,
    history_filter_database: String,
    history_filter_failed_only: bool,
    history_limit: usize,
    history_has_more: bool,
    history_last_input: Option<std::time::Instant>,
    history_dirty: bool,
    history_confirm_clear: bool,

    // Full Database Backup (#16) & Streaming Restore (#17)
    backup_modal: Option<BackupModalState>,
    restore_modal: Option<RestoreModalState>,

    // Slow Queries (#20)
    slow_queries_enabled: Option<bool>,
    slow_queries_result: Option<QueryResult>,
    slow_queries_loading: bool,
    slow_queries_only_current_db: bool,
    slow_queries_limit: usize,
    slow_queries_confirm_reset: bool,
    slow_queries_error: Option<String>,

    // Routine Call Modal (#35)
    routine_call_modal: Option<RoutineCallModalState>,
}

impl MysqlRustyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: Arc<Runtime>) -> Self {
        configure_modern_dark_theme(&cc.egui_ctx);
        let (tx, rx) = unbounded_channel();
        let mut config = AppConfig::load();
        let history_store = match crate::history::HistoryStore::open(&AppConfig::history_db_path())
        {
            Ok(store) => {
                if !config.history.is_empty() {
                    for old in config.history.drain(..) {
                        let _ = store.insert(&crate::history::HistoryRecord {
                            id: 0,
                            ts: old.timestamp,
                            profile_id: String::new(),
                            profile_name: "(imported)".to_string(),
                            database: old.database,
                            sql: old.sql,
                            duration_ms: old.duration_ms,
                            row_count: old.row_count as u64,
                            success: old.success,
                            error: None,
                        });
                    }
                    let _ = config.save();
                }
                let _ = store.prune(config.query.history_retention);
                Some(Arc::new(store))
            }
            Err(e) => {
                tracing::warn!("Failed to open SQLite history store: {e}");
                None
            }
        };

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
            connection_lost: None,
            last_keepalive_ping: std::time::Instant::now(),
            pending_retry_query: None,
            auto_retry_attempted: false,
            history_store,
            history_records: Vec::new(),
            history_filter_current_conn: false,
            history_filter_database: String::new(),
            history_filter_failed_only: false,
            history_limit: 100,
            history_has_more: false,
            history_last_input: None,
            history_dirty: true,
            history_confirm_clear: false,
            backup_modal: None,
            restore_modal: None,
            slow_queries_enabled: None,
            slow_queries_result: None,
            slow_queries_loading: false,
            slow_queries_only_current_db: false,
            slow_queries_limit: 50,
            slow_queries_confirm_reset: false,
            slow_queries_error: None,
            routine_call_modal: None,
        }
    }
}

impl eframe::App for MysqlRustyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_async_events();

        // Debounced SQLite History Refresh (200ms)
        if self.history_dirty
            && self
                .history_last_input
                .map(|t| t.elapsed().as_millis() >= 200)
                .unwrap_or(true)
        {
            self.history_dirty = false;
            self.history_last_input = None;
            self.refresh_history();
        }

        // Periodic Connection Keepalive Ping (every 60 seconds)
        if self.active_session.is_some()
            && self.connection_lost.is_none()
            && self.last_keepalive_ping.elapsed().as_secs() >= 60
        {
            self.last_keepalive_ping = std::time::Instant::now();
            if let Some(session) = self.active_session.clone() {
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let res = session.ping().await.map_err(|e| format!("{e:#}"));
                    let _ = tx.send(AsyncEvent::KeepaliveResult(res));
                });
            }
        }

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
        self.render_backup_modal(&ctx);
        self.render_restore_modal(&ctx);
        self.render_routine_call_modal(&ctx);
    }
}
