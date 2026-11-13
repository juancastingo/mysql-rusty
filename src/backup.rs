use crate::db::ActiveSession;
#[cfg(test)]
use crate::db::CellValue;
use crate::export::sql_literal;
use crate::sql_reader::SqlStatementReader;
use anyhow::Result;
use mysql_async::prelude::Queryable;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct BackupOptions {
    pub database: String,
    pub tables: Vec<String>,
    pub include_data: bool,
    pub include_drop: bool,
    pub include_views: bool,
    pub include_routines: bool,
    pub include_triggers: bool,
    pub batch_rows: usize,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            database: String::new(),
            tables: Vec::new(),
            include_data: true,
            include_drop: true,
            include_views: true,
            include_routines: true,
            include_triggers: true,
            batch_rows: 500,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackupProgress {
    Table {
        name: String,
        index: usize,
        total: usize,
    },
    Rows {
        table: String,
        done: u64,
    },
    Done {
        path: PathBuf,
        bytes: u64,
    },
    Failed(String),
    Cancelled,
}

pub fn format_backup_header(version: &str, database: &str, timestamp: &str) -> String {
    format!(
        "-- mysql-rusty dump v1 | server {version} | database `{database}` | {timestamp}\n\
         SET NAMES utf8mb4;\n\
         SET FOREIGN_KEY_CHECKS=0;\n\
         SET UNIQUE_CHECKS=0;\n\
         SET SQL_MODE='NO_AUTO_VALUE_ON_ZERO';\n\n"
    )
}

pub fn format_backup_footer() -> &'static str {
    "\nSET FOREIGN_KEY_CHECKS=1;\nSET UNIQUE_CHECKS=1;\n"
}

fn quote_ident(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

pub async fn run_backup(
    session: Arc<ActiveSession>,
    opts: BackupOptions,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(BackupProgress) + Send + 'static,
) -> Result<()> {
    let res = run_backup_inner(&session, &opts, &path, &cancel, &progress).await;
    match res {
        Ok(true) => {
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            progress(BackupProgress::Done { path, bytes });
            Ok(())
        }
        Ok(false) => {
            let _ = std::fs::remove_file(&path);
            progress(BackupProgress::Cancelled);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            let msg = format!("{e:#}");
            progress(BackupProgress::Failed(msg.clone()));
            Err(e)
        }
    }
}

async fn run_backup_inner(
    session: &ActiveSession,
    opts: &BackupOptions,
    path: &PathBuf,
    cancel: &AtomicBool,
    progress: &impl Fn(BackupProgress),
) -> Result<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);

    let version = session
        .server_version()
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    let timestamp = chrono::Utc::now().to_rfc3339();
    writer.write_all(format_backup_header(&version, &opts.database, &timestamp).as_bytes())?;

    let objects = session.list_objects(&opts.database).await?;
    let tables: Vec<String> = if opts.tables.is_empty() {
        objects.tables.clone()
    } else {
        opts.tables.clone()
    };
    let total = tables.len();

    for (idx, table) in tables.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        progress(BackupProgress::Table {
            name: table.clone(),
            index: idx + 1,
            total,
        });

        let q_table = quote_ident(table);
        if opts.include_drop {
            writeln!(writer, "DROP TABLE IF EXISTS {q_table};")?;
        }
        let ddl = session.show_create("TABLE", &opts.database, table).await?;
        writeln!(writer, "{ddl};\n")?;

        if opts.include_data {
            let batch_size = opts.batch_rows.max(1);
            let mut total_rows = 0u64;
            let table_name = table.clone();
            let result = session
                .stream_rows(&opts.database, table, batch_size, |cols, rows| {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(anyhow::anyhow!("__CANCELLED__"));
                    }
                    if rows.is_empty() {
                        return Ok(());
                    }
                    let col_list = cols
                        .iter()
                        .map(|c| quote_ident(c))
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(writer, "INSERT INTO {q_table} ({col_list}) VALUES")?;
                    for (r_idx, row) in rows.iter().enumerate() {
                        let vals = row.iter().map(sql_literal).collect::<Vec<_>>().join(", ");
                        if r_idx + 1 == rows.len() {
                            writeln!(writer, "  ({vals});")?;
                        } else {
                            writeln!(writer, "  ({vals}),")?;
                        }
                    }
                    total_rows += rows.len() as u64;
                    progress(BackupProgress::Rows {
                        table: table_name.clone(),
                        done: total_rows,
                    });
                    Ok(())
                })
                .await;

            match result {
                Ok(()) => {}
                Err(e) if e.to_string().contains("__CANCELLED__") => return Ok(false),
                Err(e) => return Err(e),
            }
            writeln!(writer)?;
        }
    }

    if opts.include_views {
        for view in &objects.views {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let q_view = quote_ident(view);
            if opts.include_drop {
                writeln!(writer, "DROP VIEW IF EXISTS {q_view};")?;
            }
            let ddl = session.show_create("VIEW", &opts.database, view).await?;
            writeln!(writer, "{ddl};\n")?;
        }
    }

    if opts.include_routines {
        for proc_name in &objects.procedures {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let q_proc = quote_ident(proc_name);
            if opts.include_drop {
                writeln!(writer, "DROP PROCEDURE IF EXISTS {q_proc};")?;
            }
            let ddl = session
                .show_create("PROCEDURE", &opts.database, proc_name)
                .await?;
            writeln!(writer, "DELIMITER ;;\n{ddl};;\nDELIMITER ;\n")?;
        }
        for func_name in &objects.functions {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let q_func = quote_ident(func_name);
            if opts.include_drop {
                writeln!(writer, "DROP FUNCTION IF EXISTS {q_func};")?;
            }
            let ddl = session
                .show_create("FUNCTION", &opts.database, func_name)
                .await?;
            writeln!(writer, "DELIMITER ;;\n{ddl};;\nDELIMITER ;\n")?;
        }
    }

    if opts.include_triggers {
        for trig_name in &objects.triggers {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let q_trig = quote_ident(trig_name);
            if opts.include_drop {
                writeln!(writer, "DROP TRIGGER IF EXISTS {q_trig};")?;
            }
            let ddl = session
                .show_create("TRIGGER", &opts.database, trig_name)
                .await?;
            writeln!(writer, "DELIMITER ;;\n{ddl};;\nDELIMITER ;\n")?;
        }
    }

    writer.write_all(format_backup_footer().as_bytes())?;
    writer.flush()?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    Stop,
    Continue,
}

