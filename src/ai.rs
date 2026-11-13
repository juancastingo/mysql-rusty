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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlClause {
    FromOrJoin,
    On,
    SelectOrExpr,
    StartOrUnknown,
}

#[derive(Debug, Clone)]
struct TableRef {
    table: String,
    alias: Option<String>,
}

fn current_statement_prefix(sql_prefix: &str) -> &str {
    let bytes = sql_prefix.as_bytes();
    let mut in_sq = false;
    let mut in_dq = false;
    let mut in_bt = false;
    let mut last_semi_end = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && (in_sq || in_dq) {
            i += 2;
            continue;
        }
        match b {
            b'\'' if !in_dq && !in_bt => in_sq = !in_sq,
            b'"' if !in_sq && !in_bt => in_dq = !in_dq,
            b'`' if !in_sq && !in_dq => in_bt = !in_bt,
            b';' if !in_sq && !in_dq && !in_bt => last_semi_end = i + 1,
            _ => {}
        }
        i += 1;
    }
    &sql_prefix[last_semi_end..]
}

fn strip_ident_quotes(s: &str) -> &str {
    s.trim_matches(|c| c == '`' || c == '"' || c == ',' || c == ';')
}

fn is_sql_clause_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "SELECT"
            | "FROM"
            | "WHERE"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "CROSS"
            | "OUTER"
            | "ON"
            | "USING"
            | "GROUP"
            | "ORDER"
            | "BY"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "UNION"
            | "ALL"
            | "SET"
            | "VALUES"
            | "INTO"
            | "AS"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "IS"
            | "NULL"
            | "LIKE"
            | "BETWEEN"
            | "EXISTS"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "FOR"
            | "LOCK"
    )
}

fn extract_table_refs_in_scope(stmt: &str) -> Vec<TableRef> {
    let raw_words: Vec<&str> = stmt
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .filter(|w| !w.is_empty())
        .collect();

    let mut refs = Vec::new();
    let mut i = 0;
    while i < raw_words.len() {
        let upper = raw_words[i].to_uppercase();
        let expects_table = matches!(upper.as_str(), "FROM" | "JOIN" | "UPDATE" | "INTO");
        if expects_table && i + 1 < raw_words.len() {
            let tbl_raw = strip_ident_quotes(raw_words[i + 1]);
            let tbl_upper = tbl_raw.to_uppercase();
            if !tbl_raw.is_empty()
                && !is_sql_clause_keyword(&tbl_upper)
                && !tbl_raw.starts_with('(')
            {
                let mut alias = None;
                if i + 2 < raw_words.len() {
                    let next_raw = strip_ident_quotes(raw_words[i + 2]);
                    let next_upper = next_raw.to_uppercase();
                    if next_upper == "AS" {
                        if i + 3 < raw_words.len() {
                            let al = strip_ident_quotes(raw_words[i + 3]);
                            if !al.is_empty() && !is_sql_clause_keyword(&al.to_uppercase()) {
                                alias = Some(al.to_string());
                            }
                        }
                    } else if !next_raw.is_empty() && !is_sql_clause_keyword(&next_upper) {
                        alias = Some(next_raw.to_string());
                    }
                }
                refs.push(TableRef {
                    table: tbl_raw.to_string(),
                    alias,
                });
            }
        }
        i += 1;
    }
    refs
}

fn detect_sql_clause(stmt_before_token: &str) -> SqlClause {
    let trimmed = stmt_before_token.trim_end();
    if trimmed.is_empty() {
        return SqlClause::StartOrUnknown;
    }
    if trimmed.ends_with(',') || trimmed.ends_with('(') || trimmed.ends_with('=') {
        return SqlClause::SelectOrExpr;
    }
    let words: Vec<String> = trimmed
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_uppercase())
        .collect();

    if let Some(last) = words.last() {
        if matches!(last.as_str(), "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE") {
            return SqlClause::FromOrJoin;
        }
        if matches!(last.as_str(), "ON" | "USING") {
            return SqlClause::On;
        }
        if matches!(
            last.as_str(),
            "SELECT"
                | "DISTINCT"
                | "WHERE"
                | "AND"
                | "OR"
                | "HAVING"
                | "SET"
                | "BY"
                | "CASE"
                | "WHEN"
                | "THEN"
                | "ELSE"
        ) {
            return SqlClause::SelectOrExpr;
        }
    }

    // Walk backwards to find the most recent major clause keyword
    for w in words.iter().rev() {
        match w.as_str() {
            "FROM" | "JOIN" | "INTO" | "UPDATE" => {
                // If user already typed a table name after FROM/JOIN, next token is likely a keyword or alias
                return SqlClause::StartOrUnknown;
            }
            "ON" => return SqlClause::On,
            "SELECT" | "WHERE" | "HAVING" | "SET" => return SqlClause::SelectOrExpr,
            _ => {}
        }
    }
    SqlClause::StartOrUnknown
}

fn match_score(candidate: &str, query_lower: &str) -> Option<i32> {
    if query_lower.is_empty() {
        return Some(10);
    }
    let cand_lower = candidate.to_lowercase();
    if cand_lower == query_lower {
        return Some(130);
    }
    if cand_lower.starts_with(query_lower) {
        return Some(100);
    }
    // Word-boundary match after `_` (e.g. `id` -> `user_id`, `at` -> `created_at`)
    if cand_lower.contains(&format!("_{query_lower}")) {
        return Some(65);
    }
    if cand_lower.contains(query_lower) {
        return Some(35);
    }
    None
}

