use crate::config::{ConnectionProfile, SslMode};
use crate::demo::{DemoStore, build_demo_schema, execute_demo_query};
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
pub struct ViewInfo {
    pub name: String,
    pub definer: String,
    pub is_updatable: String,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutineInfo {
    pub name: String,
    pub routine_type: String,
    pub data_type: String,
    pub definer: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TriggerInfo {
    pub name: String,
    pub event: String,
    pub table: String,
    pub timing: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventInfo {
    pub name: String,
    pub status: String,
    pub execute_at: Option<String>,
    pub interval_value: Option<String>,
    pub interval_field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutineParamInfo {
    pub ordinal: u64,
    pub mode: String,
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct UserInfo {
    pub user: String,
    pub host: String,
    pub plugin: String,
    pub account_locked: bool,
    pub password_expired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseSchema {
    pub name: String,
    pub tables: Vec<TableSchema>,
    #[serde(default)]
    pub views: Vec<ViewInfo>,
    #[serde(default)]
    pub procedures: Vec<RoutineInfo>,
    #[serde(default)]
    pub functions: Vec<RoutineInfo>,
    #[serde(default)]
    pub triggers: Vec<TriggerInfo>,
    #[serde(default)]
    pub events: Vec<EventInfo>,
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
                || sql_lower
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == name_lower)
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
        s.push('…');
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
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub sql: String,
    pub database: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
    pub last_insert_id: Option<u64>,
    pub duration_ms: u64,
    pub editable_table: Option<String>,
    pub primary_key_column: Option<String>,
    pub status_message: String,
    pub truncated: bool,
}

pub struct SshTunnelGuard {
    child: Child,
}

impl SshTunnelGuard {
    pub fn check_alive(&mut self) -> Result<()> {
        match self.child.try_wait() {
            Ok(Some(status)) => Err(anyhow!("SSH tunnel process exited (status: {status})")),
            Ok(None) => Ok(()),
            Err(err) => Err(anyhow!("SSH tunnel status check failed: {err}")),
        }
    }
}

impl Drop for SshTunnelGuard {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Returns `true` if `err` represents a dropped or broken MySQL/SSH connection.
pub fn is_connection_error(err: &anyhow::Error) -> bool {
    if let Some(mysql_err) = err.downcast_ref::<mysql_async::Error>() {
        return is_mysql_conn_err(mysql_err);
    }
    let msg = err.to_string();
    msg.contains("SSH tunnel process exited")
}

fn is_mysql_conn_err(err: &mysql_async::Error) -> bool {
    match err {
        mysql_async::Error::Io(_) => true,
        mysql_async::Error::Driver(d) => matches!(
            d,
            mysql_async::DriverError::ConnectionClosed | mysql_async::DriverError::PoolDisconnected
        ),
        mysql_async::Error::Server(s) => s.code == 2006 || s.code == 2013,
        _ => false,
    }
}

pub enum ActiveSession {
    RealMysql {
        pool: Pool,
        editor_conn: Arc<tokio::sync::Mutex<Option<mysql_async::Conn>>>,
        editor_current_db: Arc<tokio::sync::Mutex<Option<String>>>,
        editor_conn_id: Arc<std::sync::atomic::AtomicU64>,
        _ssh_guard: Option<Arc<Mutex<SshTunnelGuard>>>,
    },
    DemoSandbox {
        store: DemoStore,
    },
}

impl ActiveSession {
    #[allow(dead_code)]
    pub fn dialect(&self) -> crate::dialect::Dialect {
        match self {
            Self::DemoSandbox { .. } | Self::RealMysql { .. } => crate::dialect::Dialect::MySql,
        }
    }

    pub async fn connect(profile: ConnectionProfile) -> Result<Self> {
        if profile.is_demo {
            tracing::info!("Connecting to built-in DemoSandbox");
            return Ok(Self::DemoSandbox {
                store: DemoStore::default(),
            });
        }

        tracing::info!(
            host = %profile.host,
            port = profile.port,
            ssl_mode = ?profile.ssl_mode,
            ssh_enabled = profile.ssh.enabled,
            "Connecting to MySQL/MariaDB server"
        );

        let mut connect_host = profile.host.clone();
        let mut connect_port = profile.port;
        let mut ssh_guard = None;

        if profile.ssh.enabled && !profile.ssh.ssh_host.trim().is_empty() {
            // Support "hostname:port" format in ssh_host
            let (ssh_host, ssh_port) =
                if let Some((h, p)) = profile.ssh.ssh_host.trim().split_once(':') {
                    (
                        h.trim().to_string(),
                        p.trim().parse::<u16>().unwrap_or(profile.ssh.ssh_port),
                    )
                } else {
                    (
                        profile.ssh.ssh_host.trim().to_string(),
                        profile.ssh.ssh_port,
                    )
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

            tracing::info!(forward_spec = %forward_spec, ssh_port, "Spawning SSH tunnel");
            let mut child = cmd
                .spawn()
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to spawn SSH tunnel subprocess");
                    e
                })
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
                    tracing::warn!(exit_status = %exit_status, error = %clean_err, "SSH tunnel exited prematurely");
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
                if tokio::net::TcpStream::connect(("127.0.0.1", local_port))
                    .await
                    .is_ok()
                {
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
            if err_str.contains("Connection refused")
                || err_str.contains("os error 111")
                || err_str.contains("os error 61")
            {
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
            SslMode::Required | SslMode::VerifyCa | SslMode::VerifyIdentity => {
                let ssl_opts = build_ssl_opts(&profile)?;
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
                let ssl_opts = build_ssl_opts(&profile)?;
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

        let editor = pool.get_conn().await?;
        let editor_id = u64::from(editor.id());
        tracing::info!(editor_conn_id = editor_id, host = %profile.host, port = profile.port, "Connected to MySQL/MariaDB");

        Ok(Self::RealMysql {
            pool,
            editor_conn: Arc::new(tokio::sync::Mutex::new(Some(editor))),
            editor_current_db: Arc::new(tokio::sync::Mutex::new(None)),
            editor_conn_id: Arc::new(std::sync::atomic::AtomicU64::new(editor_id)),
            _ssh_guard: ssh_guard,
        })
    }

    pub async fn reconnect(&self, profile: &ConnectionProfile) -> Result<Self> {
        Self::connect(profile.clone()).await
    }

    pub async fn ping(&self) -> Result<()> {
        match self {
            Self::DemoSandbox { .. } => Ok(()),
            Self::RealMysql {
                pool, _ssh_guard, ..
            } => {
                if let Some(guard) = _ssh_guard {
                    let mut locked = guard.lock().unwrap();
                    locked.check_alive()?;
                }
                let mut conn = pool.get_conn().await?;
                conn.ping().await?;
                Ok(())
            }
        }
    }

    pub fn editor_connection_id(&self) -> Option<u64> {
        match self {
            Self::RealMysql { editor_conn_id, .. } => {
                let id = editor_conn_id.load(std::sync::atomic::Ordering::Relaxed);
                if id == 0 { None } else { Some(id) }
            }
            Self::DemoSandbox { .. } => None,
        }
    }

    async fn lock_editor_conn<'a>(
        pool: &Pool,
        slot: &'a Arc<tokio::sync::Mutex<Option<mysql_async::Conn>>>,
        current_db: &Arc<tokio::sync::Mutex<Option<String>>>,
        conn_id_atomic: &Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<tokio::sync::MutexGuard<'a, Option<mysql_async::Conn>>> {
        let mut guard = slot.lock().await;
        let needs_reconnect = match guard.as_mut() {
            None => true,
            Some(conn) => conn.ping().await.is_err(),
        };
        if needs_reconnect {
            *guard = None;
            *current_db.lock().await = None;
            conn_id_atomic.store(0, std::sync::atomic::Ordering::Relaxed);
            let conn = pool.get_conn().await?;
            conn_id_atomic.store(u64::from(conn.id()), std::sync::atomic::Ordering::Relaxed);
            *guard = Some(conn);
        }
        Ok(guard)
    }

    fn is_connection_error(err: &mysql_async::Error) -> bool {
        is_mysql_conn_err(err)
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

                type ColumnInfoRow = (
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                    String,
                );
                let columns_rows: Vec<ColumnInfoRow> = conn
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

                type ViewRow = (String, Option<String>, Option<String>, Option<String>);
                let view_rows: Vec<ViewRow> = conn
                    .exec(
                        "SELECT TABLE_NAME, DEFINER, IS_UPDATABLE, VIEW_DEFINITION
                         FROM information_schema.VIEWS
                         WHERE TABLE_SCHEMA = ?
                         ORDER BY TABLE_NAME",
                        (database,),
                    )
                    .await
                    .unwrap_or_default();
                let views = view_rows
                    .into_iter()
                    .map(|(name, definer, is_updatable, definition)| ViewInfo {
                        name,
                        definer: definer.unwrap_or_default(),
                        is_updatable: is_updatable.unwrap_or_else(|| "NO".to_string()),
                        definition,
                    })
                    .collect();

                type RoutineRow = (
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                );
                let routine_rows: Vec<RoutineRow> =
                    conn.exec(
                        "SELECT ROUTINE_NAME, ROUTINE_TYPE, DTD_IDENTIFIER, DEFINER, ROUTINE_DEFINITION
                         FROM information_schema.ROUTINES
                         WHERE ROUTINE_SCHEMA = ?
                         ORDER BY ROUTINE_NAME",
                        (database,),
                    )
                    .await
                    .unwrap_or_default();
                let mut procedures = Vec::new();
                let mut functions = Vec::new();
                for (name, r_type, dtd, definer, defn) in routine_rows {
                    let info = RoutineInfo {
                        name,
                        routine_type: r_type.clone(),
                        data_type: dtd.unwrap_or_default(),
                        definer: definer.unwrap_or_default(),
                        definition: defn.unwrap_or_default(),
                    };
                    if r_type.eq_ignore_ascii_case("FUNCTION") {
                        functions.push(info);
                    } else {
                        procedures.push(info);
                    }
                }

                let trigger_rows: Vec<(String, String, String, String, Option<String>)> = conn
                    .exec(
                        "SELECT TRIGGER_NAME, EVENT_MANIPULATION, EVENT_OBJECT_TABLE, ACTION_TIMING, ACTION_STATEMENT
                         FROM information_schema.TRIGGERS
                         WHERE TRIGGER_SCHEMA = ?
                         ORDER BY TRIGGER_NAME",
                        (database,),
                    )
                    .await
                    .unwrap_or_default();
                let triggers = trigger_rows
                    .into_iter()
                    .map(|(name, event, table, timing, stmt)| TriggerInfo {
                        name,
                        event,
                        table,
                        timing,
                        statement: stmt.unwrap_or_default(),
                    })
                    .collect();

                type EventRow = (
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                );
                let event_rows: Vec<EventRow> =
                    conn.exec(
                        "SELECT EVENT_NAME, STATUS, CAST(EXECUTE_AT AS CHAR), CAST(INTERVAL_VALUE AS CHAR), INTERVAL_FIELD
                         FROM information_schema.EVENTS
                         WHERE EVENT_SCHEMA = ?
                         ORDER BY EVENT_NAME",
                        (database,),
                    )
                    .await
                    .unwrap_or_default();
                let events = event_rows
                    .into_iter()
                    .map(
                        |(name, status, execute_at, interval_value, interval_field)| EventInfo {
                            name,
                            status,
                            execute_at,
                            interval_value,
                            interval_field,
                        },
                    )
                    .collect();

                Ok(DatabaseSchema {
                    name: database.to_string(),
                    tables,
                    views,
                    procedures,
                    functions,
                    triggers,
                    events,
                })
            }
        }
    }

    pub async fn execute_query(
        &self,
        database: &str,
        sql: &str,
        schema: Option<&DatabaseSchema>,
        max_rows: Option<usize>,
    ) -> Result<QueryResult> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("SQL query is empty."));
        }

        let sql_preview: String = trimmed.chars().take(200).collect();
        tracing::debug!(database, sql_preview = %sql_preview, "Executing SQL query");

        match self {
            Self::DemoSandbox { store, .. } => {
                execute_demo_query(store, database, trimmed, schema, max_rows)
            }
            Self::RealMysql {
                pool,
                editor_conn,
                editor_current_db,
                editor_conn_id,
                ..
            } => {
                let start = Instant::now();
                let mut guard =
                    Self::lock_editor_conn(pool, editor_conn, editor_current_db, editor_conn_id)
                        .await?;
                let conn = guard.as_mut().expect("editor_conn initialized");

                let db_trimmed = database.trim();
                if !db_trimmed.is_empty() {
                    let needs_use = {
                        let cur = editor_current_db.lock().await;
                        cur.as_deref() != Some(db_trimmed)
                    };
                    if needs_use {
                        let use_stmt = format!(
                            "USE {}",
                            crate::dialect::Dialect::MySql.quote_ident(db_trimmed)
                        );
                        if let Err(err) = conn.query_drop(use_stmt).await {
                            if Self::is_connection_error(&err) {
                                *guard = None;
                                *editor_current_db.lock().await = None;
                                editor_conn_id.store(0, std::sync::atomic::Ordering::Relaxed);
                            }
                            return Err(err.into());
                        }
                        *editor_current_db.lock().await = Some(db_trimmed.to_string());
                    }
                }

                let mut result = match conn.query_iter(trimmed).await {
                    Ok(res) => res,
                    Err(err) => {
                        tracing::warn!(database, error = %err, "SQL query execution failed");
                        if Self::is_connection_error(&err) {
                            *guard = None;
                            *editor_current_db.lock().await = None;
                            editor_conn_id.store(0, std::sync::atomic::Ordering::Relaxed);
                        }
                        return Err(err.into());
                    }
                };
                if trimmed
                    .get(..4)
                    .is_some_and(|p| p.eq_ignore_ascii_case("USE "))
                {
                    *editor_current_db.lock().await = None;
                }
                let columns: Vec<String> = result
                    .columns()
                    .as_ref()
                    .map(|cols| cols.iter().map(|c| c.name_str().into_owned()).collect())
                    .unwrap_or_default();

                let mut rows: Vec<Vec<CellValue>> = Vec::new();
                let mut truncated = false;
                loop {
                    match result.next().await {
                        Ok(Some(mut row)) => {
                            if max_rows.is_some_and(|cap| rows.len() >= cap) {
                                truncated = true;
                                continue;
                            }
                            let mut cells = Vec::with_capacity(columns.len());
                            for idx in 0..columns.len() {
                                let val = row.take::<Value, _>(idx).unwrap_or(Value::NULL);
                                cells.push(CellValue::from_mysql_value(val));
                            }
                            rows.push(cells);
                        }
                        Ok(None) => break,
                        Err(err) => {
                            drop(result);
                            if Self::is_connection_error(&err) {
                                *guard = None;
                                *editor_current_db.lock().await = None;
                                editor_conn_id.store(0, std::sync::atomic::Ordering::Relaxed);
                            }
                            return Err(err.into());
                        }
                    }
                }
                let affected = result.affected_rows();
                let last_id = result.last_insert_id();
                drop(result);

                let duration_ms = start.elapsed().as_millis() as u64;
                tracing::info!(
                    database,
                    duration_ms,
                    row_count = rows.len(),
                    affected_rows = affected,
                    truncated,
                    "SQL query finished"
                );

                let (editable_table, primary_key_column) =
                    detect_editable_table_and_pk(trimmed, &columns, schema);

                let status_message = if truncated {
                    format!(
                        "Showing first {} rows (result truncated at max_rows = {})",
                        rows.len(),
                        rows.len()
                    )
                } else if !columns.is_empty() {
                    format!("{} rows returned in {} ms", rows.len(), duration_ms)
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
                    rows,
                    affected_rows: affected,
                    last_insert_id: last_id,
                    duration_ms,
                    editable_table,
                    primary_key_column,
                    status_message,
                    truncated,
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
                        if let Some(row) = tbl.rows.get_mut(edit.row_idx)
                            && let Some(cell) = row.get_mut(edit.col_idx)
                        {
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
                    return Ok(count);
                }
                Err(anyhow!("Table `{table}` not found in Sandbox environment"))
            }
            Self::RealMysql {
                pool,
                editor_conn,
                editor_current_db,
                editor_conn_id,
                ..
            } => {
                let mut guard =
                    Self::lock_editor_conn(pool, editor_conn, editor_current_db, editor_conn_id)
                        .await?;
                let conn = guard.as_mut().expect("editor_conn initialized");
                let mut total_affected = 0;
                for edit in edits {
                    let d = crate::dialect::Dialect::MySql;
                    let stmt = format!(
                        "UPDATE {} SET {} = ? WHERE {} = ?",
                        d.quote_qualified(database, table),
                        d.quote_ident(&edit.column_name),
                        d.quote_ident(&edit.pk_column)
                    );
                    let new_val: Value = if edit.new_value.eq_ignore_ascii_case("NULL") {
                        Value::NULL
                    } else {
                        Value::Bytes(edit.new_value.as_bytes().to_vec())
                    };
                    if let Err(err) = conn.exec_drop(stmt, (new_val, edit.pk_value.clone())).await {
                        if Self::is_connection_error(&err) {
                            *guard = None;
                            *editor_current_db.lock().await = None;
                            editor_conn_id.store(0, std::sync::atomic::Ordering::Relaxed);
                        }
                        return Err(err.into());
                    }
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
                    rows,
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms: 1,
                    editable_table: None,
                    primary_key_column: None,
                    status_message: "3 processes active (Sandbox)".to_string(),
                    truncated: false,
                })
            }
            Self::RealMysql { pool, .. } => {
                let start = Instant::now();
                let mut conn = pool.get_conn().await?;
                let mut result = conn.query_iter("SHOW FULL PROCESSLIST").await?;
                let columns: Vec<String> = result
                    .columns()
                    .as_ref()
                    .map(|cols| cols.iter().map(|c| c.name_str().into_owned()).collect())
                    .unwrap_or_default();

                let raw_rows: Vec<Row> = result.collect().await?;
                let mut rows = Vec::new();
                for r in raw_rows {
                    let mut row_cells = Vec::new();
                    for idx in 0..columns.len() {
                        let val: Value = r.get(idx).unwrap_or(Value::NULL);
                        row_cells.push(CellValue::from_mysql_value(val));
                    }
                    rows.push(row_cells);
                }

                let duration_ms = start.elapsed().as_millis().max(1) as u64;

                Ok(QueryResult {
                    sql: "SHOW FULL PROCESSLIST".to_string(),
                    database: "mysql".to_string(),
                    columns,
                    rows,
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms,
                    editable_table: None,
                    primary_key_column: None,
                    status_message: format!("Process list retrieved in {duration_ms} ms"),
                    truncated: false,
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
                map.insert(
                    "Innodb_buffer_pool_bytes_data".to_string(),
                    "1073741824".to_string(),
                );
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

    pub async fn list_server_variables(&self) -> Result<Vec<(String, String)>> {
        match self {
            Self::DemoSandbox { .. } => {
                Ok(vec![
                    ("auto_increment_increment".into(), "1".into()),
                    ("autocommit".into(), "ON".into()),
                    ("character_set_client".into(), "utf8mb4".into()),
                    ("character_set_connection".into(), "utf8mb4".into()),
                    ("character_set_database".into(), "utf8mb4".into()),
                    ("character_set_server".into(), "utf8mb4".into()),
                    ("collation_connection".into(), "utf8mb4_0900_ai_ci".into()),
                    ("collation_server".into(), "utf8mb4_0900_ai_ci".into()),
                    ("default_storage_engine".into(), "InnoDB".into()),
                    ("event_scheduler".into(), "ON".into()),
                    ("foreign_key_checks".into(), "ON".into()),
                    ("general_log".into(), "OFF".into()),
                    ("innodb_buffer_pool_size".into(), "134217728".into()),
                    ("innodb_file_per_table".into(), "ON".into()),
                    ("innodb_flush_log_at_trx_commit".into(), "1".into()),
                    ("innodb_lock_wait_timeout".into(), "50".into()),
                    ("interactive_timeout".into(), "28800".into()),
                    ("log_error_verbosity".into(), "2".into()),
                    ("max_allowed_packet".into(), "67108864".into()),
                    ("max_connections".into(), "151".into()),
                    ("net_read_timeout".into(), "30".into()),
                    ("net_write_timeout".into(), "60".into()),
                    ("slow_query_log".into(), "ON".into()),
                    ("sql_mode".into(), "ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION".into()),
                    ("time_zone".into(), "SYSTEM".into()),
                    ("transaction_isolation".into(), "REPEATABLE-READ".into()),
                    ("version".into(), "8.0.36-0ubuntu0.22.04.1".into()),
                    ("wait_timeout".into(), "28800".into()),
                ])
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let rows: Vec<Row> = conn.query("SHOW VARIABLES").await?;
                let mut vars = Vec::new();
                for r in rows {
                    if let (Some(k), Some(v)) = (r.get::<String, _>(0), r.get::<String, _>(1)) {
                        vars.push((k, v));
                    }
                }
                vars.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(vars)
            }
        }
    }

    pub async fn server_version(&self) -> Result<String> {
        match self {
            Self::DemoSandbox { .. } => Ok("8.4.0-demo".to_string()),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let ver: Option<String> = conn.query_first("SELECT VERSION()").await?;
                Ok(ver.unwrap_or_else(|| "unknown".to_string()))
            }
        }
    }

    pub async fn show_create(&self, kind: &str, db: &str, name: &str) -> Result<String> {
        match self {
            Self::DemoSandbox { .. } => Err(anyhow::anyhow!("not supported in demo")),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let qual = crate::dialect::Dialect::MySql.quote_qualified(db, name);
                let (sql, col_idx) = match kind.to_ascii_uppercase().as_str() {
                    "TABLE" => (format!("SHOW CREATE TABLE {qual}"), 1usize),
                    "VIEW" => (format!("SHOW CREATE VIEW {qual}"), 1usize),
                    "PROCEDURE" => (format!("SHOW CREATE PROCEDURE {qual}"), 2usize),
                    "FUNCTION" => (format!("SHOW CREATE FUNCTION {qual}"), 2usize),
                    "TRIGGER" => (format!("SHOW CREATE TRIGGER {qual}"), 2usize),
                    other => return Err(anyhow::anyhow!("Unsupported object kind: {other}")),
                };
                let row: Option<mysql_async::Row> = conn.query_first(sql).await?;
                let row = row.ok_or_else(|| anyhow::anyhow!("{kind} `{db}`.`{name}` not found"))?;
                row.get::<String, _>(col_idx)
                    .ok_or_else(|| anyhow::anyhow!("Missing DDL in SHOW CREATE {kind} output"))
            }
        }
    }

    pub async fn list_objects(&self, db: &str) -> Result<DbObjects> {
        match self {
            Self::DemoSandbox { .. } => Err(anyhow::anyhow!("not supported in demo")),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let tbl_rows: Vec<(String, String)> = conn
                    .exec(
                        "SELECT table_name, table_type FROM information_schema.TABLES WHERE table_schema = ? ORDER BY table_name",
                        (db,),
                    )
                    .await?;
                let mut out = DbObjects::default();
                for (name, kind) in tbl_rows {
                    if kind.eq_ignore_ascii_case("VIEW") {
                        out.views.push(name);
                    } else {
                        out.tables.push(name);
                    }
                }
                let routine_rows: Vec<(String, String)> = conn
                    .exec(
                        "SELECT routine_name, routine_type FROM information_schema.ROUTINES WHERE routine_schema = ? ORDER BY routine_name",
                        (db,),
                    )
                    .await?;
                for (name, kind) in routine_rows {
                    if kind.eq_ignore_ascii_case("FUNCTION") {
                        out.functions.push(name);
                    } else {
                        out.procedures.push(name);
                    }
                }
                out.triggers = conn
                    .exec(
                        "SELECT trigger_name FROM information_schema.TRIGGERS WHERE trigger_schema = ? ORDER BY trigger_name",
                        (db,),
                    )
                    .await?;
                Ok(out)
            }
        }
    }

    pub async fn stream_rows<F>(
        &self,
        db: &str,
        table: &str,
        batch: usize,
        mut on_batch: F,
    ) -> Result<()>
    where
        F: FnMut(&[String], Vec<Vec<CellValue>>) -> Result<()>,
    {
        match self {
            Self::DemoSandbox { .. } => Err(anyhow::anyhow!("not supported in demo")),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let sql = format!(
                    "SELECT * FROM {}",
                    crate::dialect::Dialect::MySql.quote_qualified(db, table)
                );
                let mut result = conn.query_iter(sql).await?;
                let columns: Vec<String> = result
                    .columns()
                    .as_ref()
                    .map(|cols| cols.iter().map(|c| c.name_str().into_owned()).collect())
                    .unwrap_or_default();
                let batch_cap = batch.max(1);
                let mut buf: Vec<Vec<CellValue>> = Vec::with_capacity(batch_cap);
                while let Some(mut row) = result.next().await? {
                    let mut cells = Vec::with_capacity(columns.len());
                    for idx in 0..columns.len() {
                        let val = row.take::<Value, _>(idx).unwrap_or(Value::NULL);
                        cells.push(CellValue::from_mysql_value(val));
                    }
                    buf.push(cells);
                    if buf.len() >= batch_cap {
                        let chunk = std::mem::replace(&mut buf, Vec::with_capacity(batch_cap));
                        on_batch(&columns, chunk)?;
                    }
                }
                if !buf.is_empty() {
                    on_batch(&columns, buf)?;
                }
                Ok(())
            }
        }
    }

    pub async fn open_pool_conn(&self, db: &str) -> Result<mysql_async::Conn> {
        match self {
            Self::DemoSandbox { .. } => Err(anyhow::anyhow!("not supported in demo")),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let db_trimmed = db.trim();
                if !db_trimmed.is_empty() {
                    let sql = format!(
                        "USE {}",
                        crate::dialect::Dialect::MySql.quote_ident(db_trimmed)
                    );
                    conn.query_drop(sql).await?;
                }
                Ok(conn)
            }
        }
    }

    pub async fn performance_schema_enabled(&self) -> Result<bool> {
        match self {
            Self::DemoSandbox { .. } => Ok(true),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let val: Option<u64> = conn.query_first("SELECT @@performance_schema").await?;
                Ok(val.unwrap_or(0) == 1)
            }
        }
    }

    pub async fn top_statement_digests(
        &self,
        limit: usize,
        schema_filter: Option<&str>,
    ) -> Result<QueryResult> {
        match self {
            Self::DemoSandbox { .. } => Ok(QueryResult {
                sql: "/* performance_schema digests (demo) */".to_string(),
                database: schema_filter.unwrap_or("shop").to_string(),
                columns: vec![
                    "SCHEMA_NAME".into(),
                    "DIGEST_TEXT".into(),
                    "COUNT_STAR".into(),
                    "total_s".into(),
                    "avg_ms".into(),
                    "max_ms".into(),
                    "SUM_ROWS_EXAMINED".into(),
                    "SUM_ROWS_SENT".into(),
                    "SUM_NO_INDEX_USED".into(),
                    "SUM_NO_GOOD_INDEX_USED".into(),
                    "FIRST_SEEN".into(),
                    "LAST_SEEN".into(),
                    "QUERY_SAMPLE_TEXT".into(),
                ],
                rows: vec![
                    vec![
                        CellValue::Text("shop".into()),
                        CellValue::Text("SELECT * FROM `orders` WHERE `status` = ? ORDER BY `created_at` DESC".into()),
                        CellValue::Integer(142),
                        CellValue::Float(4.82),
                        CellValue::Float(33.94),
                        CellValue::Float(190.5),
                        CellValue::Integer(48000),
                        CellValue::Integer(280),
                        CellValue::Integer(142),
                        CellValue::Integer(0),
                        CellValue::Text("2026-11-01 08:00:00".into()),
                        CellValue::Text("2026-11-13 09:00:00".into()),
                        CellValue::Text("SELECT * FROM `orders` WHERE `status` = 'pending' ORDER BY `created_at` DESC".into()),
                    ],
                ],
                affected_rows: 0,
                last_insert_id: None,
                duration_ms: 2,
                status_message: "1 slow query digest (demo)".into(),
                editable_table: None,
                primary_key_column: None,
                truncated: false,
            }),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let ver: String = conn
                    .query_first("SELECT VERSION()")
                    .await?
                    .unwrap_or_default();
                let has_sample = !ver.contains("MariaDB") && supports_query_sample_text(&ver);
                let sample_col = if has_sample {
                    ", QUERY_SAMPLE_TEXT"
                } else {
                    ""
                };
                let limit_val = limit.clamp(1, 500) as u64;
                let sql = if schema_filter.is_some_and(|s| !s.trim().is_empty()) {
                    format!(
                        "SELECT SCHEMA_NAME, DIGEST_TEXT, COUNT_STAR,
                                ROUND(SUM_TIMER_WAIT/1e12, 3) AS total_s,
                                ROUND(AVG_TIMER_WAIT/1e9, 2) AS avg_ms,
                                ROUND(MAX_TIMER_WAIT/1e9, 2) AS max_ms,
                                SUM_ROWS_EXAMINED, SUM_ROWS_SENT, SUM_NO_INDEX_USED, SUM_NO_GOOD_INDEX_USED,
                                CAST(FIRST_SEEN AS CHAR), CAST(LAST_SEEN AS CHAR){sample_col}
                         FROM performance_schema.events_statements_summary_by_digest
                         WHERE DIGEST_TEXT IS NOT NULL AND SCHEMA_NAME = ?
                         ORDER BY SUM_TIMER_WAIT DESC LIMIT ?"
                    )
                } else {
                    format!(
                        "SELECT SCHEMA_NAME, DIGEST_TEXT, COUNT_STAR,
                                ROUND(SUM_TIMER_WAIT/1e12, 3) AS total_s,
                                ROUND(AVG_TIMER_WAIT/1e9, 2) AS avg_ms,
                                ROUND(MAX_TIMER_WAIT/1e9, 2) AS max_ms,
                                SUM_ROWS_EXAMINED, SUM_ROWS_SENT, SUM_NO_INDEX_USED, SUM_NO_GOOD_INDEX_USED,
                                CAST(FIRST_SEEN AS CHAR), CAST(LAST_SEEN AS CHAR){sample_col}
                         FROM performance_schema.events_statements_summary_by_digest
                         WHERE DIGEST_TEXT IS NOT NULL
                         ORDER BY SUM_TIMER_WAIT DESC LIMIT ?"
                    )
                };
                let start = Instant::now();
                let mut result = if let Some(sf) = schema_filter.filter(|s| !s.trim().is_empty()) {
                    conn.exec_iter(sql.as_str(), (sf, limit_val)).await?
                } else {
                    conn.exec_iter(sql.as_str(), (limit_val,)).await?
                };
                let columns: Vec<String> = result
                    .columns()
                    .as_ref()
                    .map(|cols| cols.iter().map(|c| c.name_str().into_owned()).collect())
                    .unwrap_or_default();
                let mut rows = Vec::new();
                while let Some(mut row) = result.next().await? {
                    let mut cells = Vec::with_capacity(columns.len());
                    for idx in 0..columns.len() {
                        let val = row.take::<Value, _>(idx).unwrap_or(Value::NULL);
                        cells.push(CellValue::from_mysql_value(val));
                    }
                    rows.push(cells);
                }
                let duration_ms = start.elapsed().as_millis() as u64;
                Ok(QueryResult {
                    sql,
                    database: schema_filter.unwrap_or_default().to_string(),
                    columns,
                    status_message: format!("Loaded {} statement digest(s)", rows.len()),
                    rows,
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms,
                    editable_table: None,
                    primary_key_column: None,
                    truncated: false,
                })
            }
        }
    }

    pub async fn reset_statement_digests(&self) -> Result<()> {
        match self {
            Self::DemoSandbox { .. } => Ok(()),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                conn.query_drop(
                    "TRUNCATE TABLE performance_schema.events_statements_summary_by_digest",
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn tls_status(&self) -> (Option<String>, Option<String>) {
        match self {
            Self::DemoSandbox { .. } => (None, None),
            Self::RealMysql { pool, .. } => {
                let Ok(mut conn) = pool.get_conn().await else {
                    return (None, None);
                };
                let ver: Option<(String, String)> = conn
                    .query_first("SHOW SESSION STATUS LIKE 'Ssl_version'")
                    .await
                    .ok()
                    .flatten();
                let cipher: Option<(String, String)> = conn
                    .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
                    .await
                    .ok()
                    .flatten();
                (
                    ver.map(|(_, v)| v).filter(|v| !v.is_empty()),
                    cipher.map(|(_, v)| v).filter(|v| !v.is_empty()),
                )
            }
        }
    }

    pub async fn fetch_object_ddl(&self, db: &str, obj_type: &str, name: &str) -> Result<String> {
        match self {
            Self::DemoSandbox { .. } => match obj_type.to_ascii_uppercase().as_str() {
                "VIEW" => Ok(format!(
                    "CREATE VIEW `{name}` AS\nSELECT o.id, o.user_id, o.total_amount, o.status\nFROM orders o\nWHERE o.total_amount >= 100;"
                )),
                "PROCEDURE" => Ok(format!(
                    "CREATE PROCEDURE `{name}`(IN p_user_id INT, OUT p_total DECIMAL(10,2))\nBEGIN\n  SELECT COALESCE(SUM(total_amount), 0) INTO p_total\n  FROM orders WHERE user_id = p_user_id;\nEND;"
                )),
                _ => Ok(format!("-- Mock DDL for {obj_type} `{db}`.`{name}`")),
            },
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let qual = crate::dialect::Dialect::MySql.quote_qualified(db, name);
                let (sql, col_idx) = match obj_type.to_ascii_uppercase().as_str() {
                    "TABLE" => (format!("SHOW CREATE TABLE {qual}"), 1usize),
                    "VIEW" => (format!("SHOW CREATE VIEW {qual}"), 1usize),
                    "PROCEDURE" => (format!("SHOW CREATE PROCEDURE {qual}"), 2usize),
                    "FUNCTION" => (format!("SHOW CREATE FUNCTION {qual}"), 2usize),
                    "TRIGGER" => (format!("SHOW CREATE TRIGGER {qual}"), 2usize),
                    "EVENT" => (format!("SHOW CREATE EVENT {qual}"), 3usize),
                    other => return Err(anyhow!("Unsupported object type: {other}")),
                };
                let row: Option<Row> = conn.query_first(sql).await?;
                let row = row.ok_or_else(|| anyhow!("{obj_type} `{db}`.`{name}` not found"))?;
                row.get::<String, _>(col_idx)
                    .ok_or_else(|| anyhow!("Missing DDL in SHOW CREATE {obj_type}"))
            }
        }
    }

    pub async fn fetch_routine_parameters(
        &self,
        db: &str,
        routine_name: &str,
    ) -> Result<Vec<RoutineParamInfo>> {
        match self {
            Self::DemoSandbox { .. } => {
                if routine_name == "sp_customer_summary" {
                    Ok(vec![
                        RoutineParamInfo {
                            ordinal: 1,
                            mode: "IN".to_string(),
                            name: "p_user_id".to_string(),
                            data_type: "INT".to_string(),
                        },
                        RoutineParamInfo {
                            ordinal: 2,
                            mode: "OUT".to_string(),
                            name: "p_total".to_string(),
                            data_type: "DECIMAL(10,2)".to_string(),
                        },
                    ])
                } else {
                    Ok(Vec::new())
                }
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                type ParamRow = (u64, Option<String>, Option<String>, Option<String>);
                let rows: Vec<ParamRow> = conn
                    .exec(
                        "SELECT ORDINAL_POSITION, PARAMETER_MODE, PARAMETER_NAME, DTD_IDENTIFIER
                         FROM information_schema.PARAMETERS
                         WHERE SPECIFIC_SCHEMA = ? AND SPECIFIC_NAME = ? AND ORDINAL_POSITION > 0
                         ORDER BY ORDINAL_POSITION",
                        (db, routine_name),
                    )
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|(ordinal, mode, name, data_type)| RoutineParamInfo {
                        ordinal,
                        mode: mode.unwrap_or_else(|| "IN".to_string()),
                        name: name.unwrap_or_else(|| format!("arg_{ordinal}")),
                        data_type: data_type.unwrap_or_else(|| "VARCHAR(255)".to_string()),
                    })
                    .collect())
            }
        }
    }

    pub async fn run_explain_analyze(&self, database: &str, sql: &str) -> Result<String> {
        let statements = crate::syntax::split_sql_statements(sql);
        let stmt = statements
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No SQL statement to explain."))?;
        let trimmed = stmt.trim().trim_end_matches(';').trim();
        let target_sql = if trimmed
            .split_whitespace()
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("EXPLAIN"))
        {
            trimmed
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim())
                .unwrap_or(trimmed)
                .to_string()
        } else {
            trimmed.to_string()
        };

