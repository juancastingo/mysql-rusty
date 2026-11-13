use std::io::BufRead;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteKind {
    Single,
    Double,
    Backtick,
}

pub struct SqlStatementReader<R: BufRead> {
    reader: R,
    delimiter: String,
    bytes_read: u64,
    eof: bool,
    buf: String,
    pending_line: String,
    in_quote: Option<QuoteKind>,
    in_block_comment: bool,
    in_conditional_comment: bool,
}

impl<R: BufRead> SqlStatementReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            delimiter: ";".to_string(),
            bytes_read: 0,
            eof: false,
            buf: String::new(),
            pending_line: String::new(),
            in_quote: None,
            in_block_comment: false,
            in_conditional_comment: false,
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    fn try_parse_delimiter_line(line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed
            .get(..9)
            .is_some_and(|p| p.eq_ignore_ascii_case("DELIMITER"))
        {
            let rest = trimmed[9..].trim_start();
            if trimmed
                .as_bytes()
                .get(9)
                .is_some_and(|b| b.is_ascii_whitespace())
                && !rest.is_empty()
            {
                let delim = rest.split_whitespace().next().unwrap_or(";").to_string();
                return Some(delim);
            }
        }
        None
    }
}

impl<R: BufRead> Iterator for SqlStatementReader<R> {
    type Item = std::io::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = if !self.pending_line.is_empty() {
                std::mem::take(&mut self.pending_line)
            } else if self.eof {
                let rem = self.buf.trim().to_string();
                self.buf.clear();
                if rem.is_empty() {
                    return None;
                }
                return Some(Ok(rem));
            } else {
                let mut next_line = String::new();
                match self.reader.read_line(&mut next_line) {
                    Ok(0) => {
                        self.eof = true;
                        continue;
                    }
                    Ok(n) => {
                        self.bytes_read += n as u64;
                        next_line
                    }
                    Err(e) => return Some(Err(e)),
                }
            };

            if self.in_quote.is_none()
                && !self.in_block_comment
                && self.buf.trim().is_empty()
                && let Some(new_delim) = Self::try_parse_delimiter_line(&line)
            {
                self.buf.clear();
                self.delimiter = new_delim;
                continue;
            }

            let bytes = line.as_bytes();
            let mut i = 0;
            let delim_bytes = self.delimiter.as_bytes();

            while i < bytes.len() {
                if let Some(q) = self.in_quote {
                    let q_byte = match q {
                        QuoteKind::Single => b'\'',
                        QuoteKind::Double => b'"',
                        QuoteKind::Backtick => b'`',
                    };
                    if bytes[i] == b'\\' && q != QuoteKind::Backtick && i + 1 < bytes.len() {
                        let c1 = bytes[i] as char;
                        let ch = line[i + 1..].chars().next().unwrap();
                        self.buf.push(c1);
                        self.buf.push(ch);
                        i += 1 + ch.len_utf8();
                        continue;
                    }
                    if bytes[i] == q_byte {
                        if i + 1 < bytes.len() && bytes[i + 1] == q_byte {
                            self.buf.push(q_byte as char);
                            self.buf.push(q_byte as char);
                            i += 2;
                            continue;
                        }
                        self.in_quote = None;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    self.buf.push(ch);
                    i += ch.len_utf8();
                    continue;
                }

                if self.in_block_comment {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        if self.in_conditional_comment {
                            self.buf.push_str("*/");
                            self.in_conditional_comment = false;
                        }
                        self.in_block_comment = false;
                        i += 2;
                        continue;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    if self.in_conditional_comment {
                        self.buf.push(ch);
                    }
                    i += ch.len_utf8();
                    continue;
                }

                // Line comments: -- (followed by whitespace or EOL) or #
                if bytes[i] == b'#' {
                    break;
                }
                if bytes[i] == b'-'
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'-'
                    && (i + 2 >= bytes.len() || bytes[i + 2].is_ascii_whitespace())
                {
                    break;
                }

                // Block comment start (keep MySQL conditional comments /*! ... */)
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    if i + 2 < bytes.len() && bytes[i + 2] == b'!' {
                        self.in_block_comment = true;
                        self.in_conditional_comment = true;
                        self.buf.push_str("/*!");
                        i += 3;
                    } else {
                        self.in_block_comment = true;
                        self.in_conditional_comment = false;
                        i += 2;
                    }
                    continue;
                }

                // Quote start
                match bytes[i] {
                    b'\'' => {
                        self.in_quote = Some(QuoteKind::Single);
                        self.buf.push('\'');
                        i += 1;
                        continue;
                    }
                    b'"' => {
                        self.in_quote = Some(QuoteKind::Double);
                        self.buf.push('"');
                        i += 1;
                        continue;
                    }
                    b'`' => {
                        self.in_quote = Some(QuoteKind::Backtick);
                        self.buf.push('`');
                        i += 1;
                        continue;
                    }
                    _ => {}
                }

                // Delimiter match
                if !delim_bytes.is_empty()
                    && i + delim_bytes.len() <= bytes.len()
                    && &bytes[i..i + delim_bytes.len()] == delim_bytes
                {
                    let stmt = self.buf.trim().to_string();
                    self.buf.clear();
                    i += delim_bytes.len();
                    if !stmt.is_empty() {
                        if i < bytes.len() {
                            self.pending_line = line[i..].to_string();
                        }
                        return Some(Ok(stmt));
                    }
                    continue;
                }

                let ch = line[i..].chars().next().unwrap();
                self.buf.push(ch);
                i += ch.len_utf8();
            }

