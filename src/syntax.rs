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

/// Converts a character index into a valid UTF-8 byte offset in `text`.
pub fn char_idx_to_byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(text.len())
}

/// Splits `sql` at `cursor_char_idx` into `(prefix, suffix)` slices.
pub fn split_sql_at_cursor(sql: &str, cursor_char_idx: usize) -> (&str, &str) {
    let byte_idx = char_idx_to_byte_idx(sql, cursor_char_idx);
    sql.split_at(byte_idx)
}

/// Replaces `replace_prefix_bytes` immediately before `cursor_char_idx` with `insertion`
/// and returns the updated cursor character index.
pub fn replace_at_cursor(
    sql: &mut String,
    cursor_char_idx: usize,
    replace_prefix_bytes: usize,
    insertion: &str,
) -> usize {
    let cursor_byte = char_idx_to_byte_idx(sql, cursor_char_idx);
    let start_byte = cursor_byte
        .saturating_sub(replace_prefix_bytes)
        .min(cursor_byte);
    // Ensure `start_byte` lies on a UTF-8 char boundary
    let mut safe_start = start_byte;
    while safe_start > 0 && !sql.is_char_boundary(safe_start) {
        safe_start -= 1;
    }
    sql.replace_range(safe_start..cursor_byte, insertion);
    sql[..safe_start].chars().count() + insertion.chars().count()
}

/// Builds a syntax-highlighted `LayoutJob` for SQL code, optionally appending an inline
/// Copilot-style ghost completion when the cursor is at the end of the buffer.
pub fn highlight_sql(
    text: &str,
    ghost_suffix: Option<&str>,
    cursor_char_idx: Option<usize>,
    font_size: f32,
) -> LayoutJob {
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

    let cursor_at_end = cursor_char_idx
        .map(|idx| idx >= text.chars().count())
        .unwrap_or(true);
    if cursor_at_end {
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
    }

    job
}

/// Formats a SQL query with proper indentation, uppercase keywords, and clause line breaks.
pub fn format_sql_pretty(sql: &str) -> String {
    let mut options = sqlformat::FormatOptions::default();
    options.indent = sqlformat::Indent::Spaces(2);
    options.uppercase = Some(true);
    options.lines_between_queries = 2;
    let formatted = sqlformat::format(sql, &sqlformat::QueryParams::None, &options);
    if formatted.trim().is_empty() {
        sql.to_string()
    } else {
        formatted
    }
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

struct TerminalLimitInfo {
    limit_keyword_start: usize,
    clause_end: usize,
    current_limit: Option<usize>,
    offset_prefix: Option<String>,
    offset_suffix: Option<String>,
}

fn find_terminal_limit(sql: &str) -> Option<TerminalLimitInfo> {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    let mut last_limit_pos: Option<usize> = None;

    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if ch == '*' && i + 1 < n && chars[i + 1] == '/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if in_single_quote {
            if ch == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if ch == '\'' {
                if i + 1 < n && chars[i + 1] == '\'' {
                    i += 2;
                    continue;
                }
                in_single_quote = false;
            }
            i += 1;
            continue;
        }
        if in_double_quote {
            if ch == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if ch == '"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if ch == '`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }

        if ch == '-' && i + 1 < n && chars[i + 1] == '-' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if ch == '#' {
            in_line_comment = true;
            i += 1;
            continue;
        }
        if ch == '/' && i + 1 < n && chars[i + 1] == '*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if ch == '\'' {
            in_single_quote = true;
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double_quote = true;
            i += 1;
            continue;
        }
        if ch == '`' {
            in_backtick = true;
            i += 1;
            continue;
        }

        let prev_ok = i == 0 || (!chars[i - 1].is_alphanumeric() && chars[i - 1] != '_');
        if prev_ok && i + 5 <= n {
            let slice: String = chars[i..i + 5].iter().collect();
            if slice.eq_ignore_ascii_case("LIMIT") {
                let next_ok = i + 5 == n || (!chars[i + 5].is_alphanumeric() && chars[i + 5] != '_');
                if next_ok {
                    last_limit_pos = Some(i);
                    i += 5;
                    continue;
                }
            }
        }
        i += 1;
    }

    let limit_char_idx = last_limit_pos?;
    let after_limit_chars = &chars[limit_char_idx + 5..];
    let after_limit_str: String = after_limit_chars.iter().collect();
    let trimmed_after = after_limit_str.trim_end();
    let trimmed_without_semi = trimmed_after.trim_end_matches(';').trim_end();

    let upper_after = trimmed_without_semi.to_uppercase();
    if upper_after.contains(')') {
        return None;
    }
    for kw in &["WHERE", "ORDER BY", "GROUP BY", "HAVING", "JOIN", "UNION", "SELECT", "FROM"] {
        if upper_after.split_whitespace().any(|w| w == *kw) {
            return None;
        }
    }

    let args_str = trimmed_without_semi.trim();
    if args_str.is_empty() {
        return None;
    }

    let byte_limit_start = sql
        .char_indices()
        .nth(limit_char_idx)
        .map(|(b, _)| b)?;

    let last_char_idx = limit_char_idx + 5 + after_limit_str.trim_end().trim_end_matches(';').chars().count();
    let byte_clause_end = sql
        .char_indices()
        .nth(last_char_idx)
        .map(|(b, _)| b)
        .unwrap_or(sql.len());

    if args_str.contains(',') {
        let parts: Vec<&str> = args_str.splitn(2, ',').collect();
        let offset_part = parts[0].trim();
        let count_part = parts[1].trim();
        let count = count_part.parse::<usize>().ok()?;
        return Some(TerminalLimitInfo {
            limit_keyword_start: byte_limit_start,
            clause_end: byte_clause_end,
            current_limit: Some(count),
            offset_prefix: Some(format!("{offset_part}, ")),
            offset_suffix: None,
        });
    }

    if upper_after.contains("OFFSET") {
        let parts: Vec<&str> = upper_after.split("OFFSET").collect();
        let count_part = parts[0].trim();
        let offset_part = parts.get(1).map(|s| s.trim()).unwrap_or("");
        let count = count_part.parse::<usize>().ok()?;
        return Some(TerminalLimitInfo {
            limit_keyword_start: byte_limit_start,
            clause_end: byte_clause_end,
            current_limit: Some(count),
            offset_prefix: None,
            offset_suffix: Some(format!(" OFFSET {offset_part}")),
        });
    }

    if let Ok(count) = args_str.parse::<usize>() {
        return Some(TerminalLimitInfo {
            limit_keyword_start: byte_limit_start,
            clause_end: byte_clause_end,
            current_limit: Some(count),
            offset_prefix: None,
            offset_suffix: None,
        });
    }

    None
}

