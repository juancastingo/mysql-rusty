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
