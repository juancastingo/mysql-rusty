use crate::db::{ActiveSession, CellValue, ColumnInfo, TableSchema};
use crate::export::sql_literal;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

fn sql_str(s: &str) -> String {
    sql_literal(&CellValue::Text(s.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum MockFieldType {
    AutoIncrementSkip,
    ForeignKeySample,
    FullName,
    Email,
    Phone,
    UuidV4,
    Username,
    City,
    Country,
    Company,
    Date,
    DateTime,
    Boolean,
    IntegerRange(i64, i64),
    DecimalRange(f64, f64),
    EnumPick(Vec<String>),
    LoremSentence,
    Null,
}

impl MockFieldType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AutoIncrementSkip => "Auto-Increment (Skip)",
            Self::ForeignKeySample => "FK Parent Sample",
            Self::FullName => "Full Name",
            Self::Email => "Email",
            Self::Phone => "Phone",
            Self::UuidV4 => "UUID v4",
            Self::Username => "Username",
            Self::City => "City",
            Self::Country => "Country",
            Self::Company => "Company",
            Self::Date => "Date (YYYY-MM-DD)",
            Self::DateTime => "DateTime",
            Self::Boolean => "Boolean (0/1)",
            Self::IntegerRange(_, _) => "Integer Range",
            Self::DecimalRange(_, _) => "Decimal Range",
            Self::EnumPick(_) => "Enum Pick",
            Self::LoremSentence => "Text / Sentence",
            Self::Null => "NULL",
        }
    }

    pub fn preset_variants() -> Vec<Self> {
        vec![
            Self::AutoIncrementSkip,
            Self::ForeignKeySample,
            Self::FullName,
            Self::Email,
            Self::Phone,
            Self::UuidV4,
            Self::Username,
            Self::City,
            Self::Country,
            Self::Company,
            Self::Date,
            Self::DateTime,
            Self::Boolean,
            Self::IntegerRange(1, 1000),
            Self::DecimalRange(1.0, 999.99),
            Self::LoremSentence,
            Self::Null,
        ]
    }
}

pub fn parse_enum_values(column_type: &str) -> Option<Vec<String>> {
    let lower = column_type.trim().to_ascii_lowercase();
    if !lower.starts_with("enum(") && !lower.starts_with("set(") {
        return None;
    }
    let open = column_type.find('(')?;
    let close = column_type.rfind(')')?;
    if close <= open + 1 {
        return None;
    }
    let inner = &column_type[open + 1..close];
    let vals: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if vals.is_empty() { None } else { Some(vals) }
}

pub fn infer_field_type(col: &ColumnInfo, fk_cols: &HashSet<String>) -> MockFieldType {
    if col.extra.to_ascii_lowercase().contains("auto_increment") {
        return MockFieldType::AutoIncrementSkip;
    }
    if fk_cols.contains(&col.name.to_ascii_lowercase()) {
        return MockFieldType::ForeignKeySample;
    }
    if let Some(vals) = parse_enum_values(&col.data_type) {
        return MockFieldType::EnumPick(vals);
    }

    let name = col.name.to_ascii_lowercase();
    let dtype = col.data_type.to_ascii_lowercase();
    let ctype = dtype.as_str();

    if name.contains("email") || name.contains("e_mail") {
        return MockFieldType::Email;
    }
    if name.contains("phone") || name.contains("mobile") || name.contains("tel") {
        return MockFieldType::Phone;
    }
    if name.contains("uuid") || name.contains("guid") {
        return MockFieldType::UuidV4;
    }
    if name.contains("username") || name.contains("user_name") || name == "handle" {
        return MockFieldType::Username;
    }
    if name == "name" || name.contains("full_name") || name.contains("first_name") {
        return MockFieldType::FullName;
    }
    if name.contains("city") {
        return MockFieldType::City;
    }
    if name.contains("country") {
        return MockFieldType::Country;
    }
    if name.contains("company") || name.contains("org") {
        return MockFieldType::Company;
    }
    if ctype == "tinyint(1)" || dtype == "boolean" || dtype == "bool" || name.starts_with("is_") {
        return MockFieldType::Boolean;
    }
    if dtype == "date" || name == "birth_date" || name == "dob" {
        return MockFieldType::Date;
    }
    if dtype == "datetime"
        || dtype == "timestamp"
        || name.ends_with("_at")
        || name.contains("timestamp")
    {
        return MockFieldType::DateTime;
    }
    if matches!(
        dtype.as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint"
    ) {
        if name.contains("age") {
            return MockFieldType::IntegerRange(18, 85);
        }
        if name.contains("qty") || name.contains("quantity") || name.contains("count") {
            return MockFieldType::IntegerRange(1, 50);
        }
        return MockFieldType::IntegerRange(1, 10_000);
    }
    if matches!(dtype.as_str(), "decimal" | "numeric" | "float" | "double") {
        return MockFieldType::DecimalRange(5.0, 999.99);
    }
    if name == "status" {
        return MockFieldType::EnumPick(vec![
            "active".to_string(),
            "pending".to_string(),
            "completed".to_string(),
        ]);
    }
    MockFieldType::LoremSentence
}

