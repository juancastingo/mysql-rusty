use super::*;

impl MysqlRustyApp {
    pub(crate) fn save_active_workspace_snapshot(&mut self) {
        let Some(idx) = self.active_workspace_idx else {
            return;
        };
        if self.active_session.is_none() {
            return;
        }
        if let Some(ws) = self.open_workspaces.get_mut(idx) {
            ws.databases = self.databases.clone();
            ws.active_database = self.active_database.clone();
            ws.schemas = self.schemas.clone();
            ws.tabs = self.tabs.clone();
            ws.active_tab_idx = self.active_tab_idx;
            ws.in_transaction = self.in_transaction;
            ws.uncommitted_mutations = self.uncommitted_mutations;
            ws.er_state = self.er_canvas_state.clone();
        }
    }

    pub(crate) fn switch_to_workspace(&mut self, target_idx: Option<usize>) {
        if self.active_workspace_idx == target_idx {
            return;
        }
        self.save_active_workspace_snapshot();
        match target_idx {
            None => {
                self.active_workspace_idx = None;
                self.active_session = None;
                self.active_profile_id = None;
                self.databases.clear();
                self.schemas.clear();
                self.active_database.clear();
                self.in_transaction = false;
                self.uncommitted_mutations = 0;
                self.sidebar_tab = SidebarTab::Connections;
            }
            Some(idx) => {
                if let Some(ws) = self.open_workspaces.get(idx).cloned() {
                    self.active_workspace_idx = Some(idx);
                    self.active_profile_id = Some(ws.profile_id);
                    self.active_session = Some(ws.session);
                    self.databases = ws.databases;
                    self.active_database = ws.active_database;
                    self.schemas = ws.schemas;
                    self.tabs = if ws.tabs.is_empty() {
                        vec![QueryTab::new("Query 1", "")]
                    } else {
                        ws.tabs
                    };
                    self.active_tab_idx = ws.active_tab_idx.min(self.tabs.len().saturating_sub(1));
                    self.in_transaction = ws.in_transaction;
                    self.uncommitted_mutations = ws.uncommitted_mutations;
                    self.er_canvas_state = ws.er_state;
                    self.sidebar_tab = SidebarTab::Schema;
                    self.status_banner = format!("Switched to connection `{}`.", ws.title);
                }
            }
        }
    }

    pub(crate) fn close_workspace(&mut self, idx: usize) {
        if idx >= self.open_workspaces.len() {
            return;
        }
        let closed_title = self.open_workspaces[idx].title.clone();
        self.open_workspaces.remove(idx);

        if self.open_workspaces.is_empty() {
            self.active_workspace_idx = None;
            self.active_session = None;
            self.active_profile_id = None;
            self.databases.clear();
            self.schemas.clear();
            self.active_database.clear();
            self.in_transaction = false;
            self.uncommitted_mutations = 0;
            self.sidebar_tab = SidebarTab::Connections;
            self.status_banner = format!("Disconnected from `{closed_title}`.");
            return;
        }

        match self.active_workspace_idx {
            Some(cur) if cur == idx => {
                let next_idx = idx.min(self.open_workspaces.len() - 1);
                self.active_workspace_idx = None; // prevent saving removed ws
                self.switch_to_workspace(Some(next_idx));
                self.status_banner = format!("Closed `{closed_title}`.");
            }
            Some(cur) if idx < cur => {
                self.active_workspace_idx = Some(cur - 1);
                self.status_banner = format!("Closed `{closed_title}`.");
            }
            _ => {
                self.status_banner = format!("Closed `{closed_title}`.");
            }
        }
    }

    pub(crate) fn disconnect(&mut self) {
        if let Some(idx) = self.active_workspace_idx {
            self.close_workspace(idx);
        } else {
            self.active_session = None;
            self.active_profile_id = None;
            self.databases.clear();
            self.schemas.clear();
            self.active_database.clear();
            self.sidebar_tab = SidebarTab::Connections;
            self.status_banner = "Disconnected. Select a connection to begin.".to_string();
        }
    }

