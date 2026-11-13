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
    #[serde(default = "default_true")]
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

fn default_ui_scale() -> f32 {
    1.0
}

fn default_editor_font_size() -> f32 {
    14.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    #[serde(default)]
    pub vim_mode: bool,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            vim_mode: false,
            theme: ThemePreference::Dark,
            ui_scale: default_ui_scale(),
            editor_font_size: default_editor_font_size(),
        }
    }
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

        cfg
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
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

        let json = serde_json::to_string_pretty(&config_to_save)?;
        fs::write(path, json)?;
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_keyring_fallback() {
        use crate::config::{delete_secret, get_secret, set_secret};
        let _ = set_secret("test_key", "secret_value");
        let _ = get_secret("test_key");
        let _ = delete_secret("test_key");
    }
}
