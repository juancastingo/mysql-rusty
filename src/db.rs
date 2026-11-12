use crate::config::{ConnectionProfile, SslMode};
use anyhow::{Context, Result, anyhow};
use mysql_async::prelude::*;
use mysql_async::{OptsBuilder, Pool, Row, SslOpts, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
    pub extra: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    pub constraint_name: String,
    pub column_name: String,
    pub referenced_table: String,
    pub referenced_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub index_name: String,
    pub column_name: String,
    pub non_unique: bool,
    pub index_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub table_type: String, // BASE TABLE or VIEW
    pub engine: String,
    pub approx_rows: u64,
    pub columns: Vec<ColumnInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub indexes: Vec<IndexInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseSchema {
    pub name: String,
    pub tables: Vec<TableSchema>,
}

impl DatabaseSchema {
    /// Generates a compact DDL context string for AI autocomplete and natural-language SQL prompts.
    pub fn to_ai_ddl_context(&self) -> String {
        let mut out = format!("-- Database: `{}`\n", self.name);
        for t in &self.tables {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| {
                    let pk = if c.is_primary_key { " PK" } else { "" };
                    format!("{} {}{}", c.name, c.data_type, pk)
                })
                .collect();
            out.push_str(&format!("TABLE `{}` ({});\n", t.name, cols.join(", ")));
            for fk in &t.foreign_keys {
                out.push_str(&format!(
                    "-- FK: `{}`.`{}` -> `{}`.`{}`\n",
                    t.name, fk.column_name, fk.referenced_table, fk.referenced_column
                ));
            }
        }
        out
    }

    /// Generates a pruned DDL context containing only tables relevant to the given SQL prefix,
    /// plus directly related foreign-key tables (1-hop). This keeps prompt token count minimal and speeds up TTFT.
    pub fn to_pruned_ai_ddl_context(&self, sql_query: &str) -> String {
        let sql_lower = sql_query.to_lowercase();
        let mut relevant_tables: Vec<&TableSchema> = Vec::new();
        let mut mentioned_names: Vec<String> = Vec::new();

        for t in &self.tables {
            let name_lower = t.name.to_lowercase();
            // Check if table name appears in query as a distinct token or with backticks
            if sql_lower.contains(&format!("`{name_lower}`"))
                || sql_lower.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == name_lower)
            {
                relevant_tables.push(t);
                mentioned_names.push(t.name.clone());
            }
        }

        // Include 1-hop foreign key related tables
        if !mentioned_names.is_empty() {
            for t in &self.tables {
                if !mentioned_names.contains(&t.name) {
                    let links_to_mentioned = t
                        .foreign_keys
                        .iter()
                        .any(|fk| mentioned_names.contains(&fk.referenced_table));
                    let mentioned_links_to_this = relevant_tables.iter().any(|rt| {
                        rt.foreign_keys
                            .iter()
                            .any(|fk| fk.referenced_table.eq_ignore_ascii_case(&t.name))
                    });
                    if links_to_mentioned || mentioned_links_to_this {
                        relevant_tables.push(t);
                    }
                }
            }
        }

        // If no specific tables are matched yet (e.g. user just typed `SELECT `), or DB has few tables, fallback to full
        if relevant_tables.is_empty() || self.tables.len() <= 3 {
            return self.to_ai_ddl_context();
        }

        let mut out = format!("-- Database: `{}` (Filtered active tables)\n", self.name);
        for t in &relevant_tables {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| {
                    let pk = if c.is_primary_key { " PK" } else { "" };
                    format!("{} {}{}", c.name, c.data_type, pk)
                })
                .collect();
            out.push_str(&format!("TABLE `{}` ({});\n", t.name, cols.join(", ")));
            for fk in &t.foreign_keys {
                out.push_str(&format!(
                    "-- FK: `{}`.`{}` -> `{}`.`{}`\n",
                    t.name, fk.column_name, fk.referenced_table, fk.referenced_column
                ));
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CellValue {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Bytes(Vec<u8>),
}

impl CellValue {
    pub fn display_text(&self) -> String {
        match self {
            Self::Null => "NULL".to_string(),
            Self::Text(s) => s.clone(),
            Self::Integer(i) => i.to_string(),
            Self::Float(f) => format!("{f}"),
            Self::Bytes(b) => format!("0x{}", hex_preview(b)),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn from_mysql_value(val: Value) -> Self {
        match val {
            Value::NULL => Self::Null,
            Value::Int(i) => Self::Integer(i),
            Value::UInt(u) => Self::Integer(u as i64),
            Value::Float(f) => Self::Float(f as f64),
            Value::Double(d) => Self::Float(d),
            Value::Bytes(bytes) => match String::from_utf8(bytes) {
                Ok(s) => Self::Text(s),
                Err(err) => Self::Bytes(err.into_bytes()),
            },
            Value::Date(year, month, day, hour, minute, second, micro) => {
                if hour == 0 && minute == 0 && second == 0 && micro == 0 {
                    Self::Text(format!("{year:04}-{month:02}-{day:02}"))
                } else {
                    Self::Text(format!(
                        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
                    ))
                }
            }
            Value::Time(is_neg, days, hours, minutes, seconds, _micro) => {
                let sign = if is_neg { "-" } else { "" };
                let total_hours = days * 24 + u32::from(hours);
                Self::Text(format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}"))
            }
        }
    }
}

fn hex_preview(bytes: &[u8]) -> String {
    let limit = bytes.len().min(16);
    let mut s = String::with_capacity(limit * 2);
    for b in &bytes[..limit] {
        s.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > limit {
        s.push_str("…");
    }
    s
}

#[derive(Debug, Clone)]
pub struct PendingCellEdit {
    pub row_idx: usize,
    pub col_idx: usize,
    pub column_name: String,
    pub pk_column: String,
    pub pk_value: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub sql: String,
    pub database: String,
    pub columns: Vec<String>,
    pub column_types: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
    pub last_insert_id: Option<u64>,
    pub duration_ms: u64,
    pub editable_table: Option<String>,
    pub primary_key_column: Option<String>,
    pub status_message: String,
}

pub struct SshTunnelGuard {
    child: Child,
}

impl Drop for SshTunnelGuard {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[derive(Clone)]
pub struct DemoStore {
    pub tables: Arc<Mutex<HashMap<String, QueryResult>>>,
}

impl Default for DemoStore {
    fn default() -> Self {
        let mut map = HashMap::new();
        map.insert("users".to_string(), demo_users_table());
        map.insert("products".to_string(), demo_products_table());
        map.insert("orders".to_string(), demo_orders_table());
        map.insert("order_items".to_string(), demo_order_items_table());
        map.insert("payments".to_string(), demo_payments_table());
        map.insert("events_daily".to_string(), demo_events_daily_table());
        map.insert("pageviews_summary".to_string(), demo_pageviews_summary_table());
        map.insert("tables".to_string(), demo_tables_table());
        map.insert("columns".to_string(), demo_columns_table());
        Self {
            tables: Arc::new(Mutex::new(map)),
        }
    }
}

pub enum ActiveSession {
    RealMysql {
        profile: ConnectionProfile,
        pool: Pool,
        _ssh_guard: Option<Arc<Mutex<SshTunnelGuard>>>,
    },
    DemoSandbox {
        profile: ConnectionProfile,
        store: DemoStore,
    },
}

impl ActiveSession {
    pub async fn connect(profile: ConnectionProfile) -> Result<Self> {
        if profile.is_demo {
            return Ok(Self::DemoSandbox {
                profile,
                store: DemoStore::default(),
            });
        }

        let mut connect_host = profile.host.clone();
        let mut connect_port = profile.port;
        let mut ssh_guard = None;

        if profile.ssh.enabled && !profile.ssh.ssh_host.trim().is_empty() {
            // Support "hostname:port" format in ssh_host
            let (ssh_host, ssh_port) = if let Some((h, p)) = profile.ssh.ssh_host.trim().split_once(':') {
                (h.trim().to_string(), p.trim().parse::<u16>().unwrap_or(profile.ssh.ssh_port))
            } else {
                (profile.ssh.ssh_host.trim().to_string(), profile.ssh.ssh_port)
            };

            // Find an open local port (prefer local_bind_port if available, otherwise bind to free ephemeral port)
            let local_port = {
                let preferred = profile.ssh.local_bind_port;
                if preferred > 0 && std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
                    preferred
                } else {
                    std::net::TcpListener::bind("127.0.0.1:0")
                        .and_then(|l| l.local_addr())
                        .map(|a| a.port())
                        .unwrap_or(if preferred > 0 { preferred } else { 13306 })
                }
            };

            let forward_spec = format!("{}:{}:{}", local_port, profile.host.trim(), profile.port);
            let mut cmd = Command::new("ssh");
            cmd.arg("-N")
                .arg("-L")
                .arg(&forward_spec)
                .arg("-p")
                .arg(ssh_port.to_string())
                .arg("-o")
                .arg("ExitOnForwardFailure=yes")
                .arg("-o")
                .arg("StrictHostKeyChecking=accept-new");

            if !profile.ssh.ssh_key_path.trim().is_empty() {
                let expanded = expand_tilde(&profile.ssh.ssh_key_path);
                cmd.arg("-i").arg(expanded);
            }

            let target = if profile.ssh.ssh_user.trim().is_empty() {
                ssh_host
            } else {
                format!("{}@{}", profile.ssh.ssh_user.trim(), ssh_host)
            };
            cmd.arg(target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());

            let mut child = cmd
                .spawn()
                .context("Failed to start SSH tunnel subprocess (`ssh`)")?;

            // Wait for SSH tunnel to establish and bind to local port (up to 15 seconds)
            let mut port_ready = false;
            let start = tokio::time::Instant::now();
            let timeout = std::time::Duration::from_secs(15);

            while start.elapsed() < timeout {
                // 1. Check if SSH process exited prematurely (e.g. auth failure, unreachable host)
                if let Ok(Some(exit_status)) = child.try_wait() {
                    let mut err_msg = String::new();
                    if let Some(mut err_pipe) = child.stderr.take() {
                        use tokio::io::AsyncReadExt;
                        let _ = err_pipe.read_to_string(&mut err_msg).await;
                    }
                    let clean_err = err_msg.trim();
                    return Err(anyhow!(
                        "SSH tunnel failed (exit code: {}):\n{}",
                        exit_status,
                        if clean_err.is_empty() {
                            "SSH process closed connection immediately. Check username (e.g. 'ec2-user' vs 'ubuntu'), private key permissions, and security groups."
                        } else {
                            clean_err
                        }
                    ));
                }

                // 2. Probe if local forwarding port is accepting connections
                if tokio::net::TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                    // Small grace period for SSH forwarding pipe initialization
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    port_ready = true;
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }

            if !port_ready {
                let _ = child.kill().await;
                let mut err_msg = String::new();
                if let Some(mut err_pipe) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let _ = err_pipe.read_to_string(&mut err_msg).await;
                }
                let clean_err = err_msg.trim();
                return Err(anyhow!(
                    "Timed out waiting for SSH tunnel to open port 127.0.0.1:{local_port} (waited 15s).\n{}",
                    if clean_err.is_empty() {
                        "SSH tunnel did not bind local port in time. Check network latency or firewall."
                    } else {
                        clean_err
                    }
                ));
            }

            connect_host = "127.0.0.1".to_string();
            connect_port = local_port;
            ssh_guard = Some(Arc::new(Mutex::new(SshTunnelGuard { child })));
        }

        let base_builder = OptsBuilder::default()
            .ip_or_hostname(&connect_host)
            .tcp_port(connect_port)
            .user(Some(profile.user.clone()))
            .pass(if profile.password.is_empty() {
                None
            } else {
                Some(profile.password.clone())
            })
            .db_name(if profile.default_database.trim().is_empty() {
                None
            } else {
                Some(profile.default_database.clone())
            });

        let host_for_err = connect_host.clone();
        let format_conn_err = move |e: anyhow::Error| -> anyhow::Error {
            let err_str = format!("{e:#}");
            if err_str.contains("Connection refused") || err_str.contains("os error 111") || err_str.contains("os error 61") {
                anyhow::anyhow!(
                    "Connection refused (os error 111): No MySQL/MariaDB service is listening on {host_for_err}:{connect_port}.\n\n\
                     • If your database is running on your personal computer, note that mysql-rusty runs in a remote virtual machine, so '127.0.0.1' points to this remote VM. Enter your computer's public/LAN IP or use the built-in SSH Tunnel option.\n\
                     • To test queries, ER diagrams, AI completion, and table designer without an external database, click 'Connect' on the built-in 'Sandbox Demo' profile in the Servers tab."
                )
            } else {
                e
            }
        };

        let pool = match profile.ssl_mode {
            SslMode::Disabled => {
                let pool = Pool::new(base_builder.ssl_opts(None::<SslOpts>));
                let mut conn = pool
                    .get_conn()
                    .await
                    .map_err(|e| format_conn_err(anyhow::anyhow!(e)))
                    .context("Failed to connect to MySQL/MariaDB server (SSL disabled)")?;
                let _: Option<u8> = conn.query_first("SELECT 1").await?;
                pool
            }
            SslMode::Required => {
                let mut ssl_opts = SslOpts::default();
                if profile.ssl_accept_invalid_certs {
                    ssl_opts = ssl_opts
                        .with_danger_accept_invalid_certs(true)
                        .with_danger_skip_domain_validation(true);
                }
                let pool = Pool::new(base_builder.ssl_opts(Some(ssl_opts)));
                let mut conn = pool
                    .get_conn()
                    .await
                    .map_err(|e| format_conn_err(anyhow::anyhow!(e)))
                    .context("Failed to connect to MySQL/MariaDB server over SSL/TLS")?;
                let _: Option<u8> = conn.query_first("SELECT 1").await?;
                pool
            }
            SslMode::Preferred => {
                let mut ssl_opts = SslOpts::default();
                if profile.ssl_accept_invalid_certs {
                    ssl_opts = ssl_opts
                        .with_danger_accept_invalid_certs(true)
                        .with_danger_skip_domain_validation(true);
                }
                let ssl_pool = Pool::new(base_builder.clone().ssl_opts(Some(ssl_opts)));
                match ssl_pool.get_conn().await {
                    Ok(mut conn) => {
                        let _: Option<u8> = conn.query_first("SELECT 1").await?;
                        ssl_pool
                    }
                    Err(ssl_err) => {
                        // In Preferred mode, if SSL fails, gracefully fall back to unencrypted connection
                        let plain_pool = Pool::new(base_builder.ssl_opts(None::<SslOpts>));
                        match plain_pool.get_conn().await {
                            Ok(mut conn) => {
                                let _: Option<u8> = conn.query_first("SELECT 1").await?;
                                plain_pool
                            }
                            Err(_) => {
                                return Err(format_conn_err(anyhow::anyhow!(ssl_err)))
                                    .context("Failed to connect to MySQL/MariaDB server");
                            }
                        }
                    }
                }
            }
        };

        Ok(Self::RealMysql {
            profile,
            pool,
            _ssh_guard: ssh_guard,
        })
    }

    pub async fn list_databases(&self) -> Result<Vec<String>> {
        match self {
            Self::DemoSandbox { .. } => Ok(vec![
                "ecommerce_prod".to_string(),
                "analytics_dw".to_string(),
                "information_schema".to_string(),
            ]),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let dbs: Vec<String> = conn.query("SHOW DATABASES").await?;
                Ok(dbs)
            }
        }
    }

    pub async fn introspect_database(&self, database: &str) -> Result<DatabaseSchema> {
        match self {
            Self::DemoSandbox { .. } => Ok(build_demo_schema(database)),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;

                let tables_rows: Vec<(String, String, Option<String>, Option<u64>)> = conn
                    .exec(
                        "SELECT TABLE_NAME, TABLE_TYPE, ENGINE, TABLE_ROWS \
                         FROM information_schema.TABLES \
                         WHERE TABLE_SCHEMA = ? \
                         ORDER BY TABLE_NAME",
                        (database,),
                    )
                    .await?;

                let columns_rows: Vec<(
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    String,
                )> = conn
                    .exec(
                        "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, EXTRA \
                         FROM information_schema.COLUMNS \
                         WHERE TABLE_SCHEMA = ? \
                         ORDER BY TABLE_NAME, ORDINAL_POSITION",
                        (database,),
                    )
                    .await?;

                let fk_rows: Vec<(String, String, String, String, String)> = conn
                    .exec(
                        "SELECT TABLE_NAME, CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                         FROM information_schema.KEY_COLUMN_USAGE \
                         WHERE TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME IS NOT NULL \
                         ORDER BY TABLE_NAME, COLUMN_NAME",
                        (database,),
                    )
                    .await?;

                let idx_rows: Vec<(String, String, String, u8, String)> = conn
                    .exec(
                        "SELECT TABLE_NAME, INDEX_NAME, COLUMN_NAME, NON_UNIQUE, INDEX_TYPE \
                         FROM information_schema.STATISTICS \
                         WHERE TABLE_SCHEMA = ? \
                         ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX",
                        (database,),
                    )
                    .await?;

                let mut cols_by_table: HashMap<String, Vec<ColumnInfo>> = HashMap::new();
                for (t_name, c_name, c_type, nullable, key, def_val, extra) in columns_rows {
                    cols_by_table.entry(t_name).or_default().push(ColumnInfo {
                        name: c_name,
                        data_type: c_type,
                        is_nullable: nullable == "YES",
                        is_primary_key: key == "PRI",
                        default_value: def_val,
                        extra,
                    });
                }

                let mut fks_by_table: HashMap<String, Vec<ForeignKeyInfo>> = HashMap::new();
                for (t_name, constraint, col, ref_table, ref_col) in fk_rows {
                    fks_by_table
                        .entry(t_name)
                        .or_default()
                        .push(ForeignKeyInfo {
                            constraint_name: constraint,
                            column_name: col,
                            referenced_table: ref_table,
                            referenced_column: ref_col,
                        });
                }

                let mut idxs_by_table: HashMap<String, Vec<IndexInfo>> = HashMap::new();
                for (t_name, idx_name, col_name, non_unique, idx_type) in idx_rows {
                    idxs_by_table.entry(t_name).or_default().push(IndexInfo {
                        index_name: idx_name,
                        column_name: col_name,
                        non_unique: non_unique != 0,
                        index_type: idx_type,
                    });
                }

                let mut tables = Vec::with_capacity(tables_rows.len());
                for (t_name, t_type, engine, approx_rows) in tables_rows {
                    tables.push(TableSchema {
                        columns: cols_by_table.remove(&t_name).unwrap_or_default(),
                        foreign_keys: fks_by_table.remove(&t_name).unwrap_or_default(),
                        indexes: idxs_by_table.remove(&t_name).unwrap_or_default(),
                        name: t_name,
                        table_type: t_type,
                        engine: engine.unwrap_or_else(|| "InnoDB".to_string()),
                        approx_rows: approx_rows.unwrap_or(0),
                    });
                }

                Ok(DatabaseSchema {
                    name: database.to_string(),
                    tables,
                })
            }
        }
    }

    pub async fn execute_query(
        &self,
        database: &str,
        sql: &str,
        schema: Option<&DatabaseSchema>,
    ) -> Result<QueryResult> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("SQL query is empty."));
        }

        match self {
            Self::DemoSandbox { store, .. } => {
                execute_demo_query(store, database, trimmed, schema)
            }
            Self::RealMysql { pool, .. } => {
                let start = Instant::now();
                let mut conn = pool.get_conn().await?;
                if !database.trim().is_empty() {
                    let use_stmt = format!("USE `{}`", database.replace('`', "``"));
                    conn.query_drop(use_stmt).await?;
                }

                let mut result = conn.query_iter(trimmed).await?;
                let columns_meta: Vec<(String, String)> = result
                    .columns()
                    .as_ref()
                    .map(|cols| {
                        cols.iter()
                            .map(|c| {
                                (
                                    c.name_str().into_owned(),
                                    format!("{:?}", c.column_type()),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let raw_rows: Vec<Row> = result.collect().await?;
                let affected = result.affected_rows();
                let last_id = result.last_insert_id();
                drop(result);

                let duration_ms = start.elapsed().as_millis() as u64;
                let columns: Vec<String> = columns_meta.iter().map(|(c, _)| c.clone()).collect();
                let column_types: Vec<String> =
                    columns_meta.iter().map(|(_, t)| t.clone()).collect();

                let mut rows = Vec::with_capacity(raw_rows.len());
                for mut row in raw_rows {
                    let mut cells = Vec::with_capacity(columns.len());
                    for idx in 0..columns.len() {
                        let val = row.take::<Value, _>(idx).unwrap_or(Value::NULL);
                        cells.push(CellValue::from_mysql_value(val));
                    }
                    rows.push(cells);
                }

                let (editable_table, primary_key_column) =
                    detect_editable_table_and_pk(trimmed, &columns, schema);

                let status_message = if !columns.is_empty() {
                    format!(
                        "{} rows returned in {} ms",
                        rows.len(),
                        duration_ms
                    )
                } else {
                    format!(
                        "Query executed successfully ({} rows affected) in {} ms",
                        affected, duration_ms
                    )
                };

                Ok(QueryResult {
                    sql: trimmed.to_string(),
                    database: database.to_string(),
                    columns,
                    column_types,
                    rows,
                    affected_rows: affected,
                    last_insert_id: last_id,
                    duration_ms,
                    editable_table,
                    primary_key_column,
                    status_message,
                })
            }
        }
    }

    pub async fn apply_cell_edits(
        &self,
        database: &str,
        table: &str,
        edits: &[PendingCellEdit],
    ) -> Result<u64> {
        if edits.is_empty() {
            return Ok(0);
        }

        match self {
            Self::DemoSandbox { store, .. } => {
                let mut guard = store.tables.lock().unwrap();
                if let Some(tbl) = guard.get_mut(table) {
                    let mut count = 0;
                    for edit in edits {
                        if let Some(row) = tbl.rows.get_mut(edit.row_idx) {
                            if let Some(cell) = row.get_mut(edit.col_idx) {
                                if edit.new_value.eq_ignore_ascii_case("NULL") {
                                    *cell = CellValue::Null;
                                } else if let Ok(i) = edit.new_value.parse::<i64>() {
                                    *cell = CellValue::Integer(i);
                                } else if let Ok(f) = edit.new_value.parse::<f64>() {
                                    *cell = CellValue::Float(f);
                                } else {
                                    *cell = CellValue::Text(edit.new_value.clone());
                                }
                                count += 1;
                            }
                        }
                    }
                    return Ok(count);
                }
                Err(anyhow!("Table `{table}` not found in Sandbox environment"))
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let mut total_affected = 0;
                for edit in edits {
                    let stmt = format!(
                        "UPDATE `{}`.`{}` SET `{}` = ? WHERE `{}` = ?",
                        database.replace('`', "``"),
                        table.replace('`', "``"),
                        edit.column_name.replace('`', "``"),
                        edit.pk_column.replace('`', "``")
                    );
                    let new_val: Value = if edit.new_value.eq_ignore_ascii_case("NULL") {
                        Value::NULL
                    } else {
                        Value::Bytes(edit.new_value.as_bytes().to_vec())
                    };
                    conn.exec_drop(stmt, (new_val, edit.pk_value.clone()))
                        .await?;
                    total_affected += conn.affected_rows();
                }
                Ok(total_affected)
            }
        }
    }

    pub async fn list_processes(&self) -> Result<QueryResult> {
        match self {
            Self::DemoSandbox { .. } => {
                let columns = vec![
                    "Id".to_string(),
                    "User".to_string(),
                    "Host".to_string(),
                    "db".to_string(),
                    "Command".to_string(),
                    "Time".to_string(),
                    "State".to_string(),
                    "Info".to_string(),
                ];
                let column_types = vec![
                    "BIGINT".to_string(),
                    "VARCHAR".to_string(),
                    "VARCHAR".to_string(),
                    "VARCHAR".to_string(),
                    "VARCHAR".to_string(),
                    "INT".to_string(),
                    "VARCHAR".to_string(),
                    "TEXT".to_string(),
                ];
                let rows = vec![
                    vec![
                        CellValue::Integer(101),
                        CellValue::Text("app_user".to_string()),
                        CellValue::Text("192.168.1.15:42120".to_string()),
                        CellValue::Text("ecommerce_prod".to_string()),
                        CellValue::Text("Query".to_string()),
                        CellValue::Integer(2),
                        CellValue::Text("executing".to_string()),
                        CellValue::Text("SELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC".to_string()),
                    ],
                    vec![
                        CellValue::Integer(102),
                        CellValue::Text("analytics".to_string()),
                        CellValue::Text("10.0.0.84:51800".to_string()),
                        CellValue::Text("ecommerce_prod".to_string()),
                        CellValue::Text("Sleep".to_string()),
                        CellValue::Integer(145),
                        CellValue::Text(String::new()),
                        CellValue::Null,
                    ],
                    vec![
                        CellValue::Integer(103),
                        CellValue::Text("root".to_string()),
                        CellValue::Text("localhost".to_string()),
                        CellValue::Text("ecommerce_prod".to_string()),
                        CellValue::Text("Query".to_string()),
                        CellValue::Integer(0),
                        CellValue::Text("starting".to_string()),
                        CellValue::Text("SHOW FULL PROCESSLIST".to_string()),
                    ],
                ];
                Ok(QueryResult {
                    sql: "SHOW FULL PROCESSLIST".to_string(),
                    database: "mysql".to_string(),
                    columns,
                    column_types,
                    rows,
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms: 1,
                    editable_table: None,
                    primary_key_column: None,
                    status_message: "3 processes active (Sandbox)".to_string(),
                })
            }
            Self::RealMysql { pool, .. } => {
                let start = Instant::now();
                let mut conn = pool.get_conn().await?;
                let mut result = conn.query_iter("SHOW FULL PROCESSLIST").await?;
                let columns_meta: Vec<(String, String)> = result
                    .columns()
                    .as_ref()
                    .map(|cols| {
                        cols.iter()
                            .map(|c| (c.name_str().into_owned(), format!("{:?}", c.column_type())))
                            .collect()
                    })
                    .unwrap_or_default();

                let raw_rows: Vec<Row> = result.collect().await?;
                let mut rows = Vec::new();
                for r in raw_rows {
                    let mut row_cells = Vec::new();
                    for idx in 0..columns_meta.len() {
                        let val: Value = r.get(idx).unwrap_or(Value::NULL);
                        row_cells.push(CellValue::from_mysql_value(val));
                    }
                    rows.push(row_cells);
                }

                let columns = columns_meta.iter().map(|(n, _)| n.clone()).collect();
                let column_types = columns_meta.into_iter().map(|(_, t)| t).collect();
                let duration_ms = start.elapsed().as_millis().max(1) as u64;

                Ok(QueryResult {
                    sql: "SHOW FULL PROCESSLIST".to_string(),
                    database: "mysql".to_string(),
                    columns,
                    column_types,
                    rows,
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms,
                    editable_table: None,
                    primary_key_column: None,
                    status_message: format!("Process list retrieved in {duration_ms} ms"),
                })
            }
        }
    }

    pub async fn kill_query(&self, connection_id: u64) -> Result<()> {
        match self {
            Self::DemoSandbox { .. } => Ok(()),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let kill_stmt = format!("KILL QUERY {connection_id}");
                conn.query_drop(kill_stmt).await?;
                Ok(())
            }
        }
    }

    pub async fn server_status_summary(&self) -> Result<HashMap<String, String>> {
        match self {
            Self::DemoSandbox { .. } => {
                let mut map = HashMap::new();
                map.insert("Uptime".to_string(), "86420".to_string());
                map.insert("Threads_connected".to_string(), "14".to_string());
                map.insert("Questions".to_string(), "148293".to_string());
                map.insert("Slow_queries".to_string(), "3".to_string());
                map.insert("Innodb_buffer_pool_bytes_data".to_string(), "1073741824".to_string());
                Ok(map)
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let rows: Vec<Row> = conn.query("SHOW GLOBAL STATUS").await?;
                let mut map = HashMap::new();
                for r in rows {
                    if let (Some(k), Some(v)) = (r.get::<String, _>(0), r.get::<String, _>(1)) {
                        map.insert(k, v);
                    }
                }
                Ok(map)
            }
        }
    }
}

fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// Detects if a SELECT query targets a single table and includes its primary key column so inline editing works safely.
pub fn detect_editable_table_and_pk(
    sql: &str,
    result_columns: &[String],
    schema: Option<&DatabaseSchema>,
) -> (Option<String>, Option<String>) {
    let upper = sql.to_uppercase();
    if !upper.trim_start().starts_with("SELECT")
        || upper.contains(" JOIN ")
        || upper.contains(" GROUP BY ")
        || upper.contains(" UNION ")
    {
        return (None, None);
    }

    let tokens: Vec<&str> = sql.split_whitespace().collect();
    let mut from_table: Option<String> = None;
    for window in tokens.windows(2) {
        if window[0].eq_ignore_ascii_case("FROM") {
            let raw = window[1]
                .trim_matches(|c: char| c == '`' || c == ';' || c == ',')
                .split('.')
                .last()
                .unwrap_or("");
            if !raw.is_empty() {
                from_table = Some(raw.to_string());
            }
            break;
        }
    }

    let Some(table_name) = from_table else {
        return (None, None);
    };

    if let Some(db_schema) = schema {
        if let Some(t_schema) = db_schema
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(&table_name))
        {
            if let Some(pk_col) = t_schema.columns.iter().find(|c| c.is_primary_key) {
                if result_columns
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&pk_col.name))
                {
                    return (Some(t_schema.name.clone()), Some(pk_col.name.clone()));
                }
            }
        }
    }

    // Fallback if schema wasn't loaded yet and `id` column is present
    if result_columns.iter().any(|c| c.eq_ignore_ascii_case("id")) {
        return (Some(table_name), Some("id".to_string()));
    }

    (None, None)
}

