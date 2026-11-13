use crate::db::{CellValue, QueryResult};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowDiffStatus {
    Same,
    OnlyInLeft,
    OnlyInRight,
    Modified,
}

#[derive(Debug, Clone)]
pub struct RowDiffEntry {
    pub key_display: String,
    pub status: RowDiffStatus,
    pub left_row: Option<Vec<String>>,
    pub right_row: Option<Vec<String>>,
    pub changed_cols: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct DataDiffReport {
    pub columns: Vec<String>,
    pub entries: Vec<RowDiffEntry>,
    pub same_count: usize,
    pub left_only_count: usize,
    pub right_only_count: usize,
    pub modified_count: usize,
}

fn row_to_strings(row: &[CellValue], num_cols: usize) -> Vec<String> {
    (0..num_cols)
        .map(|i| {
            row.get(i)
                .map(|c| {
                    if c.is_null() {
                        "NULL".to_string()
                    } else {
                        c.display_text()
                    }
                })
                .unwrap_or_else(|| "NULL".to_string())
        })
        .collect()
}

fn format_sql_val_str(s: &str) -> String {
    if s == "NULL" {
        "NULL".to_string()
    } else if s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok() {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

/// Compares two `QueryResult` datasets keyed by `key_col_idx` (e.g. primary key column).
pub fn compute_data_diff(
    left: &QueryResult,
    right: &QueryResult,
    key_col_idx: usize,
) -> DataDiffReport {
    let columns = if !left.columns.is_empty() {
        left.columns.clone()
    } else {
        right.columns.clone()
    };
    let num_cols = columns.len();

    let mut left_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (idx, row) in left.rows.iter().enumerate() {
        let key = row
            .get(key_col_idx)
            .map(|c| c.display_text())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("__row_{idx}"));
        left_map.insert(key, row_to_strings(row, num_cols));
    }

    let mut right_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (idx, row) in right.rows.iter().enumerate() {
        let key = row
            .get(key_col_idx)
            .map(|c| c.display_text())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("__row_{idx}"));
        right_map.insert(key, row_to_strings(row, num_cols));
    }

    let mut all_keys: BTreeSet<String> = BTreeSet::new();
    all_keys.extend(left_map.keys().cloned());
    all_keys.extend(right_map.keys().cloned());

    let mut entries = Vec::new();
    let mut same_count = 0;
    let mut left_only_count = 0;
    let mut right_only_count = 0;
    let mut modified_count = 0;

    for key in all_keys {
        match (left_map.remove(&key), right_map.remove(&key)) {
            (Some(l_row), Some(r_row)) => {
                let changed_cols: Vec<usize> = (0..num_cols)
                    .filter(|&c| l_row.get(c) != r_row.get(c))
                    .collect();
                if changed_cols.is_empty() {
                    same_count += 1;
                    entries.push(RowDiffEntry {
                        key_display: key,
                        status: RowDiffStatus::Same,
                        left_row: Some(l_row),
                        right_row: Some(r_row),
                        changed_cols,
                    });
                } else {
                    modified_count += 1;
                    entries.push(RowDiffEntry {
                        key_display: key,
                        status: RowDiffStatus::Modified,
                        left_row: Some(l_row),
                        right_row: Some(r_row),
                        changed_cols,
                    });
                }
            }
            (Some(l_row), None) => {
                left_only_count += 1;
                entries.push(RowDiffEntry {
                    key_display: key,
                    status: RowDiffStatus::OnlyInLeft,
                    left_row: Some(l_row),
                    right_row: None,
                    changed_cols: Vec::new(),
                });
            }
            (None, Some(r_row)) => {
                right_only_count += 1;
                entries.push(RowDiffEntry {
                    key_display: key,
                    status: RowDiffStatus::OnlyInRight,
                    left_row: None,
                    right_row: Some(r_row),
                    changed_cols: Vec::new(),
                });
            }
            (None, None) => {}
        }
    }

    DataDiffReport {
        columns,
        entries,
        same_count,
        left_only_count,
        right_only_count,
        modified_count,
    }
}

/// Generates SQL statements to synchronize `target_table` (Right) to match Source (Left).
pub fn generate_data_diff_sync_sql(
    target_table: &str,
    report: &DataDiffReport,
    key_col_idx: usize,
) -> String {
    if report.columns.is_empty() {
        return String::new();
    }
    let pk_col = report
        .columns
        .get(key_col_idx)
        .cloned()
        .unwrap_or_else(|| "id".to_string());
    let cols_sql = report
        .columns
        .iter()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut stmts = Vec::new();
    for entry in &report.entries {
        match entry.status {
            RowDiffStatus::OnlyInLeft => {
                if let Some(l_row) = &entry.left_row {
                    let vals = l_row
                        .iter()
                        .map(|v| format_sql_val_str(v))
                        .collect::<Vec<_>>()
                        .join(", ");
                    stmts.push(format!(
                        "INSERT INTO `{target_table}` ({cols_sql}) VALUES ({vals});"
                    ));
                }
            }
            RowDiffStatus::Modified => {
                if let Some(l_row) = &entry.left_row {
                    let sets = entry
                        .changed_cols
                        .iter()
                        .filter(|&&c| c != key_col_idx)
                        .filter_map(|&c| {
                            let col_name = report.columns.get(c)?;
                            let val = l_row.get(c)?;
                            Some(format!("`{col_name}` = {}", format_sql_val_str(val)))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !sets.is_empty() {
                        stmts.push(format!(
                            "UPDATE `{target_table}` SET {sets} WHERE `{pk_col}` = {};",
                            format_sql_val_str(&entry.key_display)
                        ));
                    }
                }
            }
            RowDiffStatus::OnlyInRight => {
                stmts.push(format!(
                    "DELETE FROM `{target_table}` WHERE `{pk_col}` = {};",
                    format_sql_val_str(&entry.key_display)
                ));
            }
            RowDiffStatus::Same => {}
        }
    }

    if stmts.is_empty() {
        "-- Tables are identical; no synchronization statements required.".to_string()
    } else {
        stmts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_row_copy_dump_and_data_diff() {
        use crate::data_diff::{RowDiffStatus, compute_data_diff, generate_data_diff_sync_sql};
        use crate::db::{CellValue, ColumnInfo, QueryResult, TableSchema};
        use crate::export::{
            format_rows_as_json, format_rows_as_sql_insert, format_rows_as_sql_update,
            generate_table_dump_sql,
        };

        let left = QueryResult {
            sql: "SELECT id, name FROM users".to_string(),
            database: "dev".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec![CellValue::Integer(1), CellValue::Text("Alice".to_string())],
                vec![CellValue::Integer(2), CellValue::Text("Bob".to_string())],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: Some("users".to_string()),
            primary_key_column: Some("id".to_string()),
            status_message: "OK".to_string(),
            truncated: false,
        };

        let right = QueryResult {
            sql: "SELECT id, name FROM users".to_string(),
            database: "prod".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("Alice V2".to_string()),
                ],
                vec![
                    CellValue::Integer(3),
                    CellValue::Text("Charlie".to_string()),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: Some("users".to_string()),
            primary_key_column: Some("id".to_string()),
            status_message: "OK".to_string(),
            truncated: false,
        };

        let json_one = format_rows_as_json(&left, &[0]);
        assert!(json_one.contains("\"name\": \"Alice\""));

        let ins = format_rows_as_sql_insert("users", &left, &[0, 1]);
        assert!(ins.contains("INSERT INTO `users` (`id`, `name`) VALUES"));
        assert!(ins.contains("(1, 'Alice')"));

        let upd = format_rows_as_sql_update("users", "id", &left, &[0]);
        assert_eq!(upd, "UPDATE `users` SET `name` = 'Alice' WHERE `id` = 1;");

        let schema = TableSchema {
            name: "users".to_string(),
            table_type: "BASE TABLE".to_string(),
            engine: "InnoDB".to_string(),
            approx_rows: 2,
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    is_nullable: false,
                    default_value: None,
                    extra: "auto_increment".to_string(),
                    is_primary_key: true,
                },
                ColumnInfo {
                    name: "name".to_string(),
                    data_type: "VARCHAR(100)".to_string(),
                    is_nullable: false,
                    default_value: None,
                    extra: String::new(),
                    is_primary_key: false,
                },
            ],
            foreign_keys: vec![],
            indexes: vec![],
        };
        let dump = generate_table_dump_sql(&schema, Some(&left), true, true, 1);
        assert!(dump.contains("DROP TABLE IF EXISTS `users`;"));
        assert!(dump.contains("CREATE TABLE `users`"));
        assert!(dump.contains("INSERT INTO `users`"));

        let report = compute_data_diff(&left, &right, 0);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.left_only_count, 1);
        assert_eq!(report.right_only_count, 1);
        assert!(
            report
                .entries
                .iter()
                .any(|e| e.key_display == "1" && e.status == RowDiffStatus::Modified)
        );

        let sync = generate_data_diff_sync_sql("users", &report, 0);
        assert!(sync.contains("UPDATE `users` SET `name` = 'Alice' WHERE `id` = 1;"));
    }

    #[test]
    fn snapshot_data_diff_report_and_sync_sql() {
        use crate::data_diff::{compute_data_diff, generate_data_diff_sync_sql};
        use crate::db::{CellValue, QueryResult};
        use crate::test_fixtures::sample_result;

        let left = sample_result();
        let right = QueryResult {
            sql: left.sql.clone(),
            database: left.database.clone(),
            columns: left.columns.clone(),
            rows: vec![
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("O'Brien \"Changed\"".to_string()),
                    CellValue::Text("Line1\nLine2, with comma".to_string()),
                    CellValue::Float(99.0),
                ],
                vec![
                    CellValue::Integer(3),
                    CellValue::Text("Bob".to_string()),
                    CellValue::Text("Simple".to_string()),
                    CellValue::Null,
                ],
                vec![
                    CellValue::Integer(4),
                    CellValue::Text("Extra".to_string()),
                    CellValue::Null,
                    CellValue::Float(1.5),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 4,
            editable_table: left.editable_table.clone(),
            primary_key_column: left.primary_key_column.clone(),
            status_message: String::new(),
            truncated: false,
        };

        let report = compute_data_diff(&left, &right, 0);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.left_only_count, 1);
        assert_eq!(report.right_only_count, 1);
        let sync = generate_data_diff_sync_sql("users", &report, 0);
        insta::assert_snapshot!(sync);
    }
}
