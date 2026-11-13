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
            } else if field_trimmed.parse::<f64>().is_ok() && !field_trimmed.starts_with('0')
                || field_trimmed == "0"
                || field_trimmed.starts_with("0.")
            {
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

/// Inspects a CSV content string and returns its headers, first N sample rows, and total row count.
pub fn inspect_csv_content(
    csv_content: &str,
    max_samples: usize,
) -> Result<(Vec<String>, Vec<Vec<String>>, usize)> {
    let trimmed = csv_content.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("CSV file is empty."));
    }
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(trimmed.as_bytes());

    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(|h| h.trim().to_string())
        .collect();
    let mut sample_rows = Vec::new();
    let mut total_count = 0;

    for result in reader.records() {
        let rec = result?;
        if sample_rows.len() < max_samples {
            sample_rows.push(rec.iter().map(|s| s.to_string()).collect());
        }
        total_count += 1;
    }

    Ok((headers, sample_rows, total_count))
}

/// Generates executable INSERT statements mapping CSV columns to target DB table columns.
pub fn generate_csv_import_sql(
    target_table: &str,
    headers: &[String],
    mappings: &[(String, String)], // (csv_col, db_col)
    csv_content: &str,
    create_table: bool,
) -> Result<String> {
    let mut stmts = Vec::new();
    let clean_table = target_table.trim().trim_matches('`');

    let active_mappings: Vec<(usize, &String)> = mappings
        .iter()
        .filter(|(_, db)| !db.trim().is_empty())
        .filter_map(|(csv_col, db_col)| {
            headers
                .iter()
                .position(|h| h == csv_col)
                .map(|idx| (idx, db_col))
        })
        .collect();

    if active_mappings.is_empty() {
        return Err(anyhow::anyhow!("No active column mappings configured."));
    }

    if create_table {
        let mut col_defs = Vec::new();
        col_defs.push("  `id` INT AUTO_INCREMENT PRIMARY KEY".to_string());
        for (_, db_col) in &active_mappings {
            col_defs.push(format!("  `{}` TEXT NULL", db_col.trim_matches('`')));
        }
        stmts.push(format!(
            "CREATE TABLE IF NOT EXISTS `{clean_table}` (\n{}\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;",
            col_defs.join(",\n")
        ));
    }

    let col_names = active_mappings
        .iter()
        .map(|(_, db_col)| format!("`{}`", db_col.trim_matches('`')))
        .collect::<Vec<_>>()
        .join(", ");

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(csv_content.trim().as_bytes());

    let mut value_rows = Vec::new();
    for result in reader.records() {
        let rec = result?;
        let mut row_vals = Vec::new();
        for &(idx, _) in &active_mappings {
            let field_trimmed = rec.get(idx).map(|s| s.trim()).unwrap_or("");
            if field_trimmed.is_empty() || field_trimmed.eq_ignore_ascii_case("null") {
                row_vals.push("NULL".to_string());
            } else if field_trimmed.parse::<i64>().is_ok() {
                row_vals.push(field_trimmed.to_string());
            } else if (field_trimmed.parse::<f64>().is_ok() && !field_trimmed.starts_with('0'))
                || field_trimmed == "0"
                || field_trimmed.starts_with("0.")
            {
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
        return Err(anyhow::anyhow!("No rows found to import in CSV."));
    }

    // Chunk into batches of 200 rows per INSERT statement for safe execution
    for chunk in value_rows.chunks(200) {
        stmts.push(format!(
            "INSERT INTO `{clean_table}` ({col_names}) VALUES\n{};",
            chunk.join(",\n")
        ));
    }

    Ok(stmts.join("\n\n"))
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
    out.push_str(
        &result
            .columns
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | "),
    );
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
        "Smith",
        "Garcia",
        "Johnson",
        "Rodriguez",
        "Brown",
        "Martinez",
        "Davis",
        "Lopez",
        "Wilson",
        "Gonzalez",
    ];
    let statuses = ["active", "pending", "completed", "cancelled", "processing"];
    let domains = ["example.com", "testmail.org", "company.io", "sample.net"];
    let cities = [
        "New York",
        "Madrid",
        "London",
        "Buenos Aires",
        "Tokyo",
        "Berlin",
        "Paris",
        "Toronto",
    ];
    let countries = ["USA", "Spain", "Germany", "Japan", "Argentina", "UK"];

    let mut out = format!("-- Mock Seed Data for `{clean_table}` ({count} rows)\n");
    out.push_str(&format!(
        "INSERT INTO `{clean_table}` ({col_names}) VALUES\n"
    ));

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

/// Formats all unique non-null values from a specific column as a SQL `IN (...)` clause.
pub fn format_column_as_sql_in(
    result: &QueryResult,
    col_idx: usize,
    row_indices: &[usize],
) -> String {
    let mut vals = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &r_idx in row_indices {
        if let Some(cell) = result.rows.get(r_idx).and_then(|r| r.get(col_idx)) {
            if cell.is_null() {
                continue;
            }
            let text = cell.display_text();
            if !seen.insert(text.clone()) {
                continue;
            }
            if text.parse::<i64>().is_ok() || text.parse::<f64>().is_ok() {
                vals.push(text);
            } else {
                vals.push(format!("'{}'", text.replace('\'', "''")));
            }
        }
    }
    format!("IN ({})", vals.join(", "))
}

/// Formats all values from a specific column as a JavaScript/TypeScript array.
pub fn format_column_as_js_array(
    result: &QueryResult,
    col_idx: usize,
    row_indices: &[usize],
) -> String {
    let mut vals = Vec::new();
    for &r_idx in row_indices {
        if let Some(cell) = result.rows.get(r_idx).and_then(|r| r.get(col_idx)) {
            if cell.is_null() {
                vals.push("null".to_string());
                continue;
            }
            let text = cell.display_text();
            if text.parse::<i64>().is_ok() || text.parse::<f64>().is_ok() {
                vals.push(text);
            } else {
                vals.push(format!(
                    "\"{}\"",
                    text.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
        }
    }
    format!("[{}]", vals.join(", "))
}

fn format_sql_cell_literal(cell: &CellValue) -> String {
    if cell.is_null() {
        return "NULL".to_string();
    }
    match cell {
        CellValue::Integer(n) => n.to_string(),
        CellValue::Float(f) => f.to_string(),
        _ => {
            let text = cell.display_text();
            if text.parse::<i64>().is_ok() || text.parse::<f64>().is_ok() {
                text
            } else {
                format!("'{}'", text.replace('\'', "''"))
            }
        }
    }
}

/// Formats selected rows as a pretty-printed JSON array of objects (or single object if 1 row).
pub fn format_rows_as_json(result: &QueryResult, row_indices: &[usize]) -> String {
    let mut items = Vec::new();
    for &r_idx in row_indices {
        let Some(row) = result.rows.get(r_idx) else {
            continue;
        };
        let mut map = Map::new();
        for (c_idx, col) in result.columns.iter().enumerate() {
            let val = match row.get(c_idx) {
                None | Some(CellValue::Null) => Value::Null,
                Some(CellValue::Integer(n)) => Value::Number((*n).into()),
                Some(cell) => Value::String(cell.display_text()),
            };
            map.insert(col.clone(), val);
        }
        items.push(Value::Object(map));
    }
    if items.len() == 1 {
        serde_json::to_string_pretty(&items[0]).unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
    }
}

/// Formats selected rows as SQL INSERT statements.
pub fn format_rows_as_sql_insert(
    table: &str,
    result: &QueryResult,
    row_indices: &[usize],
) -> String {
    if result.columns.is_empty() || row_indices.is_empty() {
        return String::new();
    }
    let cols_sql = result
        .columns
        .iter()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut value_tuples = Vec::new();
    for &r_idx in row_indices {
        let Some(row) = result.rows.get(r_idx) else {
            continue;
        };
        let vals = (0..result.columns.len())
            .map(|c_idx| {
                row.get(c_idx)
                    .map(format_sql_cell_literal)
                    .unwrap_or_else(|| "NULL".to_string())
            })
            .collect::<Vec<_>>()
            .join(", ");
        value_tuples.push(format!("({vals})"));
    }
    if value_tuples.is_empty() {
        return String::new();
    }
    format!(
        "INSERT INTO `{table}` ({cols_sql}) VALUES\n  {};",
        value_tuples.join(",\n  ")
    )
}

/// Formats selected rows as SQL UPDATE statements keyed by `pk_col`.
pub fn format_rows_as_sql_update(
    table: &str,
    pk_col: &str,
    result: &QueryResult,
    row_indices: &[usize],
) -> String {
    let Some(pk_idx) = result.columns.iter().position(|c| c == pk_col) else {
        return String::new();
    };
    let mut stmts = Vec::new();
    for &r_idx in row_indices {
        let Some(row) = result.rows.get(r_idx) else {
            continue;
        };
        let pk_lit = row
            .get(pk_idx)
            .map(format_sql_cell_literal)
            .unwrap_or_else(|| "NULL".to_string());
        let sets = result
            .columns
            .iter()
            .enumerate()
            .filter(|(c_idx, _)| *c_idx != pk_idx)
            .map(|(c_idx, col)| {
                let lit = row
                    .get(c_idx)
                    .map(format_sql_cell_literal)
                    .unwrap_or_else(|| "NULL".to_string());
                format!("`{col}` = {lit}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        if !sets.is_empty() {
            stmts.push(format!(
                "UPDATE `{table}` SET {sets} WHERE `{pk_col}` = {pk_lit};"
            ));
        }
    }
    stmts.join("\n")
}

/// Generates a CREATE TABLE DDL string from `TableSchema`.
pub fn format_table_create_ddl(schema: &TableSchema) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut pks: Vec<String> = Vec::new();
    for col in &schema.columns {
        let null_str = if col.is_nullable { "NULL" } else { "NOT NULL" };
        let def_str = col
            .default_value
            .as_ref()
            .map(|d| format!(" DEFAULT {d}"))
            .unwrap_or_default();
        let extra = if col.extra.is_empty() {
            String::new()
        } else {
            format!(" {}", col.extra)
        };
        lines.push(format!(
            "  `{}` {} {}{}{}",
            col.name, col.data_type, null_str, def_str, extra
        ));
        if col.is_primary_key {
            pks.push(format!("`{}`", col.name));
        }
    }
    if !pks.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", pks.join(", ")));
    }
    for fk in &schema.foreign_keys {
        lines.push(format!(
            "  CONSTRAINT `{}` FOREIGN KEY (`{}`) REFERENCES `{}` (`{}`)",
            fk.constraint_name, fk.column_name, fk.referenced_table, fk.referenced_column
        ));
    }
    let engine = if schema.engine.is_empty() {
        "InnoDB"
    } else {
        schema.engine.as_str()
    };
    format!(
        "CREATE TABLE `{}` (\n{}\n) ENGINE={};",
        schema.name,
        lines.join(",\n"),
        engine
    )
}

/// Generates a full SQL dump (optional DROP + CREATE TABLE + batched INSERTs) for a table.
pub fn generate_table_dump_sql(
    schema: &TableSchema,
    result: Option<&QueryResult>,
    include_drop: bool,
    include_data: bool,
    batch_size: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("-- Dump for table `{}`\n", schema.name));
    if include_drop {
        out.push_str(&format!("DROP TABLE IF EXISTS `{}`;\n", schema.name));
    }
    out.push_str(&format_table_create_ddl(schema));
    out.push('\n');

    if include_data {
        if let Some(res) = result {
            if !res.rows.is_empty() && !res.columns.is_empty() {
                let step = batch_size.max(1);
                let indices: Vec<usize> = (0..res.rows.len()).collect();
                for chunk in indices.chunks(step) {
                    out.push('\n');
                    out.push_str(&format_rows_as_sql_insert(&schema.name, res, chunk));
                    out.push('\n');
                }
            }
        }
    }
    out
}