/// Computes instant (<1ms) context-aware schema & keyword completions for the current SQL prefix.
pub fn compute_local_completions(
    sql_prefix: &str,
    schema: Option<&DatabaseSchema>,
    max_items: usize,
) -> (String, Vec<CompletionItem>) {
    let stmt = current_statement_prefix(sql_prefix);
    let last_token = stmt
        .rsplit(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == '=')
        .next()
        .unwrap_or("");

    let prefix_lower = last_token.to_lowercase();
    let stmt_before_token = &stmt[..stmt.len().saturating_sub(last_token.len())];
    let clause = detect_sql_clause(stmt_before_token);
    let table_refs = extract_table_refs_in_scope(stmt);

    // Avoid spamming suggestions on completely empty buffer
    if last_token.is_empty() && stmt.trim().is_empty() {
        return (String::new(), Vec::new());
    }

    let mut scored: Vec<(i32, CompletionItem)> = Vec::new();

    if let Some(db) = schema {
        // 1. Check `alias.column` or `table.column` pattern
        if let Some((tbl_prefix, col_prefix)) = prefix_lower.split_once('.') {
            // Prefer explicit aliases / table names in scope first
            let mut matched_tables: Vec<(&crate::db::TableSchema, i32)> = Vec::new();
            for tr in &table_refs {
                let alias_match = tr
                    .alias
                    .as_deref()
                    .map(|a| a.eq_ignore_ascii_case(tbl_prefix))
                    .unwrap_or(false);
                if (alias_match || tr.table.eq_ignore_ascii_case(tbl_prefix))
                    && let Some(t) = db
                        .tables
                        .iter()
                        .find(|t| t.name.eq_ignore_ascii_case(&tr.table))
                {
                    matched_tables.push((t, 50));
                }
            }
            if matched_tables.is_empty() {
                for t in &db.tables {
                    let short_alias = t.name.chars().next().unwrap_or('t').to_string();
                    if t.name.eq_ignore_ascii_case(tbl_prefix) {
                        matched_tables.push((t, 30));
                    } else if short_alias.eq_ignore_ascii_case(tbl_prefix) {
                        matched_tables.push((t, 10));
                    }
                }
            }

            let raw_tbl_prefix = last_token.split('.').next().unwrap_or(tbl_prefix);
            for (t, scope_bonus) in matched_tables {
                for c in &t.columns {
                    if let Some(m_score) = match_score(&c.name, col_prefix) {
                        scored.push((
                            m_score + scope_bonus,
                            CompletionItem {
                                label: format!("{raw_tbl_prefix}.{}", c.name),
                                insert_text: format!("{raw_tbl_prefix}.{}", c.name),
                                detail: format!("{} ({})", t.name, c.data_type),
                                kind: CompletionKind::Column,
                            },
                        ));
                    }
                }
            }
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
            let items: Vec<CompletionItem> =
                scored.into_iter().take(max_items).map(|(_, i)| i).collect();
            return (last_token.to_string(), items);
        }

        // If no token is typed yet, only suggest when we're in a clear clause (FROM, JOIN, ON, WHERE, SELECT, etc.)
        if last_token.is_empty() && clause == SqlClause::StartOrUnknown {
            return (String::new(), Vec::new());
        }

        // 2. Suggest Foreign Key JOINs if user is typing `JOIN` or after `JOIN ` / `ON `
        let upper_before = stmt_before_token.trim_end().to_uppercase();
        let in_join_ctx = upper_before.ends_with("JOIN")
            || (!prefix_lower.is_empty() && "join".starts_with(&prefix_lower));
        if in_join_ctx {
            for t in &db.tables {
                let t_in_scope = table_refs
                    .iter()
                    .any(|r| r.table.eq_ignore_ascii_case(&t.name));
                for fk in &t.foreign_keys {
                    let join_snippet = if upper_before.ends_with("JOIN") {
                        format!(
                            "{} ON {}.{} = {}.{}",
                            fk.referenced_table,
                            t.name,
                            fk.column_name,
                            fk.referenced_table,
                            fk.referenced_column
                        )
                    } else {
                        format!(
                            "JOIN {} ON {}.{} = {}.{}",
                            fk.referenced_table,
                            t.name,
                            fk.column_name,
                            fk.referenced_table,
                            fk.referenced_column
                        )
                    };
                    let bonus = if t_in_scope { 145 } else { 85 };
                    scored.push((
                        bonus,
                        CompletionItem {
                            label: join_snippet.clone(),
                            insert_text: join_snippet,
                            detail: format!("FK {}", fk.constraint_name),
                            kind: CompletionKind::ForeignKeyJoin,
                        },
                    ));
                }
            }
        }

        // 3. Suggest ON conditions between tables in scope
        if clause == SqlClause::On && !table_refs.is_empty() {
            for tr in &table_refs {
                if let Some(t) = db
                    .tables
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(&tr.table))
                {
                    let lhs = tr.alias.as_deref().unwrap_or(&t.name);
                    for fk in &t.foreign_keys {
                        let rhs_ref = table_refs
                            .iter()
                            .find(|r| r.table.eq_ignore_ascii_case(&fk.referenced_table));
                        let rhs = rhs_ref
                            .and_then(|r| r.alias.as_deref())
                            .unwrap_or(&fk.referenced_table);
                        let cond =
                            format!("{lhs}.{} = {rhs}.{}", fk.column_name, fk.referenced_column);
                        if match_score(&cond, &prefix_lower).is_some() || last_token.is_empty() {
                            scored.push((
                                150,
                                CompletionItem {
                                    label: cond.clone(),
                                    insert_text: cond,
                                    detail: format!("FK {}", fk.constraint_name),
                                    kind: CompletionKind::ForeignKeyJoin,
                                },
                            ));
                        }
                    }
                }
            }
        }

        // 4. Suggest tables (prioritized in FROM / JOIN)
        let table_clause_bonus = match clause {
            SqlClause::FromOrJoin => 60,
            SqlClause::StartOrUnknown => 20,
            _ => -10,
        };
        for t in &db.tables {
            if let Some(m_score) = match_score(&t.name, &prefix_lower) {
                scored.push((
                    m_score + table_clause_bonus,
                    CompletionItem {
                        label: t.name.clone(),
                        insert_text: t.name.clone(),
                        detail: format!("Table (~{} rows)", t.approx_rows),
                        kind: CompletionKind::Table,
                    },
                ));
            }
        }

        // 5. Suggest columns (strongly prioritize tables currently in FROM / JOIN)
        if clause != SqlClause::FromOrJoin {
            let col_clause_bonus = match clause {
                SqlClause::SelectOrExpr | SqlClause::On => 35,
                _ => 0,
            };
            for t in &db.tables {
                let active_ref = table_refs
                    .iter()
                    .find(|r| r.table.eq_ignore_ascii_case(&t.name));
                let scope_bonus = if active_ref.is_some() { 55 } else { -15 };
                let use_alias_prefix =
                    table_refs.len() > 1 && active_ref.and_then(|r| r.alias.as_ref()).is_some();
                for c in &t.columns {
                    if let Some(m_score) = match_score(&c.name, &prefix_lower) {
                        let insert_text = if use_alias_prefix {
                            format!("{}.{}", active_ref.unwrap().alias.as_ref().unwrap(), c.name)
                        } else {
                            c.name.clone()
                        };
                        scored.push((
                            m_score + col_clause_bonus + scope_bonus,
                            CompletionItem {
                                label: insert_text.clone(),
                                insert_text,
                                detail: format!("{}.{} ({})", t.name, c.name, c.data_type),
                                kind: CompletionKind::Column,
                            },
                        ));
                    }
                }
            }
        }
    }

    if !last_token.is_empty() {
        let keywords = [
            ("SELECT", "Query columns"),
            ("FROM", "Target table"),
            ("WHERE", "Filter rows"),
            ("JOIN", "Inner join table"),
            ("LEFT JOIN", "Left outer join"),
            ("INNER JOIN", "Inner join table"),
            ("ON", "Join condition"),
            ("AND", "Logical AND"),
            ("OR", "Logical OR"),
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
            ("AVG()", "Average numeric expression"),
            ("ROUND()", "Round decimal"),
            ("COALESCE()", "Fallback on NULL"),
        ];

        let kw_bonus = match clause {
            SqlClause::StartOrUnknown => 25,
            SqlClause::SelectOrExpr => 5,
            _ => -10,
        };
        for (kw, desc) in keywords {
            if let Some(m_score) = match_score(kw, &prefix_lower)
                && (kw.len() > last_token.len() || !kw.eq_ignore_ascii_case(last_token))
            {
                scored.push((
                    m_score + kw_bonus,
                    CompletionItem {
                        label: kw.to_string(),
                        insert_text: kw.to_string(),
                        detail: desc.to_string(),
                        kind: if kw.contains('(') {
                            CompletionKind::Function
                        } else {
                            CompletionKind::Keyword
                        },
                    },
                ));
            }
        }
    }

    // Deduplicate by label (keeping highest score) and sort
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(max_items);
    for (_, item) in scored {
        if seen.insert(item.label.to_lowercase()) {
            items.push(item);
            if items.len() >= max_items {
                break;
            }
        }
    }
    (last_token.to_string(), items)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotDeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub async fn request_copilot_device_code() -> Result<CopilotDeviceCodeResponse> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .header("User-Agent", "GitHubCopilot/1.180.0")
        .json(&json!({
            "client_id": "01ab8ac9400c4e429b23",
            "scope": "read:user"
        }))
        .send()
        .await
        .context("Failed to request Copilot device code from GitHub")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("GitHub Device Code returned HTTP {status}: {body}"));
    }

    let res: CopilotDeviceCodeResponse = resp.json().await?;
    Ok(res)
}