        if crate::syntax::is_write_query(&target_sql) {
            return Err(anyhow!(
                "AI Explain & Optimize only runs EXPLAIN on read-only queries."
            ));
        }

        let explain_analyze = format!("EXPLAIN ANALYZE {target_sql}");
        if let Ok(res) = self
            .execute_query(database, &explain_analyze, None, Some(200))
            .await
            && !res.rows.is_empty()
        {
            return Ok(crate::export::to_markdown_table(&res));
        }

        let explain_json = format!("EXPLAIN FORMAT=JSON {target_sql}");
        if let Ok(res) = self
            .execute_query(database, &explain_json, None, Some(200))
            .await
            && let Some(cell) = res.rows.first().and_then(|r| r.first())
        {
            return Ok(cell.display_text());
        }

        let explain_std = format!("EXPLAIN {target_sql}");
        let res = self
            .execute_query(database, &explain_std, None, Some(200))
            .await?;
        Ok(crate::export::to_markdown_table(&res))
    }

    pub async fn list_users(&self) -> Result<Vec<UserInfo>> {
        match self {
            Self::DemoSandbox { store, .. } => {
                let guard = store
                    .users
                    .lock()
                    .map_err(|_| anyhow!("Demo user store poisoned"))?;
                Ok(guard.iter().map(|(u, _)| u.clone()).collect())
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                type UserRow = (
                    String,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                );
                let rows: Vec<UserRow> = conn
                    .query(
                        "SELECT user, host, plugin, account_locked, password_expired
                         FROM mysql.user
                         ORDER BY user, host",
                    )
                    .await?;
                Ok(rows
                    .into_iter()
                    .map(|(user, host, plugin, locked, expired)| UserInfo {
                        user,
                        host,
                        plugin: plugin.unwrap_or_else(|| "caching_sha2_password".to_string()),
                        account_locked: locked.as_deref() == Some("Y"),
                        password_expired: expired.as_deref() == Some("Y"),
                    })
                    .collect())
            }
        }
    }

    pub async fn fetch_user_grants(&self, user: &str, host: &str) -> Result<Vec<String>> {
        match self {
            Self::DemoSandbox { store, .. } => {
                let guard = store
                    .users
                    .lock()
                    .map_err(|_| anyhow!("Demo user store poisoned"))?;
                Ok(guard
                    .iter()
                    .find(|(u, _)| u.user == user && u.host == host)
                    .map(|(_, g)| g.clone())
                    .unwrap_or_default())
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                let sql = format!(
                    "SHOW GRANTS FOR '{}'@'{}'",
                    escape_sql_str(user),
                    escape_sql_str(host)
                );
                let grants: Vec<String> = conn.query(sql).await?;
                Ok(grants)
            }
        }
    }

    pub async fn create_user(
        &self,
        user: &str,
        host: &str,
        password: &str,
        auth_plugin: &str,
    ) -> Result<()> {
        let sql = build_create_user_sql(user, host, password, auth_plugin);
        match self {
            Self::DemoSandbox { store, .. } => {
                let mut guard = store
                    .users
                    .lock()
                    .map_err(|_| anyhow!("Demo user store poisoned"))?;
                if guard.iter().any(|(u, _)| u.user == user && u.host == host) {
                    return Err(anyhow!("User `{user}`@`{host}` already exists"));
                }
                guard.push((
                    UserInfo {
                        user: user.to_string(),
                        host: host.to_string(),
                        plugin: auth_plugin.to_string(),
                        account_locked: false,
                        password_expired: false,
                    },
                    vec![format!("GRANT USAGE ON *.* TO `{user}`@`{host}`")],
                ));
                Ok(())
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                conn.query_drop(sql).await?;
                Ok(())
            }
        }
    }

    pub async fn drop_user(&self, user: &str, host: &str) -> Result<()> {
        let sql = format!(
            "DROP USER '{}'@'{}'",
            escape_sql_str(user),
            escape_sql_str(host)
        );
        match self {
            Self::DemoSandbox { store, .. } => {
                let mut guard = store
                    .users
                    .lock()
                    .map_err(|_| anyhow!("Demo user store poisoned"))?;
                guard.retain(|(u, _)| !(u.user == user && u.host == host));
                Ok(())
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                conn.query_drop(sql).await?;
                Ok(())
            }
        }
    }

    pub async fn update_user_password(
        &self,
        user: &str,
        host: &str,
        new_password: &str,
    ) -> Result<()> {
        let sql = format!(
            "ALTER USER '{}'@'{}' IDENTIFIED BY '{}'",
            escape_sql_str(user),
            escape_sql_str(host),
            escape_sql_str(new_password)
        );
        match self {
            Self::DemoSandbox { .. } => Ok(()),
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                conn.query_drop(sql).await?;
                Ok(())
            }
        }
    }

    pub async fn apply_user_privileges(
        &self,
        user: &str,
        host: &str,
        grants: &[String],
        revokes: &[String],
    ) -> Result<()> {
        let stmts = build_grant_revoke_sql(user, host, "*.*", grants, revokes);
        match self {
            Self::DemoSandbox { store, .. } => {
                let mut guard = store
                    .users
                    .lock()
                    .map_err(|_| anyhow!("Demo user store poisoned"))?;
                if let Some((_, existing)) = guard
                    .iter_mut()
                    .find(|(u, _)| u.user == user && u.host == host)
                {
                    let mut set = parse_granted_privileges(existing);
                    for g in grants {
                        set.insert(g.to_ascii_uppercase());
                    }
                    for r in revokes {
                        set.remove(&r.to_ascii_uppercase());
                    }
                    let mut list: Vec<String> = set.into_iter().collect();
                    list.sort();
                    *existing = if list.is_empty() {
                        vec![format!("GRANT USAGE ON *.* TO `{user}`@`{host}`")]
                    } else {
                        vec![format!(
                            "GRANT {} ON *.* TO `{user}`@`{host}`",
                            list.join(", ")
                        )]
                    };
                }
                Ok(())
            }
            Self::RealMysql { pool, .. } => {
                let mut conn = pool.get_conn().await?;
                for stmt in stmts {
                    conn.query_drop(stmt).await?;
                }
                conn.query_drop("FLUSH PRIVILEGES").await?;
                Ok(())
            }
        }
    }
}

