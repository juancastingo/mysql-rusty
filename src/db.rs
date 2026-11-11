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
            let local_port = profile.ssh.local_bind_port;
            let forward_spec = format!("{}:{}:{}", local_port, profile.host, profile.port);
            let mut cmd = Command::new("ssh");
            cmd.arg("-N")
                .arg("-L")
                .arg(&forward_spec)
                .arg("-p")
                .arg(profile.ssh.ssh_port.to_string())
                .arg("-o")
                .arg("ExitOnForwardFailure=yes")
                .arg("-o")
                .arg("StrictHostKeyChecking=accept-new");

            if !profile.ssh.ssh_key_path.trim().is_empty() {
                let expanded = expand_tilde(&profile.ssh.ssh_key_path);
                cmd.arg("-i").arg(expanded);
            }

            let target = if profile.ssh.ssh_user.trim().is_empty() {
                profile.ssh.ssh_host.clone()
            } else {
                format!("{}@{}", profile.ssh.ssh_user, profile.ssh.ssh_host)
            };
            cmd.arg(target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());

            let child = cmd
                .spawn()
                .context("Failed to start SSH tunnel subprocess (`ssh`)")?;

            // Brief wait for tunnel bind
            tokio::time::sleep(std::time::Duration::from_millis(450)).await;
            connect_host = "127.0.0.1".to_string();
            connect_port = local_port;
            ssh_guard = Some(Arc::new(Mutex::new(SshTunnelGuard { child })));
        }

        let mut builder = OptsBuilder::default()
            .ip_or_hostname(connect_host)
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

        match profile.ssl_mode {
            SslMode::Disabled => {
                builder = builder.ssl_opts(None::<SslOpts>);
            }
            SslMode::Preferred | SslMode::Required => {
                builder = builder.ssl_opts(Some(SslOpts::default()));
            }
        }

        let pool = Pool::new(builder);
        // Verify connectivity immediately
        let mut conn = pool
            .get_conn()
            .await
            .context("Failed to connect to MySQL/MariaDB server")?;
        let _: Option<u8> = conn.query_first("SELECT 1").await?;

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
            tables: vec![TableSchema {
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
            }],
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

fn execute_demo_query(
    store: &DemoStore,
    database: &str,
    sql: &str,
    _schema: Option<&DatabaseSchema>,
) -> Result<QueryResult> {
    let lower = sql.to_lowercase();
    let guard = store.tables.lock().unwrap();

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
            status_message: "3 aggregated rows in 3 ms (Sandbox)".into(),
        });
    }

    for (name, res) in guard.iter() {
        if lower.contains(name) {
            let mut cloned = res.clone();
            cloned.sql = sql.to_string();
            return Ok(cloned);
        }
    }

    // Default fallback to users table in demo mode
    let mut fallback = guard.get("users").unwrap().clone();
    fallback.sql = sql.to_string();
    Ok(fallback)
}
