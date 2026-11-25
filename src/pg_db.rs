use crate::config::{ConnectionProfile, SslMode};
use crate::db::{
    CellValue, ColumnInfo, DatabaseSchema, ForeignKeyInfo, IndexInfo, PendingCellEdit, QueryResult,
    SshTunnelGuard, TableSchema, ViewInfo,
};
use crate::dialect::Dialect;
use anyhow::{Context, Result, anyhow};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use rustls::ClientConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_postgres::{NoTls, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

pub struct PgSessionState {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub default_db: String,
    pub ssl_mode: SslMode,
    pub accept_invalid_certs: bool,
    pub pools: tokio::sync::Mutex<HashMap<String, Pool>>,
    pub _ssh_guard: Option<Arc<Mutex<SshTunnelGuard>>>,
}

#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn build_rustls_connector(accept_invalid_certs: bool) -> MakeRustlsConnect {
    let config = if accept_invalid_certs {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth()
    } else {
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };
    MakeRustlsConnect::new(config)
}

impl PgSessionState {
    pub async fn pool_for_db(&self, database: &str) -> Result<Pool> {
        let db_name = if database.trim().is_empty() {
            if self.default_db.trim().is_empty() {
                "postgres"
            } else {
                self.default_db.trim()
            }
        } else {
            database.trim()
        };

        let mut pools = self.pools.lock().await;
        if let Some(existing) = pools.get(db_name) {
            return Ok(existing.clone());
        }

        let mut pg_cfg = tokio_postgres::Config::new();
        pg_cfg.host(&self.host);
        pg_cfg.port(self.port);
        pg_cfg.user(&self.user);
        if !self.password.is_empty() {
            pg_cfg.password(&self.password);
        }
        pg_cfg.dbname(db_name);

        let build_pool = |use_tls: bool| -> Result<Pool> {
            let mgr_cfg = ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            };
            let mgr = if !use_tls {
                Manager::from_config(pg_cfg.clone(), NoTls, mgr_cfg)
            } else {
                let effective_accept_invalid = match self.ssl_mode {
                    SslMode::VerifyCa | SslMode::VerifyIdentity => false,
                    _ => self.accept_invalid_certs,
                };
                let tls = build_rustls_connector(effective_accept_invalid);
                Manager::from_config(pg_cfg.clone(), tls, mgr_cfg)
            };
            Pool::builder(mgr)
                .max_size(8)
                .runtime(Runtime::Tokio1)
                .build()
                .context("Failed to build PostgreSQL pool")
        };

        let pool = match self.ssl_mode {
            SslMode::Disabled => build_pool(false)?,
            SslMode::Preferred => {
                let tls_pool = build_pool(true)?;
                match tls_pool.get().await {
                    Ok(_) => tls_pool,
                    Err(tls_err) => {
                        tracing::warn!(
                            error = %tls_err,
                            "PostgreSQL TLS connection failed under SslMode::Preferred, falling back to NoTls"
                        );
                        build_pool(false)?
                    }
                }
            }
            SslMode::Required | SslMode::VerifyCa | SslMode::VerifyIdentity => build_pool(true)?,
        };
        pools.insert(db_name.to_string(), pool.clone());
        Ok(pool)
    }

    pub async fn default_client(&self) -> Result<deadpool_postgres::Object> {
        let pool = self.pool_for_db(&self.default_db).await?;
        pool.get()
            .await
            .context("Failed to acquire PostgreSQL connection")
    }
}

pub async fn ping(pg: &PgSessionState) -> Result<()> {
    if let Some(guard) = &pg._ssh_guard {
        let mut locked = guard
            .lock()
            .map_err(|_| anyhow!("SSH tunnel mutex poisoned"))?;
        locked.check_alive()?;
    }
    let client = pg.default_client().await?;
    client.simple_query("SELECT 1").await?;
    Ok(())
}

pub async fn connect_postgres(
    profile: &ConnectionProfile,
    connect_host: &str,
    connect_port: u16,
    ssh_guard: Option<Arc<Mutex<SshTunnelGuard>>>,
) -> Result<PgSessionState> {
    let default_db = if profile.default_database.trim().is_empty() {
        "postgres".to_string()
    } else {
        profile.default_database.trim().to_string()
    };
    let state = PgSessionState {
        host: connect_host.to_string(),
        port: connect_port,
        user: profile.user.clone(),
        password: profile.password.clone(),
        default_db,
        ssl_mode: profile.ssl_mode.clone(),
        accept_invalid_certs: profile.ssl_accept_invalid_certs,
        pools: tokio::sync::Mutex::new(HashMap::new()),
        _ssh_guard: ssh_guard,
    };
    // Verify connectivity immediately
    let client = state.default_client().await?;
    client.simple_query("SELECT 1").await?;
    Ok(state)
}

pub async fn list_databases(pg: &PgSessionState) -> Result<Vec<String>> {
    let client = pg.default_client().await?;
    let rows = client
        .query(
            "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY 1",
            &[],
        )
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<_, String>(0)).collect())
}

