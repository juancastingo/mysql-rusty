use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId};

const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "ON", "AS",
    "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE", "BETWEEN", "EXISTS", "GROUP", "BY", "ORDER",
    "ASC", "DESC", "HAVING", "LIMIT", "OFFSET", "UNION", "ALL", "DISTINCT", "INSERT", "INTO",
    "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "ALTER", "DROP", "TRUNCATE", "TABLE", "VIEW",
    "INDEX", "DATABASE", "SCHEMA", "IF", "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "DEFAULT",
    "AUTO_INCREMENT", "UNIQUE", "CONSTRAINT", "CASE", "WHEN", "THEN", "ELSE", "END", "EXPLAIN",
    "SHOW", "DESCRIBE", "DESC", "USE", "BEGIN", "COMMIT", "ROLLBACK", "TRANSACTION", "WITH",
];

const SQL_FUNCTIONS: &[&str] = &[
    "COUNT", "SUM", "AVG", "MIN", "MAX", "ROUND", "COALESCE", "IFNULL", "CONCAT", "SUBSTRING",
    "LOWER", "UPPER", "TRIM", "LENGTH", "NOW", "CURDATE", "DATE_FORMAT", "DATEDIFF", "YEAR",
    "MONTH", "DAY", "CAST", "CONVERT", "JSON_EXTRACT", "ROW_NUMBER", "RANK", "DENSE_RANK",
    "DATABASE", "VERSION",
];

const SQL_TYPES: &[&str] = &[
    "INT", "INTEGER", "BIGINT", "SMALLINT", "TINYINT", "UNSIGNED", "VARCHAR", "CHAR", "TEXT",
    "LONGTEXT", "MEDIUMTEXT", "DECIMAL", "NUMERIC", "FLOAT", "DOUBLE", "BOOLEAN", "BOOL", "DATE",
    "DATETIME", "TIMESTAMP", "TIME", "JSON", "ENUM", "BLOB",
];

/// Builds a syntax-highlighted `LayoutJob` for SQL code, optionally appending an inline
/// Copilot-style ghost completion right after the text.
pub fn highlight_sql(text: &str, ghost_suffix: Option<&str>, font_size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mono = FontId::new(font_size, FontFamily::Monospace);

    let kw_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(56, 189, 248), // Sky blue
        ..Default::default()
    };
    let fn_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(192, 132, 252), // Purple
        ..Default::default()
    };
    let type_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(45, 212, 191), // Teal
        ..Default::default()
    };
    let str_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(163, 230, 53), // Lime green
        ..Default::default()
    };
    let num_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(251, 146, 60), // Orange
        ..Default::default()
    };
    let comment_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(100, 116, 139), // Slate gray
        italics: true,
        ..Default::default()
    };
    let ident_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(226, 232, 240), // Light slate
        ..Default::default()
    };
    let punct_fmt = TextFormat {
        font_id: mono.clone(),
        color: Color32::from_rgb(148, 163, 184),
        ..Default::default()
    };

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Line comment `-- ...`
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            job.append(&text[start..i], 0.0, comment_fmt.clone());
            continue;
        }

        // Block comment `/* ... */`
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            } else {
                i = len;
            }
            job.append(&text[start..i], 0.0, comment_fmt.clone());
            continue;
        }

        // Single-quoted or double-quoted string
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            job.append(&text[start..i], 0.0, str_fmt.clone());
            continue;
        }

        // Backtick identifier
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            job.append(&text[start..i], 0.0, ident_fmt.clone());
            continue;
        }

        // Number literal
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            job.append(&text[start..i], 0.0, num_fmt.clone());
            continue;
        }

        // Word / Identifier / Keyword
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let upper = word.to_ascii_uppercase();
            let fmt = if SQL_KEYWORDS.contains(&upper.as_str()) {
                &kw_fmt
            } else if SQL_FUNCTIONS.contains(&upper.as_str()) {
                &fn_fmt
            } else if SQL_TYPES.contains(&upper.as_str()) {
                &type_fmt
            } else {
                &ident_fmt
            };
            job.append(word, 0.0, fmt.clone());
            continue;
        }

        // Any other UTF-8 character or punctuation
        let ch = text[i..].chars().next().unwrap();
        let next_i = i + ch.len_utf8();
        job.append(&text[i..next_i], 0.0, punct_fmt.clone());
        i = next_i;
    }

    if let Some(ghost) = ghost_suffix {
        if !ghost.is_empty() {
            let ghost_fmt = TextFormat {
                font_id: mono,
                color: Color32::from_rgba_unmultiplied(125, 211, 252, 130),
                italics: true,
                ..Default::default()
            };
            job.append(ghost, 0.0, ghost_fmt);
        }
    }

    job
}

/// Formats a SQL query by upper-casing major clauses and placing them on clean lines.
pub fn format_sql_pretty(sql: &str) -> String {
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let major_clauses = [
        "SELECT ",
        "FROM ",
        "LEFT JOIN ",
        "RIGHT JOIN ",
        "INNER JOIN ",
        "JOIN ",
        "WHERE ",
        "GROUP BY ",
        "HAVING ",
        "ORDER BY ",
        "LIMIT ",
    ];

    let mut out = collapsed;
    for clause in major_clauses {
        let lower_clause = clause.to_lowercase();
        if let Some(pos) = out.to_lowercase().find(&lower_clause) {
            if pos > 0 {
                let prefix = out[..pos].trim_end();
                let suffix = &out[pos + clause.len()..];
                out = format!("{prefix}\n{clause}{suffix}");
            } else {
                let suffix = &out[clause.len()..];
                out = format!("{clause}{suffix}");
            }
        }
    }
    out
}

