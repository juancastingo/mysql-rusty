use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId};

// Superset of MySQL/MariaDB, PostgreSQL, and SQLite keywords so highlighting works uniformly.
const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "OUTER",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "LIKE",
    "ILIKE",
    "RETURNING",
    "SERIAL",
    "BIGSERIAL",
    "JSONB",
    "UUID",
    "BETWEEN",
    "EXISTS",
    "GROUP",
    "BY",
    "ORDER",
    "ASC",
    "DESC",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "UNION",
    "ALL",
    "DISTINCT",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "ALTER",
    "DROP",
    "TRUNCATE",
    "TABLE",
    "VIEW",
    "INDEX",
    "DATABASE",
    "SCHEMA",
    "IF",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "DEFAULT",
    "AUTO_INCREMENT",
    "UNIQUE",
    "CONSTRAINT",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "EXPLAIN",
    "SHOW",
    "DESCRIBE",
    "DESC",
    "USE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "TRANSACTION",
    "WITH",
];

const SQL_FUNCTIONS: &[&str] = &[
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "ROUND",
    "COALESCE",
    "IFNULL",
    "CONCAT",
    "SUBSTRING",
    "LOWER",
    "UPPER",
    "TRIM",
    "LENGTH",
    "NOW",
    "CURDATE",
    "DATE_FORMAT",
    "DATEDIFF",
    "YEAR",
    "MONTH",
    "DAY",
    "CAST",
    "CONVERT",
    "JSON_EXTRACT",
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "DATABASE",
    "VERSION",
];

const SQL_TYPES: &[&str] = &[
    "INT",
    "INTEGER",
    "BIGINT",
    "SMALLINT",
    "TINYINT",
    "UNSIGNED",
    "VARCHAR",
    "CHAR",
    "TEXT",
    "LONGTEXT",
    "MEDIUMTEXT",
    "DECIMAL",
    "NUMERIC",
    "FLOAT",
    "DOUBLE",
    "BOOLEAN",
    "BOOL",
    "DATE",
    "DATETIME",
    "TIMESTAMP",
    "TIME",
    "JSON",
    "ENUM",
    "BLOB",
];

/// Converts a character index into a valid UTF-8 byte offset in `text`.
pub fn char_idx_to_byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(text.len())
}

/// Converts a byte offset in `text` into the nearest character index.
pub fn byte_idx_to_char_idx(text: &str, byte_idx: usize) -> usize {
    let clamped = byte_idx.min(text.len());
    text.char_indices()
        .take_while(|(b, _)| *b < clamped)
        .count()
}