#[derive(Debug, Clone)]
pub struct RestoreErrorEntry {
    pub statement_preview: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum RestoreProgress {
    Tick {
        bytes_done: u64,
        bytes_total: u64,
        statements_ok: u64,
        statements_failed: u64,
        last_error: Option<RestoreErrorEntry>,
    },
    Done {
        bytes_total: u64,
        statements_ok: u64,
        statements_failed: u64,
    },
    Failed(String),
    Cancelled {
        statements_ok: u64,
        statements_failed: u64,
    },
}

pub async fn run_sql_file(
    session: Arc<ActiveSession>,
    database: String,
    path: PathBuf,
    policy: OnError,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(RestoreProgress) + Send + 'static,
) -> Result<()> {
    let bytes_total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("Failed to open {}: {e}", path.display());
            progress(RestoreProgress::Failed(msg.clone()));
            return Err(anyhow::anyhow!(msg));
        }
    };

    let mut conn = match session.open_pool_conn(&database).await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{e:#}");
            progress(RestoreProgress::Failed(msg.clone()));
            return Err(e);
        }
    };

    let mut reader = SqlStatementReader::new(BufReader::with_capacity(64 * 1024, file));
    let mut statements_ok = 0u64;
    let mut statements_failed = 0u64;
    let mut last_tick = Instant::now();

    while let Some(item) = reader.next() {
        if cancel.load(Ordering::Relaxed) {
            progress(RestoreProgress::Cancelled {
                statements_ok,
                statements_failed,
            });
            return Ok(());
        }

        let stmt = match item {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("I/O error reading SQL file: {e}");
                progress(RestoreProgress::Failed(msg.clone()));
                return Err(anyhow::anyhow!(msg));
            }
        };

        let mut latest_err: Option<RestoreErrorEntry> = None;
        if let Err(e) = conn.query_drop(&stmt).await {
            statements_failed += 1;
            let preview: String = stmt.chars().take(120).collect();
            let entry = RestoreErrorEntry {
                statement_preview: preview,
                message: format!("{e:#}"),
            };
            latest_err = Some(entry.clone());
            if policy == OnError::Stop {
                progress(RestoreProgress::Tick {
                    bytes_done: reader.bytes_read(),
                    bytes_total,
                    statements_ok,
                    statements_failed,
                    last_error: Some(entry.clone()),
                });
                progress(RestoreProgress::Failed(entry.message.clone()));
                return Err(anyhow::anyhow!(entry.message));
            }
        } else {
            statements_ok += 1;
        }

        if latest_err.is_some() || last_tick.elapsed().as_millis() >= 200 {
            last_tick = Instant::now();
            progress(RestoreProgress::Tick {
                bytes_done: reader.bytes_read(),
                bytes_total,
                statements_ok,
                statements_failed,
                last_error: latest_err,
            });
        }
    }

    progress(RestoreProgress::Done {
        bytes_total,
        statements_ok,
        statements_failed,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_header_footer_and_sql_literal() {
        let hdr = format_backup_header("8.4.0", "shop", "2026-01-01T00:00:00Z");
        assert!(hdr.contains("-- mysql-rusty dump v1 | server 8.4.0 | database `shop`"));
        assert!(hdr.contains("SET FOREIGN_KEY_CHECKS=0;"));
        let ftr = format_backup_footer();
        assert!(ftr.contains("SET FOREIGN_KEY_CHECKS=1;"));

        assert_eq!(sql_literal(&CellValue::Null), "NULL");
        assert_eq!(sql_literal(&CellValue::Integer(42)), "42");
        assert_eq!(sql_literal(&CellValue::Float(3.5)), "3.5");
        assert_eq!(
            sql_literal(&CellValue::Text("a'b\\c\nd".to_string())),
            "'a''b\\\\c\\nd'"
        );
        assert_eq!(
            sql_literal(&CellValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])),
            "X'DEADBEEF'"
        );
    }
}
