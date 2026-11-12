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
}
