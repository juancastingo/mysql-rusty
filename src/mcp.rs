use crate::config::{AppConfig, ConnectionProfile};
use crate::db::{ActiveSession, DatabaseSchema};
use crate::export::to_markdown_table;
use crate::syntax::{is_write_query, split_sql_statements};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_MAX_ROWS: usize = 200;
const MAX_ALLOWED_ROWS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    pub profile: String,
    pub database: Option<String>,
    pub max_rows: usize,
    pub allow_writes: bool,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub fn parse_mcp_args(args: &[String]) -> Result<McpConfig> {
    let mut profile: Option<String> = None;
    let mut database: Option<String> = None;
    let mut max_rows = DEFAULT_MAX_ROWS;
    let mut allow_writes = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--help" || arg == "-h" {
            return Err(anyhow!(
                "Usage: mysql-rusty mcp --profile <name|id> [--database <db>] [--max-rows <n>] [--allow-writes]"
            ));
        } else if arg == "--profile" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| anyhow!("Missing value for --profile"))?;
            profile = Some(val.clone());
        } else if let Some(val) = arg.strip_prefix("--profile=") {
            profile = Some(val.to_string());
        } else if arg == "--database" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| anyhow!("Missing value for --database"))?;
            database = Some(val.clone());
        } else if let Some(val) = arg.strip_prefix("--database=") {
            database = Some(val.to_string());
        } else if arg == "--max-rows" {
            i += 1;
            let val = args
                .get(i)
                .ok_or_else(|| anyhow!("Missing value for --max-rows"))?;
            let parsed: usize = val
                .parse()
                .with_context(|| format!("Invalid --max-rows value: {val}"))?;
            max_rows = parsed.clamp(1, MAX_ALLOWED_ROWS);
        } else if let Some(val) = arg.strip_prefix("--max-rows=") {
            let parsed: usize = val
                .parse()
                .with_context(|| format!("Invalid --max-rows value: {val}"))?;
            max_rows = parsed.clamp(1, MAX_ALLOWED_ROWS);
        } else if arg == "--allow-writes" {
            allow_writes = true;
        } else {
            return Err(anyhow!("Unknown MCP argument: {arg}"));
        }
        i += 1;
    }

    let profile = profile
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("--profile <name|id> is required for MCP mode"))?;

    Ok(McpConfig {
        profile,
        database,
        max_rows,
        allow_writes,
    })
}

pub fn resolve_profile(config: &AppConfig, cfg: &McpConfig) -> Result<ConnectionProfile> {
    let query = cfg.profile.trim();
    let mut profile = config
        .connections
        .iter()
        .find(|p| p.id == query || p.name.eq_ignore_ascii_case(query))
        .cloned()
        .or_else(|| {
            if query.eq_ignore_ascii_case("demo") || query.eq_ignore_ascii_case("demo sandbox") {
                Some(ConnectionProfile::demo_profile())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("Connection profile '{}' not found", cfg.profile))?;

    if let Some(db) = &cfg.database
        && !db.trim().is_empty()
    {
        profile.default_database = db.trim().to_string();
    }
    Ok(profile)
}

pub fn validate_mcp_sql(sql: &str, allow_writes: bool) -> Result<String> {
    let statements = split_sql_statements(sql);
    if statements.is_empty() {
        return Err(anyhow!("SQL query is empty."));
    }
    if statements.len() > 1 {
        return Err(anyhow!(
            "Multi-statement SQL is not allowed in MCP mode (got {} statements).",
            statements.len()
        ));
    }
    let stmt = statements.into_iter().next().unwrap();
    if !allow_writes && is_write_query(&stmt) {
        return Err(anyhow!(
            "Write/DDL queries are blocked in read-only MCP mode. Start with --allow-writes to permit them."
        ));
    }
    Ok(stmt)
}

fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "list_databases",
                "description": "List available databases/schemas on the connected MySQL server.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "list_tables",
                "description": "List tables, views, procedures, functions, triggers, and events in a database.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "database": {
                            "type": "string",
                            "description": "Database name (defaults to the profile's configured database)"
                        }
                    }
                }
            },
            {
                "name": "describe_schema",
                "description": "Return DDL/schema context for a database, optionally filtered to specific table names.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "database": {
                            "type": "string",
                            "description": "Database name (defaults to the profile's configured database)"
                        },
                        "tables": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of table names to include"
                        }
                    }
                }
            },
            {
                "name": "run_query",
                "description": "Execute a single SQL query and return results as a Markdown table. Read-only unless --allow-writes was passed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "sql": {
                            "type": "string",
                            "description": "Single SQL statement to execute"
                        },
                        "max_rows": {
                            "type": "integer",
                            "description": "Optional maximum number of rows to return"
                        }
                    },
                    "required": ["sql"]
                }
            },
            {
                "name": "explain_query",
                "description": "Run EXPLAIN FORMAT=JSON on a single read-only SQL query to inspect its execution plan.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "sql": {
                            "type": "string",
                            "description": "Single SQL query to explain"
                        }
                    },
                    "required": ["sql"]
                }
            }
        ]
    })
}

