use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SslMode {
    Disabled,
    #[default]
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

impl SslMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Preferred => "Preferred",
            Self::Required => "Required",
            Self::VerifyCa => "Verify CA",
            Self::VerifyIdentity => "Verify Identity",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Disabled => "Unencrypted plaintext connection.",
            Self::Preferred => {
                "Use TLS if supported by the server; otherwise fall back to plaintext."
            }
            Self::Required => "Require TLS encryption; optionally allow self-signed certificates.",
            Self::VerifyCa => "Require TLS and verify certificate chain against the configured CA.",
            Self::VerifyIdentity => "Require TLS, verify CA chain, and verify server hostname.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub enabled: bool,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_key_path: String,
    pub local_bind_port: u16,
}

impl Default for SshTunnelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_user: "ubuntu".to_string(),
            ssh_key_path: "~/.ssh/id_ed25519".to_string(),
            local_bind_port: 13306,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Environment {
    #[default]
    Development,
    Staging,
    Production,
}

impl Environment {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Development => "Dev",
            Self::Staging => "Staging",
            Self::Production => "Production",
        }
    }

    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            Self::Development => (34, 197, 94), // Emerald green
            Self::Staging => (234, 179, 8),     // Amber yellow
            Self::Production => (239, 68, 68),  // Bright red
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub color_hex: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub default_database: String,
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub ssl_accept_invalid_certs: bool,
    #[serde(default)]
    pub ssl_ca_path: String,
    #[serde(default)]
    pub ssl_client_cert_path: String,
    #[serde(default)]
    pub ssl_client_key_path: String,
    pub ssh: SshTunnelConfig,
    pub is_demo: bool,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub favorite_tables: Vec<String>,
    #[serde(default)]
    pub dialect: crate::dialect::Dialect,
    #[serde(default)]
    pub sqlite_path: String,
    #[serde(default)]
    pub is_favorite: bool,
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: "Localhost MySQL".to_string(),
            color_hex: "#38bdf8".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            user: "root".to_string(),
            password: String::new(),
            default_database: String::new(),
            ssl_mode: SslMode::Preferred,
            ssl_accept_invalid_certs: false,
            ssl_ca_path: String::new(),
            ssl_client_cert_path: String::new(),
            ssl_client_key_path: String::new(),
            ssh: SshTunnelConfig::default(),
            is_demo: false,
            environment: Environment::Development,
            read_only: false,
            favorite_tables: Vec::new(),
            dialect: crate::dialect::Dialect::MySql,
            sqlite_path: String::new(),
            is_favorite: false,
        }
    }
}