    pub(crate) fn connect_to_profile(&mut self, profile: ConnectionProfile) {
        if let Some(existing_idx) = self
            .open_workspaces
            .iter()
            .position(|ws| ws.profile_id == profile.id)
        {
            self.switch_to_workspace(Some(existing_idx));
            return;
        }
        self.status_banner = format!("Connecting to `{}` ({})...", profile.name, profile.host);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match ActiveSession::connect(profile.clone()).await {
                Ok(session) => {
                    let session = Arc::new(session);
                    let dbs = session.list_databases().await.unwrap_or_default();
                    let target_db = if !profile.default_database.is_empty() {
                        profile.default_database.clone()
                    } else {
                        dbs.first().cloned().unwrap_or_default()
                    };
                    let initial_schema = if !target_db.is_empty() {
                        session.introspect_database(&target_db).await.ok()
                    } else {
                        None
                    };
                    let _ = tx.send(AsyncEvent::Connected {
                        profile_id: profile.id,
                        session,
                        databases: dbs,
                        initial_schema,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AsyncEvent::ConnectionFailed(format!("{e:#}")));
                }
            }
        });
    }

    pub(crate) fn reload_active_schema(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let db = self.active_database.clone();
        if db.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match session.introspect_database(&db).await {
                Ok(schema) => {
                    let _ = tx.send(AsyncEvent::SchemaLoaded(schema));
                }
                Err(e) => {
                    if crate::db::is_connection_error(&e) {
                        let _ = tx.send(AsyncEvent::KeepaliveResult(Err(format!("{e:#}"))));
                    }
                }
            }
        });
    }

    pub(crate) fn reconnect_active_session(&mut self) {
        let Some(profile_id) = self.active_profile_id.clone() else {
            return;
        };
        let Some(profile) = self
            .config
            .connections
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
        else {
            return;
        };
        let session = self.active_session.clone();
        let tx = self.tx.clone();
        self.status_banner = format!("Reconnecting to `{}`...", profile.name);
        self.rt.spawn(async move {
            let res = match session {
                Some(s) => s.reconnect(&profile).await,
                None => ActiveSession::connect(profile).await,
            };
            let _ = tx.send(AsyncEvent::Reconnected(
                res.map(|s| (profile_id, Arc::new(s)))
                    .map_err(|e| format!("{e:#}")),
            ));
        });
    }

    pub(crate) fn save_active_tab_direct(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        // Determine destination path: use existing file_path or auto-assign inside queries_dir
        let path = if let Some(existing_path) = &tab.file_path {
            existing_path.clone()
        } else {
            let safe_title = sanitize_filename(&tab.title);
            let base_name = if safe_title.is_empty() {
                "query".to_string()
            } else {
                safe_title
            };
            let mut candidate = AppConfig::queries_dir().join(format!("{base_name}.sql"));
            let mut counter = 1;
            while candidate.exists() {
                candidate = AppConfig::queries_dir().join(format!("{base_name}_{counter}.sql"));
                counter += 1;
            }
            tab.file_path = Some(candidate.clone());
            candidate
        };

        // Write SQL directly to file
        if let Err(e) = std::fs::write(&path, &tab.sql) {
            self.status_banner = format!("Error saving query: {e}");
            return;
        }

        tab.is_dirty = false;

        // Auto-sync into Saved Queries library (config.snippets)
        let tab_id = tab.id.clone();
        let tab_title = tab.title.clone();
        let tab_sql = tab.sql.clone();
        let active_db = if self.active_database.is_empty() {
            None
        } else {
            Some(self.active_database.clone())
        };
        let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

        if let Some(existing) = self.config.snippets.iter_mut().find(|s| s.id == tab_id) {
            existing.title = tab_title.clone();
            existing.sql = tab_sql;
            if existing.database.is_none() {
                existing.database = active_db;
            }
        } else {
            self.config.snippets.push(SavedSnippet {
                id: tab_id,
                title: tab_title.clone(),
                sql: tab_sql,
                database: active_db,
                description: String::new(),
                created_at: now_str,
            });
        }

        let _ = self.config.save();
        let display_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&tab_title);
        self.status_banner = format!("Saved '{display_name}' directly to {}", path.display());
    }

    pub(crate) fn rename_tab(&mut self, idx: usize, new_name: String) {
        let clean_name = new_name.trim();
        if clean_name.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let old_title = tab.title.clone();
        tab.title = clean_name.to_string();

        // If file_path was previously saved in queries_dir, rename the file too
        if let Some(old_path) = tab.file_path.take() {
            let safe = sanitize_filename(clean_name);
            let new_path = AppConfig::queries_dir().join(format!("{safe}.sql"));
            let _ = std::fs::rename(&old_path, &new_path);
            tab.file_path = Some(new_path);
        }

        // Also update in config.snippets if present
        let tab_id = tab.id.clone();
        if let Some(snip) = self
            .config
            .snippets
            .iter_mut()
            .find(|s| s.id == tab_id || s.title == old_title)
        {
            snip.title = clean_name.to_string();
            let _ = self.config.save();
        }

        self.status_banner = format!("Renamed query to '{clean_name}'");
    }

    pub(crate) fn rename_saved_query(&mut self, snippet_id: &str, new_title: &str) {
        let clean = new_title.trim();
        if clean.is_empty() {
            return;
        }
        if let Some(snip) = self.config.snippets.iter_mut().find(|s| s.id == snippet_id) {
            let old_title = snip.title.clone();
            snip.title = clean.to_string();

            for tab in &mut self.tabs {
                if tab.id == snippet_id || tab.title == old_title {
                    tab.title = clean.to_string();
                }
            }

            let old_file =
                AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(&old_title)));
            let new_file =
                AppConfig::queries_dir().join(format!("{}.sql", sanitize_filename(clean)));
            if old_file.exists() {
                let _ = std::fs::rename(old_file, new_file);
            }

            let _ = self.config.save();
            self.status_banner = format!("Renamed query to '{clean}'");
        }
    }

    pub(crate) fn save_active_tab_to_file(&mut self, save_as: bool) {
        if !save_as {
            self.save_active_tab_direct();
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };
        let default_name = format!("{}.sql", sanitize_filename(&tab.title));
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("SQL Files", &["sql", "txt"])
            .save_file()
        {
            if let Err(e) = std::fs::write(&path, &tab.sql) {
                self.status_banner = format!("Failed to export SQL file: {e}");
            } else {
                self.status_banner = format!("Exported query to {}", path.display());
            }
        }
    }

    pub(crate) fn open_sql_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("SQL Files", &["sql", "txt"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("query.sql")
                        .to_string();
                    let is_current_tab_empty = self
                        .tabs
                        .get(self.active_tab_idx)
                        .map(|t| t.sql.trim().is_empty())
                        .unwrap_or(false);

                    if is_current_tab_empty {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.title = filename;
                            tab.sql = content;
                            tab.file_path = Some(path.clone());
                            tab.is_dirty = false;
                        }
                    } else {
                        let mut new_tab = QueryTab::new(&filename, content);
                        new_tab.file_path = Some(path.clone());
                        self.tabs.push(new_tab);
                        self.active_tab_idx = self.tabs.len() - 1;
                    }
                    self.status_banner = format!("Opened file: {}", path.display());
                }
                Err(e) => {
                    self.status_banner = format!("Failed to read SQL file: {e}");
                }
            }
        }
    }

    pub(crate) fn open_save_query_modal(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            self.new_snippet_title = if !tab.title.is_empty() && !tab.title.starts_with("Query ") {
                tab.title.clone()
            } else {
                tab.sql
                    .lines()
                    .next()
                    .unwrap_or("My Query")
                    .trim_start_matches("--")
                    .trim()
                    .chars()
                    .take(45)
                    .collect::<String>()
            };
            if self.new_snippet_title.is_empty() {
                self.new_snippet_title = "My Query".to_string();
            }
            self.new_snippet_db = self.active_database.clone();
            self.new_snippet_desc.clear();
            self.show_save_snippet_modal = true;
        }
    }

    pub(crate) fn execute_sql_in_current_tab(
        &mut self,
        raw_sql: String,
        prefix_override: Option<&str>,
    ) {
        if self.active_session.is_none() {
            self.status_banner = "Not connected to any database session.".to_string();
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.last_error = Some("Not connected to any database. Please click 'Connect' on a profile in the Servers tab.".to_string());
            }
            return;
        }
        let mut sql = match prefix_override {
            Some(prefix) => format!("{prefix} {}", raw_sql.trim()),
            None => raw_sql.trim().to_string(),
        };
        if let Some(tab) = self.tabs.get(self.active_tab_idx)
            && !tab.query_params.is_empty()
        {
            sql = substitute_sql_parameters(&sql, &tab.query_params);
        }
        if sql.is_empty() {
            return;
        }

        let trimmed_lower = sql.trim().to_lowercase();
        if trimmed_lower.starts_with("use ") {
            let target = sql.trim()[4..]
                .trim()
                .trim_matches(|c| c == ';' || c == '`' || c == ' ');
            if !target.is_empty() {
                self.active_database = target.to_string();
                if !self.databases.contains(&self.active_database) {
                    self.databases.push(self.active_database.clone());
                }
                self.reload_active_schema();
            }
        }

        // Production Guard & Read-Only Safety Checks
        if let Some(prof) = self
            .config
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
        {
            if prof.read_only && is_write_query(&sql) {
                self.status_banner = "Query blocked: connection is in Read-Only mode.".to_string();
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.last_error = Some("[BLOCKED] Connection is in Read-Only mode. Write operations (INSERT, UPDATE, DELETE, DROP, ALTER, TRUNCATE) are prohibited.".to_string());
                }
                return;
            }

            if let Some(reason) = is_potentially_destructive(&sql) {
                self.safety_warning_msg =
                    format!("[WARNING] Destructive Query Detected:\n{reason}");
                self.pending_safe_sql = sql.clone();
                self.show_safety_modal = true;
                return;
            } else if prof.environment == Environment::Production && is_write_query(&sql) {
                self.safety_warning_msg = format!(
                    "[PRODUCTION GUARD]\nYou are connected to '{}' [PRODUCTION]. Confirm you want to execute this write query.",
                    prof.name
                );
                self.pending_safe_sql = sql.clone();
                self.show_safety_modal = true;
                return;
            }
        }

        self.execute_sql_forced(sql);
    }

    pub(crate) fn execute_sql_forced(&mut self, sql: String) {
        let max_rows = Some(self.config.query.max_rows);
        self.execute_sql_with_limit(sql, max_rows);
    }

    pub(crate) fn execute_sql_with_limit(&mut self, sql: String, max_rows: Option<usize>) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };

        tab.is_running = true;
        tab.last_error = None;
        tab.pending_edits.clear();
        tab.editing_cell = None;

        if self.in_transaction && is_write_query(&sql) {
            self.uncommitted_mutations += 1;
        }

        let tab_id = tab.id.clone();
        let db = self.active_database.clone();
        let schema = self.schemas.get(&db).cloned();
        let tx = self.tx.clone();
        let timeout_secs = self.config.query.timeout_secs;

        self.status_banner = format!("Running query on `{db}`...");
        let statements = split_sql_statements(&sql);
        let join_handle = self.rt.spawn(async move {
            async fn run_with_timeout(
                session: &ActiveSession,
                db: &str,
                stmt: &str,
                schema: Option<&DatabaseSchema>,
                timeout_secs: u64,
                max_rows: Option<usize>,
            ) -> Result<QueryResult, (String, bool)> {
                if timeout_secs > 0 {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        session.execute_query(db, stmt, schema, max_rows),
                    )
                    .await
                    {
                        Ok(res) => {
                            res.map_err(|e| (format!("{e:#}"), crate::db::is_connection_error(&e)))
                        }
                        Err(_) => {
                            if let Some(conn_id) = session.editor_connection_id() {
                                let _ = session.kill_query(conn_id).await;
                            }
                            Err((
                                format!(
                                    "Query timed out after {timeout_secs}s and was cancelled on the server"
                                ),
                                false,
                            ))
                        }
                    }
                } else {
                    session
                        .execute_query(db, stmt, schema, max_rows)
                        .await
                        .map_err(|e| (format!("{e:#}"), crate::db::is_connection_error(&e)))
                }
            }

            if statements.len() > 1 {
                let mut results = Vec::with_capacity(statements.len());
                let mut any_conn_err = false;
                for stmt in statements {
                    let res = run_with_timeout(
                        &session,
                        &db,
                        &stmt,
                        schema.as_ref(),
                        timeout_secs,
                        max_rows,
                    )
                    .await;
                    match res {
                        Ok(r) => results.push(Ok(r)),
                        Err((msg, is_conn)) => {
                            any_conn_err |= is_conn;
                            results.push(Err(msg));
                        }
                    }
                }
                let _ = tx.send(AsyncEvent::MultiQueryFinished {
                    tab_id,
                    results,
                    is_connection_error: any_conn_err,
                });
            } else {
                let res =
                    run_with_timeout(&session, &db, &sql, schema.as_ref(), timeout_secs, max_rows)
                        .await;
                let (result, is_connection_error) = match res {
                    Ok(r) => (Ok(r), false),
                    Err((msg, is_conn)) => (Err(msg), is_conn),
                };
                let _ = tx.send(AsyncEvent::QueryFinished {
                    tab_id,
                    result,
                    is_connection_error,
                });
            }
        });
        tab.task_abort_handle = Some(join_handle.abort_handle());
    }

    pub fn start_transaction(&mut self) {
        self.status_banner = "Starting transaction...".to_string();
        self.execute_sql_forced("START TRANSACTION;".to_string());
    }

    pub fn commit_transaction(&mut self) {
        self.status_banner = "Committing transaction...".to_string();
        self.execute_sql_forced("COMMIT;".to_string());
    }

    pub fn rollback_transaction(&mut self) {
        self.status_banner = "Rolling back transaction...".to_string();
        self.execute_sql_forced("ROLLBACK;".to_string());
    }

    pub fn open_csv_import_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select CSV file to import")
            .add_filter("CSV Files", &["csv"])
            .pick_file()
        {
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let default_table = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            match std::fs::read_to_string(&path) {
                Ok(content) => match inspect_csv_content(&content, 5) {
                    Ok((headers, sample_rows, total_rows)) => {
                        let mappings = headers.iter().map(|h| (h.clone(), h.clone())).collect();
                        self.csv_wizard = Some(CsvImportWizard {
                            file_path: Some(path),
                            file_name,
                            headers,
                            sample_rows,
                            total_rows,
                            target_table: default_table.clone(),
                            new_table_name: default_table,
                            create_new_table: false,
                            column_mappings: mappings,
                            is_importing: false,
                            status_message: None,
                            error_message: None,
                        });
                    }
                    Err(e) => {
                        self.status_banner = format!("Failed to inspect CSV: {e}");
                    }
                },
                Err(e) => {
                    self.status_banner = format!("Failed to read CSV file: {e}");
                }
            }
        }
    }

    pub fn execute_csv_import(&mut self) {
        let Some(wizard) = &mut self.csv_wizard else {
            return;
        };
        let Some(path) = &wizard.file_path else {
            return;
        };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                wizard.error_message = Some(format!("Failed to read file: {e}"));
                return;
            }
        };

        let target_name = if wizard.create_new_table {
            wizard.new_table_name.trim().to_string()
        } else {
            wizard.target_table.trim().to_string()
        };

        match generate_csv_import_sql(
            &target_name,
            &wizard.headers,
            &wizard.column_mappings,
            &content,
            wizard.create_new_table,
        ) {
            Ok(sql) => {
                let count = wizard.total_rows;
                self.tabs
                    .push(QueryTab::new(format!("Import {target_name}"), sql));
                self.active_tab_idx = self.tabs.len() - 1;
                self.status_banner =
                    format!("Generated import SQL for `{target_name}` ({count} rows).");
                self.csv_wizard = None;
            }
            Err(e) => {
                wizard.error_message = Some(format!("Error generating import SQL: {e}"));
            }
        }
    }

    pub(crate) fn execute_current_tab_query(&mut self, prefix_override: Option<&str>) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let sql = if let Some(selected) = &tab.selected_sql {
            selected.clone()
        } else {
            tab.sql.clone()
        };
        self.execute_sql_in_current_tab(sql, prefix_override);
    }

    pub(crate) fn execute_current_statement(&mut self) {
        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let (stmt, _) = extract_statement_at_cursor(&tab.sql, tab.cursor_index);
        if !stmt.trim().is_empty() {
            self.execute_sql_in_current_tab(stmt, None);
        }
    }
}