fn tool_text_result(text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn resolve_db_arg(args: &Value, default_db: Option<&str>) -> Result<String> {
    if let Some(db) = args.get("database").and_then(|v| v.as_str())
        && !db.trim().is_empty()
    {
        return Ok(db.trim().to_string());
    }
    default_db
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow!("No database specified and profile has no default database."))
}

async fn handle_tool_call(
    session: &ActiveSession,
    cfg: &McpConfig,
    profile: &ConnectionProfile,
    params: &Value,
) -> Value {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return tool_text_result("Missing tool name in tools/call".to_string(), true);
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let default_db = Some(profile.default_database.as_str()).filter(|s| !s.is_empty());

    let result: Result<String> = match name {
        "list_databases" => session.list_databases().await.map(|dbs| dbs.join("\n")),
        "list_tables" => match resolve_db_arg(&args, default_db) {
            Ok(db) => session.introspect_database(&db).await.map(|schema| {
                let mut lines = Vec::new();
                for t in &schema.tables {
                    lines.push(format!(
                        "TABLE {} (~{} rows, {} cols)",
                        t.name,
                        t.approx_rows,
                        t.columns.len()
                    ));
                }
                for v in &schema.views {
                    lines.push(format!("VIEW {}", v.name));
                }
                for p in &schema.procedures {
                    lines.push(format!("PROCEDURE {}", p.name));
                }
                for f in &schema.functions {
                    lines.push(format!("FUNCTION {}", f.name));
                }
                if lines.is_empty() {
                    format!("No objects found in database '{db}'.")
                } else {
                    lines.join("\n")
                }
            }),
            Err(e) => Err(e),
        },
        "describe_schema" => match resolve_db_arg(&args, default_db) {
            Ok(db) => session.introspect_database(&db).await.map(|schema| {
                let filter: Option<Vec<String>> =
                    args.get("tables").and_then(|v| v.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
                            .collect()
                    });
                let filtered = if let Some(ref names) = filter {
                    DatabaseSchema {
                        name: schema.name.clone(),
                        tables: schema
                            .tables
                            .iter()
                            .filter(|t| names.contains(&t.name.to_ascii_lowercase()))
                            .cloned()
                            .collect(),
                        ..schema
                    }
                } else {
                    schema
                };
                filtered.to_ai_ddl_context()
            }),
            Err(e) => Err(e),
        },
        "run_query" => {
            let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
            let effective_allow_writes = cfg.allow_writes && !profile.read_only;
            match validate_mcp_sql(sql, effective_allow_writes) {
                Ok(stmt) => {
                    let db = resolve_db_arg(&args, default_db).unwrap_or_default();
                    let row_limit = args
                        .get("max_rows")
                        .and_then(|v| v.as_u64())
                        .map(|n| (n as usize).clamp(1, MAX_ALLOWED_ROWS))
                        .unwrap_or(cfg.max_rows);
                    let schema =
                        if !db.is_empty() && matches!(session, ActiveSession::DemoSandbox { .. }) {
                            session.introspect_database(&db).await.ok()
                        } else {
                            None
                        };
                    session
                        .execute_query(&db, &stmt, schema.as_ref(), Some(row_limit))
                        .await
                        .map(|res| {
                            let md = to_markdown_table(&res);
                            let mut out = if md.is_empty() {
                                format!(
                                    "Query executed in {} ms. Affected rows: {}.",
                                    res.duration_ms, res.affected_rows
                                )
                            } else {
                                md
                            };
                            if res.truncated {
                                out.push_str(&format!(
                                    "\n\n*(Result truncated to {row_limit} rows)*"
                                ));
                            }
                            out
                        })
                }
                Err(e) => Err(e),
            }
        }
        "explain_query" => {
            let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
            match validate_mcp_sql(sql, false) {
                Ok(stmt) => {
                    let db = resolve_db_arg(&args, default_db).unwrap_or_default();
                    let explain_sql = format!("EXPLAIN FORMAT=JSON {stmt}");
                    session
                        .execute_query(&db, &explain_sql, None, Some(10))
                        .await
                        .map(|res| {
                            if let Some(first_row) = res.rows.first()
                                && let Some(first_cell) = first_row.first()
                            {
                                first_cell.display_text()
                            } else {
                                to_markdown_table(&res)
                            }
                        })
                }
                Err(e) => Err(e),
            }
        }
        other => Err(anyhow!("Unknown MCP tool: {other}")),
    };

    match result {
        Ok(text) => tool_text_result(text, false),
        Err(err) => tool_text_result(format!("Error: {err}"), true),
    }
}

