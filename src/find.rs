use regex::{Regex, RegexBuilder};
use std::ops::Range;

const MAX_MATCHES: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub query: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

fn build_regex(opts: &FindOptions) -> Result<Regex, String> {
    if opts.query.is_empty() {
        return Err("Empty query".to_string());
    }
    let core = if opts.regex {
        opts.query.clone()
    } else {
        regex::escape(&opts.query)
    };
    let pat = if opts.whole_word {
        format!(r"\b(?:{core})\b")
    } else {
        core
    };
    RegexBuilder::new(&pat)
        .case_insensitive(!opts.case_sensitive)
        .build()
        .map_err(|e| e.to_string())
}

pub fn find_matches(text: &str, opts: &FindOptions) -> Result<Vec<Range<usize>>, String> {
    if opts.query.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_regex(opts)?;
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        out.push(m.start()..m.end());
        if out.len() >= MAX_MATCHES {
            break;
        }
    }
    Ok(out)
}

pub fn replace_one(text: &str, m: Range<usize>, replacement: &str, opts: &FindOptions) -> String {
    if m.start > m.end
        || m.end > text.len()
        || !text.is_char_boundary(m.start)
        || !text.is_char_boundary(m.end)
    {
        return text.to_string();
    }
    let slice = &text[m.clone()];
    let replaced = if opts.regex {
        match build_regex(opts) {
            Ok(re) => re.replace(slice, replacement).into_owned(),
            Err(_) => replacement.to_string(),
        }
    } else {
        replacement.to_string()
    };
    let mut out = String::with_capacity(text.len() + replaced.len());
    out.push_str(&text[..m.start]);
    out.push_str(&replaced);
    out.push_str(&text[m.end..]);
    out
}

pub fn replace_all(text: &str, opts: &FindOptions, replacement: &str) -> (String, usize) {
    if opts.query.is_empty() {
        return (text.to_string(), 0);
    }
    let Ok(re) = build_regex(opts) else {
        return (text.to_string(), 0);
    };
    let mut count = 0usize;
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0usize;
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        out.push_str(&text[last_end..m.start()]);
        if opts.regex {
            let expanded = re.replace(&text[m.start()..m.end()], replacement);
            out.push_str(&expanded);
        } else {
            out.push_str(replacement);
        }
        last_end = m.end();
        count += 1;
        if count >= MAX_MATCHES {
            break;
        }
    }
    out.push_str(&text[last_end..]);
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_query_returns_empty() {
        let opts = FindOptions::default();
        assert_eq!(
            find_matches("SELECT 1", &opts).unwrap(),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn test_case_insensitive_default() {
        let opts = FindOptions {
            query: "select".to_string(),
            ..Default::default()
        };
        assert_eq!(
            find_matches("SELECT a, select b", &opts).unwrap(),
            vec![0..6, 10..16]
        );
    }

    #[test]
    fn test_case_sensitive() {
        let opts = FindOptions {
            query: "select".to_string(),
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(
            find_matches("SELECT a, select b", &opts).unwrap(),
            vec![10..16]
        );
    }

    #[test]
    fn test_whole_word() {
        let opts = FindOptions {
            query: "user".to_string(),
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            find_matches("user users user_id user", &opts).unwrap(),
            vec![0..4, 19..23]
        );
    }

    #[test]
    fn test_plain_text_escapes_regex_metacharacters() {
        let opts = FindOptions {
            query: "a.b[0]+".to_string(),
            ..Default::default()
        };
        assert_eq!(
            find_matches("axb0 a.b[0]+ end", &opts).unwrap(),
            vec![5..12]
        );
    }

    #[test]
    fn test_invalid_regex_returns_err() {
        let opts = FindOptions {
            query: "(unclosed".to_string(),
            regex: true,
            ..Default::default()
        };
        assert!(find_matches("abc", &opts).is_err());
    }

    #[test]
    fn test_replace_one_plain() {
        let opts = FindOptions {
            query: "foo".to_string(),
            ..Default::default()
        };
        let out = replace_one("foo bar foo", 8..11, "baz", &opts);
        assert_eq!(out, "foo bar baz");
    }

    #[test]
    fn test_replace_one_regex_capture_group() {
        let opts = FindOptions {
            query: r"(\w+)_id".to_string(),
            regex: true,
            ..Default::default()
        };
        let matches = find_matches("SELECT user_id, order_id", &opts).unwrap();
        let out = replace_one(
            "SELECT user_id, order_id",
            matches[0].clone(),
            "${1}_key",
            &opts,
        );
        assert_eq!(out, "SELECT user_key, order_id");
    }

    #[test]
    fn test_replace_all_regex_groups() {
        let opts = FindOptions {
            query: r"(\w+)_id".to_string(),
            regex: true,
            ..Default::default()
        };
        let (out, count) = replace_all("SELECT user_id, order_id FROM t", &opts, "${1}_key");
        assert_eq!(count, 2);
        assert_eq!(out, "SELECT user_key, order_key FROM t");
    }

    #[test]
    fn test_replace_all_literal_dollar_in_plain_mode() {
        let opts = FindOptions {
            query: "price".to_string(),
            regex: false,
            ..Default::default()
        };
        let (out, count) = replace_all("price + price", &opts, "$1");
        assert_eq!(count, 2);
        assert_eq!(out, "$1 + $1");
    }

    #[test]
    fn test_match_cap_10000() {
        let text = "a ".repeat(12_000);
        let opts = FindOptions {
            query: "a".to_string(),
            ..Default::default()
        };
        let matches = find_matches(&text, &opts).unwrap();
        assert_eq!(matches.len(), 10_000);
    }

    #[test]
    fn test_replace_one_invalid_utf8_boundary_does_not_panic() {
        let text = "SELECT '🦀' FROM t";
        let opts = FindOptions::default();
        // Byte 9 is inside the 4-byte emoji; must return text unchanged rather than panic
        let out = replace_one(text, 9..11, "x", &opts);
        assert_eq!(out, text);
    }
}