// ============================================================================
// Built-in Sandbox / Demo Engine (`ecommerce_prod` & `analytics_dw`)
// ============================================================================

pub fn build_demo_schema(database: &str) -> DatabaseSchema {
    if database == "analytics_dw" {
        return DatabaseSchema {
            name: "analytics_dw".to_string(),
            tables: vec![
                TableSchema {
                    name: "events_daily".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 14500,
                    columns: vec![
                        ColumnInfo {
                            name: "event_date".to_string(),
                            data_type: "date".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "channel".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "active_users".to_string(),
                            data_type: "int unsigned".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0".to_string()),
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "conversion_rate".to_string(),
                            data_type: "decimal(5,2)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0.00".to_string()),
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
                TableSchema {
                    name: "pageviews_summary".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 82000,
                    columns: vec![
                        ColumnInfo {
                            name: "path".to_string(),
                            data_type: "varchar(255)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "views".to_string(),
                            data_type: "bigint unsigned".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0".to_string()),
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "avg_time_sec".to_string(),
                            data_type: "decimal(6,2)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0.00".to_string()),
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
            ],
        };
    }

    if database == "information_schema" {
        return DatabaseSchema {
            name: "information_schema".to_string(),
            tables: vec![
                TableSchema {
                    name: "TABLES".to_string(),
                    table_type: "SYSTEM VIEW".to_string(),
                    engine: "MEMORY".to_string(),
                    approx_rows: 120,
                    columns: vec![
                        ColumnInfo {
                            name: "TABLE_SCHEMA".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_TYPE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "ENGINE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: true,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
                TableSchema {
                    name: "COLUMNS".to_string(),
                    table_type: "SYSTEM VIEW".to_string(),
                    engine: "MEMORY".to_string(),
                    approx_rows: 950,
                    columns: vec![
                        ColumnInfo {
                            name: "TABLE_SCHEMA".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "COLUMN_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "DATA_TYPE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
            ],
        };
    }

    DatabaseSchema {
        name: "ecommerce_prod".to_string(),
        tables: vec![
            TableSchema {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 8,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "full_name".to_string(),
                        data_type: "varchar(120)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "email".to_string(),
                        data_type: "varchar(180)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "country".to_string(),
                        data_type: "char(2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("US".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "plan_tier".to_string(),
                        data_type: "enum('free','pro','enterprise')".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("free".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "created_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("CURRENT_TIMESTAMP".to_string()),
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![],
                indexes: vec![
                    IndexInfo {
                        index_name: "PRIMARY".to_string(),
                        column_name: "id".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                    IndexInfo {
                        index_name: "idx_users_email".to_string(),
                        column_name: "email".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                ],
            },
            TableSchema {
                name: "products".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 6,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "sku".to_string(),
                        data_type: "varchar(40)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar(160)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "category".to_string(),
                        data_type: "varchar(60)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "unit_price".to_string(),
                        data_type: "decimal(10,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "stock".to_string(),
                        data_type: "int".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("0".to_string()),
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![],
                indexes: vec![IndexInfo {
                    index_name: "PRIMARY".to_string(),
                    column_name: "id".to_string(),
                    non_unique: false,
                    index_type: "BTREE".to_string(),
                }],
            },
            TableSchema {
                name: "orders".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 7,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "user_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "status".to_string(),
                        data_type: "enum('pending','paid','shipped','cancelled')".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("pending".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "total_amount".to_string(),
                        data_type: "decimal(12,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "created_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![ForeignKeyInfo {
                    constraint_name: "fk_orders_user_id".to_string(),
                    column_name: "user_id".to_string(),
                    referenced_table: "users".to_string(),
                    referenced_column: "id".to_string(),
                }],
                indexes: vec![
                    IndexInfo {
                        index_name: "PRIMARY".to_string(),
                        column_name: "id".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                    IndexInfo {
                        index_name: "idx_orders_user_status".to_string(),
                        column_name: "user_id".to_string(),
                        non_unique: true,
                        index_type: "BTREE".to_string(),
                    },
                ],
            },
            TableSchema {
                name: "order_items".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 8,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "order_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "product_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "quantity".to_string(),
                        data_type: "int unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("1".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "unit_price".to_string(),
                        data_type: "decimal(10,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![
                    ForeignKeyInfo {
                        constraint_name: "fk_items_order_id".to_string(),
                        column_name: "order_id".to_string(),
                        referenced_table: "orders".to_string(),
                        referenced_column: "id".to_string(),
                    },
                    ForeignKeyInfo {
                        constraint_name: "fk_items_product_id".to_string(),
                        column_name: "product_id".to_string(),
                        referenced_table: "products".to_string(),
                        referenced_column: "id".to_string(),
                    },
                ],
                indexes: vec![IndexInfo {
                    index_name: "PRIMARY".to_string(),
                    column_name: "id".to_string(),
                    non_unique: false,
                    index_type: "BTREE".to_string(),
                }],
            },
            TableSchema {
                name: "payments".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 5,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "order_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "provider".to_string(),
                        data_type: "varchar(40)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "amount".to_string(),
                        data_type: "decimal(12,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "paid_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![ForeignKeyInfo {
                    constraint_name: "fk_payments_order_id".to_string(),
                    column_name: "order_id".to_string(),
                    referenced_table: "orders".to_string(),
                    referenced_column: "id".to_string(),
                }],
                indexes: vec![],
            },
        ],
    }
}

fn demo_users_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(1),
            CellValue::Text("Elena Rostova".into()),
            CellValue::Text("elena.rostova@acme.io".into()),
            CellValue::Text("ES".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2025-09-12 10:14:22".into()),
        ],
        vec![
            CellValue::Integer(2),
            CellValue::Text("Mateo Valdes".into()),
            CellValue::Text("mateo@valdes.dev".into()),
            CellValue::Text("AR".into()),
            CellValue::Text("pro".into()),
            CellValue::Text("2025-10-03 14:30:05".into()),
        ],
        vec![
            CellValue::Integer(3),
            CellValue::Text("Sofia Lindqvist".into()),
            CellValue::Text("sofia@nordiccloud.se".into()),
            CellValue::Text("SE".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2025-11-19 08:45:10".into()),
        ],
        vec![
            CellValue::Integer(4),
            CellValue::Text("Lucas Mendoza".into()),
            CellValue::Text("lucas.mendoza@fintech.mx".into()),
            CellValue::Text("MX".into()),
            CellValue::Text("pro".into()),
            CellValue::Text("2026-01-08 19:12:49".into()),
        ],
        vec![
            CellValue::Integer(5),
            CellValue::Text("Valentina Rios".into()),
            CellValue::Text("vrios@biotech.cl".into()),
            CellValue::Text("CL".into()),
            CellValue::Text("free".into()),
            CellValue::Text("2026-02-14 11:02:33".into()),
        ],
        vec![
            CellValue::Integer(6),
            CellValue::Text("David Chen".into()),
            CellValue::Text("dchen@hyperion.ai".into()),
            CellValue::Text("US".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2026-03-01 16:50:00".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM users".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "full_name".into(),
            "email".into(),
            "country".into(),
            "plan_tier".into(),
            "created_at".into(),
        ],
        column_types: vec![
            "BIGINT UNSIGNED".into(),
            "VARCHAR(120)".into(),
            "VARCHAR(180)".into(),
            "CHAR(2)".into(),
            "ENUM".into(),
            "TIMESTAMP".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 2,
        editable_table: Some("users".into()),
        primary_key_column: Some("id".into()),
        status_message: "6 rows returned in 2 ms (Sandbox)".into(),
    }
}

fn demo_products_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(101),
            CellValue::Text("NVME-4TB-PRO".into()),
            CellValue::Text("Enterprise NVMe PCIe 5.0 4TB SSD".into()),
            CellValue::Text("Storage".into()),
            CellValue::Float(429.90),
            CellValue::Integer(48),
        ],
        vec![
            CellValue::Integer(102),
            CellValue::Text("GPU-MI300X".into()),
            CellValue::Text("AI Accelerator 192GB HBM3".into()),
            CellValue::Text("Compute".into()),
            CellValue::Float(14500.00),
            CellValue::Integer(6),
        ],
        vec![
            CellValue::Integer(103),
            CellValue::Text("RAM-DDR5-128G".into()),
            CellValue::Text("ECC DDR5 128GB 6400MHz Kit".into()),
            CellValue::Text("Memory".into()),
            CellValue::Float(580.00),
            CellValue::Integer(92),
        ],
        vec![
            CellValue::Integer(104),
            CellValue::Text("SW-100G-QSFP".into()),
            CellValue::Text("Top-of-Rack Switch 32x100GbE".into()),
            CellValue::Text("Networking".into()),
            CellValue::Float(3190.00),
            CellValue::Integer(14),
        ],
        vec![
            CellValue::Integer(105),
            CellValue::Text("UPS-3000VA-LI".into()),
            CellValue::Text("2U Rack Lithium Online UPS 3000VA".into()),
            CellValue::Text("Power".into()),
            CellValue::Float(1290.50),
            CellValue::Integer(21),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM products".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "sku".into(),
            "name".into(),
            "category".into(),
            "unit_price".into(),
            "stock".into(),
        ],
        column_types: vec![
            "BIGINT UNSIGNED".into(),
            "VARCHAR(40)".into(),
            "VARCHAR(160)".into(),
            "VARCHAR(60)".into(),
            "DECIMAL(10,2)".into(),
            "INT".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("products".into()),
        primary_key_column: Some("id".into()),
        status_message: "5 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_orders_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(5001),
            CellValue::Integer(1),
            CellValue::Text("paid".into()),
            CellValue::Float(3619.80),
            CellValue::Text("2026-10-15 09:20:11".into()),
        ],
        vec![
            CellValue::Integer(5002),
            CellValue::Integer(2),
            CellValue::Text("paid".into()),
            CellValue::Float(1009.90),
            CellValue::Text("2026-10-18 17:44:02".into()),
        ],
        vec![
            CellValue::Integer(5003),
            CellValue::Integer(3),
            CellValue::Text("shipped".into()),
            CellValue::Float(14500.00),
            CellValue::Text("2026-10-21 12:05:39".into()),
        ],
        vec![
            CellValue::Integer(5004),
            CellValue::Integer(1),
            CellValue::Text("paid".into()),
            CellValue::Float(1290.50),
            CellValue::Text("2026-11-01 15:11:28".into()),
        ],
        vec![
            CellValue::Integer(5005),
            CellValue::Integer(4),
            CellValue::Text("pending".into()),
            CellValue::Float(580.00),
            CellValue::Text("2026-11-08 20:31:00".into()),
        ],
        vec![
            CellValue::Integer(5006),
            CellValue::Integer(6),
            CellValue::Text("paid".into()),
            CellValue::Float(17690.00),
            CellValue::Text("2026-11-09 11:18:45".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM orders".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "user_id".into(),
            "status".into(),
            "total_amount".into(),
            "created_at".into(),
        ],
        column_types: vec![
            "BIGINT UNSIGNED".into(),
            "BIGINT UNSIGNED".into(),
            "ENUM".into(),
            "DECIMAL(12,2)".into(),
            "TIMESTAMP".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 2,
        editable_table: Some("orders".into()),
        primary_key_column: Some("id".into()),
        status_message: "6 rows returned in 2 ms (Sandbox)".into(),
    }
}

fn demo_order_items_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(1),
            CellValue::Integer(5001),
            CellValue::Integer(104),
            CellValue::Integer(1),
            CellValue::Float(3190.00),
        ],
        vec![
            CellValue::Integer(2),
            CellValue::Integer(5001),
            CellValue::Integer(101),
            CellValue::Integer(1),
            CellValue::Float(429.80),
        ],
        vec![
            CellValue::Integer(3),
            CellValue::Integer(5002),
            CellValue::Integer(103),
            CellValue::Integer(1),
            CellValue::Float(580.00),
        ],
        vec![
            CellValue::Integer(4),
            CellValue::Integer(5003),
            CellValue::Integer(102),
            CellValue::Integer(1),
            CellValue::Float(14500.00),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM order_items".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "order_id".into(),
            "product_id".into(),
            "quantity".into(),
            "unit_price".into(),
        ],
        column_types: vec![
            "BIGINT UNSIGNED".into(),
            "BIGINT UNSIGNED".into(),
            "BIGINT UNSIGNED".into(),
            "INT UNSIGNED".into(),
            "DECIMAL(10,2)".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("order_items".into()),
        primary_key_column: Some("id".into()),
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_payments_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(901),
            CellValue::Integer(5001),
            CellValue::Text("stripe".into()),
            CellValue::Float(3619.80),
            CellValue::Text("2026-10-15 09:20:45".into()),
        ],
        vec![
            CellValue::Integer(902),
            CellValue::Integer(5002),
            CellValue::Text("adyen".into()),
            CellValue::Float(1009.90),
            CellValue::Text("2026-10-18 17:44:22".into()),
        ],
        vec![
            CellValue::Integer(903),
            CellValue::Integer(5006),
            CellValue::Text("wire_transfer".into()),
            CellValue::Float(17690.00),
            CellValue::Text("2026-11-09 11:19:02".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM payments".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "order_id".into(),
            "provider".into(),
            "amount".into(),
            "paid_at".into(),
        ],
        column_types: vec![
            "BIGINT UNSIGNED".into(),
            "BIGINT UNSIGNED".into(),
            "VARCHAR(40)".into(),
            "DECIMAL(12,2)".into(),
            "TIMESTAMP".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("payments".into()),
        primary_key_column: Some("id".into()),
        status_message: "3 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_events_daily_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("organic_search".into()),
            CellValue::Integer(4820),
            CellValue::Float(4.25),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("paid_ads".into()),
            CellValue::Integer(3110),
            CellValue::Float(6.80),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("social_media".into()),
            CellValue::Integer(2450),
            CellValue::Float(2.90),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("direct_referral".into()),
            CellValue::Integer(1920),
            CellValue::Float(5.10),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM events_daily".into(),
        database: "analytics_dw".into(),
        columns: vec![
            "event_date".into(),
            "channel".into(),
            "active_users".into(),
            "conversion_rate".into(),
        ],
        column_types: vec![
            "DATE".into(),
            "VARCHAR(64)".into(),
            "INT UNSIGNED".into(),
            "DECIMAL(5,2)".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("events_daily".into()),
        primary_key_column: Some("event_date".into()),
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_pageviews_summary_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("/".into()),
            CellValue::Integer(42000),
            CellValue::Float(45.2),
        ],
        vec![
            CellValue::Text("/pricing".into()),
            CellValue::Integer(18500),
            CellValue::Float(78.5),
        ],
        vec![
            CellValue::Text("/docs/quickstart".into()),
            CellValue::Integer(12300),
            CellValue::Float(195.0),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM pageviews_summary".into(),
        database: "analytics_dw".into(),
        columns: vec!["path".into(), "views".into(), "avg_time_sec".into()],
        column_types: vec!["VARCHAR(255)".into(), "BIGINT UNSIGNED".into(), "DECIMAL(6,2)".into()],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("pageviews_summary".into()),
        primary_key_column: Some("path".into()),
        status_message: "3 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_tables_table() -> QueryResult {
    let rows = vec![
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("users".into()), CellValue::Text("BASE TABLE".into()), CellValue::Text("InnoDB".into())],
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("products".into()), CellValue::Text("BASE TABLE".into()), CellValue::Text("InnoDB".into())],
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("orders".into()), CellValue::Text("BASE TABLE".into()), CellValue::Text("InnoDB".into())],
        vec![CellValue::Text("analytics_dw".into()), CellValue::Text("events_daily".into()), CellValue::Text("BASE TABLE".into()), CellValue::Text("InnoDB".into())],
    ];
    QueryResult {
        sql: "SELECT * FROM information_schema.TABLES".into(),
        database: "information_schema".into(),
        columns: vec!["TABLE_SCHEMA".into(), "TABLE_NAME".into(), "TABLE_TYPE".into(), "ENGINE".into()],
        column_types: vec!["VARCHAR(64)".into(), "VARCHAR(64)".into(), "VARCHAR(64)".into(), "VARCHAR(64)".into()],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: None,
        primary_key_column: None,
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_columns_table() -> QueryResult {
    let rows = vec![
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("users".into()), CellValue::Text("id".into()), CellValue::Text("bigint unsigned".into())],
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("users".into()), CellValue::Text("full_name".into()), CellValue::Text("varchar(100)".into())],
        vec![CellValue::Text("ecommerce_prod".into()), CellValue::Text("users".into()), CellValue::Text("email".into()), CellValue::Text("varchar(150)".into())],
        vec![CellValue::Text("analytics_dw".into()), CellValue::Text("events_daily".into()), CellValue::Text("event_date".into()), CellValue::Text("date".into())],
    ];
    QueryResult {
        sql: "SELECT * FROM information_schema.COLUMNS".into(),
        database: "information_schema".into(),
        columns: vec!["TABLE_SCHEMA".into(), "TABLE_NAME".into(), "COLUMN_NAME".into(), "DATA_TYPE".into()],
        column_types: vec!["VARCHAR(64)".into(), "VARCHAR(64)".into(), "VARCHAR(64)".into(), "VARCHAR(64)".into()],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: None,
        primary_key_column: None,
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn execute_demo_query(
    store: &DemoStore,
    database: &str,
    sql: &str,
    _schema: Option<&DatabaseSchema>,
) -> Result<QueryResult> {
    let lower = sql.to_lowercase();
    let guard = store.tables.lock().unwrap();

    if lower.starts_with("show databases") || lower.starts_with("show schemas") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["Database".into()],
            column_types: vec!["VARCHAR".into()],
            rows: vec![
                vec![CellValue::Text("ecommerce_prod".into())],
                vec![CellValue::Text("analytics_dw".into())],
                vec![CellValue::Text("information_schema".into())],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "3 databases discovered in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("show tables") {
        let col = format!("Tables_in_{database}");
        let tables_list = match database {
            "analytics_dw" => vec!["events_daily", "pageviews_summary"],
            "information_schema" => vec!["COLUMNS", "TABLES"],
            _ => vec!["users", "products", "orders", "order_items", "payments"],
        };
        let rows = tables_list
            .into_iter()
            .map(|t| vec![CellValue::Text(t.into())])
            .collect();
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![col],
            column_types: vec!["VARCHAR".into()],
            rows,
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "Tables listed in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("use ") {
        let target = sql[4..].trim().trim_matches(|c| c == ';' || c == '`' || c == ' ');
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: target.to_string(),
            columns: vec![],
            column_types: vec![],
            rows: vec![],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("Database changed to `{target}` (Sandbox)"),
        });
    }

    if lower.starts_with("select database()") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["DATABASE()".into()],
            column_types: vec!["VARCHAR".into()],
            rows: vec![vec![CellValue::Text(database.to_string())]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "1 row in set (Sandbox)".into(),
        });
    }

    if lower.starts_with("explain") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![
                "id".into(),
                "select_type".into(),
                "table".into(),
                "type".into(),
                "possible_keys".into(),
                "key".into(),
                "rows".into(),
                "Extra".into(),
            ],
            column_types: vec!["INT".into(); 8],
            rows: vec![
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("SIMPLE".into()),
                    CellValue::Text("u".into()),
                    CellValue::Text("ALL".into()),
                    CellValue::Text("PRIMARY".into()),
                    CellValue::Null,
                    CellValue::Integer(6),
                    CellValue::Text("Using temporary; Using filesort".into()),
                ],
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("SIMPLE".into()),
                    CellValue::Text("o".into()),
                    CellValue::Text("ref".into()),
                    CellValue::Text("idx_orders_user_status".into()),
                    CellValue::Text("idx_orders_user_status".into()),
                    CellValue::Integer(2),
                    CellValue::Text("Using where".into()),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "EXPLAIN execution plan generated in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("show create table") {
        let tbl = sql
            .split_whitespace()
            .last()
            .unwrap_or("users")
            .trim_matches(|c| c == '`' || c == ';');
        let ddl = format!(
            "CREATE TABLE `{tbl}` (\n  `id` bigint unsigned NOT NULL AUTO_INCREMENT,\n  `created_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;"
        );
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["Table".into(), "Create Table".into()],
            column_types: vec!["VARCHAR".into(), "TEXT".into()],
            rows: vec![vec![CellValue::Text(tbl.into()), CellValue::Text(ddl)]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "DDL fetched in 1 ms (Sandbox)".into(),
        });
    }

    // Check for scalar / function expressions like SELECT 1, SELECT NOW(), SELECT VERSION()
    if lower.starts_with("select") && !lower.contains("from") {
        let expr = sql.trim()[6..].trim().trim_end_matches(';');
        let (col_name, val) = match expr.to_lowercase().as_str() {
            "version()" | "@@version" => ("VERSION()", CellValue::Text("8.0.36-MySQL-Rusty-Sandbox".into())),
            "now()" | "current_timestamp()" => ("NOW()", CellValue::Text(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())),
            "user()" | "current_user()" => ("USER()", CellValue::Text("demo_admin@localhost".into())),
            "connection_id()" => ("CONNECTION_ID()", CellValue::Integer(1)),
            _ => (expr, CellValue::Text(expr.to_string())),
        };
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![col_name.to_string()],
            column_types: vec!["VARCHAR".to_string()],
            rows: vec![vec![val]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("1 row in set at {} (Sandbox)", chrono::Local::now().format("%H:%M:%S")),
        });
    }

    if lower.contains("join") && lower.contains("users") && lower.contains("orders") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![
                "id".into(),
                "full_name".into(),
                "email".into(),
                "country".into(),
                "total_orders".into(),
                "lifetime_value".into(),
            ],
            column_types: vec![
                "BIGINT".into(),
                "VARCHAR".into(),
                "VARCHAR".into(),
                "CHAR(2)".into(),
                "BIGINT".into(),
                "DECIMAL(12,2)".into(),
            ],
            rows: vec![
                vec![
                    CellValue::Integer(6),
                    CellValue::Text("David Chen".into()),
                    CellValue::Text("dchen@hyperion.ai".into()),
                    CellValue::Text("US".into()),
                    CellValue::Integer(1),
                    CellValue::Float(17690.00),
                ],
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("Elena Rostova".into()),
                    CellValue::Text("elena.rostova@acme.io".into()),
                    CellValue::Text("ES".into()),
                    CellValue::Integer(2),
                    CellValue::Float(4910.30),
                ],
                vec![
                    CellValue::Integer(2),
                    CellValue::Text("Mateo Valdes".into()),
                    CellValue::Text("mateo@valdes.dev".into()),
                    CellValue::Text("AR".into()),
                    CellValue::Integer(1),
                    CellValue::Float(1009.90),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 3,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("3 aggregated rows in 3 ms at {} (Sandbox)", chrono::Local::now().format("%H:%M:%S")),
        });
    }

    // Direct table matching ordered from longest to shortest name
    let known_tables = [
        "pageviews_summary",
        "events_daily",
        "order_items",
        "payments",
        "products",
        "orders",
        "users",
        "tables",
        "columns",
    ];

    // Check for COUNT(*) queries
    if lower.contains("count(") && !lower.contains("group by") {
        for &tname in &known_tables {
            if lower.contains(tname) {
                if let Some(res) = guard.get(tname) {
                    let total = res.rows.len();
                    return Ok(QueryResult {
                        sql: sql.to_string(),
                        database: database.to_string(),
                        columns: vec!["COUNT(*)".to_string()],
                        column_types: vec!["BIGINT".to_string()],
                        rows: vec![vec![CellValue::Integer(total as i64)]],
                        affected_rows: 0,
                        last_insert_id: None,
                        duration_ms: 1,
                        editable_table: None,
                        primary_key_column: None,
                        status_message: format!("1 row in set at {} (Sandbox)", chrono::Local::now().format("%H:%M:%S")),
                    });
                }
            }
        }
    }

    // Match known table
    for &tname in &known_tables {
        if lower.contains(tname) {
            if let Some(res) = guard.get(tname) {
                let mut cloned = res.clone();
                cloned.sql = sql.to_string();

                // Simple WHERE filter in sandbox demo for basic interactivity
                if lower.contains("where") {
                    if let Some(where_part) = lower.split("where").nth(1) {
                        let where_clean = where_part.split("order by").next().unwrap_or(where_part);
                        let where_clean = where_clean.split("limit").next().unwrap_or(where_clean);

                        if where_clean.contains("id =") || where_clean.contains("id=") {
                            if let Some(id_str) = where_clean.split('=').nth(1) {
                                if let Ok(id_val) = id_str.trim().trim_matches(|c| c == ';' || c == ' ').parse::<i64>() {
                                    cloned.rows.retain(|r| {
                                        if let Some(CellValue::Integer(i)) = r.first() {
                                            *i == id_val
                                        } else {
                                            true
                                        }
                                    });
                                }
                            }
                        }
                    }
                }

                // Simple LIMIT in sandbox demo
                if lower.contains("limit") {
                    if let Some(limit_part) = lower.split("limit").nth(1) {
                        if let Some(num_str) = limit_part.split_whitespace().next() {
                            if let Ok(limit_n) = num_str.trim_matches(';').parse::<usize>() {
                                cloned.rows.truncate(limit_n);
                            }
                        }
                    }
                }

                cloned.status_message = format!(
                    "{} row(s) returned in 1 ms at {} (Sandbox)",
                    cloned.rows.len(),
                    chrono::Local::now().format("%H:%M:%S")
                );
                return Ok(cloned);
            }
        }
    }

    // Check for DDL / modification statements
    if lower.starts_with("insert into") || lower.starts_with("update ") || lower.starts_with("delete from") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![],
            column_types: vec![],
            rows: vec![],
            affected_rows: 1,
            last_insert_id: Some(101),
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("1 row affected at {} (Sandbox)", chrono::Local::now().format("%H:%M:%S")),
        });
    }

    if lower.starts_with("create table") || lower.starts_with("alter table") || lower.starts_with("drop table") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![],
            column_types: vec![],
            rows: vec![],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 2,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("Query OK, 0 rows affected at {} (Sandbox)", chrono::Local::now().format("%H:%M:%S")),
        });
    }

    Err(anyhow!(
        "Table '{database}.{}' doesn't exist (or syntax not recognized by Sandbox). Available tables in `{database}`: {}",
        sql.split_whitespace().last().unwrap_or("unknown").trim_matches(';'),
        match database {
            "analytics_dw" => "events_daily, pageviews_summary",
            "information_schema" => "TABLES, COLUMNS",
            _ => "users, products, orders, order_items, payments",
        }
    ))
}
