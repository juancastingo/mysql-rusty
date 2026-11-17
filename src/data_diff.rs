use crate::db::{CellValue, QueryResult};
use std::collections::{BTreeMap, BTreeSet, HashSet};

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
    pub left_cells: Option<Vec<CellValue>>,
    #[allow(dead_code)]
    pub right_cells: Option<Vec<CellValue>>,
    pub key_cells: Vec<CellValue>,
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
    } else if s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok_and(|f| f.is_finite()) {
        s.to_string()
    } else {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('\'', "''")
            .replace('\0', "\\0")
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        format!("'{escaped}'")
    }
}

pub fn format_cell_value_sql(cell: &CellValue) -> String {
    match cell {
        CellValue::Null => "NULL".to_string(),
        CellValue::Integer(i) => i.to_string(),
        CellValue::Float(f) => {
            if f.is_finite() {
                format!("{f}")
            } else {
                "NULL".to_string()
            }
        }
        CellValue::Text(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('\'', "''")
                .replace('\0', "\\0")
                .replace('\r', "\\r")
                .replace('\n', "\\n");
            format!("'{escaped}'")
        }
        CellValue::Bytes(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2 + 3);
            hex.push_str("X'");
            for b in bytes {
                use std::fmt::Write;
                let _ = write!(hex, "{b:02X}");
            }
            hex.push('\'');
            hex
        }
    }
}

fn build_row_key(
    row: &[CellValue],
    key_indices: &[usize],
    row_idx: usize,
) -> (String, Vec<CellValue>) {
    let mut parts = Vec::with_capacity(key_indices.len());
    let mut cells = Vec::with_capacity(key_indices.len());
    for &ki in key_indices {
        let cell = row.get(ki).cloned().unwrap_or(CellValue::Null);
        parts.push(cell.display_text());
        cells.push(cell);
    }
    let joined = if parts.is_empty() || parts.iter().all(|p| p.is_empty()) {
        format!("__row_{row_idx}")
    } else {
        parts.join("\u{001F}")
    };
    (joined, cells)
}

/// Compares two `QueryResult` datasets keyed by `key_col_idx` (e.g. primary key column).
pub fn compute_data_diff(
    left: &QueryResult,
    right: &QueryResult,
    key_col_idx: usize,
) -> DataDiffReport {
    compute_data_diff_with_keys(left, right, &[key_col_idx])
}

/// Compares two `QueryResult` datasets keyed by one or more key columns (supports composite PKs).
pub fn compute_data_diff_with_keys(
    left: &QueryResult,
    right: &QueryResult,
    key_indices: &[usize],
) -> DataDiffReport {
    compute_data_diff_with_options(left, right, key_indices, &HashSet::new())
}

