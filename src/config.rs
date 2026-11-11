use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SslMode {
    Disabled,
    Preferred,
    Required,
}

impl Default for SslMode {
    fn default() -> Self {
        Self::Preferred
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
    pub ssh: SshTunnelConfig,
    pub is_demo: bool,
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
            ssh: SshTunnelConfig::default(),
            is_demo: false,
        }
    }
}

impl ConnectionProfile {
    pub fn demo_profile() -> Self {
        Self {
            id: "demo-sandbox-mysql".to_string(),
            name: "⚡ Sandbox Demo (ecommerce_prod)".to_string(),
            color_hex: "#10b981".to_string(),
            host: "sandbox.local".to_string(),
            port: 3306,
            user: "demo_admin".to_string(),
            password: String::new(),
            default_database: "ecommerce_prod".to_string(),
            ssl_mode: SslMode::Disabled,
            ssh: SshTunnelConfig::default(),
            is_demo: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AiProvider {
    Ollama,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    CliCommand,
}

impl AiProvider {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama (Local)",
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

    // Ollama
    pub ollama_endpoint: String,
    pub ollama_model: String,

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

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::Ollama,
            enable_inline_autocomplete: true,
            include_schema_context: true,
            ollama_endpoint: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5-coder:7b".to_string(),
            openai_base_url: "https://api.openai.com/v1".to_string(),
            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_model: "gpt-4o-mini".to_string(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            anthropic_model: "claude-3-7-sonnet-latest".to_string(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            gemini_model: "gemini-2.5-flash".to_string(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub connections: Vec<ConnectionProfile>,
    pub ai: AiConfig,
    pub history: Vec<QueryHistoryEntry>,
    pub snippets: Vec<SavedSnippet>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            connections: vec![
                ConnectionProfile::demo_profile(),
                ConnectionProfile::default(),
            ],
            ai: AiConfig::default(),
            history: Vec::new(),
            snippets: vec![
                SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title: "Top Customers by Revenue (JOIN + GROUP BY)".to_string(),
                    sql: "SELECT \n    u.id,\n    u.full_name,\n    u.email,\n    u.country,\n    COUNT(o.id) AS total_orders,\n    ROUND(SUM(o.total_amount), 2) AS lifetime_value\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE o.status = 'paid'\nGROUP BY u.id, u.full_name, u.email, u.country\nORDER BY lifetime_value DESC\nLIMIT 25;".to_string(),
                },
                SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title: "Inspect Table & Index Sizes".to_string(),
                    sql: "SELECT \n    table_name AS `Table`,\n    engine AS `Engine`,\n    table_rows AS `Approx Rows`,\n    ROUND((data_length + index_length) / 1024 / 1024, 2) AS `Size (MB)`\nFROM information_schema.TABLES\nWHERE table_schema = DATABASE()\nORDER BY (data_length + index_length) DESC;".to_string(),
                },
            ],
        }
    }
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("mysql-rusty").join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Self>(&content) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }
}