fn escape_sql_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

pub fn build_create_user_sql(user: &str, host: &str, password: &str, auth_plugin: &str) -> String {
    let u = escape_sql_str(user.trim());
    let h = escape_sql_str(if host.trim().is_empty() {
        "%"
    } else {
        host.trim()
    });
    let p = escape_sql_str(password);
    let plugin = auth_plugin.trim();
    if plugin.is_empty() {
        format!("CREATE USER '{u}'@'{h}' IDENTIFIED BY '{p}'")
    } else {
        format!("CREATE USER '{u}'@'{h}' IDENTIFIED WITH {plugin} BY '{p}'")
    }
}

pub fn build_grant_revoke_sql(
    user: &str,
    host: &str,
    scope: &str,
    grants: &[String],
    revokes: &[String],
) -> Vec<String> {
    let u = escape_sql_str(user.trim());
    let h = escape_sql_str(if host.trim().is_empty() {
        "%"
    } else {
        host.trim()
    });
    let sc = if scope.trim().is_empty() {
        "*.*"
    } else {
        scope.trim()
    };
    let mut stmts = Vec::new();
    if !grants.is_empty() {
        stmts.push(format!(
            "GRANT {} ON {sc} TO '{u}'@'{h}'",
            grants.join(", ")
        ));
    }
    if !revokes.is_empty() {
        stmts.push(format!(
            "REVOKE {} ON {sc} FROM '{u}'@'{h}'",
            revokes.join(", ")
        ));
    }
    stmts
}