#[derive(Debug, Clone)]
pub struct MockGenConfig {
    pub table_name: String,
    pub row_count: usize,
    pub truncate_before: bool,
    pub column_strategies: Vec<(String, MockFieldType)>,
    pub fk_parent_samples: HashMap<String, Vec<String>>,
}

impl MockGenConfig {
    pub fn from_table(table: &TableSchema, row_count: usize) -> Self {
        let fk_cols: HashSet<String> = table
            .foreign_keys
            .iter()
            .map(|fk| fk.column_name.to_ascii_lowercase())
            .collect();
        let column_strategies = table
            .columns
            .iter()
            .map(|c| (c.name.clone(), infer_field_type(c, &fk_cols)))
            .collect();
        Self {
            table_name: table.name.clone(),
            row_count,
            truncate_before: false,
            column_strategies,
            fk_parent_samples: HashMap::new(),
        }
    }
}

struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn pick<'a>(&mut self, slice: &'a [&'a str]) -> &'a str {
        let idx = (self.next_u64() as usize) % slice.len();
        slice[idx]
    }
}

const FIRST_NAMES: &[&str] = &[
    "Alice", "Bob", "Carlos", "Diana", "Elena", "Frank", "Grace", "Hector", "Iris", "Julian",
    "Karen", "Liam", "Maya", "Noah", "Olivia", "Pablo",
];
const LAST_NAMES: &[&str] = &[
    "Garcia", "Smith", "Chen", "Patel", "Kowalski", "Rossi", "Muller", "Sato", "Silva", "Brown",
    "Lopez", "Kim",
];
const CITIES: &[&str] = &[
    "Buenos Aires",
    "Madrid",
    "New York",
    "Tokyo",
    "Berlin",
    "Toronto",
    "London",
    "Sao Paulo",
];
const COUNTRIES: &[&str] = &["AR", "ES", "US", "JP", "DE", "CA", "GB", "BR"];
const COMPANIES: &[&str] = &[
    "Acme Corp",
    "Northwind Labs",
    "Globex Systems",
    "Initech",
    "Stark Data",
    "Wayne Cloud",
];
const WORDS: &[&str] = &[
    "alpha", "beta", "cloud", "delta", "edge", "fast", "grid", "hyper", "index", "kernel",
    "metric", "node", "pulse", "query", "rust", "stream",
];

fn generate_cell_sql(
    col_name: &str,
    strategy: &MockFieldType,
    row_idx: usize,
    rng: &mut Lcg,
    fk_samples: &HashMap<String, Vec<String>>,
) -> String {
    match strategy {
        MockFieldType::AutoIncrementSkip => unreachable!("skipped before cell gen"),
        MockFieldType::Null => "NULL".to_string(),
        MockFieldType::ForeignKeySample => {
            if let Some(samples) = fk_samples.get(col_name)
                && !samples.is_empty()
            {
                let idx = (rng.next_u64() as usize) % samples.len();
                return samples[idx].clone();
            }
            // Fallback when parent table is empty or offline
            ((row_idx % 10) + 1).to_string()
        }
        MockFieldType::FullName => {
            let full = format!("{} {}", rng.pick(FIRST_NAMES), rng.pick(LAST_NAMES));
            sql_str(&full)
        }
        MockFieldType::Email => {
            let first = rng.pick(FIRST_NAMES).to_ascii_lowercase();
            let last = rng.pick(LAST_NAMES).to_ascii_lowercase();
            sql_str(&format!("{first}.{last}.{}@example.com", row_idx + 1))
        }
        MockFieldType::Phone => {
            let num = 1000 + (rng.next_u64() % 9000);
            let num2 = 1000 + (rng.next_u64() % 9000);
            sql_str(&format!("+1-555-{num}-{num2}"))
        }
        MockFieldType::UuidV4 => sql_str(&uuid::Uuid::new_v4().to_string()),
        MockFieldType::Username => {
            let first = rng.pick(FIRST_NAMES).to_ascii_lowercase();
            sql_str(&format!("{first}_{}", 100 + row_idx))
        }
        MockFieldType::City => sql_str(rng.pick(CITIES)),
        MockFieldType::Country => sql_str(rng.pick(COUNTRIES)),
        MockFieldType::Company => sql_str(rng.pick(COMPANIES)),
        MockFieldType::Date => {
            let day = 1 + (rng.next_u64() % 28);
            let month = 1 + (rng.next_u64() % 12);
            let year = 2020 + (rng.next_u64() % 6);
            sql_str(&format!("{year:04}-{month:02}-{day:02}"))
        }
        MockFieldType::DateTime => {
            let day = 1 + (rng.next_u64() % 28);
            let month = 1 + (rng.next_u64() % 12);
            let year = 2023 + (rng.next_u64() % 4);
            let hour = rng.next_u64() % 24;
            let min = rng.next_u64() % 60;
            sql_str(&format!(
                "{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:00"
            ))
        }
        MockFieldType::Boolean => ((rng.next_u64() % 2) as u8).to_string(),
        MockFieldType::IntegerRange(min, max) => {
            let lo = *min.min(max);
            let hi = *min.max(max);
            let span = (hi - lo).max(1) as u64 + 1;
            let val = lo + (rng.next_u64() % span) as i64;
            val.to_string()
        }
        MockFieldType::DecimalRange(min, max) => {
            let lo = min.min(*max);
            let hi = min.max(*max);
            let frac = (rng.next_u64() % 10_000) as f64 / 10_000.0;
            format!("{:.2}", lo + (hi - lo) * frac)
        }
        MockFieldType::EnumPick(vals) => {
            if vals.is_empty() {
                "NULL".to_string()
            } else {
                let idx = (rng.next_u64() as usize) % vals.len();
                sql_str(&vals[idx])
            }
        }
        MockFieldType::LoremSentence => {
            let w1 = rng.pick(WORDS);
            let w2 = rng.pick(WORDS);
            let w3 = rng.pick(WORDS);
            sql_str(&format!("{w1} {w2} {w3} #{}", row_idx + 1))
        }
    }
}

