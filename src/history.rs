use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct HistoryStore {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct HistoryRecord {
    #[allow(dead_code)]
    pub id: i64,
    pub ts: String, // RFC3339
    pub profile_id: String,
    pub profile_name: String,
    pub database: String,
    pub sql: String,
    pub duration_ms: u64,
    pub row_count: u64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub text: String,
    pub profile_id: Option<String>,
    pub database: Option<String>,
    pub only_failed: bool,
    pub limit: usize,
    pub offset: usize,
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            _ => out.push(ch),
        }
    }
    out
}

impl HistoryStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS query_history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts TEXT NOT NULL,
                 profile_id TEXT NOT NULL,
                 profile_name TEXT NOT NULL,
                 database_name TEXT NOT NULL,
                 sql TEXT NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 row_count INTEGER NOT NULL,
                 success INTEGER NOT NULL,
                 error TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_history_ts ON query_history(ts DESC);
             CREATE INDEX IF NOT EXISTS idx_history_profile ON query_history(profile_id);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, r: &HistoryRecord) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("History mutex poisoned"))?;
        conn.execute(
            "INSERT INTO query_history (ts, profile_id, profile_name, database_name, sql, duration_ms, row_count, success, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                r.ts,
                r.profile_id,
                r.profile_name,
                r.database,
                r.sql,
                r.duration_ms as i64,
                r.row_count as i64,
                if r.success { 1i64 } else { 0i64 },
                r.error,
            ],
        )?;
        Ok(())
    }

    pub fn search(&self, q: &HistoryQuery) -> anyhow::Result<Vec<HistoryRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("History mutex poisoned"))?;

        let mut clauses: Vec<&str> = Vec::new();
        let mut owned_params: Vec<rusqlite::types::Value> = Vec::new();

        if !q.text.trim().is_empty() {
            let pattern = format!("%{}%", escape_like(q.text.trim()));
            clauses.push("(sql LIKE ? ESCAPE '\\' OR database_name LIKE ? ESCAPE '\\')");
            owned_params.push(rusqlite::types::Value::Text(pattern.clone()));
            owned_params.push(rusqlite::types::Value::Text(pattern));
        }
        if let Some(pid) = &q.profile_id
            && !pid.is_empty()
        {
            clauses.push("profile_id = ?");
            owned_params.push(rusqlite::types::Value::Text(pid.clone()));
        }
        if let Some(db) = &q.database
            && !db.is_empty()
        {
            clauses.push("database_name = ?");
            owned_params.push(rusqlite::types::Value::Text(db.clone()));
        }
        if q.only_failed {
            clauses.push("success = 0");
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let limit = if q.limit == 0 { 100 } else { q.limit } as i64;
        let offset = q.offset as i64;
        owned_params.push(rusqlite::types::Value::Integer(limit));
        owned_params.push(rusqlite::types::Value::Integer(offset));

        let sql = format!(
            "SELECT id, ts, profile_id, profile_name, database_name, sql, duration_ms, row_count, success, error
             FROM query_history
             {where_sql}
             ORDER BY ts DESC, id DESC
             LIMIT ? OFFSET ?"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(owned_params.iter()), |row| {
            Ok(HistoryRecord {
                id: row.get(0)?,
                ts: row.get(1)?,
                profile_id: row.get(2)?,
                profile_name: row.get(3)?,
                database: row.get(4)?,
                sql: row.get(5)?,
                duration_ms: row.get::<_, i64>(6)?.max(0) as u64,
                row_count: row.get::<_, i64>(7)?.max(0) as u64,
                success: row.get::<_, i64>(8)? != 0,
                error: row.get(9)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn prune(&self, keep_last: usize) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("History mutex poisoned"))?;
        let deleted = conn.execute(
            "DELETE FROM query_history
             WHERE id NOT IN (
                 SELECT id FROM query_history ORDER BY ts DESC, id DESC LIMIT ?1
             )",
            params![keep_last as i64],
        )?;
        Ok(deleted)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("History mutex poisoned"))?;
        conn.execute("DELETE FROM query_history", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db_path() -> PathBuf {
        std::env::temp_dir().join(format!("mysql_rusty_hist_{}.sqlite3", uuid::Uuid::new_v4()))
    }

    fn rec(
        ts: &str,
        pid: &str,
        db: &str,
        sql: &str,
        success: bool,
        err: Option<&str>,
    ) -> HistoryRecord {
        HistoryRecord {
            id: 0,
            ts: ts.to_string(),
            profile_id: pid.to_string(),
            profile_name: "Test".to_string(),
            database: db.to_string(),
            sql: sql.to_string(),
            duration_ms: 12,
            row_count: 3,
            success,
            error: err.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_history_insert_search_like_escape_prune_and_failed() {
        let path = temp_db_path();
        let store = HistoryStore::open(&path).expect("open history store");

        store
            .insert(&rec(
                "2026-01-01T10:00:00Z",
                "p1",
                "app",
                "SELECT 100% FROM users",
                true,
                None,
            ))
            .unwrap();
        store
            .insert(&rec(
                "2026-01-01T10:01:00Z",
                "p1",
                "app",
                "SELECT user_id FROM orders",
                true,
                None,
            ))
            .unwrap();
        store
            .insert(&rec(
                "2026-01-01T10:02:00Z",
                "p2",
                "billing",
                "SELECT * FROM missing_tbl",
                false,
                Some("Table doesn't exist"),
            ))
            .unwrap();

        // LIKE escaping for '%' and '_'
        let pct_hits = store
            .search(&HistoryQuery {
                text: "100%".to_string(),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(pct_hits.len(), 1);
        assert!(pct_hits[0].sql.contains("100%"));

        let under_hits = store
            .search(&HistoryQuery {
                text: "user_id".to_string(),
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(under_hits.len(), 1);
        assert!(under_hits[0].sql.contains("user_id"));

        // only_failed + profile filter
        let failed = store
            .search(&HistoryQuery {
                only_failed: true,
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(failed.len(), 1);
        assert!(!failed[0].success);
        assert_eq!(failed[0].error.as_deref(), Some("Table doesn't exist"));

        // prune keep_last = 2
        let deleted = store.prune(2).unwrap();
        assert_eq!(deleted, 1);
        let remaining = store
            .search(&HistoryQuery {
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].ts, "2026-01-01T10:02:00Z");

        store.clear().unwrap();
        assert!(
            store
                .search(&HistoryQuery {
                    limit: 50,
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_file(&path);
    }
}