pub const ALL_PRIVILEGES: &[&str] = &[
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "CREATE",
    "ALTER",
    "DROP",
    "INDEX",
    "CREATE VIEW",
    "SHOW VIEW",
    "SUPER",
    "PROCESS",
    "RELOAD",
    "REPLICATION CLIENT",
    "SHOW DATABASES",
];

pub fn parse_granted_privileges(grants: &[String]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for line in grants {
        let upper = line.to_ascii_uppercase();
        if upper.contains("ALL PRIVILEGES") {
            for p in ALL_PRIVILEGES {
                out.insert((*p).to_string());
            }
            continue;
        }
        if let Some(after_grant) = upper.strip_prefix("GRANT ")
            && let Some((privs_part, _)) = after_grant.split_once(" ON ")
        {
            for p in privs_part.split(',') {
                let clean = p.trim();
                if ALL_PRIVILEGES.contains(&clean) {
                    out.insert(clean.to_string());
                }
            }
        }
    }
    out
}

pub fn generate_routine_call_sql(
    routine_name: &str,
    params: &[RoutineParamInfo],
    inputs: &[String],
) -> String {
    let mut call_args = Vec::with_capacity(params.len());
    let mut out_vars = Vec::new();
    for (idx, p) in params.iter().enumerate() {
        let mode = p.mode.to_ascii_uppercase();
        if mode == "OUT" || mode == "INOUT" {
            let var_name = format!("@{}", p.name.trim_start_matches('@'));
            if mode == "INOUT" {
                let val = inputs.get(idx).map(|s| s.trim()).unwrap_or("NULL");
                let formatted = if val.is_empty() { "NULL" } else { val };
                out_vars.push((var_name.clone(), Some(formatted.to_string())));
            } else {
                out_vars.push((var_name.clone(), None));
            }
            call_args.push(var_name);
        } else {
            let val = inputs.get(idx).map(|s| s.trim()).unwrap_or("NULL");
            call_args.push(if val.is_empty() {
                "NULL".to_string()
            } else {
                val.to_string()
            });
        }
    }

    let mut lines = Vec::new();
    for (var, init) in &out_vars {
        if let Some(v) = init {
            lines.push(format!("SET {var} = {v};"));
        }
    }
    lines.push(format!(
        "CALL {}({});",
        crate::dialect::Dialect::MySql.quote_ident(routine_name),
        call_args.join(", ")
    ));
    if !out_vars.is_empty() {
        let selects: Vec<String> = out_vars
            .iter()
            .map(|(v, _)| format!("{v} AS `{}`", v.trim_start_matches('@')))
            .collect();
        lines.push(format!("SELECT {};", selects.join(", ")));
    }
    lines.join("\n")
}

