mod chart;
mod grid;
mod multi;
mod stats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultViewMode {
    Grid,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiResultLayout {
    #[default]
    Tabs,
    Stacked,
    SideBySide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Bar,
    Line,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualFilterOp {
    Contains,
    Equals,
    NotEquals,
    StartsWith,
    GreaterThan,
    LessThan,
    IsNull,
    IsNotNull,
}

impl VisualFilterOp {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Equals => "= (exact)",
            Self::NotEquals => "!= (not equal)",
            Self::StartsWith => "starts with",
            Self::GreaterThan => "> (greater)",
            Self::LessThan => "< (less)",
            Self::IsNull => "IS NULL / empty",
            Self::IsNotNull => "IS NOT NULL",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CellInspectorState {
    pub table_name: Option<String>,
    pub column_name: String,
    pub row_index: usize,
    pub raw_value: String,
    pub formatted_json: Option<String>,
}
