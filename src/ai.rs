use crate::config::{AiConfig, AiProvider};
use crate::db::DatabaseSchema;
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionKind {
    Table,
    Column,
    ForeignKeyJoin,
    Function,
    Keyword,
}

impl CompletionKind {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Table => "TBL",
            Self::Column => "COL",
            Self::ForeignKeyJoin => "JOIN",
            Self::Function => "FN",
            Self::Keyword => "KW",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub extracted_sql: Option<String>,
}

/// Computes instant (<1ms) context-aware schema & keyword completions for the current SQL prefix.
pub fn compute_local_completions(
    sql: &str,
    schema: Option<&DatabaseSchema>,
    max_items: usize,
) -> (String, Vec<CompletionItem>) {
    let trimmed_end = sql;
    let last_token = trimmed_end
        .rsplit(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == '=')
        .next()
        .unwrap_or("");

    if last_token.is_empty() {
        return (String::new(), Vec::new());
    }

    let prefix_lower = last_token.to_lowercase();
    let mut items = Vec::new();

    if let Some(db) = schema {
        // Check `alias.column` or `table.column` pattern
        if let Some((tbl_prefix, col_prefix)) = prefix_lower.split_once('.') {
            for t in &db.tables {
                let short_alias = t.name.chars().next().unwrap_or('t').to_string();
                if t.name.eq_ignore_ascii_case(tbl_prefix)
                    || short_alias.eq_ignore_ascii_case(tbl_prefix)
                {
                    for c in &t.columns {
                        if c.name.to_lowercase().starts_with(col_prefix) {
                            items.push(CompletionItem {
                                label: format!("{tbl_prefix}.{}", c.name),
                                insert_text: format!("{tbl_prefix}.{}", c.name),
                                detail: format!("{} ({})", t.name, c.data_type),
                                kind: CompletionKind::Column,
                            });
                        }
                    }
                }
            }
            items.truncate(max_items);
            return (last_token.to_string(), items);
        }

        // Suggest Foreign Key JOINs if user is typing `JOIN`
        let upper_before = sql.to_uppercase();
        if upper_before.ends_with("JOIN ") || "join".starts_with(&prefix_lower) {
            for t in &db.tables {
                for fk in &t.foreign_keys {
                    let join_snippet = format!(
                        "JOIN {} ON {}.{} = {}.{}",
                        fk.referenced_table,
                        t.name,
                        fk.column_name,
                        fk.referenced_table,
                        fk.referenced_column
                    );
                    items.push(CompletionItem {
                        label: join_snippet.clone(),
                        insert_text: join_snippet,
                        detail: format!("FK {}", fk.constraint_name),
                        kind: CompletionKind::ForeignKeyJoin,
                    });
                }
            }
        }

        // Suggest tables
        for t in &db.tables {
            if t.name.to_lowercase().starts_with(&prefix_lower) {
                items.push(CompletionItem {
                    label: t.name.clone(),
                    insert_text: t.name.clone(),
                    detail: format!("Table (~{} rows)", t.approx_rows),
                    kind: CompletionKind::Table,
                });
            }
        }

        // Suggest columns across tables
        for t in &db.tables {
            for c in &t.columns {
                if c.name.to_lowercase().starts_with(&prefix_lower) {
                    items.push(CompletionItem {
                        label: c.name.clone(),
                        insert_text: c.name.clone(),
                        detail: format!("{}.{} ({})", t.name, c.name, c.data_type),
                        kind: CompletionKind::Column,
                    });
                }
            }
        }
    }

    let keywords = [
        ("SELECT", "Query columns"),
        ("FROM", "Target table"),
        ("WHERE", "Filter rows"),
        ("JOIN", "Inner join table"),
        ("LEFT JOIN", "Left outer join"),
        ("GROUP BY", "Aggregate grouping"),
        ("ORDER BY", "Sort result set"),
        ("HAVING", "Filter aggregates"),
        ("LIMIT", "Restrict row count"),
        ("INSERT INTO", "Insert rows"),
        ("UPDATE", "Modify rows"),
        ("DELETE FROM", "Remove rows"),
        ("EXPLAIN", "Inspect query execution plan"),
        ("SHOW CREATE TABLE", "Inspect table DDL"),
        ("COUNT(*)", "Aggregate row count"),
        ("SUM()", "Sum numeric expression"),
        ("ROUND()", "Round decimal"),
        ("COALESCE()", "Fallback on NULL"),
    ];

    for (kw, desc) in keywords {
        if kw.to_lowercase().starts_with(&prefix_lower) && kw.len() > last_token.len() {
            items.push(CompletionItem {
                label: kw.to_string(),
                insert_text: kw.to_string(),
                detail: desc.to_string(),
                kind: if kw.contains('(') {
                    CompletionKind::Function
                } else {
                    CompletionKind::Keyword
                },
            });
        }
    }

    items.truncate(max_items);
    (last_token.to_string(), items)
}