fn build_ssl_opts(profile: &ConnectionProfile) -> Result<SslOpts> {
    let mut ssl_opts = SslOpts::default();

    let ca_raw = profile.ssl_ca_path.trim();
    if !ca_raw.is_empty() {
        let ca_path = std::path::PathBuf::from(expand_tilde(ca_raw));
        if !ca_path.exists() {
            return Err(anyhow::anyhow!("CA file not found: {}", ca_path.display()));
        }
        ssl_opts = ssl_opts.with_root_certs(vec![ca_path.into()]);
    }

    let cert_raw = profile.ssl_client_cert_path.trim();
    let key_raw = profile.ssl_client_key_path.trim();
    if !cert_raw.is_empty() || !key_raw.is_empty() {
        if cert_raw.is_empty() || key_raw.is_empty() {
            return Err(anyhow::anyhow!(
                "Both client certificate and client key paths must be provided"
            ));
        }
        let cert_path = std::path::PathBuf::from(expand_tilde(cert_raw));
        if !cert_path.exists() {
            return Err(anyhow::anyhow!(
                "Client certificate file not found: {}",
                cert_path.display()
            ));
        }
        let key_path = std::path::PathBuf::from(expand_tilde(key_raw));
        if !key_path.exists() {
            return Err(anyhow::anyhow!(
                "Client key file not found: {}",
                key_path.display()
            ));
        }
        ssl_opts = ssl_opts.with_client_identity(Some(mysql_async::ClientIdentity::new(
            cert_path.into(),
            key_path.into(),
        )));
    }

    match profile.ssl_mode {
        SslMode::Required | SslMode::Preferred => {
            if profile.ssl_accept_invalid_certs {
                ssl_opts = ssl_opts
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true);
            }
        }
        SslMode::VerifyCa => {
            ssl_opts = ssl_opts
                .with_danger_accept_invalid_certs(false)
                .with_danger_skip_domain_validation(true);
        }
        SslMode::VerifyIdentity => {
            ssl_opts = ssl_opts
                .with_danger_accept_invalid_certs(false)
                .with_danger_skip_domain_validation(false);
        }
        SslMode::Disabled => {}
    }

    Ok(ssl_opts)
}