pub async fn poll_copilot_device_token(device_code: &str) -> Result<Result<String, String>> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .header("User-Agent", "GitHubCopilot/1.180.0")
        .json(&json!({
            "client_id": "01ab8ac9400c4e429b23",
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
        }))
        .send()
        .await
        .context("Failed to poll token from GitHub")?;

    let val: serde_json::Value = resp.json().await?;
    if let Some(token) = val["access_token"].as_str() {
        Ok(Ok(token.to_string()))
    } else if let Some(err) = val["error"].as_str() {
        Ok(Err(err.to_string()))
    } else {
        Err(anyhow!("Unexpected response from GitHub: {val:?}"))
    }
}

pub async fn fetch_copilot_session_token(github_token: &str) -> Result<(String, u64)> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("token {github_token}"))
        .header("Editor-Version", "vscode/1.90.0")
        .header("Editor-Plugin-Version", "copilot/1.180.0")
        .header("User-Agent", "GitHubCopilot/1.180.0")
        .header("Accept", "application/json")
        .send()
        .await
        .context("Failed to fetch Copilot internal session token")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Copilot token exchange returned HTTP {status}: {body}"
        ));
    }

    let val: serde_json::Value = resp.json().await?;
    let token = val["token"]
        .as_str()
        .context("No session token in Copilot response")?
        .to_string();
    let expires_at = val["expires_at"].as_u64().unwrap_or(0);
    Ok((token, expires_at))
}

fn primary_col_of(t: &crate::db::TableSchema) -> &str {
    t.columns
        .iter()
        .find(|c| c.is_primary_key)
        .or_else(|| t.columns.first())
        .map(|c| c.name.as_str())
        .unwrap_or("id")
}