            if !self.buf.is_empty() && !self.buf.ends_with('\n') && self.in_quote.is_some() {
                // Newlines inside quotes are already in `line` because `read_line` includes `\n`.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn collect_stmts(sql: &str) -> Vec<String> {
        SqlStatementReader::new(BufReader::new(Cursor::new(sql.as_bytes())))
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn test_basic_multiple_statements_and_bytes_read() {
        let sql = "SELECT 1; SELECT 2;\nSELECT 3;";
        let mut reader = SqlStatementReader::new(BufReader::new(Cursor::new(sql.as_bytes())));
        let stmts: Vec<String> = (&mut reader).map(|r| r.unwrap()).collect();
        assert_eq!(stmts, vec!["SELECT 1", "SELECT 2", "SELECT 3"]);
        assert_eq!(reader.bytes_read(), sql.len() as u64);
    }

    #[test]
    fn test_quotes_containing_semicolon() {
        let sql = "INSERT INTO t VALUES ('a;b', \"c;d\", `e;f`); SELECT 1;";
        assert_eq!(
            collect_stmts(sql),
            vec!["INSERT INTO t VALUES ('a;b', \"c;d\", `e;f`)", "SELECT 1"]
        );
    }

    #[test]
    fn test_escaped_and_doubled_quotes() {
        let sql = r#"INSERT INTO t VALUES ('it\'s;ok', 'say ''hi;'''); SELECT 2;"#;
        assert_eq!(
            collect_stmts(sql),
            vec![
                r#"INSERT INTO t VALUES ('it\'s;ok', 'say ''hi;''')"#,
                "SELECT 2"
            ]
        );
    }

    #[test]
    fn test_line_comments_with_semicolons() {
        let sql = "-- comment; ignored\n# hash; ignored\nSELECT 1; -- trailing; ignored\n";
        assert_eq!(collect_stmts(sql), vec!["SELECT 1"]);
    }

    #[test]
    fn test_block_comments_with_semicolons() {
        let sql = "/* multi;\nline; comment; */ SELECT /* inline; */ 42;";
        assert_eq!(collect_stmts(sql), vec!["SELECT  42"]);
    }

    #[test]
    fn test_conditional_comments_preserved() {
        let sql = "/*!40101 SET NAMES utf8mb4 */;\n/*!40014 SET @OLD_FK=@@FOREIGN_KEY_CHECKS */;";
        assert_eq!(
            collect_stmts(sql),
            vec![
                "/*!40101 SET NAMES utf8mb4 */",
                "/*!40014 SET @OLD_FK=@@FOREIGN_KEY_CHECKS */"
            ]
        );
    }

    #[test]
    fn test_delimiter_procedure_body() {
        let sql = "\
DELIMITER ;;
CREATE PROCEDURE p()
BEGIN
  SELECT 1;
  SELECT 2;
END;;
DELIMITER ;
SELECT 3;";
        assert_eq!(
            collect_stmts(sql),
            vec![
                "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND",
                "SELECT 3"
            ]
        );
    }

    #[test]
    fn test_crlf_line_endings() {
        let sql = "DELIMITER $$\r\nCREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW BEGIN SET NEW.a = 1; END$$\r\nDELIMITER ;\r\nSELECT 9;\r\n";
        assert_eq!(
            collect_stmts(sql),
            vec![
                "CREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW BEGIN SET NEW.a = 1; END",
                "SELECT 9"
            ]
        );
    }

    #[test]
    fn test_utf8_multibyte_split_across_small_buffer() {
        let sql = "SELECT 'héllo 🚀 世界; test'; SELECT 'αβγ';";
        let reader =
            SqlStatementReader::new(BufReader::with_capacity(4, Cursor::new(sql.as_bytes())));
        let stmts: Vec<String> = reader.map(|r| r.unwrap()).collect();
        assert_eq!(stmts, vec!["SELECT 'héllo 🚀 世界; test'", "SELECT 'αβγ'"]);
    }

    #[test]
    fn test_no_trailing_delimiter() {
        let sql = "SELECT 1;\nSELECT 2";
        assert_eq!(collect_stmts(sql), vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn test_empty_statements_skipped() {
        let sql = " ; ; \n ;\nSELECT 1;\n;\n";
        assert_eq!(collect_stmts(sql), vec!["SELECT 1"]);
    }

    #[test]
    fn test_case_insensitive_delimiter() {
        let sql = "delimiter //\nSELECT 'a;b'//\ndelimiter ;\nSELECT 2;";
        assert_eq!(collect_stmts(sql), vec!["SELECT 'a;b'", "SELECT 2"]);
    }
}