pub async fn introspect_database(pg: &PgSessionState, database: &str) -> Result<DatabaseSchema> {
    let pool = pg.pool_for_db(database).await?;
    let client = pool.get().await?;

    let tbl_rows = client
        .query(
            "SELECT table_schema, table_name, table_type
             FROM information_schema.tables
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
               AND table_schema NOT LIKE 'pg_toast%'
             ORDER BY table_schema, table_name",
            &[],
        )
        .await?;

    let col_rows = client
        .query(
            "SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default
             FROM information_schema.columns
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
             ORDER BY table_schema, table_name, ordinal_position",
            &[],
        )
        .await?;

    let pk_rows = client
        .query(
            "SELECT tc.table_schema, tc.table_name, kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
             WHERE tc.constraint_type = 'PRIMARY KEY'
               AND tc.table_schema NOT IN ('pg_catalog', 'information_schema')",
            &[],
        )
        .await?;

    let fk_rows = client
        .query(
            "SELECT tc.table_schema, tc.table_name, tc.constraint_name,
                    kcu.column_name, ccu.table_name AS ref_table, ccu.column_name AS ref_col
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
             JOIN information_schema.constraint_column_usage ccu
               ON tc.constraint_name = ccu.constraint_name
              AND tc.table_schema = ccu.table_schema
             WHERE tc.constraint_type = 'FOREIGN KEY'
               AND tc.table_schema NOT IN ('pg_catalog', 'information_schema')",
            &[],
        )
        .await?;

    let idx_rows = client
        .query(
            "SELECT schemaname, tablename, indexname, indexdef
             FROM pg_indexes
             WHERE schemaname NOT IN ('pg_catalog', 'information_schema')",
            &[],
        )
        .await?;

    let mut pk_map: HashMap<(String, String), Vec<String>> = HashMap::new();
    for r in pk_rows {
        let key = (r.get::<_, String>(0), r.get::<_, String>(1));
        pk_map.entry(key).or_default().push(r.get(2));
    }

    let mut fk_map: HashMap<(String, String), Vec<ForeignKeyInfo>> = HashMap::new();
    for r in fk_rows {
        let key = (r.get::<_, String>(0), r.get::<_, String>(1));
        fk_map.entry(key).or_default().push(ForeignKeyInfo {
            constraint_name: r.get(2),
            column_name: r.get(3),
            referenced_table: r.get(4),
            referenced_column: r.get(5),
        });
    }

    let mut idx_map: HashMap<(String, String), Vec<IndexInfo>> = HashMap::new();
    for r in idx_rows {
        let key = (r.get::<_, String>(0), r.get::<_, String>(1));
        let idx_name: String = r.get(2);
        let def: String = r.get(3);
        idx_map.entry(key).or_default().push(IndexInfo {
            index_name: idx_name,
            column_name: String::new(),
            non_unique: !def.to_ascii_uppercase().contains("UNIQUE"),
            index_type: "BTREE".to_string(),
        });
    }

    let mut col_map: HashMap<(String, String), Vec<ColumnInfo>> = HashMap::new();
    for r in col_rows {
        let schema_name: String = r.get(0);
        let table_name: String = r.get(1);
        let col_name: String = r.get(2);
        let is_pk = pk_map
            .get(&(schema_name.clone(), table_name.clone()))
            .is_some_and(|pks| pks.contains(&col_name));
        let is_nullable = r.get::<_, String>(4).eq_ignore_ascii_case("YES");
        col_map
            .entry((schema_name, table_name))
            .or_default()
            .push(ColumnInfo {
                name: col_name,
                data_type: r.get(3),
                is_nullable,
                is_primary_key: is_pk,
                default_value: r.get(5),
                extra: String::new(),
            });
    }

    let mut tables = Vec::new();
    let mut views = Vec::new();
    for r in tbl_rows {
        let schema_name: String = r.get(0);
        let table_name: String = r.get(1);
        let table_type: String = r.get(2);
        let display_name = if schema_name == "public" {
            table_name.clone()
        } else {
            format!("{schema_name}.{table_name}")
        };
        if table_type.eq_ignore_ascii_case("VIEW") {
            views.push(ViewInfo {
                name: display_name,
                definition: None,
                definer: String::new(),
                is_updatable: "NO".to_string(),
            });
        } else {
            let key = (schema_name, table_name);
            tables.push(TableSchema {
                name: display_name,
                table_type,
                engine: "PostgreSQL".to_string(),
                approx_rows: 0,
                columns: col_map.remove(&key).unwrap_or_default(),
                foreign_keys: fk_map.remove(&key).unwrap_or_default(),
                indexes: idx_map.remove(&key).unwrap_or_default(),
            });
        }
    }

    Ok(DatabaseSchema {
        name: if database.trim().is_empty() {
            pg.default_db.clone()
        } else {
            database.to_string()
        },
        tables,
        views,
        procedures: Vec::new(),
        functions: Vec::new(),
        triggers: Vec::new(),
        events: Vec::new(),
    })
}

