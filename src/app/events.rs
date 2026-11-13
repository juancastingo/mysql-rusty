use super::*;

impl MysqlRustyApp {
    pub(crate) fn process_async_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AsyncEvent::Connected {
                    profile_id,
                    session,
                    databases,
                    initial_schema,
                } => {
                    self.save_active_workspace_snapshot();

                    let (conn_title, env_color) = self
                        .config
                        .connections
                        .iter()
                        .find(|c| c.id == profile_id)
                        .map(|c| (c.name.clone(), c.environment.color_rgb()))
                        .unwrap_or_else(|| ("Connection".to_string(), (56, 189, 248)));

                    let mut schemas_map = HashMap::new();
                    let active_db = if let Some(schema) = initial_schema {
                        let name = schema.name.clone();
                        schemas_map.insert(name.clone(), schema);
                        name
                    } else {
                        databases.first().cloned().unwrap_or_default()
                    };

                    // Keep scratchpad tabs for the very first workspace; start fresh tab for extra connections
                    let initial_tabs = if self.open_workspaces.is_empty() && !self.tabs.is_empty() {
                        let mut tabs_clone = self.tabs.clone();
                        if let Some(tab) = tabs_clone.first_mut()
                            && tab.sql.trim().is_empty()
                        {
                            if let Some(schema) = schemas_map.get(&active_db) {
                                if let Some(first_tbl) = schema.tables.first() {
                                    tab.sql =
                                        format!("SELECT * FROM `{}` LIMIT 100;", first_tbl.name);
                                } else {
                                    tab.sql = "SHOW TABLES;".to_string();
                                }
                            } else {
                                tab.sql = "SHOW TABLES;".to_string();
                            }
                        }
                        tabs_clone
                    } else {
                        let starter_sql = if let Some(schema) = schemas_map.get(&active_db) {
                            if let Some(first_tbl) = schema.tables.first() {
                                format!("SELECT * FROM `{}` LIMIT 100;", first_tbl.name)
                            } else {
                                "SHOW TABLES;".to_string()
                            }
                        } else {
                            "SHOW TABLES;".to_string()
                        };
                        vec![QueryTab::new("Query 1", starter_sql)]
                    };

                    let new_ws = OpenConnectionWorkspace {
                        profile_id: profile_id.clone(),
                        title: conn_title,
                        env_color,
                        session: session.clone(),
                        databases: databases.clone(),
                        active_database: active_db.clone(),
                        schemas: schemas_map.clone(),
                        tabs: initial_tabs.clone(),
                        active_tab_idx: 0,
                        in_transaction: false,
                        uncommitted_mutations: 0,
                        er_state: ErCanvasState::default(),
                    };

                    self.open_workspaces.push(new_ws);
                    let new_idx = self.open_workspaces.len() - 1;
                    self.active_workspace_idx = Some(new_idx);
                    self.active_profile_id = Some(profile_id);
                    self.active_session = Some(session);
                    self.databases = databases;
                    self.active_database = active_db;
                    self.schemas = schemas_map;
                    self.tabs = initial_tabs;
                    self.active_tab_idx = 0;
                    self.in_transaction = false;
                    self.uncommitted_mutations = 0;
                    self.er_canvas_state = ErCanvasState::default();

                    self.status_banner = format!(
                        "Connected to database `{}` ({} schemas discovered)",
                        self.active_database,
                        self.databases.len()
                    );
                    self.sidebar_tab = SidebarTab::Schema;

                    // Run default query automatically on first connect
                    if self
                        .tabs
                        .first()
                        .map(|t| !t.sql.trim().is_empty() && t.last_result.is_none())
                        .unwrap_or(false)
                    {
                        self.execute_current_tab_query(None);
                    }
                }
                AsyncEvent::ConnectionFailed(err) => {
                    self.status_banner = format!("Connection error: {err}");
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.last_error = Some(format!("Connection error: {err}"));
                    }
                }
                AsyncEvent::SchemaLoaded(schema) => {
                    self.status_banner = format!(
                        "Loaded schema `{}` ({} tables)",
                        schema.name,
                        schema.tables.len()
                    );
                    self.schemas.insert(schema.name.clone(), schema);
                }
                AsyncEvent::QueryFinished {
                    tab_id,
                    result,
                    is_connection_error,
                } => {
                    self.bottom_tab = BottomTab::ResultsGrid;
                    let mut should_auto_retry = false;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_running = false;
                        tab.task_abort_handle = None;
                        match result {
                            Ok(res) => {
                                self.connection_lost = None;
                                self.pending_retry_query = None;
                                self.auto_retry_attempted = false;
                                let upper_sql = res
                                    .sql
                                    .trim()
                                    .trim_end_matches(';')
                                    .trim()
                                    .to_ascii_uppercase();
                                match upper_sql.as_str() {
                                    "START TRANSACTION" | "BEGIN" | "BEGIN WORK" => {
                                        self.in_transaction = true;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner =
                                            "Transaction started (Manual Commit Mode).".to_string();
                                    }
                                    "COMMIT" | "COMMIT WORK" => {
                                        self.in_transaction = false;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner =
                                            "Transaction committed successfully.".to_string();
                                    }
                                    "ROLLBACK" | "ROLLBACK WORK" => {
                                        self.in_transaction = false;
                                        self.uncommitted_mutations = 0;
                                        self.status_banner = "Transaction rolled back.".to_string();
                                    }
                                    _ => {
                                        self.status_banner = res.status_message.clone();
                                    }
                                }
                                self.config.history.insert(
                                    0,
                                    QueryHistoryEntry {
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        database: res.database.clone(),
                                        sql: res.sql.clone(),
                                        duration_ms: res.duration_ms,
                                        row_count: res.rows.len(),
                                        success: true,
                                    },
                                );
                                self.config.history.truncate(50);
                                let _ = self.config.save();
                                tab.last_result = Some(res);
                                tab.multi_results.clear();
                                tab.active_result_idx = 0;
                                tab.last_error = None;
                                tab.current_page = 0;
                            }
                            Err(err) => {
                                if is_connection_error {
                                    if !self.auto_retry_attempted
                                        && !crate::syntax::is_write_query(&tab.sql)
                                    {
                                        self.auto_retry_attempted = true;
                                        let max_rows = if self.config.query.max_rows > 0 {
                                            Some(self.config.query.max_rows)
                                        } else {
                                            None
                                        };
                                        self.pending_retry_query =
                                            Some((tab.id.clone(), tab.sql.clone(), max_rows));
                                        self.connection_lost = Some(err.clone());
                                        self.status_banner =
                                            "Connection lost — attempting auto-reconnect..."
                                                .to_string();
                                        should_auto_retry = true;
                                    } else {
                                        self.connection_lost = Some(err.clone());
                                        self.status_banner = format!("Connection lost: {err}");
                                    }
                                } else {
                                    self.status_banner = format!("Query failed: {err}");
                                }
                                tab.last_error = Some(err);
                                tab.last_result = None;
                                tab.multi_results.clear();
                            }
                        }
                    }
                    if should_auto_retry {
                        self.reconnect_active_session();
                    }
                }
                AsyncEvent::MultiQueryFinished {
                    tab_id,
                    results,
                    is_connection_error,
                } => {
                    if is_connection_error {
                        self.connection_lost =
                            Some("Connection lost during multi-query execution".to_string());
                    }
                    self.bottom_tab = BottomTab::ResultsGrid;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_running = false;
                        tab.task_abort_handle = None;
                        tab.multi_results.clear();
                        tab.active_result_idx = 0;
                        let mut first_err = None;
                        for r in results {
                            match r {
                                Ok(res) => tab.multi_results.push(res),
                                Err(e) => {
                                    if first_err.is_none() {
                                        first_err = Some(e);
                                    }
                                }
                            }
                        }
                        if let Some(first) = tab.multi_results.first().cloned() {
                            self.status_banner = format!(
                                "Executed {} statements ({} result sets)",
                                tab.multi_results.len(),
                                tab.multi_results.len()
                            );
                            tab.last_result = Some(first);
                            tab.last_error = first_err;
                            tab.current_page = 0;
                        } else if let Some(err) = first_err {
                            self.status_banner = format!("Multi-query failed: {err}");
                            tab.last_error = Some(err);
                            tab.last_result = None;
                        }
                    }
                }
                AsyncEvent::CellEditsApplied {
                    tab_id,
                    updated_count,
                } => match updated_count {
                    Ok(cnt) => {
                        self.status_banner =
                            format!("Successfully committed {cnt} inline cell update(s).");
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.pending_edits.clear();
                        }
                        self.execute_current_tab_query(None);
                    }
                    Err(e) => {
                        self.status_banner = format!("Failed to commit inline edits: {e}");
                    }
                },
                AsyncEvent::InlineAiCompletion {
                    tab_id,
                    req_id,
                    result,
                } => {
                    self.is_inline_ai_loading = false;
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        // Stale request check: discard if user continued typing or triggered a newer request
                        if tab.inline_req_id != req_id {
                            continue;
                        }
                        match result {
                            Ok(completion) if !completion.is_empty() => {
                                tab.ghost_completion = Some(completion);
                                self.status_banner =
                                    "AI inline suggestion ready — Press Tab to accept.".to_string();
                            }
                            Ok(_) => {
                                self.status_banner =
                                    "AI returned an empty inline completion.".to_string();
                            }
                            Err(err) => {
                                // Fallback to heuristic completion so the user still gets a smart suggestion
                                let schema = self.schemas.get(&self.active_database);
                                let (prefix, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                                tab.ghost_completion =
                                    compute_heuristic_ghost_completion(prefix, schema);
                                self.status_banner = format!(
                                    "AI provider unreachable ({err}); using local schema heuristic."
                                );
                            }
                        }
                    }
                }
                AsyncEvent::ChatAiReply(res) => {
                    self.is_chat_loading = false;
                    match res {
                        Ok(msg) => self.chat_messages.push(msg),
                        Err(err) => self.chat_messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: format!(
                                "**[AI Provider Error]** {err}\n\nTip: Check **AI Settings** in the top toolbar to configure Ollama, OpenAI, Anthropic, Gemini, or your local CLI (`agy`)."
                            ),
                            extracted_sql: None,
                        }),
                    }
                }
                AsyncEvent::ServerAdminLoaded { status, processes } => {
                    self.is_server_admin_loading = false;
                    match status {
                        Ok(metrics) => {
                            let now = std::time::Instant::now();
                            let current_questions = metrics
                                .get("Questions")
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            let current_threads = metrics
                                .get("Threads_connected")
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(1.0);

                            if let (Some(last_time), Some(last_q)) =
                                (self.last_poll_instant, self.last_questions_count)
                            {
                                let elapsed = now.duration_since(last_time).as_secs_f64();
                                if elapsed > 0.05 {
                                    let delta_q = current_questions.saturating_sub(last_q) as f64;
                                    let qps = delta_q / elapsed;
                                    let idx = self.qps_history.len() as f64;
                                    self.qps_history.push([idx, qps]);
                                    self.threads_history.push([idx, current_threads]);
                                    if self.qps_history.len() > 60 {
                                        self.qps_history.remove(0);
                                        self.threads_history.remove(0);
                                    }
                                }
                            } else {
                                self.qps_history.push([0.0, 0.0]);
                                self.threads_history.push([0.0, current_threads]);
                            }
                            self.last_poll_instant = Some(now);
                            self.last_questions_count = Some(current_questions);
                            self.server_status_metrics = metrics;
                        }
                        Err(e) => self.status_banner = format!("Server status error: {e}"),
                    }
                    match processes {
                        Ok(procs) => self.server_processes = Some(procs),
                        Err(e) => self.status_banner = format!("Process list error: {e}"),
                    }
                }
                AsyncEvent::QueryKilled(res) => match res {
                    Ok(id) => {
                        self.status_banner =
                            format!("Successfully killed query for connection #{id}");
                        self.refresh_server_admin();
                    }
                    Err(e) => self.status_banner = format!("Failed to kill query: {e}"),
                },
                AsyncEvent::TestConnectionResult(res) => {
                    self.testing_connection = false;
                    self.test_connection_result = Some(res);
                }
                AsyncEvent::ServerVariablesLoaded(res) => {
                    self.is_loading_variables = false;
                    match res {
                        Ok(vars) => {
                            self.server_variables = vars;
                            self.status_banner =
                                format!("Loaded {} server variables.", self.server_variables.len());
                        }
                        Err(e) => {
                            self.status_banner = format!("Failed to load server variables: {e}");
                        }
                    }
                }
                AsyncEvent::CopilotDeviceCodeReady(res) => {
                    self.is_copilot_authenticating = false;
                    match res {
                        Ok(data) => {
                            self.copilot_auth_status = format!(
                                "One-time code generated: {}. Enter this code at https://github.com/login/device",
                                data.user_code
                            );
                            self.copilot_device_code_data = Some(data);
                        }
                        Err(e) => {
                            self.copilot_auth_status = format!("Failed to get device code: {e}");
                        }
                    }
                }
                AsyncEvent::CopilotPollFinished(res) => {
                    self.is_copilot_authenticating = false;
                    match res {
                        Ok(token) => {
                            self.config.ai.copilot_github_token = token;
                            self.copilot_device_code_data = None;
                            self.copilot_auth_status =
                                "✅ Successfully connected to GitHub Copilot!".to_string();
                            let _ = self.config.save();
                            self.status_banner = "GitHub Copilot connected and saved.".to_string();
                        }
                        Err(e) => {
                            if e == "authorization_pending" {
                                self.copilot_auth_status = "Waiting for authorization in browser... Click Check again once approved.".to_string();
                            } else {
                                self.copilot_auth_status =
                                    format!("Authorization check error: {e}");
                            }
                        }
                    }
                }
                AsyncEvent::FkLookupLoaded { key, result } => {
                    if let Ok(res) = result {
                        let options: Vec<(String, String)> = res
                            .rows
                            .iter()
                            .take(100)
                            .map(|row| {
                                let pk = row.first().map(|c| c.display_text()).unwrap_or_default();
                                let preview = row
                                    .iter()
                                    .skip(1)
                                    .take(2)
                                    .map(|c| c.display_text())
                                    .filter(|s| !s.is_empty() && s != "NULL")
                                    .collect::<Vec<_>>()
                                    .join(" | ");
                                let label = if preview.is_empty() {
                                    pk.clone()
                                } else {
                                    format!("{pk} — {preview}")
                                };
                                (pk, label)
                            })
                            .collect();
                        self.fk_lookup_cache.insert(key, options);
                    }
                }
                AsyncEvent::DataDiffLoaded {
                    left,
                    right,
                    key_col_idx,
                } => {
                    self.data_diff_loading = false;
                    match (left, right) {
                        (Ok(l_res), Ok(r_res)) => {
                            let report = compute_data_diff(&l_res, &r_res, key_col_idx);
                            let sync_sql = generate_data_diff_sync_sql(
                                &self.data_diff_target_table,
                                &report,
                                key_col_idx,
                            );
                            self.status_banner = format!(
                                "Data Diff: {} modified, {} only in source, {} only in target",
                                report.modified_count,
                                report.left_only_count,
                                report.right_only_count
                            );
                            self.data_diff_report = Some(report);
                            self.data_diff_sync_sql = sync_sql;
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            self.status_banner = format!("Data Diff failed: {e}");
                        }
                    }
                }
                AsyncEvent::SqlDumpLoaded {
                    sql,
                    suggested_name,
                } => {
                    self.dump_is_busy = false;
                    match sql {
                        Ok(content) => {
                            self.dump_preview_sql = content;
                            self.status_banner = format!(
                                "Generated SQL dump ({} bytes) for `{suggested_name}`.",
                                self.dump_preview_sql.len()
                            );
                        }
                        Err(e) => {
                            self.status_banner = format!("SQL Dump generation failed: {e}");
                        }
                    }
                }
                AsyncEvent::SqlDumpImportFinished(res) => {
                    self.dump_is_busy = false;
                    match res {
                        Ok(count) => {
                            self.status_banner = format!(
                                "Successfully executed {count} SQL statements from dump file."
                            );
                            self.reload_active_schema();
                        }
                        Err(e) => {
                            self.status_banner = format!("SQL Dump import failed: {e}");
                        }
                    }
                }
                AsyncEvent::RowDeletesApplied {
                    tab_id,
                    deleted_count,
                } => match deleted_count {
                    Ok(n) => {
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.pending_row_deletes.clear();
                            tab.selected_rows.clear();
                        }
                        self.status_banner = format!("Deleted {n} row(s) successfully.");
                        self.execute_current_tab_query(None);
                    }
                    Err(e) => {
                        self.status_banner = format!("Failed to delete rows: {e}");
                    }
                },
                AsyncEvent::KeepaliveResult(res) => match res {
                    Ok(()) => {
                        self.connection_lost = None;
                    }
                    Err(err) => {
                        self.connection_lost = Some(err.clone());
                        self.status_banner = format!("Connection lost: {err}");
                    }
                },
                AsyncEvent::Reconnected(res) => match res {
                    Ok((profile_id, session)) => {
                        if self.active_profile_id.as_deref() == Some(profile_id.as_str()) {
                            self.active_session = Some(session.clone());
                        }
                        if let Some(ws) = self
                            .open_workspaces
                            .iter_mut()
                            .find(|w| w.profile_id == profile_id)
                        {
                            ws.session = session;
                        }
                        self.connection_lost = None;
                        self.last_keepalive_ping = std::time::Instant::now();
                        self.status_banner = "Reconnected.".to_string();
                        if let Some((tab_id, sql, max_rows)) = self.pending_retry_query.take()
                            && let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id)
                        {
                            self.active_tab_idx = idx;
                            self.execute_sql_with_limit(sql, max_rows);
                        }
                    }
                    Err(err) => {
                        self.connection_lost = Some(err.clone());
                        self.pending_retry_query = None;
                        self.status_banner = format!("Reconnect failed: {err}");
                    }
                },
            }
        }
    }
}
