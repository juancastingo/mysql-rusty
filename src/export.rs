use crate::db::{CellValue, QueryResult};
use anyhow::Result;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

pub fn to_csv_string(result: &QueryResult) -> Result<String> {
    let mut wtr = csv::Writer::from_writer(vec![]);
    wtr.write_record(&result.columns)?;
    for row in &result.rows {
        let record: Vec<String> = row
            .iter()
            .map(|c| match c {
                CellValue::Null => String::new(),
                other => other.display_text(),
            })
            .collect();
        wtr.write_record(&record)?;
    }
    let bytes = wtr.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

pub fn to_json_string(result: &QueryResult) -> Result<String> {
    let mut items = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let mut obj = Map::new();
        for (col_name, cell) in result.columns.iter().zip(row.iter()) {
            let val = match cell {
                CellValue::Null => Value::Null,
                CellValue::Integer(i) => Value::from(*i),
                CellValue::Float(f) => Value::from(*f),
                CellValue::Text(s) => Value::String(s.clone()),
                CellValue::Bytes(_b) => Value::String(cell.display_text()),
            };
            obj.insert(col_name.clone(), val);
        }
        items.push(Value::Object(obj));
    }
    Ok(serde_json::to_string_pretty(&items)?)
}

pub fn to_sql_inserts(result: &QueryResult, target_table: &str) -> String {
    let tbl = if target_table.trim().is_empty() {
        "exported_table"
    } else {
        target_table
    };
    let cols = result
        .columns
        .iter()
        .map(|c| format!("`{}`", c.replace('`', "``")))
        .collect::<Vec<_>>()
        .join(", ");

    let mut out = String::new();
    for row in &result.rows {
        let vals = row
            .iter()
            .map(|c| match c {
                CellValue::Null => "NULL".to_string(),
                CellValue::Integer(i) => i.to_string(),
                CellValue::Float(f) => f.to_string(),
                CellValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
                CellValue::Bytes(_) => format!("'{}'", c.display_text()),
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("INSERT INTO `{tbl}` ({cols}) VALUES ({vals});\n"));
    }
    out
}

pub fn save_string_to_path(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content)?;
    Ok(())
}

/// Parses a CSV string into a MySQL multi-row batch `INSERT INTO ... VALUES (...)` statement.
pub fn parse_csv_to_batch_insert(table: &str, csv_content: &str) -> Result<String> {
    let trimmed = csv_content.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("CSV content is empty."));
    }

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(trimmed.as_bytes());

    let headers = reader.headers()?.clone();
    if headers.is_empty() {
        return Err(anyhow::anyhow!("CSV headers are empty."));
    }

    let clean_table = table.trim().trim_matches('`');
    let cols_escaped = headers
        .iter()
        .map(|h| format!("`{}`", h.trim().replace('`', "``")))
        .collect::<Vec<_>>()
        .join(", ");

    let mut value_rows = Vec::new();
    for result in reader.records() {
        let record = result?;
        let mut row_vals = Vec::new();
        for field in record.iter() {
            let field_trimmed = field.trim();
            if field_trimmed.is_empty() || field_trimmed.eq_ignore_ascii_case("null") {
                row_vals.push("NULL".to_string());
            } else if field_trimmed.parse::<i64>().is_ok() {
                row_vals.push(field_trimmed.to_string());
            } else if field_trimmed.parse::<f64>().is_ok() && !field_trimmed.starts_with('0') || field_trimmed == "0" || field_trimmed.starts_with("0.") {
                row_vals.push(field_trimmed.to_string());
            } else if field_trimmed.eq_ignore_ascii_case("true") {
                row_vals.push("1".to_string());
            } else if field_trimmed.eq_ignore_ascii_case("false") {
                row_vals.push("0".to_string());
            } else {
                row_vals.push(format!("'{}'", field_trimmed.replace('\'', "''")));
            }
        }
        value_rows.push(format!("  ({})", row_vals.join(", ")));
    }

    if value_rows.is_empty() {
        return Err(anyhow::anyhow!("CSV contains no data rows."));
    }

    Ok(format!(
        "INSERT INTO `{clean_table}` ({cols_escaped}) VALUES\n{};",
        value_rows.join(",\n")
    ))
}