pub fn generate_mock_insert_statements(cfg: &MockGenConfig, batch_size: usize) -> Vec<String> {
    let mut stmts = Vec::new();
    let table_ident = format!("`{}`", cfg.table_name.replace('`', "``"));
    if cfg.truncate_before {
        stmts.push(format!("TRUNCATE TABLE {table_ident};"));
    }

    let active_cols: Vec<&(String, MockFieldType)> = cfg
        .column_strategies
        .iter()
        .filter(|(_, strat)| *strat != MockFieldType::AutoIncrementSkip)
        .collect();

    if active_cols.is_empty() || cfg.row_count == 0 {
        return stmts;
    }

    let col_list = active_cols
        .iter()
        .map(|(name, _)| format!("`{}`", name.replace('`', "``")))
        .collect::<Vec<_>>()
        .join(", ");

    let mut rng = Lcg(0x5EED_CAFE_BABE_1337);
    let batch = batch_size.max(1);
    let mut rows_buf = Vec::with_capacity(batch);

    for row_idx in 0..cfg.row_count {
        let vals = active_cols
            .iter()
            .map(|(name, strat)| {
                generate_cell_sql(name, strat, row_idx, &mut rng, &cfg.fk_parent_samples)
            })
            .collect::<Vec<_>>()
            .join(", ");
        rows_buf.push(format!("({vals})"));

        if rows_buf.len() >= batch || row_idx + 1 == cfg.row_count {
            stmts.push(format!(
                "INSERT INTO {table_ident} ({col_list}) VALUES\n  {};",
                rows_buf.join(",\n  ")
            ));
            rows_buf.clear();
        }
    }
    stmts
}

pub async fn fetch_fk_parent_samples(
    session: &ActiveSession,
    database: &str,
    table: &TableSchema,
) -> Result<HashMap<String, Vec<String>>> {
    let mut out = HashMap::new();
    for fk in &table.foreign_keys {
        let ref_tbl = fk.referenced_table.replace('`', "``");
        let ref_col = fk.referenced_column.replace('`', "``");
        let sql = format!("SELECT DISTINCT `{ref_col}` FROM `{ref_tbl}` LIMIT 200");
        if let Ok(res) = session.execute_query(database, &sql, None, Some(200)).await {
            let samples: Vec<String> = res
                .rows
                .iter()
                .filter_map(|r| r.first())
                .filter(|c| !c.is_null())
                .map(sql_literal)
                .collect();
            if !samples.is_empty() {
                out.insert(fk.column_name.clone(), samples);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_field_types_and_fk_sampling() {
        let schema = crate::demo::build_demo_schema("ecommerce_prod");
        let orders = schema.tables.iter().find(|t| t.name == "orders").unwrap();
        let mut cfg = MockGenConfig::from_table(orders, 6);

        // user_id should be inferred as FK sample if foreign_keys has it
        if !orders.foreign_keys.is_empty() {
            let fk_col = &orders.foreign_keys[0].column_name;
            let strat = cfg
                .column_strategies
                .iter()
                .find(|(n, _)| n == fk_col)
                .map(|(_, s)| s.clone())
                .unwrap();
            assert_eq!(strat, MockFieldType::ForeignKeySample);
            cfg.fk_parent_samples
                .insert(fk_col.clone(), vec!["42".to_string(), "99".to_string()]);
        }

        cfg.truncate_before = true;
        let stmts = generate_mock_insert_statements(&cfg, 4);
        assert_eq!(stmts[0], "TRUNCATE TABLE `orders`;");
        assert_eq!(stmts.len(), 3); // 1 truncate + 2 insert batches (4 + 2)
        if !orders.foreign_keys.is_empty() {
            assert!(stmts[1].contains("42") || stmts[1].contains("99"));
        }
    }

    #[test]
    fn test_parse_enum_values() {
        assert_eq!(
            parse_enum_values("enum('pending','paid','shipped')"),
            Some(vec![
                "pending".to_string(),
                "paid".to_string(),
                "shipped".to_string()
            ])
        );
        assert_eq!(parse_enum_values("varchar(255)"), None);
    }
}