/// Computes an instant heuristic ghost completion when offline or while waiting for LLM response.
pub fn compute_heuristic_ghost_completion(
    sql: &str,
    schema: Option<&DatabaseSchema>,
) -> Option<String> {
    let trimmed = sql.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let upper = trimmed.to_uppercase();

    if let Some(db) = schema {
        let first_table = db
            .tables
            .first()
            .map(|t| t.name.as_str())
            .unwrap_or("users");

        if upper == "SELECT *" || upper == "SELECT * " {
            return Some(format!(" FROM {first_table} LIMIT 50;"));
        }
        if upper.ends_with("FROM") {
            return Some(format!(" {first_table} WHERE id = 1 LIMIT 50;"));
        }
        for t in &db.tables {
            let pattern = format!("FROM {}", t.name.to_uppercase());
            if upper.ends_with(&pattern) {
                if let Some(fk) = t.foreign_keys.first() {
                    return Some(format!(
                        "\nJOIN {} ON {}.{} = {}.{}\nLIMIT 50;",
                        fk.referenced_table,
                        t.name,
                        fk.column_name,
                        fk.referenced_table,
                        fk.referenced_column
                    ));
                }
                let pk = t
                    .columns
                    .first()
                    .map(|c| c.name.as_str())
                    .unwrap_or("id");
                return Some(format!("\nORDER BY {pk} DESC\nLIMIT 50;"));
            }
        }
    }
    None
}

/// Calls the configured AI provider (Ollama, OpenAI/Compatible, Anthropic, Gemini, or CLI like `agy`)
/// to generate a Copilot-style inline SQL completion suffix.
pub async fn request_inline_completion(
    config: &AiConfig,
    sql_prefix: &str,
    schema_ddl: &str,
) -> Result<String> {
    let system_prompt = "You are an expert MySQL 8.0 / MariaDB inline code completion engine. \
        Output ONLY the exact SQL continuation that should be appended immediately after the user's cursor. \
        Do NOT repeat the prefix already written by the user. Do NOT wrap in markdown code fences or add explanations.";

    let user_prompt = format!(
        "Active Database Schema:\n{schema_ddl}\n\nCurrent SQL before cursor:\n{sql_prefix}\n\nContinuation:"
    );

    match call_provider(config, system_prompt, &user_prompt).await {
        Ok(raw) => Ok(clean_inline_completion(&raw, sql_prefix)),
        Err(err) => {
            // If local heuristic can provide a helpful suggestion, surface error in log or fallback
            Err(err)
        }
    }
}

/// Calls the configured AI provider for the interactive SQL Assistant / Chat panel.
pub async fn request_chat_assistant(
    config: &AiConfig,
    user_message: &str,
    current_editor_sql: &str,
    schema_ddl: &str,
) -> Result<ChatMessage> {
    let system_prompt = "You are a senior MySQL/MariaDB database engineer built into MySQL-Rusty Workbench. \
        Always write clean, optimized MySQL 8.0 / MariaDB SQL queries inside ```sql ... ``` fenced blocks, \
        followed by a concise explanation of indexes, joins, or performance trade-offs in English.";

    let mut full_prompt = String::new();
    if config.include_schema_context && !schema_ddl.trim().is_empty() {
        full_prompt.push_str("Active Database Schema:\n");
        full_prompt.push_str(schema_ddl);
        full_prompt.push_str("\n\n");
    }
    if !current_editor_sql.trim().is_empty() {
        full_prompt.push_str("Current SQL in Editor:\n```sql\n");
        full_prompt.push_str(current_editor_sql.trim());
        full_prompt.push_str("\n```\n\n");
    }
    full_prompt.push_str("User Request:\n");
    full_prompt.push_str(user_message);

    let reply = call_provider(config, system_prompt, &full_prompt).await?;
    let extracted_sql = extract_sql_code_block(&reply);

    Ok(ChatMessage {
        role: "assistant".to_string(),
        content: reply,
        extracted_sql,
    })
}

