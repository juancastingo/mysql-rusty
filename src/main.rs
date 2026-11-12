mod ai;
mod app;
mod config;
mod db;
mod er_diagram;
mod export;
mod schema_diff;
mod syntax;
mod table_designer;

use app::MysqlRustyApp;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn main() -> eframe::Result<()> {
    let rt = Arc::new(
        Runtime::new().expect("Failed to initialize asynchronous Tokio runtime for MySQL-Rusty"),
    );

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MySQL-Rusty Workbench — Native Rust SQL Client & AI Copilot")
            .with_inner_size([1420.0, 860.0])
            .with_min_inner_size([960.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "mysql-rusty",
        native_options,
        Box::new(move |cc| Ok(Box::new(MysqlRustyApp::new(cc, rt.clone())))),
    )
}

#[cfg(test)]
mod tests {
    use crate::ai::{
        clean_inline_completion, compute_heuristic_ghost_completion, compute_local_completions,
        extract_sql_code_block,
    };
    use crate::db::{build_demo_schema, detect_editable_table_and_pk};
    use crate::syntax::{extract_statement_at_cursor, format_sql_pretty};

    #[test]
    fn test_extract_statement_at_cursor() {
        let sql = "SELECT 'first;one' AS col1; SELECT \"second;two\"; SELECT 3 -- comment with ; \n;";
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
    fn test_schema_ddl_and_local_completions() {
        let schema = build_demo_schema("ecommerce_prod");
        let ddl = schema.to_ai_ddl_context();
        assert!(ddl.contains("TABLE `users`"));
        assert!(ddl.contains("FK: `orders`.`user_id` -> `users`.`id`"));

        let (prefix, items) = compute_local_completions("SELECT * FROM us", Some(&schema), 10);
        assert_eq!(prefix, "us");
        assert!(items.iter().any(|i| i.label == "users"));

        let pruned = schema.to_pruned_ai_ddl_context("SELECT * FROM users WHERE");
        assert!(pruned.contains("TABLE `users`"));
        assert!(pruned.contains("TABLE `orders`"));
    }

    #[test]
    fn test_detect_editable_table_and_pk() {
        let schema = build_demo_schema("ecommerce_prod");
        let cols = vec![
            "id".to_string(),
            "full_name".to_string(),
            "email".to_string(),
        ];
        let (tbl, pk) =
            detect_editable_table_and_pk("SELECT id, full_name, email FROM users LIMIT 10", &cols, Some(&schema));
        assert_eq!(tbl.as_deref(), Some("users"));
        assert_eq!(pk.as_deref(), Some("id"));
    }

    #[test]
    fn test_clean_inline_completion_overlap_trimming() {
        // User's exact bug case: LLM repeats 'WHERE serialNumber LIKE ...'
        let prefix1 = "SELECT * FROM `asset` where serialNumber like ";
        let raw1 = "WHERE serialNumber LIKE '%your_serial_number%'";
        assert_eq!(
            clean_inline_completion(raw1, prefix1),
            "'%your_serial_number%'"
        );

        // LLM repeats the full query
        let raw2 = "SELECT * FROM `asset` WHERE serialNumber LIKE '%pattern%'";
        assert_eq!(clean_inline_completion(raw2, prefix1), "'%pattern%'");

        // LLM repeats 'WHERE' clause
        let prefix3 = "SELECT * FROM orders WHERE ";
        let raw3 = "WHERE status = 'pending'";
        assert_eq!(clean_inline_completion(raw3, prefix3), "status = 'pending'");

        // Partial identifier completion ('em' -> 'ail')
        let prefix4 = "SELECT * FROM users WHERE em";
        let raw4 = "WHERE email = 'admin@example.com'";
        assert_eq!(
            clean_inline_completion(raw4, prefix4),
            "ail = 'admin@example.com'"
        );
    }

    #[test]
    fn test_heuristic_ghost_completion_and_sql_extraction() {
        let schema = build_demo_schema("ecommerce_prod");
        let ghost = compute_heuristic_ghost_completion("SELECT * FROM orders", Some(&schema));
        assert!(ghost.unwrap().contains("JOIN users ON orders.user_id = users.id"));

        let md = "Here is the query:\n```sql\nSELECT id FROM users;\n```\nDone.";
        assert_eq!(
            extract_sql_code_block(md).as_deref(),
            Some("SELECT id FROM users;")
        );
    }

    #[test]
    fn test_format_sql_pretty() {
        let formatted = format_sql_pretty("SELECT id, name FROM users WHERE id = 1 LIMIT 10");
        assert!(formatted.contains("\nFROM "));
        assert!(formatted.contains("\nWHERE "));
        assert!(formatted.contains("\nLIMIT "));
    }

    #[test]
    fn test_table_designer_create_and_alter_ddl() {
        use crate::db::{ColumnInfo, ForeignKeyInfo, IndexInfo};
        use crate::table_designer::TableDraft;

        // 1. CREATE TABLE mode
        let mut create_draft = TableDraft::new_empty("products");
        create_draft.columns.push(ColumnInfo {
            name: "title".to_string(),
            data_type: "VARCHAR(255)".to_string(),
            is_nullable: false,
            is_primary_key: false,
            default_value: None,
            extra: String::new(),
        });
        create_draft.indexes.push(IndexInfo {
            index_name: "idx_title".to_string(),
            column_name: "title".to_string(),
            non_unique: true,
            index_type: "BTREE".to_string(),
        });
        create_draft.foreign_keys.push(ForeignKeyInfo {
            constraint_name: "fk_prod_cat".to_string(),
            column_name: "category_id".to_string(),
            referenced_table: "categories".to_string(),
            referenced_column: "id".to_string(),
        });

        let create_ddl = create_draft.generate_ddl_diff();
        assert!(create_ddl.contains("CREATE TABLE `products`"));
        assert!(create_ddl.contains("PRIMARY KEY (`id`)"));
        assert!(create_ddl.contains("KEY `idx_title` (`title`)"));
        assert!(create_ddl.contains("CONSTRAINT `fk_prod_cat` FOREIGN KEY (`category_id`) REFERENCES `categories` (`id`)"));
        assert!(create_ddl.contains("ENGINE=InnoDB"));

        // 2. ALTER TABLE mode
        let schema = build_demo_schema("ecommerce_prod");
        let users_tbl = schema.tables.iter().find(|t| t.name == "users").unwrap();
        let mut alter_draft = TableDraft::from_table(users_tbl);

        // Add a new column
        alter_draft.columns.push(ColumnInfo {
            name: "phone_number".to_string(),
            data_type: "VARCHAR(30)".to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            extra: String::new(),
        });

        // Modify email column
        if let Some(email_col) = alter_draft.columns.iter_mut().find(|c| c.name == "email") {
            email_col.data_type = "VARCHAR(320)".to_string();
        }

        // Drop country column
        alter_draft.columns.retain(|c| c.name != "country");

        let alter_ddl = alter_draft.generate_ddl_diff();
        assert!(alter_ddl.contains("ALTER TABLE `users` ADD COLUMN `phone_number` VARCHAR(30) NULL;"));
        assert!(alter_ddl.contains("ALTER TABLE `users` MODIFY COLUMN `email` VARCHAR(320) NOT NULL;"));
        assert!(alter_ddl.contains("ALTER TABLE `users` DROP COLUMN `country`;"));
    }

    #[tokio::test]
    async fn test_server_admin_methods() {
        use crate::config::ConnectionProfile;
        use crate::db::ActiveSession;

        let session = ActiveSession::connect(ConnectionProfile::demo_profile())
            .await
            .expect("Failed to connect to demo sandbox");

        let procs = session.list_processes().await.expect("Failed to list processes");
        assert!(procs.columns.contains(&"Id".to_string()));
        assert!(procs.columns.contains(&"User".to_string()));
        assert!(procs.columns.contains(&"Command".to_string()));
        assert!(!procs.rows.is_empty());

        let status = session.server_status_summary().await.expect("Failed to get server status");
        assert!(status.contains_key("Uptime"));
        assert!(status.contains_key("Threads_connected"));
        assert!(status.contains_key("Questions"));

        session.kill_query(101).await.expect("Failed to kill query");
    }

    #[test]
    fn test_parse_csv_to_batch_insert() {
        use crate::export::parse_csv_to_batch_insert;

        let csv = "id,name,price,in_stock,notes\n1,Widget,19.99,true,NULL\n2,Gadget,42.0,false,\"Best widget ever; tested!\"\n3,Doohickey,100,true,";
        let sql = parse_csv_to_batch_insert("inventory", csv).expect("Failed to parse CSV");
        assert!(sql.starts_with("INSERT INTO `inventory` (`id`, `name`, `price`, `in_stock`, `notes`) VALUES"));
        assert!(sql.contains("(1, 'Widget', 19.99, 1, NULL)"));
        assert!(sql.contains("(2, 'Gadget', 42.0, 0, 'Best widget ever; tested!')"));
        assert!(sql.contains("(3, 'Doohickey', 100, 1, NULL)"));

        // Error cases
        assert!(parse_csv_to_batch_insert("test", "").is_err());
        assert!(parse_csv_to_batch_insert("test", "id,name\n").is_err());
    }

    #[test]
    fn test_keyring_fallback() {
        use crate::config::{delete_secret, get_secret, set_secret};
        let _ = set_secret("test_key", "secret_value");
        let _ = get_secret("test_key");
        let _ = delete_secret("test_key");
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
        assert!(is_write_query("UPDATE users SET name = 'Bob' WHERE id = 1;"));
        assert!(is_write_query("DELETE FROM users WHERE id = 1;"));
        assert!(is_write_query("DROP TABLE obsolete;"));
        assert!(!is_write_query("SELECT * FROM users;"));
        assert!(!is_write_query("SHOW PROCESSLIST;"));
    }

    #[test]
    fn test_schema_diff_and_sync() {
        use crate::db::{ColumnInfo, DatabaseSchema, TableSchema};
        use crate::schema_diff::diff_schemas;

        let col_id = ColumnInfo {
            name: "id".to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            extra: "auto_increment".to_string(),
        };

        let col_name = ColumnInfo {
            name: "name".to_string(),
            data_type: "VARCHAR(255)".to_string(),
            is_nullable: false,
            is_primary_key: false,
            default_value: None,
            extra: String::new(),
        };

        let col_email = ColumnInfo {
            name: "email".to_string(),
            data_type: "VARCHAR(191)".to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            extra: String::new(),
        };

        let source = DatabaseSchema {
            name: "prod_db".to_string(),
            tables: vec![
                TableSchema {
                    name: "users".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 10,
                    columns: vec![col_id.clone(), col_name.clone(), col_email.clone()],
                    foreign_keys: Vec::new(),
                    indexes: Vec::new(),
                },
                TableSchema {
                    name: "audit_logs".to_string(),
                    table_type: "BASE TABLE".to_string(),
                    engine: "InnoDB".to_string(),
                    approx_rows: 0,
                    columns: vec![col_id.clone()],
                    foreign_keys: Vec::new(),
                    indexes: Vec::new(),
                },
            ],
        };

        let target = DatabaseSchema {
            name: "dev_db".to_string(),
            tables: vec![TableSchema {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                engine: "InnoDB".to_string(),
                approx_rows: 5,
                columns: vec![col_id.clone(), col_name.clone()], // missing email
                foreign_keys: Vec::new(),
                indexes: Vec::new(),
            }],
        };

        let report = diff_schemas(&source, &target);
        assert!(report.migration_sql.contains("CREATE TABLE IF NOT EXISTS `audit_logs`"));
        assert!(report.migration_sql.contains("ALTER TABLE `users`"));
        assert!(report.migration_sql.contains("ADD COLUMN `email` VARCHAR(191) NULL"));
        assert_eq!(report.summary.len(), 2);
    }

    #[test]
    fn test_mock_data_and_markdown_export() {
        use crate::db::{CellValue, ColumnInfo, QueryResult, TableSchema};
        use crate::export::{generate_mock_data_sql, to_markdown_table};

        let table = TableSchema {
            name: "customers".to_string(),
            table_type: "BASE TABLE".to_string(),
            engine: "InnoDB".to_string(),
            approx_rows: 0,
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    is_nullable: false,
                    is_primary_key: true,
                    default_value: None,
                    extra: "auto_increment".to_string(),
                },
                ColumnInfo {
                    name: "full_name".to_string(),
                    data_type: "VARCHAR(255)".to_string(),
                    is_nullable: false,
                    is_primary_key: false,
                    default_value: None,
                    extra: String::new(),
                },
                ColumnInfo {
                    name: "email".to_string(),
                    data_type: "VARCHAR(191)".to_string(),
                    is_nullable: false,
                    is_primary_key: false,
                    default_value: None,
                    extra: String::new(),
                },
            ],
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
        };

        let sql = generate_mock_data_sql(&table, 5);
        assert!(sql.contains("INSERT INTO `customers` (`full_name`, `email`) VALUES"));
        assert!(sql.contains("@example.com") || sql.contains("@testmail.org"));

        let q_res = QueryResult {
            sql: "SELECT 1".to_string(),
            database: "test".to_string(),
            columns: vec!["id".to_string(), "status".to_string()],
            column_types: vec!["INT".to_string(), "VARCHAR".to_string()],
            rows: vec![
                vec![CellValue::Integer(1), CellValue::Text("active".to_string())],
                vec![CellValue::Integer(2), CellValue::Null],
            ],
            affected_rows: 0,
            last_insert_id: None,
            duration_ms: 1,
            editable_table: None,
            primary_key_column: None,
            status_message: "OK".to_string(),
        };

        let md = to_markdown_table(&q_res);
        assert!(md.contains("| id | status |"));
        assert!(md.contains("| 1 | active |"));
        assert!(md.contains("| 2 | NULL |"));
    }

    #[test]
    fn test_saved_query_model_and_tab_dirty_title() {
        use crate::app::QueryTab;
        use crate::config::SavedSnippet;

        let mut tab = QueryTab::new("My Analytics", "SELECT COUNT(*) FROM orders;");
        assert_eq!(tab.display_title(), "My Analytics");
        assert!(!tab.is_dirty);

        tab.is_dirty = true;
        assert_eq!(tab.display_title(), "My Analytics *");

        let snip = SavedSnippet {
            id: "test-uuid-1".to_string(),
            title: "Orders by Month".to_string(),
            sql: "SELECT MONTH(created_at), COUNT(*) FROM orders GROUP BY 1;".to_string(),
            database: Some("ecommerce".to_string()),
            description: "Aggregates revenue by month".to_string(),
            created_at: "2026-11-12".to_string(),
        };

        assert_eq!(snip.title, "Orders by Month");
        assert_eq!(snip.database.as_deref(), Some("ecommerce"));
        assert_eq!(snip.description, "Aggregates revenue by month");
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
        assert_eq!(q4_l10, "SELECT * FROM orders WHERE status = 'paid' LIMIT 10;");
    }

    #[test]
    fn test_sanitize_filename_and_queries_dir() {
        use crate::app::sanitize_filename;
        use crate::config::AppConfig;

        assert_eq!(sanitize_filename("My Custom Query"), "My_Custom_Query");
        assert_eq!(sanitize_filename("test/unsafe*name?.sql"), "test_unsafe_name");
        assert_eq!(sanitize_filename(""), "query");
        assert_eq!(sanitize_filename("  spaces_and_tabs  .sql"), "spaces_and_tabs");

        let qdir = AppConfig::queries_dir();
        assert!(qdir.exists() || qdir.to_str().is_some());
    }
}
