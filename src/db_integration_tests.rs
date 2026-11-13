use crate::config::{ConnectionProfile, SslMode};
use crate::db::{ActiveSession, CellValue, PendingCellEdit};
use std::time::Instant;
use uuid::Uuid;

fn test_profile() -> Option<ConnectionProfile> {
    let host = std::env::var("MYSQL_RUSTY_IT_HOST").ok()?;
    if host.trim().is_empty() {
        return None;
    }
    let port = std::env::var("MYSQL_RUSTY_IT_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(3306);
    let user = std::env::var("MYSQL_RUSTY_IT_USER").unwrap_or_else(|_| "root".to_string());
    let password = std::env::var("MYSQL_RUSTY_IT_PASSWORD").unwrap_or_default();

    Some(ConnectionProfile {
        host,
        port,
        user,
        password,
        ssl_mode: SslMode::Disabled,
        ..Default::default()
    })
}

macro_rules! require_db {
    () => {
        match test_profile() {
            Some(p) => p,
            None => {
                eprintln!("skipping: MYSQL_RUSTY_IT_HOST not set");
                return;
            }
        }
    };
}

fn unique_db_name() -> String {
    let short = Uuid::new_v4().simple().to_string();
    format!("it_{}", &short[..12])
}

#[tokio::test]
async fn it_connect_and_list_databases() {
    let profile = require_db!();
    let session = std::sync::Arc::new(
        ActiveSession::connect(profile)
            .await
            .expect("connect to test MySQL"),
    );
    let dbs = session.list_databases().await.expect("list_databases");
    assert!(
        dbs.iter()
            .any(|d| d.eq_ignore_ascii_case("information_schema"))
    );
}

#[tokio::test]
async fn it_introspect_database_pk_fk_indexes() {
    let profile = require_db!();
    let session = std::sync::Arc::new(
        ActiveSession::connect(profile)
            .await
            .expect("connect to test MySQL"),
    );
    let db_name = unique_db_name();

    session
        .execute_query("", &format!("CREATE DATABASE `{db_name}`"), None, None)
        .await
        .expect("create db");

    session
        .execute_query(
            &db_name,
            "CREATE TABLE users (id INT AUTO_INCREMENT PRIMARY KEY, email VARCHAR(255) NOT NULL, INDEX idx_email (email)) ENGINE=InnoDB",
            None,
            None,
        )
        .await
        .expect("create users");

    session
        .execute_query(
            &db_name,
            "CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, user_id INT NOT NULL, CONSTRAINT fk_orders_user FOREIGN KEY (user_id) REFERENCES users(id)) ENGINE=InnoDB",
            None,
            None,
        )
        .await
        .expect("create orders");

    let schema = session
        .introspect_database(&db_name)
        .await
        .expect("introspect_database");

    let users = schema
        .tables
        .iter()
        .find(|t| t.name == "users")
        .expect("users table");
    assert!(
        users
            .columns
            .iter()
            .any(|c| c.name == "id" && c.is_primary_key)
    );
    assert!(users.indexes.iter().any(|i| i.index_name == "idx_email"));

    let orders = schema
        .tables
        .iter()
        .find(|t| t.name == "orders")
        .expect("orders table");
    assert!(
        orders
            .foreign_keys
            .iter()
            .any(|fk| fk.column_name == "user_id" && fk.referenced_table == "users")
    );

    let _ = session
        .execute_query(
            "",
            &format!("DROP DATABASE IF EXISTS `{db_name}`"),
            None,
            None,
        )
        .await;
}

#[tokio::test]
async fn it_execute_query_types_and_insert_metadata() {
    let profile = require_db!();
    let session = std::sync::Arc::new(
        ActiveSession::connect(profile)
            .await
            .expect("connect to test MySQL"),
    );
    let db_name = unique_db_name();

    session
        .execute_query("", &format!("CREATE DATABASE `{db_name}`"), None, None)
        .await
        .expect("create db");

    session
        .execute_query(
            &db_name,
            "CREATE TABLE type_test (
                id INT AUTO_INCREMENT PRIMARY KEY,
                big_u BIGINT UNSIGNED,
                dec_v DECIMAL(10,2),
                dbl_v DOUBLE,
                txt VARCHAR(100) CHARACTER SET utf8mb4,
                d_v DATE,
                dt_v DATETIME(6),
                t_v TIME,
                j_v JSON,
                b_v BLOB,
                n_v VARCHAR(20) NULL
            )",
            None,
            None,
        )
        .await
        .expect("create type_test");

    let ins = session
        .execute_query(
            &db_name,
            "INSERT INTO type_test (big_u, dec_v, dbl_v, txt, d_v, dt_v, t_v, j_v, b_v, n_v)
             VALUES (18446744073709551615, 12.34, 3.5, 'hello 🦀', '2026-01-15', '2026-01-15 10:20:30.123456', '10:20:30', '{\"a\":1}', X'DEADBEEF', NULL)",
            None,
            None,
        )
        .await
        .expect("insert row");
    assert_eq!(ins.affected_rows, 1);
    assert_eq!(ins.last_insert_id, Some(1));

    let res = session
        .execute_query(&db_name, "SELECT * FROM type_test", None, None)
        .await
        .expect("select type_test");
    assert_eq!(res.columns.len(), 11);
    assert_eq!(res.rows.len(), 1);
    assert!(matches!(res.rows[0][0], CellValue::Integer(1)));
    assert!(matches!(res.rows[0][10], CellValue::Null));
    assert!(res.rows[0][4].display_text().contains('🦀'));

    let _ = session
        .execute_query(
            "",
            &format!("DROP DATABASE IF EXISTS `{db_name}`"),
            None,
            None,
        )
        .await;
}