pub async fn serve_stdio_with<R: BufRead, W: Write>(
    session: &ActiveSession,
    cfg: &McpConfig,
    profile: &ConnectionProfile,
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(err) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {err}"),
                    }),
                };
                serde_json::to_writer(&mut writer, &resp)?;
                writeln!(writer)?;
                writer.flush()?;
                continue;
            }
        };

        // Notifications have no id and expect no response.
        let Some(id) = req.id.clone() else {
            continue;
        };

        let response = match req.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "mysql-rusty-mcp",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                })),
                error: None,
            },
            "ping" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({})),
                error: None,
            },
            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(tool_definitions()),
                error: None,
            },
            "tools/call" => {
                let res = handle_tool_call(session, cfg, profile, &req.params).await;
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(res),
                    error: None,
                }
            }
            other => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {other}"),
                }),
            },
        };

        serde_json::to_writer(&mut writer, &response)?;
        writeln!(writer)?;
        writer.flush()?;
    }
    Ok(())
}

pub fn run_mcp_cli(args: &[String]) -> Result<()> {
    let cfg = parse_mcp_args(args)?;
    let app_config = AppConfig::load();
    let profile = resolve_profile(&app_config, &cfg)?;

    let rt = tokio::runtime::Runtime::new()
        .context("Failed to initialize Tokio runtime for MCP server")?;

    rt.block_on(async move {
        let session = ActiveSession::connect(profile.clone()).await?;
        let read_only = !cfg.allow_writes || profile.read_only;
        if read_only && matches!(session, ActiveSession::RealMysql { .. }) {
            let db = profile.default_database.as_str();
            session
                .execute_query(db, "SET SESSION TRANSACTION READ ONLY", None, None)
                .await
                .context("Failed to enforce READ ONLY session")?;
            let _ = session
                .execute_query(db, "SET SESSION max_execution_time = 30000", None, None)
                .await;
        }
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        serve_stdio_with(&session, &cfg, &profile, stdin.lock(), stdout.lock()).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn parse_mcp_args_valid_and_defaults() {
        let cfg = parse_mcp_args(&s(&["--profile", "demo"])).unwrap();
        assert_eq!(cfg.profile, "demo");
        assert_eq!(cfg.database, None);
        assert_eq!(cfg.max_rows, DEFAULT_MAX_ROWS);
        assert!(!cfg.allow_writes);

        let cfg2 = parse_mcp_args(&s(&[
            "--profile=prod",
            "--database=app",
            "--max-rows=50",
            "--allow-writes",
        ]))
        .unwrap();
        assert_eq!(cfg2.profile, "prod");
        assert_eq!(cfg2.database.as_deref(), Some("app"));
        assert_eq!(cfg2.max_rows, 50);
        assert!(cfg2.allow_writes);
    }

    #[test]
    fn parse_mcp_args_requires_profile() {
        assert!(parse_mcp_args(&s(&["--database", "app"])).is_err());
        assert!(parse_mcp_args(&s(&["--unknown"])).is_err());
    }

    #[test]
    fn validate_mcp_sql_blocks_writes_and_multi_statements() {
        assert!(validate_mcp_sql("SELECT 1;", false).is_ok());
        assert!(validate_mcp_sql("INSERT INTO t VALUES (1);", false).is_err());
        assert!(validate_mcp_sql("DROP TABLE t;", false).is_err());
        assert!(validate_mcp_sql("INSERT INTO t VALUES (1);", true).is_ok());
        assert!(validate_mcp_sql("SELECT 1; SELECT 2;", true).is_err());
        assert!(validate_mcp_sql("   ", false).is_err());
    }

    #[tokio::test]
    async fn mcp_stdio_end_to_end_with_demo_profile() {
        let cfg = McpConfig {
            profile: "demo".to_string(),
            database: None,
            max_rows: 10,
            allow_writes: false,
        };
        let profile = resolve_profile(&AppConfig::default(), &cfg).unwrap();
        let session = ActiveSession::connect(profile.clone()).await.unwrap();

        let requests = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_databases","arguments":{}}}).to_string(),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_tables","arguments":{}}}).to_string(),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_query","arguments":{"sql":"SELECT id, full_name FROM users LIMIT 2"}}}).to_string(),
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run_query","arguments":{"sql":"DELETE FROM users"}}}).to_string(),
        ]
        .join("\n")
            + "\n";

        let mut output = Vec::new();
        serve_stdio_with(
            &session,
            &cfg,
            &profile,
            Cursor::new(requests.as_bytes()),
            &mut output,
        )
        .await
        .unwrap();

        let text = String::from_utf8(output).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // 6 responses (notification produces none)
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[0]["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(lines[1]["id"], 2);
        assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 5);
        assert_eq!(lines[2]["id"], 3);
        assert!(!lines[2]["result"]["isError"].as_bool().unwrap());
        assert_eq!(lines[3]["id"], 4);
        assert!(
            lines[3]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("TABLE users")
        );
        assert_eq!(lines[4]["id"], 5);
        assert!(!lines[4]["result"]["isError"].as_bool().unwrap());
        assert_eq!(lines[5]["id"], 6);
        assert!(lines[5]["result"]["isError"].as_bool().unwrap());
    }
}