async fn call_provider(
    config: &AiConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    match config.provider {
        AiProvider::Ollama => {
            let url = format!(
                "{}/api/generate",
                config.ollama_endpoint.trim_end_matches('/')
            );
            let body = json!({
                "model": config.ollama_model,
                "system": system_prompt,
                "prompt": user_prompt,
                "stream": false,
                "options": {
                    "temperature": 0.15
                }
            });
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .with_context(|| format!("Failed to reach Ollama at {url}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Ollama returned HTTP {status}: {text}"));
            }
            let val: serde_json::Value = resp.json().await?;
            Ok(val["response"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string())
        }

        AiProvider::OpenAiCompatible => {
            let url = format!(
                "{}/chat/completions",
                config.openai_base_url.trim_end_matches('/')
            );
            let body = json!({
                "model": config.openai_model,
                "temperature": 0.15,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ]
            });
            let mut req = client.post(&url).json(&body);
            if !config.openai_api_key.trim().is_empty() {
                req = req.bearer_auth(config.openai_api_key.trim());
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("OpenAI-compatible API HTTP {status}: {text}"));
            }
            let val: serde_json::Value = resp.json().await?;
            Ok(val["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string())
        }

        AiProvider::Anthropic => {
            if config.anthropic_api_key.trim().is_empty() {
                return Err(anyhow!("Anthropic API Key is empty in AI Settings."));
            }
            let body = json!({
                "model": config.anthropic_model,
                "max_tokens": 1024,
                "system": system_prompt,
                "messages": [
                    { "role": "user", "content": user_prompt }
                ]
            });
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", config.anthropic_api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Anthropic API HTTP {status}: {text}"));
            }
            let val: serde_json::Value = resp.json().await?;
            Ok(val["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string())
        }

        AiProvider::Gemini => {
            if config.gemini_api_key.trim().is_empty() {
                return Err(anyhow!("Gemini API Key is empty in AI Settings."));
            }
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                config.gemini_model.trim(),
                config.gemini_api_key.trim()
            );
            let combined = format!("{system_prompt}\n\n{user_prompt}");
            let body = json!({
                "contents": [{
                    "parts": [{ "text": combined }]
                }]
            });
            let resp = client.post(&url).json(&body).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("Gemini API HTTP {status}: {text}"));
            }
            let val: serde_json::Value = resp.json().await?;
            Ok(val["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string())
        }

        AiProvider::CliCommand => {
            let cmd_line = config.cli_command.trim();
            if cmd_line.is_empty() {
                return Err(anyhow!("CLI command is empty in AI Settings."));
            }
            let combined_prompt = format!("{system_prompt}\n\n{user_prompt}");
            let parts: Vec<&str> = cmd_line.split_whitespace().collect();
            let binary = parts[0];
            let args = &parts[1..];

            let mut cmd = Command::new(binary);
            cmd.args(args);

            if config.cli_pass_via_stdin {
                cmd.stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let mut child = cmd
                    .spawn()
                    .with_context(|| format!("Failed to spawn CLI command `{binary}`"))?;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(combined_prompt.as_bytes()).await?;
                }
                let output = child.wait_with_output().await?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow!("CLI `{binary}` exited with error: {stderr}"));
                }
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                cmd.arg(&combined_prompt);
                let output = cmd
                    .output()
                    .await
                    .with_context(|| format!("Failed to run CLI command `{binary}`"))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow!("CLI `{binary}` exited with error: {stderr}"));
                }
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
        }
    }
}

fn clean_inline_completion(raw: &str, prefix: &str) -> String {
    let mut cleaned = raw.trim().to_string();
    if let Some(block) = extract_sql_code_block(&cleaned) {
        cleaned = block;
    }
    cleaned = cleaned
        .trim_start_matches("```sql")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();

    let trimmed_prefix = prefix.trim();
    if !trimmed_prefix.is_empty() && cleaned.starts_with(trimmed_prefix) {
        cleaned = cleaned[trimmed_prefix.len()..].to_string();
    }
    if !prefix.ends_with(char::is_whitespace) && !cleaned.starts_with(' ') && !cleaned.is_empty() {
        format!(" {cleaned}")
    } else {
        cleaned
    }
}

pub fn extract_sql_code_block(markdown: &str) -> Option<String> {
    if let Some(start_idx) = markdown.find("```sql") {
        let after = &markdown[start_idx + 6..];
        if let Some(end_idx) = after.find("```") {
            return Some(after[..end_idx].trim().to_string());
        }
    }
    if let Some(start_idx) = markdown.find("```") {
        let after = &markdown[start_idx + 3..];
        if let Some(end_idx) = after.find("```") {
            let candidate = after[..end_idx].trim();
            if candidate.to_uppercase().contains("SELECT")
                || candidate.to_uppercase().contains("UPDATE")
                || candidate.to_uppercase().contains("INSERT")
                || candidate.to_uppercase().contains("CREATE")
                || candidate.to_uppercase().contains("ALTER")
            {
                return Some(candidate.to_string());
            }
        }
    }
    None
}