#[tokio::test]
async fn it_apply_cell_edits_and_quote_roundtrip() {
    let profile = require_db!();
    let session = std::sync::Arc::new(
        ActiveSession::connect(profile)
            .await
            .expect("connect to test MySQL"),
    );
    let db_name = unique_db_name();

    session
        .execute_query("", &format!("CREATE DATABASE `{db_name}`"), None, None)
        .await
        .expect("create db");
    session
        .execute_query(
            &db_name,
            "CREATE TABLE items (id INT AUTO_INCREMENT PRIMARY KEY, label VARCHAR(100))",
            None,
            None,
        )
        .await
        .expect("create items");
    session
        .execute_query(
            &db_name,
            "INSERT INTO items (label) VALUES ('initial')",
            None,
            None,
        )
        .await
        .expect("insert item");

    let affected = session
        .apply_cell_edits(
            &db_name,
            "items",
            &[PendingCellEdit {
                row_idx: 0,
                col_idx: 1,
                column_name: "label".to_string(),
                pk_column: "id".to_string(),
                pk_value: "1".to_string(),
                new_value: "O'Brien".to_string(),
            }],
        )
        .await
        .expect("apply_cell_edits");
    assert_eq!(affected, 1);

    let res = session
        .execute_query(&db_name, "SELECT label FROM items WHERE id = 1", None, None)
        .await
        .expect("select updated");
    assert_eq!(res.rows[0][0].display_text(), "O'Brien");

    let _ = session
        .execute_query(
            "",
            &format!("DROP DATABASE IF EXISTS `{db_name}`"),
            None,
            None,
        )
        .await;
}

#[tokio::test]
async fn it_list_processes_kill_query_and_server_info() {
    let profile = require_db!();
    let session = std::sync::Arc::new(
        ActiveSession::connect(profile)
            .await
            .expect("connect to test MySQL"),
    );

    let procs = session.list_processes().await.expect("list_processes");
    assert!(!procs.rows.is_empty());

    let status = session
        .server_status_summary()
        .await
        .expect("server_status_summary");
    assert!(status.contains_key("Uptime"));

    let vars = session
        .list_server_variables()
        .await
        .expect("list_server_variables");
    assert!(vars.iter().any(|(k, _)| k == "version"));

    let editor_id = session
        .editor_connection_id()
        .expect("editor_connection_id");
    let session_clone = session.clone();
    let start = Instant::now();
    let handle = tokio::spawn(async move {
        let _ = session_clone
            .execute_query("", "SELECT SLEEP(10)", None, None)
            .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    session.kill_query(editor_id).await.expect("kill_query");
    let _ = handle.await;
    assert!(start.elapsed().as_secs() < 5);
}
