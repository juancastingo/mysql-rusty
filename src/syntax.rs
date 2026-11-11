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
