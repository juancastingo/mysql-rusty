mod ai;
mod app;
mod backup;
mod config;
mod data_diff;
mod db;
#[cfg(test)]
mod db_integration_tests;
mod demo;
mod dialect;
mod er_diagram;
mod export;
mod find;
mod history;
mod mcp;
mod mock_gen;
mod pg_db;
mod schema_diff;
mod shortcuts;
mod sql_reader;
mod sqlite_db;
mod syntax;
mod table_designer;
#[cfg(test)]
pub mod test_fixtures;
mod theme;
mod tour;
mod vim;

rust_i18n::i18n!("locales", fallback = "en");

pub fn resolve_locale(pref: &str) -> &'static str {
    match pref {
        "es" => "es",
        "en" => "en",
        _ => {
            if sys_locale::get_locale()
                .map(|l| l.to_ascii_lowercase().starts_with("es"))
                .unwrap_or(false)
            {
                "es"
            } else {
                "en"
            }
        }
    }
}

pub fn apply_locale(pref: &str) {
    rust_i18n::set_locale(resolve_locale(pref));
}

use app::MysqlRustyApp;
use config::AppConfig;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn cleanup_old_logs(logs_dir: &std::path::Path, max_age_days: u64) {
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(max_age_days * 24 * 60 * 60);
    if let Ok(entries) = std::fs::read_dir(logs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
                && modified < cutoff
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let logs_dir = AppConfig::logs_dir();
    cleanup_old_logs(&logs_dir, 7);

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "mysql-rusty.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_env("MYSQL_RUSTY_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,mysql_async=warn"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false);

    let stderr_layer = if cfg!(debug_assertions) {
        Some(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "Application panicked");
        default_hook(info);
    }));

    tracing::info!(logs_dir = %logs_dir.display(), "Logging initialized");
    guard
}

fn main() -> eframe::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _log_guard = init_logging();

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("mcp") {
        if let Err(err) = mcp::run_mcp_cli(&args[2..]) {
            eprintln!("MCP server error: {err:#}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let rt = Arc::new(
        Runtime::new().expect("Failed to initialize asynchronous Tokio runtime for MySQL-Rusty"),
    );

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MySQL-Rusty — Native Rust SQL Client & AI Copilot")
        .with_inner_size([1420.0, 860.0])
        .with_min_inner_size([960.0, 620.0])
        .with_app_id("mysql-rusty");

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "mysql-rusty",
        native_options,
        Box::new(move |cc| Ok(Box::new(MysqlRustyApp::new(cc, rt.clone())))),
    )
}

#[cfg(test)]
mod i18n_tests {
    use std::collections::BTreeSet;

    fn collect_keys(val: &serde_yaml::Value, prefix: &str, out: &mut BTreeSet<String>) {
        if let serde_yaml::Value::Mapping(map) = val {
            for (k, v) in map {
                if let Some(ks) = k.as_str() {
                    if ks == "_version" {
                        continue;
                    }
                    let full = if prefix.is_empty() {
                        ks.to_string()
                    } else {
                        format!("{prefix}.{ks}")
                    };
                    if v.is_mapping() {
                        collect_keys(v, &full, out);
                    } else {
                        out.insert(full);
                    }
                }
            }
        }
    }

    #[test]
    fn test_locale_yaml_key_parity() {
        let en: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../locales/en.yml")).expect("valid en.yml");
        let es: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../locales/es.yml")).expect("valid es.yml");

        let mut en_keys = BTreeSet::new();
        let mut es_keys = BTreeSet::new();
        collect_keys(&en, "", &mut en_keys);
        collect_keys(&es, "", &mut es_keys);

        assert!(!en_keys.is_empty());
        assert_eq!(
            en_keys, es_keys,
            "en.yml and es.yml must have identical keys"
        );
    }
}
