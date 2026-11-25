use crate::db::{
    CellValue, ColumnInfo, DatabaseSchema, ForeignKeyInfo, IndexInfo, PendingCellEdit, QueryResult,
    TableSchema, ViewInfo,
};
use crate::dialect::Dialect;
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, types::ValueRef};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn open_sqlite_file(path: &Path) -> Result<Arc<Mutex<Connection>>> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open SQLite database: {}", path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to enable SQLite foreign_keys pragma")?;
    Ok(Arc::new(Mutex::new(conn)))
}

pub fn list_databases(conn: &Mutex<Connection>) -> Result<Vec<String>> {
    let guard = conn.lock().map_err(|_| anyhow!("SQLite mutex poisoned"))?;
    let mut stmt = guard.prepare("PRAGMA database_list")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut dbs = Vec::new();
    for r in rows {
        dbs.push(r?);
    }
    if dbs.is_empty() {
        dbs.push("main".to_string());
    }
    Ok(dbs)
}

pub fn introspect_database(conn: &Mutex<Connection>, database: &str) -> Result<DatabaseSchema> {
    let guard = conn.lock().map_err(|_| anyhow!("SQLite mutex poisoned"))?;
    let mut tbl_stmt = guard.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let table_names: Vec<String> = tbl_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut tables = Vec::with_capacity(table_names.len());
    for tbl_name in table_names {
        let q_tbl = Dialect::Sqlite.quote_ident(&tbl_name);

        let mut columns = Vec::new();
        let mut col_stmt = guard.prepare(&format!("PRAGMA table_info({q_tbl})"))?;
        let col_rows = col_stmt.query_map([], |row| {
            let name: String = row.get(1)?;
            let data_type: String = row.get(2)?;
            let notnull: i64 = row.get(3)?;
            let default_value: Option<String> = row.get(4)?;
            let pk: i64 = row.get(5)?;
            Ok(ColumnInfo {
                name,
                data_type: if data_type.is_empty() {
                    "ANY".to_string()
                } else {
                    data_type
                },
                is_nullable: notnull == 0 && pk == 0,
                is_primary_key: pk > 0,
                default_value,
                extra: String::new(),
            })
        })?;
        for c in col_rows {
            columns.push(c?);
        }

        let mut foreign_keys = Vec::new();
        let mut fk_stmt = guard.prepare(&format!("PRAGMA foreign_key_list({q_tbl})"))?;
        let fk_rows = fk_stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let ref_table: String = row.get(2)?;
            let from_col: String = row.get(3)?;
            let to_col: Option<String> = row.get(4)?;
            Ok(ForeignKeyInfo {
                constraint_name: format!("fk_{tbl_name}_{id}"),
                column_name: from_col,
                referenced_table: ref_table,
                referenced_column: to_col.unwrap_or_else(|| "id".to_string()),
            })
        })?;
        for fk in fk_rows {
            foreign_keys.push(fk?);
        }

        let mut indexes = Vec::new();
        let mut idx_stmt = guard.prepare(&format!("PRAGMA index_list({q_tbl})"))?;
        let idx_list: Vec<(String, i64)> = idx_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (idx_name, unique) in idx_list {
            let q_idx = Dialect::Sqlite.quote_ident(&idx_name);
            let mut info_stmt = guard.prepare(&format!("PRAGMA index_info({q_idx})"))?;
            let cols: Vec<String> = info_stmt
                .query_map([], |row| row.get::<_, Option<String>>(2))?
                .filter_map(|r| match r {
                    Ok(Some(col)) => Some(Ok(col)),
                    Ok(None) => None, // Expression index column
                    Err(e) => Some(Err(e)),
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for col in cols {
                indexes.push(IndexInfo {
                    index_name: idx_name.clone(),
                    column_name: col,
                    non_unique: unique == 0,
                    index_type: "BTREE".to_string(),
                });
            }
        }

        // Read approximate row count from sqlite_stat1 if ANALYZE has been run,
        // avoiding full-table COUNT(*) scans on every table during introspection.
        let approx_rows: u64 = guard
            .query_row(
                "SELECT CAST(stat AS INTEGER) FROM sqlite_stat1 WHERE tbl = ?1 LIMIT 1",
                [&tbl_name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            .max(0) as u64;

        tables.push(TableSchema {
            name: tbl_name,
            table_type: "BASE TABLE".to_string(),
            engine: "SQLite".to_string(),
            approx_rows,
            columns,
            foreign_keys,
            indexes,
        });
    }

    let mut view_stmt =
        guard.prepare("SELECT name FROM sqlite_master WHERE type = 'view' ORDER BY name")?;
    let views: Vec<ViewInfo> = view_stmt
        .query_map([], |row| {
            Ok(ViewInfo {
                name: row.get(0)?,
                definition: None,
                definer: String::new(),
                is_updatable: "NO".to_string(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(DatabaseSchema {
        name: if database.trim().is_empty() {
            "main".to_string()
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

fn value_ref_to_cell(v: ValueRef<'_>) -> CellValue {
    match v {
        ValueRef::Null => CellValue::Null,
        ValueRef::Integer(i) => CellValue::Integer(i),
        ValueRef::Real(f) => CellValue::Float(f),
        ValueRef::Text(b) => CellValue::Text(String::from_utf8_lossy(b).into_owned()),
        ValueRef::Blob(b) => CellValue::Bytes(b.to_vec()),
    }
}

pub fn execute_query(
    conn: &Mutex<Connection>,
    database: &str,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<QueryResult> {
    let start = Instant::now();
    let guard = conn.lock().map_err(|_| anyhow!("SQLite mutex poisoned"))?;
    let mut stmt = guard.prepare(sql)?;
    let col_count = stmt.column_count();

    if col_count > 0 {
        let columns: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("").to_string())
            .collect();
        let mut rows_out = Vec::new();
        let mut truncated = false;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if let Some(limit) = max_rows
                && rows_out.len() >= limit
            {
                truncated = true;
                break;
            }
            let mut cells = Vec::with_capacity(col_count);
            for i in 0..col_count {
                cells.push(value_ref_to_cell(row.get_ref(i)?));
            }
            rows_out.push(cells);
        }
        let duration_ms = start.elapsed().as_millis().max(1) as u64;
        let row_len = rows_out.len();
        Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns,
            rows: rows_out,
            affected_rows: 0,
            last_insert_id: None,
            duration_ms,
            editable_table: None,
            primary_key_column: None,
            truncated,
            status_message: format!("{row_len} rows returned in {duration_ms} ms"),
        })
    } else {
        let affected = stmt.execute([])? as u64;
        let last_id = guard.last_insert_rowid();
        let duration_ms = start.elapsed().as_millis().max(1) as u64;
        Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: Vec::new(),
            rows: Vec::new(),
            affected_rows: affected,
            last_insert_id: if last_id > 0 {
                Some(last_id as u64)
            } else {
                None
            },
            duration_ms,
            editable_table: None,
            primary_key_column: None,
            truncated: false,
            status_message: format!("{affected} rows affected in {duration_ms} ms"),
        })
    }
}

pub fn apply_cell_edits(
    conn: &Mutex<Connection>,
    table: &str,
    edits: &[PendingCellEdit],
) -> Result<u64> {
    let guard = conn.lock().map_err(|_| anyhow!("SQLite mutex poisoned"))?;
    let q_tbl = Dialect::Sqlite.quote_ident(table);
    let mut total = 0u64;
    for edit in edits {
        let q_col = Dialect::Sqlite.quote_ident(&edit.column_name);
        let q_pk = Dialect::Sqlite.quote_ident(&edit.pk_column);
        let sql = format!("UPDATE {q_tbl} SET {q_col} = ?1 WHERE {q_pk} = ?2");
        let new_val: Option<&str> = if edit.new_value.eq_ignore_ascii_case("NULL") {
            None
        } else {
            Some(edit.new_value.as_str())
        };
        let affected = guard.execute(&sql, rusqlite::params![new_val, edit.pk_value])?;
        total += affected as u64;
    }
    Ok(total)
}

pub fn show_create(conn: &Mutex<Connection>, kind: &str, name: &str) -> Result<String> {
    let guard = conn.lock().map_err(|_| anyhow!("SQLite mutex poisoned"))?;
    let kind_lower = kind.to_ascii_lowercase();
    let sql: String = guard
        .query_row(
            "SELECT COALESCE(sql, '') FROM sqlite_master WHERE lower(type) = ?1 AND name = ?2",
            rusqlite::params![kind_lower, name],
            |row| row.get(0),
        )
        .with_context(|| format!("Object {kind} '{name}' not found in SQLite"))?;
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_introspection_types_and_roundtrip() {
        let raw = Connection::open_in_memory().unwrap();
        raw.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                score REAL
            );
            CREATE INDEX idx_users_name ON users(name);
            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id),
                payload BLOB
            );
            INSERT INTO users (name, score) VALUES ('Alice', 99.5);
            ",
        )
        .unwrap();
        let conn = Arc::new(Mutex::new(raw));

        let dbs = list_databases(&conn).unwrap();
        assert!(dbs.contains(&"main".to_string()));

        let schema = introspect_database(&conn, "main").unwrap();
        assert_eq!(schema.tables.len(), 2);
        let users = schema.tables.iter().find(|t| t.name == "users").unwrap();
        assert!(
            users
                .columns
                .iter()
                .any(|c| c.name == "id" && c.is_primary_key)
        );
        assert!(
            users
                .indexes
                .iter()
                .any(|i| i.index_name == "idx_users_name")
        );

        let orders = schema.tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.foreign_keys.len(), 1);
        assert_eq!(orders.foreign_keys[0].referenced_table, "users");

        // Insert, select, update roundtrip
        let ins = execute_query(
            &conn,
            "main",
            "INSERT INTO orders (id, user_id, payload) VALUES (10, 1, X'DEADBEEF')",
            None,
        )
        .unwrap();
        assert_eq!(ins.affected_rows, 1);

        let sel =
            execute_query(&conn, "main", "SELECT id, name, score FROM users", Some(10)).unwrap();
        assert_eq!(sel.rows.len(), 1);
        assert_eq!(sel.rows[0][0], CellValue::Integer(1));
        assert_eq!(sel.rows[0][1], CellValue::Text("Alice".to_string()));

        let updated = apply_cell_edits(
            &conn,
            "users",
            &[PendingCellEdit {
                row_idx: 0,
                col_idx: 1,
                column_name: "name".to_string(),
                pk_column: "id".to_string(),
                pk_value: "1".to_string(),
                new_value: "Alicia".to_string(),
            }],
        )
        .unwrap();
        assert_eq!(updated, 1);

        let ddl = show_create(&conn, "TABLE", "users").unwrap();
        assert!(ddl.contains("CREATE TABLE users"));
    }

    #[test]
    fn test_sqlite_introspection_handles_expression_index() {
        let raw = Connection::open_in_memory().unwrap();
        raw.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
             CREATE INDEX idx_users_lower_name ON users(lower(name));",
        )
        .unwrap();
        let conn = Arc::new(Mutex::new(raw));
        let schema = introspect_database(&conn, "main").unwrap();
        assert!(schema.tables.iter().any(|t| t.name == "users"));
    }
}