fn supports_query_sample_text(version: &str) -> bool {
    let core = version.split('-').next().unwrap_or(version);
    let mut parts = core.split('.').filter_map(|p| p.parse::<u32>().ok());
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch) >= (8, 0, 3)
}

#[derive(Debug, Clone, Default)]
pub struct DbObjects {
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub procedures: Vec<String>,
    pub functions: Vec<String>,
    pub triggers: Vec<String>,
}

fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped).to_string_lossy().into_owned();
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
                .next_back()
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

    if let Some(db_schema) = schema
        && let Some(t_schema) = db_schema
            .tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(&table_name))
        && let Some(pk_col) = t_schema.columns.iter().find(|c| c.is_primary_key)
        && result_columns
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&pk_col.name))
    {
        return (Some(t_schema.name.clone()), Some(pk_col.name.clone()));
    }

    // Fallback if schema wasn't loaded yet and `id` column is present
    if result_columns.iter().any(|c| c.eq_ignore_ascii_case("id")) {
        return (Some(table_name), Some("id".to_string()));
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::build_demo_schema;
    #[test]
    fn test_detect_editable_table_and_pk() {
        let schema = build_demo_schema("ecommerce_prod");
        let cols = vec![
            "id".to_string(),
            "full_name".to_string(),
            "email".to_string(),
        ];
        let (tbl, pk) = detect_editable_table_and_pk(
            "SELECT id, full_name, email FROM users LIMIT 10",
            &cols,
            Some(&schema),
        );
        assert_eq!(tbl.as_deref(), Some("users"));
        assert_eq!(pk.as_deref(), Some("id"));
    }

    #[test]
    fn test_generate_routine_call_sql_in_out_inout() {
        let params = vec![
            RoutineParamInfo {
                ordinal: 1,
                mode: "IN".to_string(),
                name: "p_user_id".to_string(),
                data_type: "INT".to_string(),
            },
            RoutineParamInfo {
                ordinal: 2,
                mode: "INOUT".to_string(),
                name: "p_counter".to_string(),
                data_type: "INT".to_string(),
            },
            RoutineParamInfo {
                ordinal: 3,
                mode: "OUT".to_string(),
                name: "p_total".to_string(),
                data_type: "DECIMAL(10,2)".to_string(),
            },
        ];
        let sql = generate_routine_call_sql(
            "sp_customer_summary",
            &params,
            &["42".to_string(), "1".to_string(), "".to_string()],
        );
        assert!(sql.contains("SET @p_counter = 1;"));
        assert!(sql.contains("CALL `sp_customer_summary`(42, @p_counter, @p_total);"));
        assert!(sql.contains("SELECT @p_counter AS `p_counter`, @p_total AS `p_total`;"));
    }

    #[test]
    fn test_user_ddl_and_grant_parsing() {
        let create = build_create_user_sql("alice", "%", "s3cr'et", "caching_sha2_password");
        assert_eq!(
            create,
            "CREATE USER 'alice'@'%' IDENTIFIED WITH caching_sha2_password BY 's3cr''et'"
        );
        let stmts = build_grant_revoke_sql(
            "alice",
            "%",
            "*.*",
            &["SELECT".to_string(), "INSERT".to_string()],
            &["DELETE".to_string()],
        );
        assert_eq!(stmts[0], "GRANT SELECT, INSERT ON *.* TO 'alice'@'%'");
        assert_eq!(stmts[1], "REVOKE DELETE ON *.* FROM 'alice'@'%'");
        let parsed = parse_granted_privileges(&[
            "GRANT SELECT, INSERT, CREATE VIEW ON `shop`.* TO 'alice'@'%'".to_string(),
        ]);
        assert!(parsed.contains("SELECT"));
        assert!(parsed.contains("CREATE VIEW"));
        assert!(!parsed.contains("DELETE"));
    }

    #[tokio::test]
    async fn test_demo_sandbox_user_store() {
        let session = ActiveSession::DemoSandbox {
            store: crate::demo::DemoStore::default(),
        };
        let users = session.list_users().await.unwrap();
        assert_eq!(users.len(), 3);
        assert!(users.iter().any(|u| u.user == "admin"));
        assert!(users.iter().any(|u| u.user == "app_user"));
        assert!(users.iter().any(|u| u.user == "read_only"));
    }
    #[tokio::test]
    async fn test_server_admin_methods() {
        use crate::config::ConnectionProfile;
        use crate::db::ActiveSession;

        let session = ActiveSession::connect(ConnectionProfile::demo_profile())
            .await
            .expect("Failed to connect to demo sandbox");

        let procs = session
            .list_processes()
            .await
            .expect("Failed to list processes");
        assert!(procs.columns.contains(&"Id".to_string()));
        assert!(procs.columns.contains(&"User".to_string()));
        assert!(procs.columns.contains(&"Command".to_string()));
        assert!(!procs.rows.is_empty());

        let status = session
            .server_status_summary()
            .await
            .expect("Failed to get server status");
        assert!(status.contains_key("Uptime"));
        assert!(status.contains_key("Threads_connected"));
        assert!(status.contains_key("Questions"));

        let vars = session
            .list_server_variables()
            .await
            .expect("Failed to get server variables");
        assert!(!vars.is_empty());
        assert!(vars.iter().any(|(k, _)| k == "max_connections"));
        assert!(vars.iter().any(|(k, _)| k == "version"));

        session.kill_query(101).await.expect("Failed to kill query");
    }

    #[test]
    fn test_is_connection_error_classifier() {
        let io_err: anyhow::Error = mysql_async::Error::Io(mysql_async::IoError::Io(
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe"),
        ))
        .into();
        assert!(is_connection_error(&io_err));

        let closed_err: anyhow::Error =
            mysql_async::Error::Driver(mysql_async::DriverError::ConnectionClosed).into();
        assert!(is_connection_error(&closed_err));

        let pool_err: anyhow::Error =
            mysql_async::Error::Driver(mysql_async::DriverError::PoolDisconnected).into();
        assert!(is_connection_error(&pool_err));

        let ssh_err = anyhow!("SSH tunnel process exited (status: 255)");
        assert!(is_connection_error(&ssh_err));

        let other_err = anyhow!("Syntax error near SELECT");
        assert!(!is_connection_error(&other_err));
    }
}
