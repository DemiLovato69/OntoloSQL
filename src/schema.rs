#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSchema {
    pub tables: Vec<Table>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub primary_key: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub constraint_name: Option<String>,
    pub source_table: String,
    pub source_columns: Vec<String>,
    pub target_table: String,
    pub target_columns: Vec<String>,
}
