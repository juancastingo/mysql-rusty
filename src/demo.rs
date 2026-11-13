use crate::db::{
    CellValue, ColumnInfo, DatabaseSchema, ForeignKeyInfo, IndexInfo, QueryResult, TableSchema,
};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DemoStore {
    pub tables: Arc<Mutex<HashMap<String, QueryResult>>>,
}

impl Default for DemoStore {
    fn default() -> Self {
        let mut map = HashMap::new();
        map.insert("users".to_string(), demo_users_table());
        map.insert("products".to_string(), demo_products_table());
        map.insert("orders".to_string(), demo_orders_table());
        map.insert("order_items".to_string(), demo_order_items_table());
        map.insert("payments".to_string(), demo_payments_table());
        map.insert("events_daily".to_string(), demo_events_daily_table());
        map.insert(
            "pageviews_summary".to_string(),
            demo_pageviews_summary_table(),
        );
        map.insert("tables".to_string(), demo_tables_table());
        map.insert("columns".to_string(), demo_columns_table());
        Self {
            tables: Arc::new(Mutex::new(map)),
        }
    }
}

// ============================================================================
// Built-in Sandbox / Demo Engine (`ecommerce_prod` & `analytics_dw`)
// ============================================================================

pub fn build_demo_schema(database: &str) -> DatabaseSchema {
    if database == "analytics_dw" {
        return DatabaseSchema {
            name: "analytics_dw".to_string(),
            tables: vec![
                TableSchema {
                    name: "events_daily".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 14500,
                    columns: vec![
                        ColumnInfo {
                            name: "event_date".to_string(),
                            data_type: "date".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "channel".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "active_users".to_string(),
                            data_type: "int unsigned".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0".to_string()),
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "conversion_rate".to_string(),
                            data_type: "decimal(5,2)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0.00".to_string()),
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
                TableSchema {
                    name: "pageviews_summary".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 82000,
                    columns: vec![
                        ColumnInfo {
                            name: "path".to_string(),
                            data_type: "varchar(255)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "views".to_string(),
                            data_type: "bigint unsigned".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0".to_string()),
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "avg_time_sec".to_string(),
                            data_type: "decimal(6,2)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: Some("0.00".to_string()),
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
            ],
        };
    }

    if database == "information_schema" {
        return DatabaseSchema {
            name: "information_schema".to_string(),
            tables: vec![
                TableSchema {
                    name: "TABLES".to_string(),
                    table_type: "SYSTEM VIEW".to_string(),
                    engine: "MEMORY".to_string(),
                    approx_rows: 120,
                    columns: vec![
                        ColumnInfo {
                            name: "TABLE_SCHEMA".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_TYPE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "ENGINE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: true,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
                TableSchema {
                    name: "COLUMNS".to_string(),
                    table_type: "SYSTEM VIEW".to_string(),
                    engine: "MEMORY".to_string(),
                    approx_rows: 950,
                    columns: vec![
                        ColumnInfo {
                            name: "TABLE_SCHEMA".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "TABLE_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "COLUMN_NAME".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: true,
                            default_value: None,
                            extra: String::new(),
                        },
                        ColumnInfo {
                            name: "DATA_TYPE".to_string(),
                            data_type: "varchar(64)".to_string(),
                            is_nullable: false,
                            is_primary_key: false,
                            default_value: None,
                            extra: String::new(),
                        },
                    ],
                    foreign_keys: vec![],
                    indexes: vec![],
                },
            ],
        };
    }

    DatabaseSchema {
        name: "ecommerce_prod".to_string(),
        tables: vec![
            TableSchema {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 8,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "full_name".to_string(),
                        data_type: "varchar(120)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "email".to_string(),
                        data_type: "varchar(180)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "country".to_string(),
                        data_type: "char(2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("US".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "plan_tier".to_string(),
                        data_type: "enum('free','pro','enterprise')".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("free".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "created_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("CURRENT_TIMESTAMP".to_string()),
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![],
                indexes: vec![
                    IndexInfo {
                        index_name: "PRIMARY".to_string(),
                        column_name: "id".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                    IndexInfo {
                        index_name: "idx_users_email".to_string(),
                        column_name: "email".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                ],
            },
            TableSchema {
                name: "products".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 6,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "sku".to_string(),
                        data_type: "varchar(40)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar(160)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "category".to_string(),
                        data_type: "varchar(60)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "unit_price".to_string(),
                        data_type: "decimal(10,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "stock".to_string(),
                        data_type: "int".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("0".to_string()),
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![],
                indexes: vec![IndexInfo {
                    index_name: "PRIMARY".to_string(),
                    column_name: "id".to_string(),
                    non_unique: false,
                    index_type: "BTREE".to_string(),
                }],
            },
            TableSchema {
                name: "orders".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 7,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "user_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "status".to_string(),
                        data_type: "enum('pending','paid','shipped','cancelled')".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("pending".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "total_amount".to_string(),
                        data_type: "decimal(12,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "created_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![ForeignKeyInfo {
                    constraint_name: "fk_orders_user_id".to_string(),
                    column_name: "user_id".to_string(),
                    referenced_table: "users".to_string(),
                    referenced_column: "id".to_string(),
                }],
                indexes: vec![
                    IndexInfo {
                        index_name: "PRIMARY".to_string(),
                        column_name: "id".to_string(),
                        non_unique: false,
                        index_type: "BTREE".to_string(),
                    },
                    IndexInfo {
                        index_name: "idx_orders_user_status".to_string(),
                        column_name: "user_id".to_string(),
                        non_unique: true,
                        index_type: "BTREE".to_string(),
                    },
                ],
            },
            TableSchema {
                name: "order_items".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 8,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "order_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "product_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "quantity".to_string(),
                        data_type: "int unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: Some("1".to_string()),
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "unit_price".to_string(),
                        data_type: "decimal(10,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![
                    ForeignKeyInfo {
                        constraint_name: "fk_items_order_id".to_string(),
                        column_name: "order_id".to_string(),
                        referenced_table: "orders".to_string(),
                        referenced_column: "id".to_string(),
                    },
                    ForeignKeyInfo {
                        constraint_name: "fk_items_product_id".to_string(),
                        column_name: "product_id".to_string(),
                        referenced_table: "products".to_string(),
                        referenced_column: "id".to_string(),
                    },
                ],
                indexes: vec![IndexInfo {
                    index_name: "PRIMARY".to_string(),
                    column_name: "id".to_string(),
                    non_unique: false,
                    index_type: "BTREE".to_string(),
                }],
            },
            TableSchema {
                name: "payments".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 5,
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        extra: "auto_increment".to_string(),
                    },
                    ColumnInfo {
                        name: "order_id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "provider".to_string(),
                        data_type: "varchar(40)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "amount".to_string(),
                        data_type: "decimal(12,2)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                    ColumnInfo {
                        name: "paid_at".to_string(),
                        data_type: "timestamp".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    },
                ],
                foreign_keys: vec![ForeignKeyInfo {
                    constraint_name: "fk_payments_order_id".to_string(),
                    column_name: "order_id".to_string(),
                    referenced_table: "orders".to_string(),
                    referenced_column: "id".to_string(),
                }],
                indexes: vec![],
            },
        ],
    }
}

fn demo_users_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(1),
            CellValue::Text("Elena Rostova".into()),
            CellValue::Text("elena.rostova@acme.io".into()),
            CellValue::Text("ES".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2025-09-12 10:14:22".into()),
        ],
        vec![
            CellValue::Integer(2),
            CellValue::Text("Mateo Valdes".into()),
            CellValue::Text("mateo@valdes.dev".into()),
            CellValue::Text("AR".into()),
            CellValue::Text("pro".into()),
            CellValue::Text("2025-10-03 14:30:05".into()),
        ],
        vec![
            CellValue::Integer(3),
            CellValue::Text("Sofia Lindqvist".into()),
            CellValue::Text("sofia@nordiccloud.se".into()),
            CellValue::Text("SE".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2025-11-19 08:45:10".into()),
        ],
        vec![
            CellValue::Integer(4),
            CellValue::Text("Lucas Mendoza".into()),
            CellValue::Text("lucas.mendoza@fintech.mx".into()),
            CellValue::Text("MX".into()),
            CellValue::Text("pro".into()),
            CellValue::Text("2026-01-08 19:12:49".into()),
        ],
        vec![
            CellValue::Integer(5),
            CellValue::Text("Valentina Rios".into()),
            CellValue::Text("vrios@biotech.cl".into()),
            CellValue::Text("CL".into()),
            CellValue::Text("free".into()),
            CellValue::Text("2026-02-14 11:02:33".into()),
        ],
        vec![
            CellValue::Integer(6),
            CellValue::Text("David Chen".into()),
            CellValue::Text("dchen@hyperion.ai".into()),
            CellValue::Text("US".into()),
            CellValue::Text("enterprise".into()),
            CellValue::Text("2026-03-01 16:50:00".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM users".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "full_name".into(),
            "email".into(),
            "country".into(),
            "plan_tier".into(),
            "created_at".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 2,
        editable_table: Some("users".into()),
        primary_key_column: Some("id".into()),
        status_message: "6 rows returned in 2 ms (Sandbox)".into(),
    }
}

fn demo_products_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(101),
            CellValue::Text("NVME-4TB-PRO".into()),
            CellValue::Text("Enterprise NVMe PCIe 5.0 4TB SSD".into()),
            CellValue::Text("Storage".into()),
            CellValue::Float(429.90),
            CellValue::Integer(48),
        ],
        vec![
            CellValue::Integer(102),
            CellValue::Text("GPU-MI300X".into()),
            CellValue::Text("AI Accelerator 192GB HBM3".into()),
            CellValue::Text("Compute".into()),
            CellValue::Float(14500.00),
            CellValue::Integer(6),
        ],
        vec![
            CellValue::Integer(103),
            CellValue::Text("RAM-DDR5-128G".into()),
            CellValue::Text("ECC DDR5 128GB 6400MHz Kit".into()),
            CellValue::Text("Memory".into()),
            CellValue::Float(580.00),
            CellValue::Integer(92),
        ],
        vec![
            CellValue::Integer(104),
            CellValue::Text("SW-100G-QSFP".into()),
            CellValue::Text("Top-of-Rack Switch 32x100GbE".into()),
            CellValue::Text("Networking".into()),
            CellValue::Float(3190.00),
            CellValue::Integer(14),
        ],
        vec![
            CellValue::Integer(105),
            CellValue::Text("UPS-3000VA-LI".into()),
            CellValue::Text("2U Rack Lithium Online UPS 3000VA".into()),
            CellValue::Text("Power".into()),
            CellValue::Float(1290.50),
            CellValue::Integer(21),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM products".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "sku".into(),
            "name".into(),
            "category".into(),
            "unit_price".into(),
            "stock".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("products".into()),
        primary_key_column: Some("id".into()),
        status_message: "5 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_orders_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(5001),
            CellValue::Integer(1),
            CellValue::Text("paid".into()),
            CellValue::Float(3619.80),
            CellValue::Text("2026-10-15 09:20:11".into()),
        ],
        vec![
            CellValue::Integer(5002),
            CellValue::Integer(2),
            CellValue::Text("paid".into()),
            CellValue::Float(1009.90),
            CellValue::Text("2026-10-18 17:44:02".into()),
        ],
        vec![
            CellValue::Integer(5003),
            CellValue::Integer(3),
            CellValue::Text("shipped".into()),
            CellValue::Float(14500.00),
            CellValue::Text("2026-10-21 12:05:39".into()),
        ],
        vec![
            CellValue::Integer(5004),
            CellValue::Integer(1),
            CellValue::Text("paid".into()),
            CellValue::Float(1290.50),
            CellValue::Text("2026-11-01 15:11:28".into()),
        ],
        vec![
            CellValue::Integer(5005),
            CellValue::Integer(4),
            CellValue::Text("pending".into()),
            CellValue::Float(580.00),
            CellValue::Text("2026-11-08 20:31:00".into()),
        ],
        vec![
            CellValue::Integer(5006),
            CellValue::Integer(6),
            CellValue::Text("paid".into()),
            CellValue::Float(17690.00),
            CellValue::Text("2026-11-09 11:18:45".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM orders".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "user_id".into(),
            "status".into(),
            "total_amount".into(),
            "created_at".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 2,
        editable_table: Some("orders".into()),
        primary_key_column: Some("id".into()),
        status_message: "6 rows returned in 2 ms (Sandbox)".into(),
    }
}

fn demo_order_items_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(1),
            CellValue::Integer(5001),
            CellValue::Integer(104),
            CellValue::Integer(1),
            CellValue::Float(3190.00),
        ],
        vec![
            CellValue::Integer(2),
            CellValue::Integer(5001),
            CellValue::Integer(101),
            CellValue::Integer(1),
            CellValue::Float(429.80),
        ],
        vec![
            CellValue::Integer(3),
            CellValue::Integer(5002),
            CellValue::Integer(103),
            CellValue::Integer(1),
            CellValue::Float(580.00),
        ],
        vec![
            CellValue::Integer(4),
            CellValue::Integer(5003),
            CellValue::Integer(102),
            CellValue::Integer(1),
            CellValue::Float(14500.00),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM order_items".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "order_id".into(),
            "product_id".into(),
            "quantity".into(),
            "unit_price".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("order_items".into()),
        primary_key_column: Some("id".into()),
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_payments_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Integer(901),
            CellValue::Integer(5001),
            CellValue::Text("stripe".into()),
            CellValue::Float(3619.80),
            CellValue::Text("2026-10-15 09:20:45".into()),
        ],
        vec![
            CellValue::Integer(902),
            CellValue::Integer(5002),
            CellValue::Text("adyen".into()),
            CellValue::Float(1009.90),
            CellValue::Text("2026-10-18 17:44:22".into()),
        ],
        vec![
            CellValue::Integer(903),
            CellValue::Integer(5006),
            CellValue::Text("wire_transfer".into()),
            CellValue::Float(17690.00),
            CellValue::Text("2026-11-09 11:19:02".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM payments".into(),
        database: "ecommerce_prod".into(),
        columns: vec![
            "id".into(),
            "order_id".into(),
            "provider".into(),
            "amount".into(),
            "paid_at".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("payments".into()),
        primary_key_column: Some("id".into()),
        status_message: "3 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_events_daily_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("organic_search".into()),
            CellValue::Integer(4820),
            CellValue::Float(4.25),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("paid_ads".into()),
            CellValue::Integer(3110),
            CellValue::Float(6.80),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("social_media".into()),
            CellValue::Integer(2450),
            CellValue::Float(2.90),
        ],
        vec![
            CellValue::Text("2026-11-10".into()),
            CellValue::Text("direct_referral".into()),
            CellValue::Integer(1920),
            CellValue::Float(5.10),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM events_daily".into(),
        database: "analytics_dw".into(),
        columns: vec![
            "event_date".into(),
            "channel".into(),
            "active_users".into(),
            "conversion_rate".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("events_daily".into()),
        primary_key_column: Some("event_date".into()),
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_pageviews_summary_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("/".into()),
            CellValue::Integer(42000),
            CellValue::Float(45.2),
        ],
        vec![
            CellValue::Text("/pricing".into()),
            CellValue::Integer(18500),
            CellValue::Float(78.5),
        ],
        vec![
            CellValue::Text("/docs/quickstart".into()),
            CellValue::Integer(12300),
            CellValue::Float(195.0),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM pageviews_summary".into(),
        database: "analytics_dw".into(),
        columns: vec!["path".into(), "views".into(), "avg_time_sec".into()],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: Some("pageviews_summary".into()),
        primary_key_column: Some("path".into()),
        status_message: "3 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_tables_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("users".into()),
            CellValue::Text("BASE TABLE".into()),
            CellValue::Text("InnoDB".into()),
        ],
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("products".into()),
            CellValue::Text("BASE TABLE".into()),
            CellValue::Text("InnoDB".into()),
        ],
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("orders".into()),
            CellValue::Text("BASE TABLE".into()),
            CellValue::Text("InnoDB".into()),
        ],
        vec![
            CellValue::Text("analytics_dw".into()),
            CellValue::Text("events_daily".into()),
            CellValue::Text("BASE TABLE".into()),
            CellValue::Text("InnoDB".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM information_schema.TABLES".into(),
        database: "information_schema".into(),
        columns: vec![
            "TABLE_SCHEMA".into(),
            "TABLE_NAME".into(),
            "TABLE_TYPE".into(),
            "ENGINE".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: None,
        primary_key_column: None,
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

fn demo_columns_table() -> QueryResult {
    let rows = vec![
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("users".into()),
            CellValue::Text("id".into()),
            CellValue::Text("bigint unsigned".into()),
        ],
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("users".into()),
            CellValue::Text("full_name".into()),
            CellValue::Text("varchar(100)".into()),
        ],
        vec![
            CellValue::Text("ecommerce_prod".into()),
            CellValue::Text("users".into()),
            CellValue::Text("email".into()),
            CellValue::Text("varchar(150)".into()),
        ],
        vec![
            CellValue::Text("analytics_dw".into()),
            CellValue::Text("events_daily".into()),
            CellValue::Text("event_date".into()),
            CellValue::Text("date".into()),
        ],
    ];
    QueryResult {
        sql: "SELECT * FROM information_schema.COLUMNS".into(),
        database: "information_schema".into(),
        columns: vec![
            "TABLE_SCHEMA".into(),
            "TABLE_NAME".into(),
            "COLUMN_NAME".into(),
            "DATA_TYPE".into(),
        ],
        rows,
        affected_rows: 0,
        last_insert_id: None,
        duration_ms: 1,
        editable_table: None,
        primary_key_column: None,
        status_message: "4 rows returned in 1 ms (Sandbox)".into(),
    }
}

pub(crate) fn execute_demo_query(
    store: &DemoStore,
    database: &str,
    sql: &str,
    _schema: Option<&DatabaseSchema>,
) -> Result<QueryResult> {
    let lower = sql.to_lowercase();
    let guard = store.tables.lock().unwrap();

    if lower.starts_with("show databases") || lower.starts_with("show schemas") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["Database".into()],
            rows: vec![
                vec![CellValue::Text("ecommerce_prod".into())],
                vec![CellValue::Text("analytics_dw".into())],
                vec![CellValue::Text("information_schema".into())],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "3 databases discovered in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("show tables") {
        let col = format!("Tables_in_{database}");
        let tables_list = match database {
            "analytics_dw" => vec!["events_daily", "pageviews_summary"],
            "information_schema" => vec!["COLUMNS", "TABLES"],
            _ => vec!["users", "products", "orders", "order_items", "payments"],
        };
        let rows = tables_list
            .into_iter()
            .map(|t| vec![CellValue::Text(t.into())])
            .collect();
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![col],
            rows,
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "Tables listed in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("use ") {
        let target = sql[4..]
            .trim()
            .trim_matches(|c| c == ';' || c == '`' || c == ' ');
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: target.to_string(),
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!("Database changed to `{target}` (Sandbox)"),
        });
    }

    if lower.starts_with("select database()") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["DATABASE()".into()],
            rows: vec![vec![CellValue::Text(database.to_string())]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "1 row in set (Sandbox)".into(),
        });
    }

    if lower.starts_with("explain") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![
                "id".into(),
                "select_type".into(),
                "table".into(),
                "type".into(),
                "possible_keys".into(),
                "key".into(),
                "rows".into(),
                "Extra".into(),
            ],
            rows: vec![
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("SIMPLE".into()),
                    CellValue::Text("u".into()),
                    CellValue::Text("ALL".into()),
                    CellValue::Text("PRIMARY".into()),
                    CellValue::Null,
                    CellValue::Integer(6),
                    CellValue::Text("Using temporary; Using filesort".into()),
                ],
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("SIMPLE".into()),
                    CellValue::Text("o".into()),
                    CellValue::Text("ref".into()),
                    CellValue::Text("idx_orders_user_status".into()),
                    CellValue::Text("idx_orders_user_status".into()),
                    CellValue::Integer(2),
                    CellValue::Text("Using where".into()),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "EXPLAIN execution plan generated in 1 ms (Sandbox)".into(),
        });
    }

    if lower.starts_with("show create table") {
        let tbl = sql
            .split_whitespace()
            .last()
            .unwrap_or("users")
            .trim_matches(|c| c == '`' || c == ';');
        let ddl = format!(
            "CREATE TABLE `{tbl}` (\n  `id` bigint unsigned NOT NULL AUTO_INCREMENT,\n  `created_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;"
        );
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec!["Table".into(), "Create Table".into()],
            rows: vec![vec![CellValue::Text(tbl.into()), CellValue::Text(ddl)]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "DDL fetched in 1 ms (Sandbox)".into(),
        });
    }

    // Check for scalar / function expressions like SELECT 1, SELECT NOW(), SELECT VERSION()
    if lower.starts_with("select") && !lower.contains("from") {
        let expr = sql.trim()[6..].trim().trim_end_matches(';');
        let (col_name, val) = match expr.to_lowercase().as_str() {
            "version()" | "@@version" => (
                "VERSION()",
                CellValue::Text("8.0.36-MySQL-Rusty-Sandbox".into()),
            ),
            "now()" | "current_timestamp()" => (
                "NOW()",
                CellValue::Text(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            ),
            "user()" | "current_user()" => {
                ("USER()", CellValue::Text("demo_admin@localhost".into()))
            }
            "connection_id()" => ("CONNECTION_ID()", CellValue::Integer(1)),
            _ => (expr, CellValue::Text(expr.to_string())),
        };
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![col_name.to_string()],
            rows: vec![vec![val]],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!(
                "1 row in set at {} (Sandbox)",
                chrono::Local::now().format("%H:%M:%S")
            ),
        });
    }

    if lower.contains("join") && lower.contains("users") && lower.contains("orders") {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![
                "id".into(),
                "full_name".into(),
                "email".into(),
                "country".into(),
                "total_orders".into(),
                "lifetime_value".into(),
            ],
            rows: vec![
                vec![
                    CellValue::Integer(6),
                    CellValue::Text("David Chen".into()),
                    CellValue::Text("dchen@hyperion.ai".into()),
                    CellValue::Text("US".into()),
                    CellValue::Integer(1),
                    CellValue::Float(17690.00),
                ],
                vec![
                    CellValue::Integer(1),
                    CellValue::Text("Elena Rostova".into()),
                    CellValue::Text("elena.rostova@acme.io".into()),
                    CellValue::Text("ES".into()),
                    CellValue::Integer(2),
                    CellValue::Float(4910.30),
                ],
                vec![
                    CellValue::Integer(2),
                    CellValue::Text("Mateo Valdes".into()),
                    CellValue::Text("mateo@valdes.dev".into()),
                    CellValue::Text("AR".into()),
                    CellValue::Integer(1),
                    CellValue::Float(1009.90),
                ],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 3,
            editable_table: None,
            primary_key_column: None,
            status_message: format!(
                "3 aggregated rows in 3 ms at {} (Sandbox)",
                chrono::Local::now().format("%H:%M:%S")
            ),
        });
    }

    // Direct table matching ordered from longest to shortest name
    let known_tables = [
        "pageviews_summary",
        "events_daily",
        "order_items",
        "payments",
        "products",
        "orders",
        "users",
        "tables",
        "columns",
    ];

    // Check for COUNT(*) queries
    if lower.contains("count(") && !lower.contains("group by") {
        for &tname in &known_tables {
            if lower.contains(tname)
                && let Some(res) = guard.get(tname)
            {
                let total = res.rows.len();
                return Ok(QueryResult {
                    sql: sql.to_string(),
                    database: database.to_string(),
                    columns: vec!["COUNT(*)".to_string()],
                    rows: vec![vec![CellValue::Integer(total as i64)]],
                    affected_rows: 0,
                    last_insert_id: None,
                    duration_ms: 1,
                    editable_table: None,
                    primary_key_column: None,
                    status_message: format!(
                        "1 row in set at {} (Sandbox)",
                        chrono::Local::now().format("%H:%M:%S")
                    ),
                });
            }
        }
    }

    // Match known table
    for &tname in &known_tables {
        if lower.contains(tname)
            && let Some(res) = guard.get(tname)
        {
            let mut cloned = res.clone();
            cloned.sql = sql.to_string();

            // Simple WHERE filter in sandbox demo for basic interactivity
            if lower.contains("where")
                && let Some(where_part) = lower.split("where").nth(1)
            {
                let where_clean = where_part.split("order by").next().unwrap_or(where_part);
                let where_clean = where_clean.split("limit").next().unwrap_or(where_clean);

                if (where_clean.contains("id =") || where_clean.contains("id="))
                    && let Some(id_str) = where_clean.split('=').nth(1)
                    && let Ok(id_val) = id_str
                        .trim()
                        .trim_matches(|c| c == ';' || c == ' ')
                        .parse::<i64>()
                {
                    cloned.rows.retain(|r| {
                        if let Some(CellValue::Integer(i)) = r.first() {
                            *i == id_val
                        } else {
                            true
                        }
                    });
                }
            }

            // Simple LIMIT in sandbox demo
            if lower.contains("limit")
                && let Some(limit_part) = lower.split("limit").nth(1)
                && let Some(num_str) = limit_part.split_whitespace().next()
                && let Ok(limit_n) = num_str.trim_matches(';').parse::<usize>()
            {
                cloned.rows.truncate(limit_n);
            }

            cloned.status_message = format!(
                "{} row(s) returned in 1 ms at {} (Sandbox)",
                cloned.rows.len(),
                chrono::Local::now().format("%H:%M:%S")
            );
            return Ok(cloned);
        }
    }

    // Check for DDL / modification statements
    if lower.starts_with("insert into")
        || lower.starts_with("update ")
        || lower.starts_with("delete from")
    {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![],
            rows: vec![],
            affected_rows: 1,
            last_insert_id: Some(101),
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: format!(
                "1 row affected at {} (Sandbox)",
                chrono::Local::now().format("%H:%M:%S")
            ),
        });
    }

    if lower.starts_with("create table")
        || lower.starts_with("alter table")
        || lower.starts_with("drop table")
    {
        return Ok(QueryResult {
            sql: sql.to_string(),
            database: database.to_string(),
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 2,
            editable_table: None,
            primary_key_column: None,
            status_message: format!(
                "Query OK, 0 rows affected at {} (Sandbox)",
                chrono::Local::now().format("%H:%M:%S")
            ),
        });
    }

    Err(anyhow!(
        "Table '{database}.{}' doesn't exist (or syntax not recognized by Sandbox). Available tables in `{database}`: {}",
        sql.split_whitespace()
            .last()
            .unwrap_or("unknown")
            .trim_matches(';'),
        match database {
            "analytics_dw" => "events_daily, pageviews_summary",
            "information_schema" => "TABLES, COLUMNS",
            _ => "users, products, orders, order_items, payments",
        }
    ))
}