/// Like [`compute_data_diff_with_keys`], but ignores `excluded_cols` (such as generated columns)
/// when detecting modified rows.
pub fn compute_data_diff_with_options(
    left: &QueryResult,
    right: &QueryResult,
    key_indices: &[usize],
    excluded_cols: &HashSet<String>,
) -> DataDiffReport {
    let columns = if !left.columns.is_empty() {
        left.columns.clone()
    } else {
        right.columns.clone()
    };
    let num_cols = columns.len();
    let effective_keys: Vec<usize> = if key_indices.is_empty() {
        vec![0]
    } else {
        key_indices.to_vec()
    };

    type RowEntryMap = BTreeMap<String, (Vec<String>, Vec<CellValue>, Vec<CellValue>)>;

    let mut left_map: RowEntryMap = BTreeMap::new();
    for (idx, row) in left.rows.iter().enumerate() {
        let (key, k_cells) = build_row_key(row, &effective_keys, idx);
        let mut cells = row.to_vec();
        cells.resize(num_cols, CellValue::Null);
        left_map.insert(key, (row_to_strings(row, num_cols), cells, k_cells));
    }

    let mut right_map: RowEntryMap = BTreeMap::new();
    for (idx, row) in right.rows.iter().enumerate() {
        let (key, k_cells) = build_row_key(row, &effective_keys, idx);
        let mut cells = row.to_vec();
        cells.resize(num_cols, CellValue::Null);
        right_map.insert(key, (row_to_strings(row, num_cols), cells, k_cells));
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
        let key_display = key.replace('\u{001F}', " | ");
        match (left_map.remove(&key), right_map.remove(&key)) {
            (Some((l_row, l_cells, k_cells)), Some((r_row, r_cells, _))) => {
                let changed_cols: Vec<usize> = (0..num_cols)
                    .filter(|&c| {
                        columns
                            .get(c)
                            .is_none_or(|name| !excluded_cols.contains(name))
                            && l_row.get(c) != r_row.get(c)
                    })
                    .collect();
                if changed_cols.is_empty() {
                    same_count += 1;
                    entries.push(RowDiffEntry {
                        key_display,
                        status: RowDiffStatus::Same,
                        left_row: Some(l_row),
                        right_row: Some(r_row),
                        left_cells: Some(l_cells),
                        right_cells: Some(r_cells),
                        key_cells: k_cells,
                        changed_cols,
                    });
                } else {
                    modified_count += 1;
                    entries.push(RowDiffEntry {
                        key_display,
                        status: RowDiffStatus::Modified,
                        left_row: Some(l_row),
                        right_row: Some(r_row),
                        left_cells: Some(l_cells),
                        right_cells: Some(r_cells),
                        key_cells: k_cells,
                        changed_cols,
                    });
                }
            }
            (Some((l_row, l_cells, k_cells)), None) => {
                left_only_count += 1;
                entries.push(RowDiffEntry {
                    key_display,
                    status: RowDiffStatus::OnlyInLeft,
                    left_row: Some(l_row),
                    right_row: None,
                    left_cells: Some(l_cells),
                    right_cells: None,
                    key_cells: k_cells,
                    changed_cols: Vec::new(),
                });
            }
            (None, Some((r_row, r_cells, k_cells))) => {
                right_only_count += 1;
                entries.push(RowDiffEntry {
                    key_display,
                    status: RowDiffStatus::OnlyInRight,
                    left_row: None,
                    right_row: Some(r_row),
                    left_cells: None,
                    right_cells: Some(r_cells),
                    key_cells: k_cells,
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
#[cfg_attr(not(test), allow(dead_code))]
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

/// Like [`generate_data_diff_sync_sql`], but uses `INSERT ... ON DUPLICATE KEY UPDATE`
/// (or `INSERT IGNORE` if all columns are key) so rows that exist beyond the query
/// `LIMIT` or share a composite primary key never fail with `ERROR 1062 (Duplicate entry)`.
pub fn generate_data_diff_upsert_sql(
    target_table: &str,
    report: &DataDiffReport,
    key_col_idx: usize,
) -> String {
    generate_data_diff_upsert_sql_with_keys(target_table, report, &[key_col_idx], &HashSet::new())
}

/// Generates transactional-safe upsert/update/delete SQL using typed [`CellValue`] formatting,
/// composite primary keys (`key_indices`), and skipping generated columns (`excluded_cols`).
pub fn generate_data_diff_upsert_sql_with_keys(
    target_table: &str,
    report: &DataDiffReport,
    key_indices: &[usize],
    excluded_cols: &HashSet<String>,
) -> String {
    if report.columns.is_empty() {
        return String::new();
    }
    let effective_keys: Vec<usize> = if key_indices.is_empty() {
        vec![0]
    } else {
        key_indices.to_vec()
    };
    let active_cols: Vec<(usize, &String)> = report
        .columns
        .iter()
        .enumerate()
        .filter(|(_, name)| !excluded_cols.contains(*name))
        .collect();
    if active_cols.is_empty() {
        return String::new();
    }

    let cols_sql = active_cols
        .iter()
        .map(|(_, c)| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let update_assignments = active_cols
        .iter()
        .filter(|(idx, _)| !effective_keys.contains(idx))
        .map(|(_, c)| format!("`{c}` = VALUES(`{c}`)"))
        .collect::<Vec<_>>()
        .join(", ");

    let build_where = |entry: &RowDiffEntry| -> String {
        effective_keys
            .iter()
            .enumerate()
            .filter_map(|(k_pos, &col_idx)| {
                let col_name = report.columns.get(col_idx)?;
                let val_sql = if let Some(cell) = entry.key_cells.get(k_pos) {
                    format_cell_value_sql(cell)
                } else {
                    format_sql_val_str(&entry.key_display)
                };
                if val_sql == "NULL" {
                    Some(format!("`{col_name}` IS NULL"))
                } else {
                    Some(format!("`{col_name}` = {val_sql}"))
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    };

    let mut stmts = Vec::new();
    for entry in &report.entries {
        match entry.status {
            RowDiffStatus::OnlyInLeft => {
                let vals_opt: Option<String> = if let Some(cells) = &entry.left_cells {
                    Some(
                        active_cols
                            .iter()
                            .map(|(idx, _)| {
                                cells
                                    .get(*idx)
                                    .map(format_cell_value_sql)
                                    .unwrap_or_else(|| "NULL".to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                } else {
                    entry.left_row.as_ref().map(|l_row| {
                        active_cols
                            .iter()
                            .map(|(idx, _)| {
                                l_row
                                    .get(*idx)
                                    .map(|s| format_sql_val_str(s))
                                    .unwrap_or_else(|| "NULL".to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                };
                if let Some(vals) = vals_opt {
                    if update_assignments.is_empty() {
                        stmts.push(format!(
                            "INSERT IGNORE INTO `{target_table}` ({cols_sql}) VALUES ({vals});"
                        ));
                    } else {
                        stmts.push(format!(
                            "INSERT INTO `{target_table}` ({cols_sql}) VALUES ({vals}) ON DUPLICATE KEY UPDATE {update_assignments};"
                        ));
                    }
                }
            }
            RowDiffStatus::Modified => {
                let sets: String = active_cols
                    .iter()
                    .filter(|(idx, _)| {
                        !effective_keys.contains(idx) && entry.changed_cols.contains(idx)
                    })
                    .filter_map(|(idx, col_name)| {
                        let val_sql = if let Some(cells) = &entry.left_cells {
                            format_cell_value_sql(cells.get(*idx)?)
                        } else {
                            format_sql_val_str(entry.left_row.as_ref()?.get(*idx)?)
                        };
                        Some(format!("`{col_name}` = {val_sql}"))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if !sets.is_empty() {
                    let where_clause = build_where(entry);
                    if !where_clause.is_empty() {
                        stmts.push(format!(
                            "UPDATE `{target_table}` SET {sets} WHERE {where_clause};"
                        ));
                    }
                }
            }
            RowDiffStatus::OnlyInRight => {
                let where_clause = build_where(entry);
                if !where_clause.is_empty() {
                    stmts.push(format!(
                        "DELETE FROM `{target_table}` WHERE {where_clause};"
                    ));
                }
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

    #[test]
    fn test_format_sql_val_str_escapes_backslashes_for_json() {
        use super::format_sql_val_str;
        let json = r#"{"note": "say \"hello\"", "path": "a\\b"}"#;
        let formatted = format_sql_val_str(json);
        assert_eq!(
            formatted,
            r#"'{"note": "say \\"hello\\"", "path": "a\\\\b"}'"#
        );
        assert_eq!(format_sql_val_str("NaN"), "'NaN'");
        assert_eq!(format_sql_val_str("inf"), "'inf'");
        assert_eq!(format_sql_val_str("42"), "42");
    }

    #[test]
    fn test_typed_cell_formatting_composite_pk_and_generated_cols() {
        use super::{
            compute_data_diff_with_keys, format_cell_value_sql,
            generate_data_diff_upsert_sql_with_keys,
        };
        use crate::db::{CellValue, QueryResult};
        use std::collections::HashSet;

        assert_eq!(format_cell_value_sql(&CellValue::Null), "NULL");
        assert_eq!(
            format_cell_value_sql(&CellValue::Text("NULL".to_string())),
            "'NULL'"
        );
        assert_eq!(
            format_cell_value_sql(&CellValue::Text("00123".to_string())),
            "'00123'"
        );
        assert_eq!(
            format_cell_value_sql(&CellValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])),
            "X'DEADBEEF'"
        );

        let left = QueryResult {
            sql: String::new(),
            database: "db".into(),
            columns: vec![
                "deck_id".into(),
                "item_id".into(),
                "qty".into(),
                "gen_total".into(),
            ],
            rows: vec![
                vec![
                    CellValue::Text("d1".into()),
                    CellValue::Text("i1".into()),
                    CellValue::Integer(5),
                    CellValue::Integer(10),
                ],
                vec![
                    CellValue::Text("d1".into()),
                    CellValue::Text("i2".into()),
                    CellValue::Integer(3),
                    CellValue::Integer(6),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 0,
            editable_table: None,
            primary_key_column: None,
            status_message: String::new(),
            truncated: false,
        };
        let right = QueryResult {
            sql: String::new(),
            database: "db".into(),
            columns: left.columns.clone(),
            rows: vec![vec![
                CellValue::Text("d1".into()),
                CellValue::Text("i1".into()),
                CellValue::Integer(2),
                CellValue::Integer(4),
            ]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 0,
            editable_table: None,
            primary_key_column: None,
            status_message: String::new(),
            truncated: false,
        };

        let report = compute_data_diff_with_keys(&left, &right, &[0, 1]);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.left_only_count, 1);

        let mut excluded = HashSet::new();
        excluded.insert("gen_total".to_string());
        let sql =
            generate_data_diff_upsert_sql_with_keys("miq_deck_item", &report, &[0, 1], &excluded);
        assert!(sql.contains("WHERE `deck_id` = 'd1' AND `item_id` = 'i1';"));
        assert!(sql.contains("ON DUPLICATE KEY UPDATE `qty` = VALUES(`qty`);"));
        assert!(!sql.contains("gen_total"));
    }
}
