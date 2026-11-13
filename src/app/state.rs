use super::*;

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
        is_connection_error: bool,
    },
    MultiQueryFinished {
        tab_id: String,
        results: Vec<Result<QueryResult, String>>,
        is_connection_error: bool,
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
    KeepaliveResult(Result<(), String>),
    Reconnected(Result<(String, Arc<ActiveSession>), String>),
    HistoryLoaded(Vec<crate::history::HistoryRecord>),
    HistoryCleared,
    BackupProgress(crate::backup::BackupProgress),
    RestoreProgress(crate::backup::RestoreProgress),
    SlowQueriesLoaded(Result<(bool, QueryResult), String>),
    SlowQueriesReset(Result<(), String>),
    ObjectDdlLoaded {
        title: String,
        ddl: Result<String, String>,
    },
    RoutineCallModalReady {
        db: String,
        routine_name: String,
        params: Result<Vec<crate::db::RoutineParamInfo>, String>,
    },
    UsersLoaded(Result<Vec<crate::db::UserInfo>, String>),
    UserGrantsLoaded {
        user: String,
        host: String,
        grants: Result<Vec<String>, String>,
    },
    UserActionFinished(Result<String, String>),
    Nl2SqlFinished {
        tab_id: String,
        result: Result<crate::ai::Nl2SqlResult, String>,
    },
    AiOptimizePlanFinished {
        tab_id: String,
        result: Result<crate::ai::QueryOptimizationReport, String>,
    },
    MockFkSamplesLoaded {
        table_name: String,
        samples: Result<HashMap<String, Vec<String>>, String>,
    },
}

pub struct RoutineCallModalState {
    pub db: String,
    pub routine_name: String,
    pub params: Vec<crate::db::RoutineParamInfo>,
    pub arg_values: Vec<String>,
}

pub struct RestoreModalState {
    pub file_path: Option<std::path::PathBuf>,
    pub database: String,
    pub policy: crate::backup::OnError,
    pub running: bool,
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub statements_ok: u64,
    pub statements_failed: u64,
    pub errors: Vec<crate::backup::RestoreErrorEntry>,
    pub status: Option<String>,
    pub prod_confirm_input: String,
}

pub struct BackupModalState {
    pub database: String,
    pub all_tables: Vec<String>,
    pub selected_tables: std::collections::HashSet<String>,
    pub include_data: bool,
    pub include_drop: bool,
    pub include_views: bool,
    pub include_routines: bool,
    pub include_triggers: bool,
    pub batch_rows: usize,
    pub running: bool,
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub current_table: String,
    pub table_index: usize,
    pub table_total: usize,
    pub rows_done: u64,
    pub status: Option<String>,
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
    pub confirm_load_all: bool,
    pub view_mode: ResultViewMode,
    pub chart_type: ChartType,
    pub chart_x_col: usize,
    pub chart_y_col: usize,
    pub file_path: Option<std::path::PathBuf>,
    pub is_dirty: bool,
    pub find_open: bool,
    pub replace_open: bool,
    pub find_opts: crate::find::FindOptions,
    pub replace_text: String,
    pub find_matches: Vec<std::ops::Range<usize>>,
    pub find_current: usize,
    pub find_error: Option<String>,
    pub find_autofocus: bool,
    pub nl2sql_open: bool,
    pub nl2sql_autofocus: bool,
    pub nl2sql_prompt: String,
    pub nl2sql_loading: bool,
    pub nl2sql_result: Option<crate::ai::Nl2SqlResult>,
    pub nl2sql_error: Option<String>,
    pub ai_plan_loading: bool,
    pub ai_plan_report: Option<crate::ai::QueryOptimizationReport>,
    pub ai_plan_error: Option<String>,
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
            confirm_load_all: false,
            view_mode: ResultViewMode::Grid,
            chart_type: ChartType::Bar,
            chart_x_col: 0,
            chart_y_col: 1,
            file_path: None,
            is_dirty: false,
            find_open: false,
            replace_open: false,
            find_opts: crate::find::FindOptions::default(),
            replace_text: String::new(),
            find_matches: Vec::new(),
            find_current: 0,
            find_error: None,
            find_autofocus: false,
            nl2sql_open: false,
            nl2sql_autofocus: false,
            nl2sql_prompt: String::new(),
            nl2sql_loading: false,
            nl2sql_result: None,
            nl2sql_error: None,
            ai_plan_loading: false,
            ai_plan_report: None,
            ai_plan_error: None,
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
pub(crate) enum SidebarTab {
    Schema,
    Connections,
    SavedQueries,
    History,
}

#[derive(PartialEq, Eq)]
pub(crate) enum BottomTab {
    ResultsGrid,
    ErRelationships,
    ExecutionPlanAndLog,
    ServerAdmin,
}

#[derive(Clone)]
pub(crate) struct OpenConnectionWorkspace {
    pub(crate) profile_id: String,
    pub(crate) title: String,
    pub(crate) env_color: (u8, u8, u8),
    pub(crate) session: Arc<ActiveSession>,
    pub(crate) databases: Vec<String>,
    pub(crate) active_database: String,
    pub(crate) schemas: HashMap<String, DatabaseSchema>,
    pub(crate) tabs: Vec<QueryTab>,
    pub(crate) active_tab_idx: usize,
    pub(crate) in_transaction: bool,
    pub(crate) uncommitted_mutations: usize,
    pub(crate) er_state: ErCanvasState,
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_csv_import_wizard_logic() {
        use crate::export::{generate_csv_import_sql, inspect_csv_content};

        let csv = "first_name,last_name,email,age\nAlice,Smith,alice@example.com,29\nBob,Jones,bob@example.com,41";
        let (headers, samples, count) = inspect_csv_content(csv, 5).expect("Failed to inspect CSV");
        assert_eq!(headers, vec!["first_name", "last_name", "email", "age"]);
        assert_eq!(samples.len(), 2);
        assert_eq!(count, 2);

        let mappings = vec![
            ("first_name".to_string(), "name".to_string()),
            ("email".to_string(), "user_email".to_string()),
        ];
        let sql = generate_csv_import_sql("customers", &headers, &mappings, csv, true)
            .expect("Failed to generate import SQL");
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS `customers`"));
        assert!(sql.contains("INSERT INTO `customers` (`name`, `user_email`) VALUES"));
        assert!(sql.contains("('Alice', 'alice@example.com')"));
    }
    #[test]
    fn test_saved_query_model_and_tab_dirty_title() {
        use crate::app::QueryTab;
        use crate::config::SavedSnippet;

        let mut tab = QueryTab::new("My Analytics", "SELECT COUNT(*) FROM orders;");
        assert_eq!(tab.display_title(), "My Analytics");
        assert!(!tab.is_dirty);

        tab.is_dirty = true;
        assert_eq!(tab.display_title(), "My Analytics *");

        let snip = SavedSnippet {
            id: "test-uuid-1".to_string(),
            title: "Orders by Month".to_string(),
            sql: "SELECT MONTH(created_at), COUNT(*) FROM orders GROUP BY 1;".to_string(),
            database: Some("ecommerce".to_string()),
            description: "Aggregates revenue by month".to_string(),
            created_at: "2026-11-12".to_string(),
        };

        assert_eq!(snip.title, "Orders by Month");
        assert_eq!(snip.database.as_deref(), Some("ecommerce"));
        assert_eq!(snip.description, "Aggregates revenue by month");
    }
}
