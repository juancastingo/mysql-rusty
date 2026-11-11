mod ai;
mod app;
mod config;
mod db;
mod export;
mod syntax;

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
        compute_heuristic_ghost_completion, compute_local_completions, extract_sql_code_block,
    };
    use crate::db::{build_demo_schema, detect_editable_table_and_pk};
    use crate::syntax::format_sql_pretty;

    #[test]
    fn test_schema_ddl_and_local_completions() {
        let schema = build_demo_schema("ecommerce_prod");
        let ddl = schema.to_ai_ddl_context();
        assert!(ddl.contains("TABLE `users`"));
        assert!(ddl.contains("FK: `orders`.`user_id` -> `users`.`id`"));

        let (prefix, items) = compute_local_completions("SELECT * FROM us", Some(&schema), 10);
        assert_eq!(prefix, "us");
        assert!(items.iter().any(|i| i.label == "users"));
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
}