pub async fn execute_query(
    pg: &PgSessionState,
    database: &str,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<QueryResult> {
    let start = Instant::now();
    let pool = pg.pool_for_db(database).await?;
    let client = pool.get().await?;
    let msgs = client.simple_query(sql).await?;

    let mut columns = Vec::new();
    let mut rows = Vec::new();
    let mut affected_rows = 0u64;
    let mut truncated = false;

    for msg in msgs {
        match msg {
            SimpleQueryMessage::Row(row) => {
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                }
                if let Some(limit) = max_rows
                    && rows.len() >= limit
                {
                    truncated = true;
                    continue;
                }
                let mut cells = Vec::with_capacity(columns.len());
                for i in 0..columns.len() {
                    match row.get(i) {
                        None => cells.push(CellValue::Null),
                        Some(s) => cells.push(CellValue::Text(s.to_string())),
                    }
                }
                rows.push(cells);
            }
            SimpleQueryMessage::CommandComplete(n) => {
                affected_rows += n;
            }
            _ => {}
        }
    }

    let duration_ms = start.elapsed().as_millis().max(1) as u64;
    let status_message = if !columns.is_empty() {
        format!("{} rows returned in {duration_ms} ms", rows.len())
    } else {
        format!("{affected_rows} rows affected in {duration_ms} ms")
    };

    Ok(QueryResult {
        sql: sql.to_string(),
        database: database.to_string(),
        columns,
        rows,
        affected_rows,
        last_insert_id: None,
        duration_ms,
        editable_table: None,
        primary_key_column: None,
        truncated,
        status_message,
    })
}

pub async fn apply_cell_edits(
    pg: &PgSessionState,
    database: &str,
    table: &str,
    edits: &[PendingCellEdit],
) -> Result<u64> {
    let pool = pg.pool_for_db(database).await?;
    let client = pool.get().await?;
    let q_tbl = if let Some((schema, tbl)) = table.split_once('.') {
        Dialect::Postgres.quote_qualified(schema, tbl)
    } else {
        Dialect::Postgres.quote_ident(table)
    };
    let mut total = 0u64;
    for edit in edits {
        let q_col = Dialect::Postgres.quote_ident(&edit.column_name);
        let q_pk = Dialect::Postgres.quote_ident(&edit.pk_column);
        let sql = format!("UPDATE {q_tbl} SET {q_col} = $1 WHERE {q_pk}::text = $2");
        let new_val: Option<&str> = if edit.new_value.eq_ignore_ascii_case("NULL") {
            None
        } else {
            Some(edit.new_value.as_str())
        };
        let n = client.execute(&sql, &[&new_val, &edit.pk_value]).await?;
        total += n;
    }
    Ok(total)
}

pub async fn list_processes(pg: &PgSessionState) -> Result<QueryResult> {
    execute_query(
        pg,
        &pg.default_db,
        "SELECT pid, usename, datname, state, COALESCE((now() - query_start)::text, '') AS duration, COALESCE(query, '') AS query FROM pg_stat_activity ORDER BY pid",
        Some(500),
    )
    .await
}

pub async fn kill_query(pg: &PgSessionState, pid: u64) -> Result<()> {
    let client = pg.default_client().await?;
    let pid_i32 = i32::try_from(pid).map_err(|_| anyhow!("Invalid PostgreSQL PID: {pid}"))?;
    client
        .query_one("SELECT pg_cancel_backend($1)", &[&pid_i32])
        .await?;
    Ok(())
}

pub async fn server_status_summary(pg: &PgSessionState) -> Result<HashMap<String, String>> {
    let client = pg.default_client().await?;
    let mut map = HashMap::new();
    if let Ok(row) = client
        .query_one(
            "SELECT version(), COALESCE((now() - pg_postmaster_start_time())::text, ''), (SELECT count(*)::text FROM pg_stat_activity)",
            &[],
        )
        .await
    {
        map.insert("Version".to_string(), row.get::<_, String>(0));
        map.insert("Uptime".to_string(), row.get::<_, String>(1));
        map.insert("Threads_connected".to_string(), row.get::<_, String>(2));
    }
    Ok(map)
}

pub async fn list_server_variables(pg: &PgSessionState) -> Result<Vec<(String, String)>> {
    let client = pg.default_client().await?;
    let rows = client.query("SHOW ALL", &[]).await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
        .collect())
}

pub async fn server_version(pg: &PgSessionState) -> Result<String> {
    let client = pg.default_client().await?;
    let row = client.query_one("SHOW server_version", &[]).await?;
    Ok(row.get::<_, String>(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_postgres_connect_and_query_if_env_set() {
        let Ok(url) = std::env::var("MYSQL_RUSTY_IT_PG_URL") else {
            return;
        };
        let (client, conn) = tokio_postgres::connect(&url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let rows = client.simple_query("SELECT 1 AS ok").await.unwrap();
        assert!(!rows.is_empty());
    }
}