impl ConnectionProfile {
    pub fn demo_profile() -> Self {
        Self {
            id: "demo-sandbox-mysql".to_string(),
            name: "Sandbox Demo (ecommerce_prod)".to_string(),
            color_hex: "#10b981".to_string(),
            host: "sandbox.local".to_string(),
            port: 3306,
            user: "demo_admin".to_string(),
            password: String::new(),
            default_database: "ecommerce_prod".to_string(),
            ssl_mode: SslMode::Disabled,
            ssl_accept_invalid_certs: false,
            ssl_ca_path: String::new(),
            ssl_client_cert_path: String::new(),
            ssl_client_key_path: String::new(),
            ssh: SshTunnelConfig::default(),
            is_demo: true,
            environment: Environment::Development,
            read_only: false,
            favorite_tables: Vec::new(),
            dialect: crate::dialect::Dialect::MySql,
            sqlite_path: String::new(),
            is_favorite: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AiProvider {
    LocalSchema,
    Ollama,
    GitHubCopilot,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    CliCommand,
}

impl AiProvider {
    pub fn label(&self) -> &'static str {
        match self {
            Self::LocalSchema => "⚡ Local Schema Native (0ms, 100% Offline)",
            Self::Ollama => "💻 Ollama FIM (Fast Local Code Model)",
            Self::GitHubCopilot => "✨ GitHub Copilot (Official Account)",
            Self::OpenAiCompatible => "OpenAI / Compatible (Groq, OpenRouter, vLLM)",
            Self::Anthropic => "Anthropic (Claude)",
            Self::Gemini => "Google Gemini API",
            Self::CliCommand => "Local CLI (agy / llm / custom)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub enable_inline_autocomplete: bool,
    #[serde(default = "default_true")]
    pub enable_auto_error_fix: bool,
    pub include_schema_context: bool,

    #[serde(default = "default_debounce_ms")]
    pub autocomplete_debounce_ms: u64,
    #[serde(default = "default_max_tokens")]
    pub autocomplete_max_tokens: u32,

    // Ollama
    pub ollama_endpoint: String,
    pub ollama_model: String,
    #[serde(default = "default_true")]
    pub ollama_use_fim: bool,

    // GitHub Copilot
    #[serde(default)]
    pub copilot_github_token: String,
    #[serde(default)]
    pub copilot_session_token: String,
    #[serde(default)]
    pub copilot_token_expires_at: u64,
    #[serde(default = "default_copilot_model")]
    pub copilot_model: String,

    // OpenAI / Compatible
    pub openai_base_url: String,
    pub openai_api_key: String,
    pub openai_model: String,

    // Anthropic
    pub anthropic_api_key: String,
    pub anthropic_model: String,

    // Gemini
    pub gemini_api_key: String,
    pub gemini_model: String,

    // CLI Command (e.g. agy, llm, claude, ollama run)
    pub cli_command: String,
    pub cli_pass_via_stdin: bool,
}

fn default_copilot_model() -> String {
    "gpt-4o".to_string()
}

fn default_debounce_ms() -> u64 {
    180
}

fn default_max_tokens() -> u32 {
    35
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::LocalSchema,
            enable_inline_autocomplete: true,
            enable_auto_error_fix: true,
            include_schema_context: true,
            autocomplete_debounce_ms: 180,
            autocomplete_max_tokens: 35,
            ollama_endpoint: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5-coder:1.5b".to_string(),
            ollama_use_fim: true,
            copilot_github_token: String::new(),
            copilot_session_token: String::new(),
            copilot_token_expires_at: 0,
            copilot_model: "gpt-4o".to_string(),
            openai_base_url: "https://api.openai.com/v1".to_string(),
            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_model: "gpt-4o-mini".to_string(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            anthropic_model: "claude-3-5-haiku-latest".to_string(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            gemini_model: "gemini-2.0-flash".to_string(),
            cli_command: "agy".to_string(),
            cli_pass_via_stdin: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryHistoryEntry {
    pub timestamp: String,
    pub database: String,
    pub sql: String,
    pub duration_ms: u64,
    pub row_count: usize,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSnippet {
    pub id: String,
    pub title: String,
    pub sql: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedScratchpad {
    pub title: String,
    pub sql: String,
    #[serde(default)]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashedConnection {
    pub profile: ConnectionProfile,
    pub deleted_at: String,
}

fn default_history_retention() -> usize {
    10_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySettings {
    /// Query timeout in seconds (`0` = no timeout).
    pub timeout_secs: u64,
    /// Maximum number of rows buffered in memory per query result.
    pub max_rows: usize,
    /// Maximum number of query history entries retained in SQLite.
    #[serde(default = "default_history_retention")]
    pub history_retention: usize,
}

impl Default for QuerySettings {
    fn default() -> Self {
        Self {
            timeout_secs: 0,
            max_rows: 10_000,
            history_retention: default_history_retention(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemePreference {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "Follow System",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DarkThemePreset {
    #[default]
    Obsidian,
    Nord,
    Dracula,
    Catppuccin,
}

impl DarkThemePreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Obsidian => "Obsidian",
            Self::Nord => "Nord",
            Self::Dracula => "Dracula",
            Self::Catppuccin => "Catppuccin Mocha",
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Obsidian, Self::Nord, Self::Dracula, Self::Catppuccin]
    }

    pub fn preview_swatches(self) -> [(u8, u8, u8); 3] {
        match self {
            Self::Obsidian => [(13, 18, 28), (19, 26, 40), (56, 189, 248)],
            Self::Nord => [(46, 52, 64), (59, 66, 82), (136, 192, 208)],
            Self::Dracula => [(33, 34, 44), (40, 42, 54), (189, 147, 249)],
            Self::Catppuccin => [(24, 24, 37), (30, 30, 46), (137, 180, 250)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiDensity {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl UiDensity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Spacious => "Spacious",
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Compact, Self::Comfortable, Self::Spacious]
    }

    pub fn next(self) -> Self {
        match self {
            Self::Compact => Self::Comfortable,
            Self::Comfortable => Self::Spacious,
            Self::Spacious => Self::Compact,
        }
    }

    pub fn grid_row_height(self) -> f32 {
        match self {
            Self::Compact => 21.0,
            Self::Comfortable => 25.0,
            Self::Spacious => 30.0,
        }
    }

    pub fn grid_header_height(self) -> f32 {
        match self {
            Self::Compact => 24.0,
            Self::Comfortable => 28.0,
            Self::Spacious => 32.0,
        }
    }
}

fn default_ui_scale() -> f32 {
    1.0
}

fn default_editor_font_size() -> f32 {
    14.0
}

fn default_language() -> String {
    "system".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default)]
    pub vim_mode: bool,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub dark_preset: DarkThemePreset,
    #[serde(default)]
    pub density: UiDensity,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: f32,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub show_line_numbers: bool,
    #[serde(default = "default_true")]
    pub dashboard_grid_view: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            vim_mode: false,
            theme: ThemePreference::Dark,
            dark_preset: DarkThemePreset::Obsidian,
            density: UiDensity::Comfortable,
            ui_scale: default_ui_scale(),
            editor_font_size: default_editor_font_size(),
            language: default_language(),
            show_line_numbers: true,
            dashboard_grid_view: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSyncPreset {
    pub id: String,
    pub name: String,
    pub batch_mode: bool,
    pub source_server_name: String,
    pub source_db: String,
    pub source_table: String,
    pub target_server_name: String,
    pub target_db: String,
    pub target_table: String,
    pub key_col: usize,
    #[serde(default)]
    pub excluded_tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub connections: Vec<ConnectionProfile>,
    pub ai: AiConfig,
    #[serde(default)]
    pub query: QuerySettings,
    #[serde(default)]
    pub ui: UiSettings,
    pub history: Vec<QueryHistoryEntry>,
    pub snippets: Vec<SavedSnippet>,
    #[serde(default)]
    pub scratchpads: Vec<SavedScratchpad>,
    #[serde(default)]
    pub trashed_connections: Vec<TrashedConnection>,
    #[serde(default)]
    pub keybindings: std::collections::HashMap<crate::shortcuts::Action, String>,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default)]
    pub saved_sync_presets: Vec<SavedSyncPreset>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            connections: vec![
                ConnectionProfile::demo_profile(),
                ConnectionProfile::default(),
            ],
            ai: AiConfig::default(),
            query: QuerySettings::default(),
            ui: UiSettings::default(),
            history: Vec::new(),
            scratchpads: Vec::new(),
            trashed_connections: Vec::new(),
            keybindings: std::collections::HashMap::new(),
            onboarding_completed: false,
            saved_sync_presets: Vec::new(),
            snippets: vec![
                SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title: "Top Customers by Revenue (JOIN + GROUP BY)".to_string(),
                    sql: "SELECT \n    u.id,\n    u.full_name,\n    u.email,\n    u.country,\n    COUNT(o.id) AS total_orders,\n    ROUND(SUM(o.total_amount), 2) AS lifetime_value\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE o.status = 'paid'\nGROUP BY u.id, u.full_name, u.email, u.country\nORDER BY lifetime_value DESC\nLIMIT 25;".to_string(),
                    database: None,
                    description: "Calculates customer lifetime orders and total spend".to_string(),
                    created_at: "2026-11-12".to_string(),
                },
                SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title: "Inspect Table & Index Sizes".to_string(),
                    sql: "SELECT \n    table_name AS `Table`,\n    engine AS `Engine`,\n    table_rows AS `Approx Rows`,\n    ROUND((data_length + index_length) / 1024 / 1024, 2) AS `Size (MB)`\nFROM information_schema.TABLES\nWHERE table_schema = DATABASE()\nORDER BY (data_length + index_length) DESC;".to_string(),
                    database: None,
                    description: "Queries information_schema for physical table disk usage in MB".to_string(),
                    created_at: "2026-11-12".to_string(),
                },
            ],
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mysql-rusty")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn queries_dir() -> PathBuf {
        let dir = Self::config_dir().join("queries");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn logs_dir() -> PathBuf {
        let dir = Self::config_dir().join("logs");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn history_db_path() -> PathBuf {
        Self::config_dir().join("history.sqlite3")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut cfg = match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(parsed) => parsed,
                Err(err) => {
                    tracing::warn!(error = %err, "Failed to parse config.json, using defaults");
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::warn!(error = %err, "Failed to read config.json, using defaults");
                Self::default()
            }
        };

        // Hydrate secrets from OS keyring if available
        for conn in &mut cfg.connections {
            let key = format!("conn_pwd_{}", conn.id);
            if let Some(sec) = get_secret(&key) {
                conn.password = sec;
            }
        }
        if let Some(k) = get_secret("openai_api_key") {
            cfg.ai.openai_api_key = k;
        }
        if let Some(k) = get_secret("anthropic_api_key") {
            cfg.ai.anthropic_api_key = k;
        }
        if let Some(k) = get_secret("gemini_api_key") {
            cfg.ai.gemini_api_key = k;
        }
        if let Some(k) = get_secret("copilot_github_token") {
            cfg.ai.copilot_github_token = k;
        }

        cfg
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let mut config_to_save = self.clone();

        // Store secrets in OS Keyring
        for conn in &self.connections {
            let key = format!("conn_pwd_{}", conn.id);
            let _ = set_secret(&key, &conn.password);
            if get_secret(&key).as_deref() == Some(&conn.password) {
                // Keyring confirmed storage: clean password from disk config
                if let Some(c) = config_to_save
                    .connections
                    .iter_mut()
                    .find(|c| c.id == conn.id)
                {
                    c.password.clear();
                }
            }
        }

        // Never persist plaintext passwords for trashed connections
        for trashed in &mut config_to_save.trashed_connections {
            trashed.profile.password.clear();
        }

        let _ = set_secret("openai_api_key", &self.ai.openai_api_key);
        if get_secret("openai_api_key").as_deref() == Some(&self.ai.openai_api_key) {
            config_to_save.ai.openai_api_key.clear();
        }

        let _ = set_secret("anthropic_api_key", &self.ai.anthropic_api_key);
        if get_secret("anthropic_api_key").as_deref() == Some(&self.ai.anthropic_api_key) {
            config_to_save.ai.anthropic_api_key.clear();
        }

        let _ = set_secret("gemini_api_key", &self.ai.gemini_api_key);
        if get_secret("gemini_api_key").as_deref() == Some(&self.ai.gemini_api_key) {
            config_to_save.ai.gemini_api_key.clear();
        }

        let _ = set_secret("copilot_github_token", &self.ai.copilot_github_token);
        if get_secret("copilot_github_token").as_deref() == Some(&self.ai.copilot_github_token) {
            config_to_save.ai.copilot_github_token.clear();
        }
        // Ephemeral Copilot session tokens should never be written to disk
        config_to_save.ai.copilot_session_token.clear();
        config_to_save.ai.copilot_token_expires_at = 0;

        let json = serde_json::to_string_pretty(&config_to_save)?;
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            f.write_all(json.as_bytes())?;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        {
            fs::write(&path, json)?;
        }
        Ok(())
    }
}

pub const KEYRING_SERVICE: &str = "mysql-rusty";

pub fn set_secret(key: &str, secret: &str) -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, key) {
        Ok(entry) => {
            if secret.is_empty() {
                if let Err(err) = entry.delete_credential() {
                    tracing::debug!(key, error = %err, "Keyring delete_credential failed");
                }
            } else if let Err(err) = entry.set_password(secret) {
                tracing::warn!(key, error = %err, "Keyring set_password failed");
            }
        }
        Err(err) => {
            tracing::warn!(key, error = %err, "Keyring Entry::new failed");
        }
    }
    Ok(())
}

pub fn get_secret(key: &str) -> Option<String> {
    match keyring::Entry::new(KEYRING_SERVICE, key) {
        Ok(entry) => match entry.get_password() {
            Ok(pwd) if !pwd.is_empty() => return Some(pwd),
            Ok(_) => {}
            Err(err) => {
                tracing::debug!(key, error = %err, "Keyring get_password returned no entry");
            }
        },
        Err(err) => {
            tracing::warn!(key, error = %err, "Keyring Entry::new failed");
        }
    }
    None
}

#[allow(dead_code)]
pub fn delete_secret(key: &str) -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn extract_workbench_tag_value(block: &str, key: &str) -> Option<String> {
    let needle = format!("key=\"{key}\"");
    let mut search_from = 0;
    while let Some(rel_idx) = block[search_from..].find(&needle) {
        let key_idx = search_from + rel_idx;
        let after_key = &block[key_idx + needle.len()..];
        let Some(gt_rel) = after_key.find('>') else {
            break;
        };
        // Skip self-closing tags like <value ... key="..." />
        if gt_rel > 0 && after_key.as_bytes()[gt_rel - 1] == b'/' {
            search_from = key_idx + needle.len() + gt_rel + 1;
            continue;
        }
        let val_start = key_idx + needle.len() + gt_rel + 1;
        let rest = &block[val_start..];
        // Direct scalar value ends at </value> or </link>
        if let Some(end_rel) = rest.find("</") {
            let inner = rest[..end_rel].trim();
            // Skip container tags like parameterValues whose inner content contains child tags
            if !inner.starts_with('<') {
                return Some(unescape_xml(inner));
            }
        }
        search_from = val_start;
    }
    None
}

/// Locates MySQL Workbench's `connections.xml` on Linux, macOS, or Windows.
pub fn find_workbench_connections_xml() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".mysql/workbench/connections.xml"));
        candidates.push(
            home.join("snap/mysql-workbench-community/common/.mysql/workbench/connections.xml"),
        );
        candidates
            .push(home.join(".var/app/com.mysql.Workbench/data/mysql/workbench/connections.xml"));
        candidates.push(home.join("Library/Application Support/MySQL/Workbench/connections.xml"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("MySQL/Workbench/connections.xml"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Parses MySQL Workbench `connections.xml` into `ConnectionProfile` entries.
pub fn parse_workbench_connections_xml(xml: &str) -> Vec<ConnectionProfile> {
    let mut profiles = Vec::new();
    let marker = "struct-name=\"db.mgmt.Connection\"";
    let mut search_from = 0;

    while let Some(rel_idx) = xml[search_from..].find(marker) {
        let marker_idx = search_from + rel_idx;
        if !xml[..marker_idx].ends_with(char::is_whitespace) {
            search_from = marker_idx + marker.len();
            continue;
        }
        let Some(tag_start) = xml[..marker_idx].rfind("<value") else {
            search_from = marker_idx + marker.len();
            continue;
        };

        // Walk <value ...> and </value> tags to find the matching closing </value>
        let mut depth = 0usize;
        let mut cursor = tag_start;
        let mut block_end = None;
        while let Some(lt_rel) = xml[cursor..].find('<') {
            let lt = cursor + lt_rel;
            let Some(gt_rel) = xml[lt..].find('>') else {
                break;
            };
            let gt = lt + gt_rel;
            let tag = &xml[lt..=gt];
            if tag.starts_with("<value") {
                if !tag.ends_with("/>") {
                    depth += 1;
                }
            } else if tag.starts_with("</value>") {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    block_end = Some(gt + 1);
                    break;
                }
            }
            cursor = gt + 1;
        }

        let Some(end_idx) = block_end else {
            break;
        };
        let block = &xml[tag_start..end_idx];
        search_from = end_idx;

        let name = extract_workbench_tag_value(block, "name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Workbench Connection".to_string());
        let host = extract_workbench_tag_value(block, "hostName")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = extract_workbench_tag_value(block, "port")
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|&p| p > 0)
            .unwrap_or(3306);
        let user = extract_workbench_tag_value(block, "userName")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "root".to_string());
        let password = extract_workbench_tag_value(block, "password").unwrap_or_default();
        let default_database = extract_workbench_tag_value(block, "schema")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| extract_workbench_tag_value(block, "DbSqlEditor:LastDefaultSchema"))
            .unwrap_or_default();

        let ssl_mode =
            match extract_workbench_tag_value(block, "useSSL").and_then(|v| v.parse::<u8>().ok()) {
                Some(0) => SslMode::Disabled,
                Some(2) => SslMode::Required,
                Some(3) => SslMode::VerifyCa,
                Some(4) => SslMode::VerifyIdentity,
                _ => SslMode::Preferred,
            };
        let ssl_ca_path = extract_workbench_tag_value(block, "sslCA").unwrap_or_default();
        let ssl_client_cert_path =
            extract_workbench_tag_value(block, "sslCert").unwrap_or_default();
        let ssl_client_key_path = extract_workbench_tag_value(block, "sslKey").unwrap_or_default();

        let driver = extract_workbench_tag_value(block, "driver").unwrap_or_default();
        let ssh_raw_host = extract_workbench_tag_value(block, "sshHost").unwrap_or_default();
        let ssh_enabled = driver.contains("sshtun") || !ssh_raw_host.trim().is_empty();
        let (ssh_host, ssh_port) = if let Some((h, p)) = ssh_raw_host.rsplit_once(':') {
            if let Ok(port_num) = p.trim().parse::<u16>() {
                (h.trim().to_string(), port_num)
            } else {
                (ssh_raw_host.trim().to_string(), 22)
            }
        } else {
            (ssh_raw_host.trim().to_string(), 22)
        };
        let ssh_user = extract_workbench_tag_value(block, "sshUserName")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "ubuntu".to_string());
        let ssh_key_path = extract_workbench_tag_value(block, "sshKeyFile")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "~/.ssh/id_ed25519".to_string());

        let name_lower = name.to_ascii_lowercase();
        let (environment, color_hex) = if name_lower.contains("prod") {
            (Environment::Production, "#ef4444".to_string())
        } else if name_lower.contains("uat") || name_lower.contains("stag") {
            (Environment::Staging, "#eab308".to_string())
        } else {
            (Environment::Development, "#38bdf8".to_string())
        };

        profiles.push(ConnectionProfile {
            id: Uuid::new_v4().to_string(),
            name,
            color_hex,
            host,
            port,
            user,
            password,
            default_database,
            ssl_mode,
            ssl_accept_invalid_certs: false,
            ssl_ca_path,
            ssl_client_cert_path,
            ssl_client_key_path,
            ssh: SshTunnelConfig {
                enabled: ssh_enabled,
                ssh_host,
                ssh_port,
                ssh_user,
                ssh_key_path,
                local_bind_port: 13306,
            },
            is_demo: false,
            environment,
            read_only: false,
            favorite_tables: Vec::new(),
            dialect: crate::dialect::Dialect::MySql,
            sqlite_path: String::new(),
            is_favorite: false,
        });
    }

    profiles
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_keyring_fallback() {
        use crate::config::{delete_secret, get_secret, set_secret};
        let _ = set_secret("test_key", "secret_value");
        let _ = get_secret("test_key");
        let _ = delete_secret("test_key");
    }

    #[test]
    fn test_parse_workbench_connections_xml() {
        use crate::config::{Environment, SslMode, parse_workbench_connections_xml};
        let xml = r#"<?xml version="1.0"?>
<data grt_format="2.0">
  <value type="list" content-type="object" content-struct-name="db.mgmt.Connection">
    <value type="object" struct-name="db.mgmt.Connection" id="1">
      <link type="object" struct-name="db.mgmt.Driver" key="driver">com.mysql.rdbms.mysql.driver.native</link>
      <value type="dict" key="modules"/>
      <value type="dict" key="parameterValues">
        <value type="string" key="DbSqlEditor:LastDefaultSchema">app_db</value>
        <value type="string" key="hostName">127.0.0.1</value>
        <value type="int" key="port">33061</value>
        <value type="int" key="useSSL">0</value>
        <value type="string" key="userName">root</value>
      </value>
      <value type="string" key="name">local &amp; dev</value>
    </value>
    <value type="object" struct-name="db.mgmt.Connection" id="2">
      <link type="object" struct-name="db.mgmt.Driver" key="driver">com.mysql.rdbms.mysql.driver.native_sshtun</link>
      <value type="dict" key="parameterValues">
        <value type="string" key="hostName">db.prod.internal</value>
        <value type="int" key="port">3306</value>
        <value type="string" key="schema">prod_core</value>
        <value type="string" key="sshHost">bastion.example.com:2222</value>
        <value type="string" key="sshKeyFile">/home/user/.ssh/id_rsa</value>
        <value type="string" key="sshUserName">ec2-user</value>
        <value type="int" key="useSSL">2</value>
        <value type="string" key="userName">admin</value>
      </value>
      <value type="string" key="name">portal - prod</value>
    </value>
  </value>
</data>"#;

        let parsed = parse_workbench_connections_xml(xml);
        assert_eq!(parsed.len(), 2);

        assert_eq!(parsed[0].name, "local & dev");
        assert_eq!(parsed[0].host, "127.0.0.1");
        assert_eq!(parsed[0].port, 33061);
        assert_eq!(parsed[0].user, "root");
        assert_eq!(parsed[0].default_database, "app_db");
        assert_eq!(parsed[0].ssl_mode, SslMode::Disabled);
        assert!(!parsed[0].ssh.enabled);
        assert_eq!(parsed[0].environment, Environment::Development);

        assert_eq!(parsed[1].name, "portal - prod");
        assert_eq!(parsed[1].host, "db.prod.internal");
        assert_eq!(parsed[1].default_database, "prod_core");
        assert_eq!(parsed[1].ssl_mode, SslMode::Required);
        assert!(parsed[1].ssh.enabled);
        assert_eq!(parsed[1].ssh.ssh_host, "bastion.example.com");
        assert_eq!(parsed[1].ssh.ssh_port, 2222);
        assert_eq!(parsed[1].ssh.ssh_user, "ec2-user");
        assert_eq!(parsed[1].ssh.ssh_key_path, "/home/user/.ssh/id_rsa");
        assert_eq!(parsed[1].environment, Environment::Production);
    }
}