/// Extracts the SQL statement located at the specified character cursor index.
/// Statements are delimited by `;` outside of quotes, backticks, and comments.
/// Returns the statement text (trimmed) and its `(start_char_idx, end_char_idx)` range in `sql`.
pub fn extract_statement_at_cursor(sql: &str, cursor_char_idx: usize) -> (String, (usize, usize)) {
    if sql.trim().is_empty() {
        return (String::new(), (0, 0));
    }

    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();

    let mut raw_ranges = Vec::new();
    let mut current_start = 0;
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < n {
        let ch = chars[i];
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
        } else if in_block_comment {
            if ch == '*' && i + 1 < n && chars[i + 1] == '/' {
                in_block_comment = false;
                i += 1;
            }
        } else if in_single_quote {
            if ch == '\\' && i + 1 < n {
                i += 1;
            } else if ch == '\'' {
                if i + 1 < n && chars[i + 1] == '\'' {
                    i += 1;
                } else {
                    in_single_quote = false;
                }
            }
        } else if in_double_quote {
            if ch == '\\' && i + 1 < n {
                i += 1;
            } else if ch == '"' {
                if i + 1 < n && chars[i + 1] == '"' {
                    i += 1;
                } else {
                    in_double_quote = false;
                }
            }
        } else if in_backtick {
            if ch == '`' {
                if i + 1 < n && chars[i + 1] == '`' {
                    i += 1;
                } else {
                    in_backtick = false;
                }
            }
        } else {
            if ch == '-' && i + 1 < n && chars[i + 1] == '-' {
                in_line_comment = true;
                i += 1;
            } else if ch == '#' {
                in_line_comment = true;
            } else if ch == '/' && i + 1 < n && chars[i + 1] == '*' {
                in_block_comment = true;
                i += 1;
            } else if ch == '\'' {
                in_single_quote = true;
            } else if ch == '"' {
                in_double_quote = true;
            } else if ch == '`' {
                in_backtick = true;
            } else if ch == ';' {
                raw_ranges.push((current_start, i + 1));
                current_start = i + 1;
            }
        }
        i += 1;
    }

    if current_start < n {
        raw_ranges.push((current_start, n));
    }

    struct StmtInfo {
        raw_start: usize,
        raw_end: usize,
        trimmed_start: usize,
        trimmed_end: usize,
        text: String,
    }

    let mut stmts = Vec::new();
    for (r_start, r_end) in raw_ranges {
        let slice = &chars[r_start..r_end];
        let mut lead_ws = 0;
        while lead_ws < slice.len() && slice[lead_ws].is_whitespace() {
            lead_ws += 1;
        }
        let mut trail_ws = 0;
        while trail_ws < slice.len().saturating_sub(lead_ws)
            && slice[slice.len() - 1 - trail_ws].is_whitespace()
        {
            trail_ws += 1;
        }
        if lead_ws + trail_ws < slice.len() {
            let t_start = r_start + lead_ws;
            let t_end = r_end - trail_ws;
            let text: String = chars[t_start..t_end].iter().collect();
            stmts.push(StmtInfo {
                raw_start: r_start,
                raw_end: r_end,
                trimmed_start: t_start,
                trimmed_end: t_end,
                text,
            });
        }
    }

    if stmts.is_empty() {
        return (String::new(), (0, 0));
    }

    let clamped_cursor = cursor_char_idx.min(n);

    for stmt in &stmts {
        if clamped_cursor >= stmt.raw_start && clamped_cursor <= stmt.raw_end {
            return (stmt.text.clone(), (stmt.trimmed_start, stmt.trimmed_end));
        }
    }

    if let Some(last) = stmts.last() {
        if clamped_cursor >= last.raw_end {
            return (last.text.clone(), (last.trimmed_start, last.trimmed_end));
        }
    }

    let first = &stmts[0];
    (first.text.clone(), (first.trimmed_start, first.trimmed_end))
}

/// Checks if an SQL statement is potentially destructive (DROP, TRUNCATE, or UPDATE/DELETE without a WHERE clause).
pub fn is_potentially_destructive(sql: &str) -> Option<&'static str> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return None;
    }

    let upper = trimmed.to_uppercase();

    if upper.starts_with("DROP TABLE")
        || upper.starts_with("DROP DATABASE")
        || upper.starts_with("DROP SCHEMA")
    {
        return Some("DROP statement permanently removes schemas or tables.");
    }

    if upper.starts_with("TRUNCATE") {
        return Some("TRUNCATE statement empties the entire table immediately.");
    }

    if upper.starts_with("ALTER TABLE") && upper.contains(" DROP ") {
        return Some("ALTER TABLE with DROP column/constraint deletes schema structures permanently.");
    }

    if upper.starts_with("UPDATE ") || upper.starts_with("UPDATE\n") || upper.starts_with("UPDATE\t") {
        let has_where = upper.split_whitespace().any(|word| word == "WHERE" || word.starts_with("WHERE;"));
        if !has_where {
            return Some("UPDATE without a WHERE clause will overwrite EVERY single row in the table!");
        }
    }

    if upper.starts_with("DELETE ") || upper.starts_with("DELETE\n") || upper.starts_with("DELETE\t") {
        let has_where = upper.split_whitespace().any(|word| word == "WHERE" || word.starts_with("WHERE;"));
        if !has_where {
            return Some("DELETE without a WHERE clause will delete EVERY single row in the table!");
        }
    }

    None
}

/// Checks if an SQL statement is a write/mutation query.
pub fn is_write_query(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    upper.starts_with("INSERT ")
        || upper.starts_with("UPDATE ")
        || upper.starts_with("DELETE ")
        || upper.starts_with("DROP ")
        || upper.starts_with("ALTER ")
        || upper.starts_with("TRUNCATE ")
        || upper.starts_with("CREATE ")
        || upper.starts_with("REPLACE ")
}