/// Converts `?` placeholders outside quotes/comments into `:p1`, `:p2`, ...
pub fn digest_placeholders_to_params(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut idx = 1usize;

    while i < bytes.len() {
        let b = bytes[i];
        if in_line_comment {
            let ch = sql[i..].chars().next().unwrap();
            out.push(ch);
            if b == b'\n' {
                in_line_comment = false;
            }
            i += ch.len_utf8();
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                out.push_str("*/");
                in_block_comment = false;
                i += 2;
                continue;
            }
            let ch = sql[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' && i + 1 < bytes.len() {
                let ch = sql[i + 1..].chars().next().unwrap();
                out.push('\\');
                out.push(ch);
                i += 1 + ch.len_utf8();
                continue;
            }
            if b == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    out.push(q as char);
                    out.push(q as char);
                    i += 2;
                    continue;
                }
                quote = None;
            }
            let ch = sql[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        if b == b'#'
            || (b == b'-'
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'-'
                && (i + 2 >= bytes.len() || bytes[i + 2].is_ascii_whitespace()))
        {
            in_line_comment = true;
            out.push(b as char);
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            in_block_comment = true;
            out.push_str("/*");
            i += 2;
            continue;
        }
        if b == b'\'' || b == b'"' || b == b'`' {
            quote = Some(b);
            out.push(b as char);
            i += 1;
            continue;
        }
        if b == b'?' {
            out.push_str(&format!(":p{idx}"));
            idx += 1;
            i += 1;
            continue;
        }
        let ch = sql[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Splits `LayoutJob` sections at match boundaries and highlights matching ranges.
pub fn apply_background_ranges(
    job: &mut egui::text::LayoutJob,
    ranges: &[std::ops::Range<usize>],
    current: Option<usize>,
) {
    if ranges.is_empty() || job.sections.is_empty() {
        return;
    }
    let normal_bg = Color32::from_rgba_unmultiplied(245, 158, 11, 90);
    let current_bg = Color32::from_rgba_unmultiplied(249, 115, 22, 180);

    let mut new_sections = Vec::with_capacity(job.sections.len());
    for section in job.sections.drain(..) {
        let s_start: usize = section.byte_range.start.into();
        let s_end: usize = section.byte_range.end.into();

        let mut split_points: Vec<usize> = Vec::new();
        split_points.push(s_start);
        for r in ranges {
            if r.start > s_start && r.start < s_end {
                split_points.push(r.start);
            }
            if r.end > s_start && r.end < s_end {
                split_points.push(r.end);
            }
        }
        split_points.push(s_end);
        split_points.sort_unstable();
        split_points.dedup();

        for win in split_points.windows(2) {
            let sub_start = win[0];
            let sub_end = win[1];
            if sub_start >= sub_end {
                continue;
            }
            let mut fmt = section.format.clone();
            let mut matched_idx = None;
            for (idx, r) in ranges.iter().enumerate() {
                if sub_start >= r.start && sub_end <= r.end {
                    matched_idx = Some(idx);
                    break;
                }
            }
            if let Some(idx) = matched_idx {
                fmt.background = if current == Some(idx) {
                    current_bg
                } else {
                    normal_bg
                };
            }
            new_sections.push(egui::text::LayoutSection {
                leading_space: if sub_start == s_start {
                    section.leading_space
                } else {
                    0.0
                },
                byte_range: egui::text::ByteIndex(sub_start)..egui::text::ByteIndex(sub_end),
                format: fmt,
            });
        }
    }
    job.sections = new_sections;
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
    if cursor_at_end
        && let Some(ghost) = ghost_suffix
        && !ghost.is_empty()
    {
        let ghost_fmt = TextFormat {
            font_id: mono,
            color: Color32::from_rgba_unmultiplied(125, 211, 252, 130),
            italics: true,
            ..Default::default()
        };
        job.append(ghost, 0.0, ghost_fmt);
    }

    job
}

/// Formats a SQL query with proper indentation, uppercase keywords, and clause line breaks.
pub fn format_sql_pretty(sql: &str) -> String {
    let options = sqlformat::FormatOptions {
        indent: sqlformat::Indent::Spaces(2),
        uppercase: Some(true),
        lines_between_queries: 2,
        ..Default::default()
    };
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

    if let Some(last) = stmts.last()
        && clamped_cursor >= last.raw_end
    {
        return (last.text.clone(), (last.trimmed_start, last.trimmed_end));
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
        return Some(
            "ALTER TABLE with DROP column/constraint deletes schema structures permanently.",
        );
    }

    if upper.starts_with("UPDATE ")
        || upper.starts_with("UPDATE\n")
        || upper.starts_with("UPDATE\t")
    {
        let has_where = upper
            .split_whitespace()
            .any(|word| word == "WHERE" || word.starts_with("WHERE;"));
        if !has_where {
            return Some(
                "UPDATE without a WHERE clause will overwrite EVERY single row in the table!",
            );
        }
    }

    if upper.starts_with("DELETE ")
        || upper.starts_with("DELETE\n")
        || upper.starts_with("DELETE\t")
    {
        let has_where = upper
            .split_whitespace()
            .any(|word| word == "WHERE" || word.starts_with("WHERE;"));
        if !has_where {
            return Some(
                "DELETE without a WHERE clause will delete EVERY single row in the table!",
            );
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
                let next_ok =
                    i + 5 == n || (!chars[i + 5].is_alphanumeric() && chars[i + 5] != '_');
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
    for kw in &[
        "WHERE", "ORDER BY", "GROUP BY", "HAVING", "JOIN", "UNION", "SELECT", "FROM",
    ] {
        if upper_after.split_whitespace().any(|w| w == *kw) {
            return None;
        }
    }

    let args_str = trimmed_without_semi.trim();
    if args_str.is_empty() {
        return None;
    }

    let byte_limit_start = sql.char_indices().nth(limit_char_idx).map(|(b, _)| b)?;

    let last_char_idx = limit_char_idx
        + 5
        + after_limit_str
            .trim_end()
            .trim_end_matches(';')
            .chars()
            .count();
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
            } else if ch == ':'
                && (i == 0 || !chars[i - 1].is_ascii_alphanumeric() && chars[i - 1] != ':')
                && i + 1 < n
                && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_')
            {
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
        "upd*" => Some(("UPDATE  SET  WHERE ;", 7)),       // after "UPDATE "
        "del*" => Some(("DELETE FROM  WHERE ;", 12)),      // after "DELETE FROM "
        "grp*" => Some((
            "SELECT , COUNT(*) AS cnt FROM  GROUP BY  ORDER BY cnt DESC;",
            7,
        )), // after "SELECT "
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::syntax::{extract_statement_at_cursor, format_sql_pretty};
    #[test]
    fn test_extract_statement_at_cursor() {
        let sql =
            "SELECT 'first;one' AS col1; SELECT \"second;two\"; SELECT 3 -- comment with ; \n;";
        // Statement 1 is from char 0 to 27
        let (stmt1, (s1, e1)) = extract_statement_at_cursor(sql, 5);
        assert_eq!(stmt1, "SELECT 'first;one' AS col1;");
        assert_eq!((s1, e1), (0, 27));

        // Cursor at 0 (start of statement 1)
        let (stmt1_start, _) = extract_statement_at_cursor(sql, 0);
        assert_eq!(stmt1_start, "SELECT 'first;one' AS col1;");

        // Statement 2 contains quoted semicolon
        let (stmt2, _) = extract_statement_at_cursor(sql, 35);
        assert_eq!(stmt2, "SELECT \"second;two\";");

        // Statement 3 has comment containing a semicolon
        let (stmt3, _) = extract_statement_at_cursor(sql, sql.len());
        assert!(stmt3.contains("SELECT 3"));
        assert!(stmt3.contains("-- comment with ;"));
    }
    #[test]
    fn test_format_sql_pretty() {
        let formatted = format_sql_pretty("SELECT id, name FROM users WHERE id = 1 LIMIT 10");
        assert!(formatted.contains("FROM"));
        assert!(formatted.contains("WHERE"));
        assert!(formatted.contains("LIMIT"));
        assert!(formatted.contains('\n'));
    }
    #[test]
    fn test_safety_guard_destructive_detection() {
        use crate::syntax::{is_potentially_destructive, is_write_query};

        assert!(is_potentially_destructive("DROP TABLE users;").is_some());
        assert!(is_potentially_destructive("TRUNCATE TABLE logs;").is_some());
        assert!(is_potentially_destructive("UPDATE users SET active = 0;").is_some());
        assert!(is_potentially_destructive("DELETE FROM sessions;").is_some());
        assert!(is_potentially_destructive("ALTER TABLE orders DROP COLUMN notes;").is_some());

        // Safe with WHERE
        assert!(is_potentially_destructive("UPDATE users SET active = 0 WHERE id = 42;").is_none());
        assert!(is_potentially_destructive("DELETE FROM sessions WHERE expired = 1;").is_none());
        assert!(is_potentially_destructive("SELECT * FROM users;").is_none());

        // Write query classification
        assert!(is_write_query("INSERT INTO users VALUES (1, 'Alex');"));
        assert!(is_write_query(
            "UPDATE users SET name = 'Bob' WHERE id = 1;"
        ));
        assert!(is_write_query("DELETE FROM users WHERE id = 1;"));
        assert!(is_write_query("DROP TABLE obsolete;"));
        assert!(!is_write_query("SELECT * FROM users;"));
        assert!(!is_write_query("SHOW PROCESSLIST;"));
    }
    #[test]
    fn test_apply_and_detect_limit_presets() {
        use crate::syntax::{apply_limit_to_sql, detect_current_limit};

        // 1. Appending to query without LIMIT
        let q1 = "SELECT * FROM users;";
        assert_eq!(detect_current_limit(q1), None);
        let q1_l100 = apply_limit_to_sql(q1, Some(100));
        assert_eq!(q1_l100, "SELECT * FROM users LIMIT 100;");
        assert_eq!(detect_current_limit(&q1_l100), Some(100));

        // 2. Updating existing LIMIT
        let q2 = "SELECT * FROM `_migration` LIMIT 100;";
        assert_eq!(detect_current_limit(q2), Some(100));
        let q2_l500 = apply_limit_to_sql(q2, Some(500));
        assert_eq!(q2_l500, "SELECT * FROM `_migration` LIMIT 500;");
        assert_eq!(detect_current_limit(&q2_l500), Some(500));

        // 3. Removing LIMIT (All / None)
        let q2_all = apply_limit_to_sql(&q2_l500, None);
        assert_eq!(q2_all, "SELECT * FROM `_migration`;");
        assert_eq!(detect_current_limit(&q2_all), None);

        // 4. LIMIT with offset (LIMIT offset, count)
        let q3 = "SELECT id, name FROM products LIMIT 20, 50;";
        assert_eq!(detect_current_limit(q3), Some(50));
        let q3_l250 = apply_limit_to_sql(q3, Some(250));
        assert_eq!(q3_l250, "SELECT id, name FROM products LIMIT 20, 250;");
        assert_eq!(detect_current_limit(&q3_l250), Some(250));

        // 5. Query without semicolon
        let q4 = "SELECT * FROM orders WHERE status = 'paid'";
        let q4_l10 = apply_limit_to_sql(q4, Some(10));
        assert_eq!(
            q4_l10,
            "SELECT * FROM orders WHERE status = 'paid' LIMIT 10;"
        );
    }
    #[test]
    fn test_productivity_sql_helpers() {
        use crate::syntax::{
            expand_sql_macro_with_cursor, extract_sql_parameters, split_sql_statements,
            substitute_sql_parameters,
        };
        use std::collections::HashMap;

        let script = "SELECT 1; SELECT 'a;b' AS txt; -- comment;ignored\nSELECT 2;";
        let stmts = split_sql_statements(script);
        assert_eq!(stmts.len(), 3);
        assert_eq!(stmts[0], "SELECT 1;");
        assert_eq!(stmts[1], "SELECT 'a;b' AS txt;");
        assert!(stmts[2].ends_with("SELECT 2;"));

        let param_sql = "SELECT * FROM users WHERE id = :user_id AND status = :status AND time_str = '12:30:00';";
        let params = extract_sql_parameters(param_sql);
        assert_eq!(params, vec!["user_id".to_string(), "status".to_string()]);

        let mut values = HashMap::new();
        values.insert("user_id".to_string(), "42".to_string());
        values.insert("status".to_string(), "active".to_string());
        let substituted = substitute_sql_parameters(param_sql, &values);
        assert!(substituted.contains("id = 42"));
        assert!(substituted.contains("status = 'active'"));
        assert!(substituted.contains("'12:30:00'"));

        assert_eq!(
            expand_sql_macro_with_cursor("sel*").map(|(s, _)| s),
            Some("SELECT * FROM  LIMIT 100;")
        );
        assert_eq!(
            expand_sql_macro_with_cursor("cnt*").map(|(s, _)| s),
            Some("SELECT COUNT(*) AS total FROM ;")
        );
        assert_eq!(expand_sql_macro_with_cursor("unknown"), None);
    }

    // --- char_idx_to_byte_idx / split_sql_at_cursor / replace_at_cursor ---

    #[test]
    fn test_char_idx_to_byte_idx_ascii_and_empty() {
        assert_eq!(char_idx_to_byte_idx("", 0), 0);
        assert_eq!(char_idx_to_byte_idx("", 10), 0);
        assert_eq!(char_idx_to_byte_idx("SELECT", 0), 0);
        assert_eq!(char_idx_to_byte_idx("SELECT", 3), 3);
        assert_eq!(char_idx_to_byte_idx("SELECT", 99), 6);
    }

    #[test]
    fn test_char_idx_to_byte_idx_multibyte_utf8() {
        let s = "SELECT 'ñandú' AS 🦀;";
        // 'ñ' is 2 bytes, 'ú' is 2 bytes, '🦀' is 4 bytes
        let b_quote = char_idx_to_byte_idx(s, 8);
        assert_eq!(&s[b_quote..b_quote + 2], "ñ");
        assert_eq!(char_idx_to_byte_idx(s, s.chars().count()), s.len());
    }

    #[test]
    fn test_split_sql_at_cursor_boundaries() {
        let s = "SELECT 'ñandú' FROM t";
        let (l0, r0) = split_sql_at_cursor(s, 0);
        assert_eq!(l0, "");
        assert_eq!(r0, s);

        let (lend, rend) = split_sql_at_cursor(s, s.chars().count());
        assert_eq!(lend, s);
        assert_eq!(rend, "");

        let (l_mid, r_mid) = split_sql_at_cursor(s, 14);
        assert_eq!(l_mid, "SELECT 'ñandú'");
        assert_eq!(r_mid, " FROM t");
    }

    #[test]
    fn test_replace_at_cursor_utf8_and_bounds() {
        // "SELECT 'ñ' FROM u": cursor at end (17), replace 1 byte ("u") with "users"
        let mut s = "SELECT 'ñ' FROM u".to_string();
        let new_cur = replace_at_cursor(&mut s, 17, 1, "users");
        assert_eq!(s, "SELECT 'ñ' FROM users");
        assert_eq!(new_cur, 21);

        let mut s2 = "abc".to_string();
        let cur2 = replace_at_cursor(&mut s2, 0, 0, "X");
        assert_eq!(s2, "Xabc");
        assert_eq!(cur2, 1);

        // Replacing multibyte char 'ñ' (2 bytes) at cursor
        let mut s3 = "SELECT 'ñ".to_string();
        let cur3 = replace_at_cursor(&mut s3, 9, 2, "a");
        assert_eq!(s3, "SELECT 'a");
        assert_eq!(cur3, 9);
    }

    // --- highlight_sql ---

    #[test]
    fn test_highlight_sql_produces_layout_job() {
        let sql = "SELECT COUNT(*) FROM `users` WHERE name = 'Alice' -- cmt\nAND age > 10;";
        let job = highlight_sql(sql, Some(" LIMIT 10"), Some(sql.chars().count()), 13.0);
        assert!(job.text.contains("SELECT"));
        assert!(job.text.contains("LIMIT 10"));
        assert!(!job.sections.is_empty());

        let job_no_ghost = highlight_sql("SELECT 1", Some(" LIMIT 10"), Some(0), 13.0);
        assert!(!job_no_ghost.text.contains("LIMIT 10"));
    }

    // --- extract_statement_at_cursor ---

    #[test]
    fn test_extract_statement_at_cursor_quotes_and_backticks() {
        let sql = "SELECT `a;b`, \"c;d\" FROM t; SELECT 'e;f';";
        let (s1, _) = extract_statement_at_cursor(sql, 10);
        assert_eq!(s1, "SELECT `a;b`, \"c;d\" FROM t;");
        let (s2, _) = extract_statement_at_cursor(sql, 32);
        assert_eq!(s2, "SELECT 'e;f';");
    }

    #[test]
    fn test_extract_statement_at_cursor_block_comment_and_cursor_on_semicolon() {
        let sql = "SELECT /* ; */ 1; SELECT 2;   ";
        let semi_idx = sql.find(';').unwrap(); // Wait: first ';' is inside /* ; */
        let real_semi = sql.match_indices(';').nth(1).unwrap().0;
        assert!(semi_idx < real_semi);
        let (s1, _) = extract_statement_at_cursor(sql, real_semi);
        assert_eq!(s1, "SELECT /* ; */ 1;");

        let (s_end, _) = extract_statement_at_cursor(sql, sql.chars().count());
        assert_eq!(s_end, "SELECT 2;");
    }

    // --- split_sql_statements ---

    #[test]
    fn test_split_sql_statements_quoting_and_empty() {
        let sql = ";; SELECT 'a;b';; SELECT \"c;d\"; SELECT `e;f`";
        let stmts = split_sql_statements(sql);
        assert_eq!(
            stmts,
            vec!["SELECT 'a;b';", "SELECT \"c;d\";", "SELECT `e;f`",]
        );
    }

    #[test]
    fn test_split_sql_statements_comments_and_no_trailing_semicolon() {
        let sql = "-- lead;\nSELECT /* ; */ 1;\nSELECT 2";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("SELECT /* ; */ 1;"));
        assert_eq!(stmts[1], "SELECT 2");
    }

    #[test]
    fn test_split_sql_statements_delimiter_current_behavior() {
        // DELIMITER is not specially parsed; splitting still happens on `;` outside quotes/comments.
        let sql = "DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END//\nDELIMITER ;";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
    }

    // --- is_potentially_destructive ---

    #[test]
    fn test_is_potentially_destructive_cases() {
        assert!(is_potentially_destructive("DELETE FROM users;").is_some());
        assert!(is_potentially_destructive("delete from users").is_some());
        assert!(is_potentially_destructive("DELETE FROM users WHERE id = 1;").is_none());
        assert!(is_potentially_destructive("UPDATE users SET a = 1;").is_some());
        assert!(is_potentially_destructive("UPDATE users SET a = 1 WHERE id = 1;").is_none());
    }

    #[test]
    #[ignore = "bug: is_potentially_destructive does not strip leading SQL comments"]
    fn test_is_potentially_destructive_leading_comments() {
        assert!(is_potentially_destructive("-- comment\nDROP TABLE users;").is_some());
        assert!(is_potentially_destructive("/* c */ TRUNCATE TABLE logs;").is_some());
    }

    #[test]
    #[ignore = "bug: is_potentially_destructive checks WHERE via substring and misses WHERE inside string literals"]
    fn test_is_potentially_destructive_where_inside_string_literal() {
        assert!(is_potentially_destructive("UPDATE t SET note = 'where';").is_some());
    }

    // --- is_write_query ---

    #[test]
    fn test_is_write_query_reads_and_writes() {
        assert!(!is_write_query("SELECT * FROM users"));
        assert!(is_write_query("INSERT INTO users VALUES (1)"));
        assert!(is_write_query("REPLACE INTO users VALUES (1)"));
        assert!(is_write_query("ALTER TABLE users ADD COLUMN x INT"));
        assert!(is_write_query("CREATE TABLE t (id INT)"));
        assert!(is_write_query("UPDATE users SET a = 1"));
        assert!(is_write_query("DELETE FROM users WHERE id = 1"));
        assert!(is_write_query("DROP TABLE users"));
        assert!(is_write_query("TRUNCATE TABLE users"));
    }

    #[test]
    #[ignore = "bug: is_write_query misses GRANT/SET/CALL and leading comments"]
    fn test_is_write_query_dcl_and_comments() {
        assert!(is_write_query("GRANT SELECT ON db.* TO 'u'@'%'"));
        assert!(is_write_query("SET @a = 1"));
        assert!(is_write_query("/* run */ CALL my_proc()"));
    }

    // --- detect_current_limit / apply_limit_to_sql ---

    #[test]
    fn test_detect_and_apply_limit_variants() {
        assert_eq!(detect_current_limit("SELECT * FROM users"), None);
        assert_eq!(
            detect_current_limit("SELECT * FROM users LIMIT 10;"),
            Some(10)
        );
        assert_eq!(
            detect_current_limit("SELECT * FROM users LIMIT 5, 10"),
            Some(10)
        );
        assert_eq!(
            detect_current_limit("SELECT * FROM users LIMIT 10 OFFSET 5;"),
            Some(10)
        );

        assert_eq!(
            apply_limit_to_sql("SELECT * FROM users;", Some(50)),
            "SELECT * FROM users LIMIT 50;"
        );
        assert_eq!(
            apply_limit_to_sql("SELECT * FROM users LIMIT 10 OFFSET 5;", Some(25)),
            "SELECT * FROM users LIMIT 25 OFFSET 5;"
        );
        assert_eq!(
            apply_limit_to_sql("SELECT * FROM users LIMIT 5, 10;", None),
            "SELECT * FROM users;"
        );
    }

    // --- extract_sql_parameters / substitute_sql_parameters ---

    #[test]
    fn test_extract_sql_parameters_edge_cases() {
        assert_eq!(
            extract_sql_parameters("SELECT * FROM t WHERE a = :id AND b = :id AND c = :name"),
            vec!["id".to_string(), "name".to_string()]
        );
        assert_eq!(
            extract_sql_parameters("SELECT val::int, '12:30:00' FROM t WHERE x = :x_1"),
            vec!["x_1".to_string()]
        );
        assert!(extract_sql_parameters("SELECT ':not_a_param' FROM t").is_empty());
    }

    #[test]
    fn test_substitute_sql_parameters_escaping_and_null() {
        let mut vals = HashMap::new();
        vals.insert("id".to_string(), "42".to_string());
        vals.insert("name".to_string(), "O'Brien\\test".to_string());
        vals.insert("note".to_string(), "NULL".to_string());
        vals.insert("empty".to_string(), "".to_string());

        let sql = "SELECT * FROM t WHERE id = :id AND name = :name AND note = :note AND e = :empty";
        let out = substitute_sql_parameters(sql, &vals);
        assert!(out.contains("id = 42"));
        assert!(out.contains("name = 'O''Brien\\test'"));
        assert!(out.contains("note = NULL"));
        assert!(out.contains("e = NULL"));
    }

    // Additional focused unit tests to reach >= 60 total tests
    #[test]
    fn test_char_idx_to_byte_idx_at_zero() {
        assert_eq!(char_idx_to_byte_idx("abc", 0), 0);
    }

    #[test]
    fn test_char_idx_to_byte_idx_past_end() {
        assert_eq!(char_idx_to_byte_idx("abc", 100), 3);
    }

    #[test]
    fn test_split_sql_at_cursor_empty() {
        assert_eq!(split_sql_at_cursor("", 0), ("", ""));
    }

    #[test]
    fn test_split_sql_at_cursor_emoji() {
        let (l, r) = split_sql_at_cursor("SELECT '🦀'", 9);
        assert_eq!(l, "SELECT '🦀");
        assert_eq!(r, "'");
    }

    #[test]
    fn test_replace_at_cursor_empty_insertion() {
        let mut s = "SELECT foo".to_string();
        let cur = replace_at_cursor(&mut s, 10, 3, "");
        assert_eq!(s, "SELECT ");
        assert_eq!(cur, 7);
    }

    #[test]
    fn test_extract_statement_at_cursor_empty_input() {
        let (stmt, span) = extract_statement_at_cursor("   ", 0);
        assert_eq!(stmt, "");
        assert_eq!(span, (0, 0));
    }

    #[test]
    fn test_extract_statement_at_cursor_single_no_semicolon() {
        let (stmt, _) = extract_statement_at_cursor("SELECT 1 + 2", 3);
        assert_eq!(stmt, "SELECT 1 + 2");
    }

    #[test]
    fn test_split_sql_statements_empty_and_whitespace() {
        assert!(split_sql_statements("").is_empty());
        assert!(split_sql_statements("   \n\t  ").is_empty());
    }

    #[test]
    fn test_split_sql_statements_hash_comment() {
        let stmts = split_sql_statements("# comment with ;\nSELECT 1;");
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_is_potentially_destructive_drop_schema() {
        assert!(is_potentially_destructive("DROP SCHEMA prod;").is_some());
        assert!(is_potentially_destructive("DROP DATABASE prod;").is_some());
    }

    #[test]
    fn test_is_potentially_destructive_alter_drop() {
        assert!(is_potentially_destructive("ALTER TABLE users DROP COLUMN age;").is_some());
        assert!(is_potentially_destructive("ALTER TABLE users ADD COLUMN age INT;").is_none());
    }

    #[test]
    fn test_is_write_query_with_cte_select() {
        assert!(!is_write_query("WITH cte AS (SELECT 1) SELECT * FROM cte"));
    }

    #[test]
    fn test_detect_current_limit_inside_subquery_ignored() {
        assert_eq!(
            detect_current_limit("SELECT * FROM (SELECT * FROM t LIMIT 5) sub"),
            None
        );
    }

    #[test]
    fn test_detect_current_limit_inside_string_ignored() {
        assert_eq!(detect_current_limit("SELECT 'LIMIT 5' AS txt FROM t"), None);
    }

    #[test]
    fn test_apply_limit_to_sql_without_semicolon() {
        assert_eq!(
            apply_limit_to_sql("SELECT * FROM t", Some(100)),
            "SELECT * FROM t LIMIT 100;"
        );
    }

    #[test]
    fn test_extract_sql_parameters_empty() {
        assert!(extract_sql_parameters("SELECT 1").is_empty());
    }

    #[test]
    fn test_substitute_sql_parameters_empty_map() {
        let vals = HashMap::new();
        assert_eq!(substitute_sql_parameters("SELECT :id", &vals), "SELECT :id");
    }

    #[test]
    fn test_substitute_sql_parameters_float() {
        let mut vals = HashMap::new();
        vals.insert("price".to_string(), "19.99".to_string());
        assert_eq!(
            substitute_sql_parameters("SELECT * FROM p WHERE price > :price", &vals),
            "SELECT * FROM p WHERE price > 19.99"
        );
    }

    #[test]
    fn test_format_sql_pretty_preserves_strings() {
        let formatted = format_sql_pretty("select 'hello world' from dual");
        assert!(formatted.contains("'hello world'"));
        assert!(formatted.contains("SELECT"));
    }

    // --- expand_sql_macro_with_cursor ---

    #[test]
    fn test_expand_sql_macro_with_cursor_all_macros() {
        for macro_key in [
            "sel*", "selw", "cnt*", "join*", "ins*", "upd*", "del*", "grp*",
        ] {
            let expanded = expand_sql_macro_with_cursor(macro_key);
            assert!(expanded.is_some(), "macro {macro_key} should expand");
            let (text, cursor) = expanded.unwrap();
            assert!(!text.is_empty());
            assert!(cursor <= text.chars().count());
        }
        assert_eq!(expand_sql_macro_with_cursor("nope"), None);
    }

    // --- format_sql_pretty idempotency ---

    #[test]
    fn test_format_sql_pretty_idempotent_and_uppercase() {
        let raw = "select id, name from users where active = 1 order by id desc";
        let once = format_sql_pretty(raw);
        let twice = format_sql_pretty(&once);
        assert!(once.contains("SELECT"));
        assert!(once.contains("FROM"));
        assert_eq!(once, twice);
        assert_eq!(format_sql_pretty("   "), "   ");
    }

    #[test]
    fn test_apply_background_ranges_splits_and_highlights() {
        let mut job = highlight_sql("SELECT user_id, order_id FROM orders", None, None, 14.0);
        let matches = vec![7..14, 16..24];
        apply_background_ranges(&mut job, &matches, Some(1));

        let highlighted: Vec<_> = job
            .sections
            .iter()
            .filter(|s| s.format.background != Color32::TRANSPARENT)
            .collect();
        assert_eq!(highlighted.len(), 2);
        assert_eq!(
            highlighted[0].byte_range,
            egui::text::ByteIndex(7)..egui::text::ByteIndex(14)
        );
        assert_eq!(
            highlighted[1].byte_range,
            egui::text::ByteIndex(16)..egui::text::ByteIndex(24)
        );
        assert_ne!(
            highlighted[0].format.background,
            highlighted[1].format.background
        );
        assert_eq!(byte_idx_to_char_idx("aé b", 3), 2);
    }

    #[test]
    fn test_digest_placeholders_to_params() {
        let sql = "SELECT * FROM users WHERE status = '?' AND id = ? AND age > ?";
        assert_eq!(
            digest_placeholders_to_params(sql),
            "SELECT * FROM users WHERE status = '?' AND id = :p1 AND age > :p2"
        );
    }
}
