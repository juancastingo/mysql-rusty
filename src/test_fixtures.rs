use crate::db::{CellValue, ColumnInfo, ForeignKeyInfo, IndexInfo, QueryResult, TableSchema};

pub fn users_table() -> TableSchema {
    TableSchema {
        name: "users".to_string(),
        table_type: "BASE TABLE".to_string(),
        engine: "InnoDB".to_string(),
        approx_rows: 100,
        columns: vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "int".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                extra: "auto_increment".to_string(),
            },
            ColumnInfo {
                name: "email".to_string(),
                data_type: "varchar(255)".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: None,
                extra: String::new(),
            },
            ColumnInfo {
                name: "name".to_string(),
                data_type: "varchar(100)".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                extra: String::new(),
            },
            ColumnInfo {
                name: "created_at".to_string(),
                data_type: "datetime".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: Some("CURRENT_TIMESTAMP".to_string()),
                extra: String::new(),
            },
        ],
        foreign_keys: vec![],
        indexes: vec![IndexInfo {
            index_name: "idx_users_email".to_string(),
            column_name: "email".to_string(),
            non_unique: false,
            index_type: "BTREE".to_string(),
        }],
    }
}

pub fn orders_table() -> TableSchema {
    TableSchema {
        name: "orders".to_string(),
        table_type: "BASE TABLE".to_string(),
        engine: "InnoDB".to_string(),
        approx_rows: 250,
        columns: vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "int".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                extra: "auto_increment".to_string(),
            },
            ColumnInfo {
                name: "user_id".to_string(),
                data_type: "int".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: None,
                extra: String::new(),
            },
            ColumnInfo {
                name: "total".to_string(),
                data_type: "decimal(10,2)".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: Some("0.00".to_string()),
                extra: String::new(),
            },
        ],
        foreign_keys: vec![ForeignKeyInfo {
            constraint_name: "fk_orders_user_id".to_string(),
            column_name: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
        }],
        indexes: vec![IndexInfo {
            index_name: "idx_orders_user_id".to_string(),
            column_name: "user_id".to_string(),
            non_unique: true,
            index_type: "BTREE".to_string(),
        }],
    }
}

pub fn sample_result() -> QueryResult {
    QueryResult {
        sql: "SELECT id, name, bio, score FROM users".to_string(),
        database: "app_db".to_string(),
        columns: vec![
            "id".to_string(),
            "name".to_string(),
            "bio".to_string(),
            "score".to_string(),
        ],
        rows: vec![
            vec![
                CellValue::Integer(1),
                CellValue::Text("O'Brien \"Ace\"".to_string()),
                CellValue::Text("Line1\nLine2, with comma".to_string()),
                CellValue::Float(98.5),
            ],
            vec![
                CellValue::Integer(2),
                CellValue::Text("Ñandú 🦀".to_string()),
                CellValue::Null,
                CellValue::Float(42.0),
            ],
            vec![
                CellValue::Integer(3),
                CellValue::Text("Bob".to_string()),
                CellValue::Text("Simple".to_string()),
                CellValue::Null,
            ],
        ],
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 5,
        editable_table: Some("users".to_string()),
        primary_key_column: Some("id".to_string()),
        status_message: "3 rows returned in 5 ms".to_string(),
    }
}
