use crate::db::{CellValue, QueryResult, TableSchema};
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

/// Formats a `QueryResult` into a GitHub-flavored Markdown table.
pub fn to_markdown_table(result: &QueryResult) -> String {
    if result.columns.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("| ");
    out.push_str(&result.columns.join(" | "));
    out.push_str(" |\n| ");
    out.push_str(&result.columns.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    out.push_str(" |\n");
    for row in &result.rows {
        out.push_str("| ");
        let vals: Vec<String> = row
            .iter()
            .map(|c| match c {
                CellValue::Null => "NULL".to_string(),
                other => other.display_text().replace('|', "\\|").replace('\n', " "),
            })
            .collect();
        out.push_str(&vals.join(" | "));
        out.push_str(" |\n");
    }
    out
}

/// Intelligently generates realistic random seed data (multi-row INSERT statements)
/// based on column names and SQL types.
pub fn generate_mock_data_sql(table: &TableSchema, count: usize) -> String {
    let clean_table = table.name.trim().trim_matches('`');
    let insert_cols: Vec<_> = table
        .columns
        .iter()
        .filter(|c| !c.extra.to_lowercase().contains("auto_increment"))
        .collect();

    if insert_cols.is_empty() {
        return format!("-- Table `{clean_table}` has no writable columns.\n");
    }

    let col_names = insert_cols
        .iter()
        .map(|c| format!("`{}`", c.name.replace('`', "``")))
        .collect::<Vec<_>>()
        .join(", ");

    let first_names = [
        "Alex", "Maria", "Carlos", "Emma", "David", "Sofia", "Liam", "Lucia", "Noah", "Elena",
    ];
    let last_names = [
        "Smith", "Garcia", "Johnson", "Rodriguez", "Brown", "Martinez", "Davis", "Lopez", "Wilson",
        "Gonzalez",
    ];
    let statuses = ["active", "pending", "completed", "cancelled", "processing"];
    let domains = ["example.com", "testmail.org", "company.io", "sample.net"];
    let cities = [
        "New York", "Madrid", "London", "Buenos Aires", "Tokyo", "Berlin", "Paris", "Toronto",
    ];
    let countries = ["USA", "Spain", "Germany", "Japan", "Argentina", "UK"];

    let mut out = format!("-- Mock Seed Data for `{clean_table}` ({count} rows)\n");
    out.push_str(&format!("INSERT INTO `{clean_table}` ({col_names}) VALUES\n"));

    let mut row_strings = Vec::with_capacity(count);
    for i in 1..=count {
        let mut vals = Vec::with_capacity(insert_cols.len());
        for col in &insert_cols {
            let name_lower = col.name.to_lowercase();
            let type_upper = col.data_type.to_uppercase();

            let val_str = if name_lower.contains("email") {
                let fname = first_names[(i - 1) % first_names.len()].to_lowercase();
                let domain = domains[(i - 1) % domains.len()];
                format!("'{fname}_{i}@{domain}'")
            } else if name_lower.contains("full_name") || name_lower == "name" {
                let fname = first_names[(i - 1) % first_names.len()];
                let lname = last_names[(i * 3) % last_names.len()];
                format!("'{fname} {lname}'")
            } else if name_lower.contains("phone") {
                format!("'+1-555-{:04}'", (i * 137 + 1000) % 10000)
            } else if name_lower.contains("status") {
                let s = statuses[(i - 1) % statuses.len()];
                format!("'{s}'")
            } else if name_lower.contains("city") {
                let c = cities[(i - 1) % cities.len()];
                format!("'{c}'")
            } else if name_lower.contains("country") {
                let c = countries[(i - 1) % countries.len()];
                format!("'{c}'")
            } else if name_lower.contains("price")
                || name_lower.contains("amount")
                || name_lower.contains("total")
                || name_lower.contains("balance")
            {
                let price = ((i * 37 + 19) % 900) as f64 + 0.99;
                format!("{price:.2}")
            } else if name_lower.contains("created")
                || name_lower.contains("updated")
                || type_upper.contains("DATETIME")
                || type_upper.contains("TIMESTAMP")
            {
                let day = (i % 27) + 1;
                let hour = (i * 3) % 24;
                let min = (i * 7) % 60;
                format!("'2026-03-{day:02} {hour:02}:{min:02}:00'")
            } else if type_upper.contains("DATE") {
                let day = (i % 27) + 1;
                let month = (i % 12) + 1;
                format!("'2026-{month:02}-{day:02}'")
            } else if type_upper.contains("BOOL") || type_upper == "TINYINT(1)" {
                (i % 2).to_string()
            } else if type_upper.contains("INT") {
                ((i * 10) + 5).to_string()
            } else if type_upper.contains("DECIMAL")
                || type_upper.contains("FLOAT")
                || type_upper.contains("DOUBLE")
            {
                format!("{:.2}", (i as f64 * 12.5) % 1000.0)
            } else if type_upper.contains("JSON") {
                format!("'{{\"item_index\": {i}, \"tag\": \"mock_data\"}}'")
            } else {
                format!("'{}_{i}'", col.name)
            };
            vals.push(val_str);
        }
        row_strings.push(format!("  ({})", vals.join(", ")));
    }
    out.push_str(&row_strings.join(",\n"));
    out.push_str(";\n");
    out
}