/// Detects the currently applied terminal LIMIT count from an SQL query, if one exists.
pub fn detect_current_limit(sql: &str) -> Option<usize> {
    find_terminal_limit(sql).and_then(|info| info.current_limit)
}

/// Intelligently updates, appends, or removes a terminal LIMIT clause from an SQL statement.
/// If `new_limit` is `Some(n)`, it sets or replaces the LIMIT with `n`.
/// If `new_limit` is `None`, it cleanly strips the terminal LIMIT clause.
pub fn apply_limit_to_sql(sql: &str, new_limit: Option<usize>) -> String {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return match new_limit {
            Some(n) => format!("LIMIT {n};"),
            None => String::new(),
        };
    }

    let had_trailing_semicolon = trimmed.ends_with(';');

    if let Some(info) = find_terminal_limit(sql) {
        let before_limit = &sql[..info.limit_keyword_start];
        let after_clause = &sql[info.clause_end..];
        let has_semi_after = after_clause.contains(';');

        match new_limit {
            Some(n) => {
                let limit_body = if let Some(prefix) = &info.offset_prefix {
                    format!("LIMIT {prefix}{n}")
                } else if let Some(suffix) = &info.offset_suffix {
                    format!("LIMIT {n}{suffix}")
                } else {
                    format!("LIMIT {n}")
                };
                let before_trimmed = before_limit.trim_end();
                format!("{before_trimmed} {limit_body};")
            }
            None => {
                let before_trimmed = before_limit.trim_end();
                if has_semi_after || had_trailing_semicolon {
                    format!("{before_trimmed};")
                } else {
                    before_trimmed.to_string()
                }
            }
        }
    } else {
        match new_limit {
            Some(n) => {
                let base = trimmed.trim_end_matches(';').trim_end();
                format!("{base} LIMIT {n};")
            }
            None => sql.to_string(),
        }
    }
}

/// Splits a multi-statement SQL script into individual trimmed non-empty statements,
/// respecting string literals, backticks, and comments.
pub fn split_sql_statements(sql: &str) -> Vec<String> {
    if sql.trim().is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut stmts = Vec::new();
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
                let s: String = chars[current_start..=i].iter().collect();
                let trimmed = s.trim();
                if !trimmed.is_empty() && trimmed != ";" {
                    stmts.push(trimmed.to_string());
                }
                current_start = i + 1;
            }
        }
        i += 1;
    }

    if current_start < n {
        let s: String = chars[current_start..n].iter().collect();
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            stmts.push(trimmed.to_string());
        }
    }

    stmts
}