/// Computes an instant heuristic ghost completion when offline or while waiting for LLM response.
pub fn compute_heuristic_ghost_completion(
    sql_prefix: &str,
    schema: Option<&DatabaseSchema>,
) -> Option<String> {
    let stmt = current_statement_prefix(sql_prefix);
    let trimmed = stmt.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let has_trailing_ws = stmt.ends_with(char::is_whitespace);
    let sp = if has_trailing_ws { "" } else { " " };
    let upper = trimmed.to_uppercase();

    if let Some(db) = schema {
        let table_refs = extract_table_refs_in_scope(stmt);
        let active_table = table_refs.last().and_then(|r| {
            db.tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&r.table))
        });
        let active_col = active_table.map(primary_col_of).unwrap_or("id");
        let first_table = db
            .tables
            .first()
            .map(|t| t.name.as_str())
            .unwrap_or("users");

        // SELECT continuations
        if upper == "SELECT" {
            return Some(format!("{sp}* FROM {first_table} LIMIT 50;"));
        }
        if upper == "SELECT *" {
            return Some(format!("{sp}FROM {first_table} LIMIT 50;"));
        }
        if upper.ends_with("FROM") {
            return Some(format!("{sp}{first_table} LIMIT 50;"));
        }

        // FROM {table} -> smart JOIN or WHERE
        for t in &db.tables {
            let pattern = format!("FROM {}", t.name.to_uppercase());
            let pattern_tick = format!("FROM `{}`", t.name.to_uppercase());
            if upper.ends_with(&pattern) || upper.ends_with(&pattern_tick) {
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
                for other in &db.tables {
                    if let Some(fk) = other
                        .foreign_keys
                        .iter()
                        .find(|k| k.referenced_table.eq_ignore_ascii_case(&t.name))
                    {
                        return Some(format!(
                            "\nJOIN {} ON {}.{} = {}.{}\nLIMIT 50;",
                            other.name, other.name, fk.column_name, t.name, fk.referenced_column
                        ));
                    }
                }
                let pk = primary_col_of(t);
                return Some(format!("\nWHERE {pk} = 1\nLIMIT 50;"));
            }
        }

        // JOIN continuations (prefer active table in scope)
        if upper.ends_with("JOIN") {
            if let Some(t) = active_table
                && let Some(fk) = t.foreign_keys.first()
            {
                return Some(format!(
                    "{sp}{} ON {}.{} = {}.{}",
                    fk.referenced_table,
                    t.name,
                    fk.column_name,
                    fk.referenced_table,
                    fk.referenced_column
                ));
            }
            for t in &db.tables {
                if let Some(fk) = t.foreign_keys.first() {
                    return Some(format!(
                        "{sp}{} ON {}.{} = {}.{}",
                        fk.referenced_table,
                        t.name,
                        fk.column_name,
                        fk.referenced_table,
                        fk.referenced_column
                    ));
                }
            }
        }

        // WHERE continuations
        if upper.ends_with("WHERE") {
            return Some(format!("{sp}{active_col} = 1 LIMIT 50;"));
        }

        // ORDER BY continuations
        if upper.ends_with("ORDER BY") {
            return Some(format!("{sp}{active_col} DESC LIMIT 50;"));
        }

        // GROUP BY continuations
        if upper.ends_with("GROUP BY") {
            return Some(format!("{sp}{active_col} ORDER BY count(*) DESC LIMIT 50;"));
        }

        // INSERT INTO {table}
        for t in &db.tables {
            let pattern = format!("INSERT INTO {}", t.name.to_uppercase());
            if upper.ends_with(&pattern) {
                let cols: Vec<&str> = t.columns.iter().take(4).map(|c| c.name.as_str()).collect();
                let vals: Vec<&str> = (0..cols.len()).map(|_| "?").collect();
                return Some(format!(
                    " ({})\nVALUES ({});",
                    cols.join(", "),
                    vals.join(", ")
                ));
            }
        }

        // UPDATE {table}
        for t in &db.tables {
            let pattern = format!("UPDATE {}", t.name.to_uppercase());
            if upper.ends_with(&pattern) {
                let pk = primary_col_of(t);
                let col = t
                    .columns
                    .iter()
                    .find(|c| c.name != pk)
                    .map(|c| c.name.as_str())
                    .unwrap_or("name");
                return Some(format!("{sp}SET {col} = ? WHERE {pk} = ?;"));
            }
        }

        // DELETE FROM {table}
        for t in &db.tables {
            let pattern = format!("DELETE FROM {}", t.name.to_uppercase());
            if upper.ends_with(&pattern) {
                let pk = primary_col_of(t);
                return Some(format!("{sp}WHERE {pk} = ? LIMIT 1;"));
            }
        }
    } else {
        if upper == "SELECT *" {
            return Some(format!("{sp}FROM users WHERE id = 1 LIMIT 50;"));
        }
        if upper.ends_with("FROM") {
            return Some(format!("{sp}users WHERE id = 1 LIMIT 50;"));
        }
    }

    if upper == "SHOW" {
        return Some(format!("{sp}TABLES;"));
    }
    if upper == "EXPLAIN" {
        return Some(format!("{sp}SELECT * FROM users LIMIT 50;"));
    }

    None
}

