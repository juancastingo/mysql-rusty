use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Dialect {
    #[default]
    MySql,
    Postgres,
    Sqlite,
}

#[allow(dead_code)]
impl Dialect {
    pub fn quote_ident(&self, s: &str) -> String {
        match self {
            Self::MySql => format!("`{}`", s.replace('`', "``")),
            Self::Postgres | Self::Sqlite => format!("\"{}\"", s.replace('"', "\"\"")),
        }
    }

    pub fn quote_qualified(&self, db: &str, t: &str) -> String {
        if db.trim().is_empty() {
            self.quote_ident(t)
        } else {
            format!("{}.{}", self.quote_ident(db), self.quote_ident(t))
        }
    }

    pub fn string_literal(&self, s: &str) -> String {
        match self {
            Self::MySql => {
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('\'', "''")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\0', "\\0");
                format!("'{escaped}'")
            }
            Self::Postgres | Self::Sqlite => {
                let escaped = s.replace('\'', "''");
                format!("'{escaped}'")
            }
        }
    }

    pub fn limit_clause(&self, n: usize) -> String {
        format!("LIMIT {n}")
    }

    pub fn supports_use_database(&self) -> bool {
        matches!(self, Self::MySql)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MySql => "MySQL / MariaDB",
            Self::Postgres => "PostgreSQL",
            Self::Sqlite => "SQLite",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_ident_and_qualified() {
        assert_eq!(Dialect::MySql.quote_ident("a`b"), "`a``b`");
        assert_eq!(Dialect::Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(Dialect::Sqlite.quote_ident("users"), "\"users\"");

        assert_eq!(
            Dialect::MySql.quote_qualified("app", "users"),
            "`app`.`users`"
        );
        assert_eq!(
            Dialect::Postgres.quote_qualified("public", "users"),
            "\"public\".\"users\""
        );
        assert_eq!(Dialect::Sqlite.quote_qualified("", "users"), "\"users\"");
    }

    #[test]
    fn test_string_literal_and_capabilities() {
        assert_eq!(Dialect::MySql.string_literal("a'b\\c"), "'a''b\\\\c'");
        assert_eq!(Dialect::Postgres.string_literal("a'b\\c"), "'a''b\\c'");
        assert_eq!(Dialect::Sqlite.string_literal("a'b"), "'a''b'");
        assert_eq!(Dialect::MySql.limit_clause(100), "LIMIT 100");
        assert!(Dialect::MySql.supports_use_database());
        assert!(!Dialect::Postgres.supports_use_database());
        assert!(!Dialect::Sqlite.supports_use_database());
    }
}