/// Extracts unique `:param_name` template parameters from a SQL string,
/// ignoring tokens inside string literals, backticks, or comments.
pub fn extract_sql_parameters(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut params = Vec::new();

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
                in_single_quote = false;
            }
        } else if in_double_quote {
            if ch == '\\' && i + 1 < n {
                i += 1;
            } else if ch == '"' {
                in_double_quote = false;
            }
        } else if in_backtick {
            if ch == '`' {
                in_backtick = false;
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
            } else if ch == ':' && (i == 0 || !chars[i - 1].is_ascii_alphanumeric() && chars[i - 1] != ':') {
                if i + 1 < n && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') {
                    let mut j = i + 1;
                    while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let name: String = chars[i + 1..j].iter().collect();
                    if !params.contains(&name) {
                        params.push(name);
                    }
                    i = j - 1;
                }
            }
        }
        i += 1;
    }

    params
}

/// Substitutes `:param_name` template parameters with safely formatted SQL literals.
pub fn substitute_sql_parameters(
    sql: &str,
    params: &std::collections::HashMap<String, String>,
) -> String {
    if params.is_empty() {
        return sql.to_string();
    }

    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(sql.len() + 32);

    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < n {
        let ch = chars[i];
        if in_line_comment {
            out.push(ch);
            if ch == '\n' {
                in_line_comment = false;
            }
        } else if in_block_comment {
            out.push(ch);
            if ch == '*' && i + 1 < n && chars[i + 1] == '/' {
                out.push('/');
                in_block_comment = false;
                i += 1;
            }
        } else if in_single_quote {
            out.push(ch);
            if ch == '\\' && i + 1 < n {
                i += 1;
                out.push(chars[i]);
            } else if ch == '\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            out.push(ch);
            if ch == '\\' && i + 1 < n {
                i += 1;
                out.push(chars[i]);
            } else if ch == '"' {
                in_double_quote = false;
            }
        } else if in_backtick {
            out.push(ch);
            if ch == '`' {
                in_backtick = false;
            }
        } else {
            if ch == '-' && i + 1 < n && chars[i + 1] == '-' {
                in_line_comment = true;
                out.push(ch);
                out.push('-');
                i += 1;
            } else if ch == '#' {
                in_line_comment = true;
                out.push(ch);
            } else if ch == '/' && i + 1 < n && chars[i + 1] == '*' {
                in_block_comment = true;
                out.push(ch);
                out.push('*');
                i += 1;
            } else if ch == '\'' {
                in_single_quote = true;
                out.push(ch);
            } else if ch == '"' {
                in_double_quote = true;
                out.push(ch);
            } else if ch == '`' {
                in_backtick = true;
                out.push(ch);
            } else if ch == ':'
                && (i == 0 || (!chars[i - 1].is_ascii_alphanumeric() && chars[i - 1] != ':'))
                && i + 1 < n
                && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_')
            {
                let mut j = i + 1;
                while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                if let Some(val) = params.get(&name) {
                    let trimmed = val.trim();
                    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                        out.push_str("NULL");
                    } else if trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok() {
                        out.push_str(trimmed);
                    } else {
                        out.push('\'');
                        out.push_str(&trimmed.replace('\'', "''"));
                        out.push('\'');
                    }
                    i = j - 1;
                } else {
                    out.push(ch);
                }
            } else {
                out.push(ch);
            }
        }
        i += 1;
    }

    out
}

/// Expands short SQL productivity macros (e.g. `sel*`, `cnt*`, `join*`) into full templates
/// and returns `(expanded_text, cursor_char_offset)` indicating where the user should type next.
pub fn expand_sql_macro_with_cursor(token: &str) -> Option<(&'static str, usize)> {
    match token.trim().to_lowercase().as_str() {
        "sel*" => Some(("SELECT * FROM  LIMIT 100;", 14)), // after "SELECT * FROM "
        "selw" => Some(("SELECT * FROM  WHERE  LIMIT 100;", 14)), // after "SELECT * FROM "
        "cnt*" => Some(("SELECT COUNT(*) AS total FROM ;", 30)), // before ";"
        "join*" => Some(("INNER JOIN  ON .id = ._id", 11)), // after "INNER JOIN "
        "ins*" => Some(("INSERT INTO  () VALUES ();", 12)), // after "INSERT INTO "
        "upd*" => Some(("UPDATE  SET  WHERE ;", 7)), // after "UPDATE "
        "del*" => Some(("DELETE FROM  WHERE ;", 12)), // after "DELETE FROM "
        "grp*" => Some(("SELECT , COUNT(*) AS cnt FROM  GROUP BY  ORDER BY cnt DESC;", 7)), // after "SELECT "
        _ => None,
    }
}


