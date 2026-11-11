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