/// Calls the configured AI provider (Ollama, GitHub Copilot, OpenAI/Compatible, Anthropic, Gemini, or CLI like `agy`)
/// to generate a Copilot-style inline SQL completion suffix.
pub async fn request_inline_completion(
    config: &AiConfig,
    sql_prefix: &str,
    sql_suffix: &str,
    schema_ddl: &str,
) -> Result<String> {
    if config.provider == AiProvider::LocalSchema {
        return Ok(String::new());
    }

    let system_prompt = "You are a code completion model for MySQL 8.0 / MariaDB SQL. \
        Output ONLY the exact SQL continuation that should be inserted at <CURSOR>. \
        Never repeat words, keywords, clauses, table names, or column names already written before or after <CURSOR>. \
        Do NOT output markdown fences, comments, or explanations.";

    let user_prompt = if sql_suffix.trim().is_empty() {
        format!(
            "/* Active Database Schema */\n{schema_ddl}\n\n/* Complete the SQL query directly from cursor: */\n{sql_prefix}<CURSOR>\n\n/* Continuation: */"
        )
    } else {
        format!(
            "/* Active Database Schema */\n{schema_ddl}\n\n/* Fill in the missing SQL at <CURSOR> without repeating surrounding text: */\n{sql_prefix}<CURSOR>{sql_suffix}\n\n/* Insertion at <CURSOR>: */"
        )
    };

    let started = std::time::Instant::now();
    let prompt_len = user_prompt.len();
    match call_provider(config, system_prompt, &user_prompt, true).await {
        Ok(raw) => {
            tracing::debug!(
                provider = ?config.provider,
                prompt_len,
                latency_ms = started.elapsed().as_millis() as u64,
                "AI inline completion finished"
            );
            Ok(clean_inline_completion_with_suffix(
                &raw, sql_prefix, sql_suffix,
            ))
        }
        Err(err) => {
            tracing::warn!(
                provider = ?config.provider,
                prompt_len,
                latency_ms = started.elapsed().as_millis() as u64,
                error = %err,
                "AI inline completion failed"
            );
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

    let started = std::time::Instant::now();
    let prompt_len = full_prompt.len();
    let reply = match call_provider(config, system_prompt, &full_prompt, false).await {
        Ok(r) => {
            tracing::info!(
                provider = ?config.provider,
                prompt_len,
                latency_ms = started.elapsed().as_millis() as u64,
                "AI chat request finished"
            );
            r
        }
        Err(err) => {
            tracing::warn!(
                provider = ?config.provider,
                prompt_len,
                latency_ms = started.elapsed().as_millis() as u64,
                error = %err,
                "AI chat request failed"
            );
            return Err(err);
        }
    };
    let extracted_sql = extract_sql_code_block(&reply);

    Ok(ChatMessage {
        role: "assistant".to_string(),
        content: reply,
        extracted_sql,
    })
}

#[derive(Debug, Clone)]
pub struct Nl2SqlResult {
    pub sql: String,
    pub explanation: String,
    pub assumptions: Vec<String>,
}

pub fn build_nl2sql_schema_context(schema: &DatabaseSchema, max_chars: usize) -> String {
    let raw = schema.to_ai_ddl_context();
    if raw.len() <= max_chars {
        return raw;
    }
    let mut out = String::new();
    for line in raw.lines() {
        if out.len() + line.len() + 1 > max_chars {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn parse_nl2sql_response(raw: &str) -> Nl2SqlResult {
    let extracted = extract_sql_code_block(raw).unwrap_or_else(|| raw.trim().to_string());
    let sql = crate::syntax::format_sql_pretty(&extracted);
    let mut assumptions = Vec::new();
    let mut explanation_lines = Vec::new();
    let mut in_fence = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.is_empty() {
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            assumptions.push(bullet.trim().to_string());
        } else {
            explanation_lines.push(trimmed.to_string());
        }
    }

    let explanation = if explanation_lines.is_empty() {
        "Generated SQL query from your natural language prompt.".to_string()
    } else {
        explanation_lines.join(" ")
    };

    Nl2SqlResult {
        sql,
        explanation,
        assumptions,
    }
}

pub async fn generate_sql_from_natural_language(
    prompt: &str,
    schema: &DatabaseSchema,
    config: &AiConfig,
) -> Result<Nl2SqlResult> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Prompt is empty."));
    }

    if config.provider == AiProvider::LocalSchema {
        let lower = trimmed.to_lowercase();
        let matched_table = schema
            .tables
            .iter()
            .find(|t| lower.contains(&t.name.to_lowercase()))
            .or_else(|| schema.tables.first());
        let sql = if let Some(tbl) = matched_table {
            let cols: Vec<String> = tbl
                .columns
                .iter()
                .take(6)
                .map(|c| format!("`{}`", c.name))
                .collect();
            let select_cols = if cols.is_empty() {
                "*".to_string()
            } else {
                cols.join(", ")
            };
            let limit = if lower.contains("top 5") { 5 } else { 25 };
            format!(
                "SELECT\n  {select_cols}\nFROM `{}`\nLIMIT {limit};",
                tbl.name
            )
        } else {
            "SELECT 1;".to_string()
        };
        return Ok(Nl2SqlResult {
            sql,
            explanation: format!(
                "Synthesized locally from schema `{}` (configure Ollama/OpenAI/Anthropic/Gemini/CLI for full NL2SQL).",
                schema.name
            ),
            assumptions: vec![format!(
                "Target table: {}",
                matched_table.map(|t| t.name.as_str()).unwrap_or("(none)")
            )],
        });
    }

    let schema_ctx = build_nl2sql_schema_context(schema, 6_000);
    let system_prompt = "You are an expert MySQL/MariaDB SQL generator. \
        Translate the user's natural language request (English or Spanish) into a single accurate MySQL 8.0 / MariaDB query \
        grounded strictly in the provided schema. Output the SQL inside a ```sql ... ``` fenced block, \
        followed by a 1-sentence explanation and bullet points (`- ...`) for any inferred tables, joins, or filters.";
    let user_prompt = format!("Database Schema:\n{schema_ctx}\n\nRequest:\n{trimmed}");
    let reply = call_provider(config, system_prompt, &user_prompt, false).await?;
    Ok(parse_nl2sql_response(&reply))
}

async fn call_provider(
    config: &AiConfig,
    system_prompt: &str,
    user_prompt: &str,
    is_inline: bool,
) -> Result<String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(if is_inline {
            20
        } else {
            45
        }))
        .build()?;

    match config.provider {
        AiProvider::LocalSchema => {
            // Local Schema mode generates completions in 0ms without network calls
            Ok(String::new())
        }

        AiProvider::GitHubCopilot => {
            let gh_token = config.copilot_github_token.trim();
            if gh_token.is_empty() {
                return Err(anyhow!(
                    "Not logged in to GitHub Copilot. Please open AI Settings (Ctrl+Shift+A) and authenticate with your GitHub account."
                ));
            }

            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Refresh session token if missing or expiring within 60s
            let session_token = if config.copilot_session_token.is_empty()
                || config.copilot_token_expires_at <= now_secs + 60
            {
                let (new_token, _) = fetch_copilot_session_token(gh_token).await?;
                new_token
            } else {
                config.copilot_session_token.clone()
            };

            let model = if config.copilot_model.trim().is_empty() {
                "gpt-4o"
            } else {
                config.copilot_model.trim()
            };
            let max_tokens = if is_inline {
                config.autocomplete_max_tokens.clamp(10, 100)
            } else {
                1024
            };

            let mut body = json!({
                "model": model,
                "temperature": if is_inline { 0.1 } else { 0.2 },
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ],
                "max_tokens": max_tokens
            });
            if is_inline {
                body["stop"] = json!([";", "\n\n", "```"]);
            }

            let resp = client
                .post("https://api.githubcopilot.com/chat/completions")
                .header("Authorization", format!("Bearer {session_token}"))
                .header("Editor-Version", "vscode/1.90.0")
                .header("Editor-Plugin-Version", "copilot/1.180.0")
                .header("User-Agent", "GitHubCopilot/1.180.0")
                .header("Openai-Organization", "github-copilot")
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("GitHub Copilot API HTTP {status}: {text}"));
            }

            let val: serde_json::Value = resp.json().await?;
            Ok(val["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string())
        }

        AiProvider::Ollama => {
            let url = format!(
                "{}/api/generate",
                config.ollama_endpoint.trim_end_matches('/')
            );
            let max_tokens = config.autocomplete_max_tokens.clamp(10, 200);

            let body = if is_inline && config.ollama_use_fim {
                // Ultra-fast Fill-In-The-Middle (FIM) prompt for code models (Qwen2.5-Coder, DeepSeek)
                json!({
                    "model": config.ollama_model,
                    "prompt": format!("<|fim_prefix|>{user_prompt}<|fim_suffix|><|fim_middle|>"),
                    "raw": true,
                    "stream": false,
                    "options": {
                        "temperature": 0.1,
                        "num_predict": max_tokens,
                        "stop": [";", "\n\n", "```", "<|fim_pad|>", "<|endoftext|>", "<|file_separator|>"]
                    }
                })
            } else {
                let mut options = json!({
                    "temperature": if is_inline { 0.1 } else { 0.2 },
                });
                if is_inline {
                    options["num_predict"] = json!(max_tokens);
                    options["stop"] = json!([";", "\n\n", "```"]);
                }
                json!({
                    "model": config.ollama_model,
                    "system": system_prompt,
                    "prompt": user_prompt,
                    "stream": false,
                    "options": options,
                })
            };

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
            let mut body = json!({
                "model": config.openai_model,
                "temperature": if is_inline { 0.1 } else { 0.2 },
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ]
            });
            if is_inline {
                let max_tokens = config.autocomplete_max_tokens.clamp(10, 200);
                body["max_tokens"] = json!(max_tokens);
                body["stop"] = json!([";", "\n\n", "```"]);
            }
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
            let max_tokens = if is_inline {
                config.autocomplete_max_tokens.clamp(10, 200)
            } else {
                1024
            };
            let mut body = json!({
                "model": config.anthropic_model,
                "max_tokens": max_tokens,
                "system": system_prompt,
                "messages": [
                    { "role": "user", "content": user_prompt }
                ]
            });
            if is_inline {
                body["stop_sequences"] = json!([";\n", "\n\n", "```"]);
            }
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
            let mut body = json!({
                "contents": [{
                    "parts": [{ "text": combined }]
                }]
            });
            if is_inline {
                let max_tokens = config.autocomplete_max_tokens.clamp(10, 200);
                body["generationConfig"] = json!({
                    "temperature": 0.1,
                    "maxOutputTokens": max_tokens,
                    "stopSequences": [";\n", "\n\n", "```"]
                });
            }
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

#[derive(Debug, Clone, PartialEq)]
struct SqlOverlapToken {
    text: String,
    normalized: String,
    start: usize,
    end: usize,
}

fn tokenize_for_overlap(s: &str) -> Vec<SqlOverlapToken> {
    let mut tokens = Vec::new();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, ch) = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        // Quoted string or identifier: '...', "...", `...`
        if ch == '\'' || ch == '"' || ch == '`' {
            let quote = ch;
            let mut j = i + 1;
            while j < chars.len() && chars[j].1 != quote {
                if chars[j].1 == '\\' && j + 1 < chars.len() {
                    j += 2;
                } else {
                    j += 1;
                }
            }
            if j < chars.len() {
                j += 1; // include closing quote
            }
            let end = if j < chars.len() { chars[j].0 } else { s.len() };
            let raw = &s[start..end];
            let norm = raw
                .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                .to_lowercase();
            tokens.push(SqlOverlapToken {
                text: raw.to_string(),
                normalized: norm,
                start,
                end,
            });
            i = j;
            continue;
        }

        // Word (alphanumeric or underscore)
        if ch.is_alphanumeric() || ch == '_' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].1.is_alphanumeric() || chars[j].1 == '_') {
                j += 1;
            }
            let end = if j < chars.len() { chars[j].0 } else { s.len() };
            let raw = &s[start..end];
            tokens.push(SqlOverlapToken {
                text: raw.to_string(),
                normalized: raw.to_lowercase(),
                start,
                end,
            });
            i = j;
            continue;
        }

        // Compound operators like !=, <=, >=, <>
        let next_ch = if i + 1 < chars.len() {
            Some(chars[i + 1].1)
        } else {
            None
        };
        if let Some(next) = next_ch {
            let op2 = matches!(
                (ch, next),
                ('!', '=') | ('<', '=') | ('>', '=') | ('<', '>')
            );
            if op2 {
                let end = if i + 2 < chars.len() {
                    chars[i + 2].0
                } else {
                    s.len()
                };
                let raw = &s[start..end];
                tokens.push(SqlOverlapToken {
                    text: raw.to_string(),
                    normalized: raw.to_string(),
                    start,
                    end,
                });
                i += 2;
                continue;
            }
        }

        // Single punctuation / operator
        let end = if i + 1 < chars.len() {
            chars[i + 1].0
        } else {
            s.len()
        };
        let raw = &s[start..end];
        tokens.push(SqlOverlapToken {
            text: raw.to_string(),
            normalized: raw.to_lowercase(),
            start,
            end,
        });
        i += 1;
    }
    tokens
}

#[allow(dead_code)]
pub fn clean_inline_completion(raw: &str, prefix: &str) -> String {
    clean_inline_completion_with_suffix(raw, prefix, "")
}

pub fn clean_inline_completion_with_suffix(raw: &str, prefix: &str, suffix: &str) -> String {
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

    let p_tokens = tokenize_for_overlap(prefix);
    let c_tokens = tokenize_for_overlap(&cleaned);
    let mut partial_found = false;

    if !p_tokens.is_empty() && !c_tokens.is_empty() {
        // 1. Exact full-token suffix/prefix match
        let max_k_candidate = p_tokens.len().min(c_tokens.len());
        let mut best_k = 0;
        let mut best_c_end = 0;

        for k in (1..=max_k_candidate).rev() {
            let p_slice = &p_tokens[p_tokens.len() - k..];
            let c_slice = &c_tokens[..k];
            let matches = p_slice
                .iter()
                .zip(c_slice.iter())
                .all(|(p, c)| p.normalized == c.normalized);
            if matches {
                best_k = k;
                best_c_end = c_tokens[k - 1].end;
                break;
            }
        }

        if best_k > 0 {
            cleaned = cleaned[best_c_end..].trim().to_string();
        } else {
            // 2. Partial-token match: 0..k full tokens match, and p.last() is a prefix of c[k]
            let last_p = &p_tokens[p_tokens.len() - 1];
            let max_partial_k = (p_tokens.len() - 1).min(c_tokens.len() - 1);

            for k in (0..=max_partial_k).rev() {
                let leading_match = if k > 0 {
                    let p_slice = &p_tokens[p_tokens.len() - 1 - k..p_tokens.len() - 1];
                    let c_slice = &c_tokens[..k];
                    p_slice
                        .iter()
                        .zip(c_slice.iter())
                        .all(|(p, c)| p.normalized == c.normalized)
                } else {
                    true
                };

                if leading_match {
                    let cand_c = &c_tokens[k];
                    if cand_c.normalized.starts_with(&last_p.normalized)
                        && cand_c.normalized.len() > last_p.normalized.len()
                    {
                        let offset = cand_c.start + last_p.text.len();
                        if offset <= cleaned.len() {
                            cleaned = cleaned[offset..].trim_start().to_string();
                            partial_found = true;
                            break;
                        }
                    }
                }
            }

            if !partial_found {
                // Also check if prefix is non-empty and cleaned literally starts with trimmed prefix
                let trimmed_p = prefix.trim();
                if !trimmed_p.is_empty()
                    && cleaned
                        .to_lowercase()
                        .starts_with(&trimmed_p.to_lowercase())
                {
                    cleaned = cleaned[trimmed_p.len()..].trim().to_string();
                }
            }
        }
    }

    // Trim trailing overlap with `sql_suffix` if LLM repeated text after cursor
    if !suffix.trim().is_empty() && !cleaned.is_empty() {
        let c_end_tokens = tokenize_for_overlap(&cleaned);
        let s_tokens = tokenize_for_overlap(suffix);
        if !c_end_tokens.is_empty() && !s_tokens.is_empty() {
            let max_k = c_end_tokens.len().min(s_tokens.len());
            for k in (1..=max_k).rev() {
                let c_slice = &c_end_tokens[c_end_tokens.len() - k..];
                let s_slice = &s_tokens[..k];
                if c_slice
                    .iter()
                    .zip(s_slice.iter())
                    .all(|(c, s)| c.normalized == s.normalized)
                {
                    let cut_at = c_end_tokens[c_end_tokens.len() - k].start;
                    cleaned = cleaned[..cut_at].trim_end().to_string();
                    break;
                }
            }
        }
    }

    if !partial_found
        && !prefix.ends_with(char::is_whitespace)
        && !cleaned.starts_with(' ')
        && !cleaned.is_empty()
        && !cleaned.starts_with([';', ',', ')'])
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::build_demo_schema;
    #[test]
    fn test_schema_ddl_and_local_completions() {
        let schema = build_demo_schema("ecommerce_prod");
        let ddl = schema.to_ai_ddl_context();
        assert!(ddl.contains("TABLE `users`"));
        assert!(ddl.contains("FK: `orders`.`user_id` -> `users`.`id`"));

        let (prefix, items) = compute_local_completions("SELECT * FROM us", Some(&schema), 10);
        assert_eq!(prefix, "us");
        assert!(items.iter().any(|i| i.label == "users"));

        let pruned = schema.to_pruned_ai_ddl_context("SELECT * FROM users WHERE");
        assert!(pruned.contains("TABLE `users`"));
        assert!(pruned.contains("TABLE `orders`"));
    }
    #[test]
    fn test_clean_inline_completion_overlap_trimming() {
        // User's exact bug case: LLM repeats 'WHERE serialNumber LIKE ...'
        let prefix1 = "SELECT * FROM `asset` where serialNumber like ";
        let raw1 = "WHERE serialNumber LIKE '%your_serial_number%'";
        assert_eq!(
            clean_inline_completion(raw1, prefix1),
            "'%your_serial_number%'"
        );

        // LLM repeats the full query
        let raw2 = "SELECT * FROM `asset` WHERE serialNumber LIKE '%pattern%'";
        assert_eq!(clean_inline_completion(raw2, prefix1), "'%pattern%'");

        // LLM repeats 'WHERE' clause
        let prefix3 = "SELECT * FROM orders WHERE ";
        let raw3 = "WHERE status = 'pending'";
        assert_eq!(clean_inline_completion(raw3, prefix3), "status = 'pending'");

        // Partial identifier completion ('em' -> 'ail')
        let prefix4 = "SELECT * FROM users WHERE em";
        let raw4 = "WHERE email = 'admin@example.com'";
        assert_eq!(
            clean_inline_completion(raw4, prefix4),
            "ail = 'admin@example.com'"
        );
    }
    #[test]
    fn test_heuristic_ghost_completion_and_sql_extraction() {
        let schema = build_demo_schema("ecommerce_prod");
        let ghost = compute_heuristic_ghost_completion("SELECT * FROM orders", Some(&schema));
        assert!(
            ghost
                .unwrap()
                .contains("JOIN users ON orders.user_id = users.id")
        );

        let md = "Here is the query:\n```sql\nSELECT id FROM users;\n```\nDone.";
        assert_eq!(
            extract_sql_code_block(md).as_deref(),
            Some("SELECT id FROM users;")
        );
    }
    #[test]
    fn test_three_autocomplete_options_and_heuristic() {
        use crate::config::{AiConfig, AiProvider};
        let schema = build_demo_schema("ecommerce_prod");

        // 1. Local Schema heuristic tests
        let select_ghost = compute_heuristic_ghost_completion("SELECT *", Some(&schema));
        assert!(select_ghost.unwrap().contains("FROM users LIMIT 50;"));

        let where_ghost =
            compute_heuristic_ghost_completion("SELECT * FROM users WHERE", Some(&schema));
        assert!(where_ghost.unwrap().contains("id = 1 LIMIT 50;"));

        let insert_ghost = compute_heuristic_ghost_completion("INSERT INTO users", Some(&schema));
        assert!(insert_ghost.unwrap().contains("VALUES"));

        // 2. AiConfig default provider is LocalSchema
        let cfg = AiConfig::default();
        assert_eq!(cfg.provider, AiProvider::LocalSchema);
        assert!(cfg.ollama_use_fim);

        // 3. Provider labels
        assert!(AiProvider::LocalSchema.label().contains("0ms"));
        assert!(AiProvider::Ollama.label().contains("FIM"));
        assert!(AiProvider::GitHubCopilot.label().contains("GitHub Copilot"));
    }
    #[test]
    fn test_smart_autocomplete_alias_clause_cursor_and_fim() {
        use crate::ai::clean_inline_completion_with_suffix;
        use crate::syntax::{replace_at_cursor, split_sql_at_cursor};

        let schema = build_demo_schema("ecommerce_prod");

        // 1. Alias resolution from `FROM users AS usr`
        let (_, alias_items) = compute_local_completions(
            "SELECT usr.em FROM users AS usr WHERE usr.em",
            Some(&schema),
            10,
        );
        assert!(alias_items.iter().any(|i| i.label == "usr.email"));

        // 2. Substring / word-boundary matching (`mail` matches `email`)
        let (_, fuzzy_items) =
            compute_local_completions("SELECT * FROM users WHERE mail", Some(&schema), 10);
        assert!(fuzzy_items.iter().any(|i| i.label == "email"));

        // 3. Clause awareness: after `FROM `, tables are suggested first
        let (_, from_items) = compute_local_completions("SELECT * FROM ", Some(&schema), 5);
        assert!(!from_items.is_empty());
        assert_eq!(from_items[0].kind, crate::ai::CompletionKind::Table);

        // 4. Cursor-aware split & replacement in middle of query
        let mut sql = "SELECT * FROM us WHERE id = 1".to_string();
        let cursor_idx = "SELECT * FROM us".chars().count();
        let (prefix, suffix) = split_sql_at_cursor(&sql, cursor_idx);
        assert_eq!(prefix, "SELECT * FROM us");
        assert_eq!(suffix, " WHERE id = 1");
        let new_cursor = replace_at_cursor(&mut sql, cursor_idx, "us".len(), "users");
        assert_eq!(sql, "SELECT * FROM users WHERE id = 1");
        assert_eq!(new_cursor, "SELECT * FROM users".chars().count());

        // 5. FIM suffix overlap trimming
        let cleaned = clean_inline_completion_with_suffix(
            "users WHERE id = 1",
            "SELECT * FROM ",
            " WHERE id = 1",
        );
        assert_eq!(cleaned, "users");
    }

    #[test]
    fn test_nl2sql_response_parsing_and_schema_budget() {
        let raw = "Here is the query:\n```sql\nselect id, email from users limit 5;\n```\nLists the first 5 users.\n- Uses `users` table\n- Applies LIMIT 5";
        let parsed = parse_nl2sql_response(raw);
        assert!(parsed.sql.to_uppercase().contains("SELECT"));
        assert!(parsed.sql.contains("users"));
        assert_eq!(parsed.assumptions.len(), 2);
        assert!(parsed.explanation.contains("first 5 users"));

        let schema = crate::demo::build_demo_schema("ecommerce_prod");
        let ctx_full = build_nl2sql_schema_context(&schema, 10_000);
        assert!(ctx_full.contains("TABLE `users`"));
        assert!(ctx_full.contains("FK:"));
        let ctx_capped = build_nl2sql_schema_context(&schema, 120);
        assert!(ctx_capped.len() <= 120);
    }
}
