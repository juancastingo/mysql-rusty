use super::super::*;

mod backup_restore;
mod csv_import;
mod data_diff;
mod mock_data;
mod schema_diff;
mod sql_dump;

impl MysqlRustyApp {
    fn diff_server_options(&self) -> Vec<(Option<usize>, String)> {
        if self.open_workspaces.is_empty() {
            if self.active_session.is_some() {
                vec![(None, "Active Server".to_string())]
            } else {
                Vec::new()
            }
        } else {
            self.open_workspaces
                .iter()
                .enumerate()
                .map(|(i, ws)| (Some(i), ws.title.clone()))
                .collect()
        }
    }

    fn diff_server_label(&self, ws_idx: Option<usize>) -> String {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.title.clone();
        }
        "Active Server".to_string()
    }

    fn diff_server_databases(&self, ws_idx: Option<usize>) -> Vec<String> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.databases.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.databases.clone();
        }
        self.databases.clone()
    }

    fn diff_server_active_db(&self, ws_idx: Option<usize>) -> String {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.active_database.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.active_database.clone();
        }
        self.active_database.clone()
    }

    pub(crate) fn diff_server_schema(
        &self,
        ws_idx: Option<usize>,
        db: &str,
    ) -> Option<&DatabaseSchema> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.schemas.get(db).map(|s| s.as_ref());
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.schemas.get(db).map(|s| s.as_ref());
        }
        None
    }

    fn diff_server_session(&self, ws_idx: Option<usize>) -> Option<Arc<ActiveSession>> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.active_session.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return Some(ws.session.clone());
        }
        self.active_session.clone()
    }

    pub(crate) fn diff_included_table_set(
        &self,
        src: &DatabaseSchema,
        tgt: &DatabaseSchema,
    ) -> std::collections::HashSet<String> {
        let mut included = std::collections::HashSet::new();
        for t in &src.tables {
            if !self.diff_excluded_tables.contains(&t.name) {
                included.insert(t.name.clone());
            }
        }
        for t in &tgt.tables {
            if !self.diff_excluded_tables.contains(&t.name) {
                included.insert(t.name.clone());
            }
        }
        included
    }

    pub(crate) fn spawn_load_workspace_schema(&mut self, ws_idx: Option<usize>, db: String) {
        let Some(session) = self.diff_server_session(ws_idx) else {
            return;
        };
        if db.trim().is_empty() {
            return;
        }
        self.diff_loading_schemas = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session
                .introspect_database(&db)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::WorkspaceSchemaLoaded {
                ws_idx,
                db,
                schema: res,
            });
        });
    }

    fn diff_server_profile(&self, ws_idx: Option<usize>) -> Option<&ConnectionProfile> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        let profile_id = if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            Some(ws.profile_id.as_str())
        } else {
            self.active_profile_id.as_deref()
        }?;
        self.config.connections.iter().find(|c| c.id == profile_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_db_sync_confirm(
        &mut self,
        title: impl Into<String>,
        src_ws_idx: Option<usize>,
        src_db: &str,
        tgt_ws_idx: Option<usize>,
        tgt_db: &str,
        extra_scope: Option<&str>,
        sql: String,
        reload_target_schema: bool,
    ) {
        let stmts: Vec<String> = split_sql_statements(&sql)
            .into_iter()
            .filter(|s| {
                s.lines().any(|line| {
                    let t = line.trim();
                    !t.is_empty() && !t.starts_with("--")
                })
            })
            .collect();
        if stmts.is_empty() {
            self.status_banner = "No executable SQL statements to sync.".to_string();
            return;
        }

        let src_srv = self.diff_server_label(src_ws_idx);
        let tgt_srv = self.diff_server_label(tgt_ws_idx);
        let suffix = extra_scope.map(|s| format!(" ({s})")).unwrap_or_default();
        let source_label = format!("{src_srv} / {src_db}{suffix}");
        let target_label = format!("{tgt_srv} / {tgt_db}{suffix}");

        let tgt_prof = self.diff_server_profile(tgt_ws_idx);
        let target_is_prod = tgt_prof.is_some_and(|p| p.environment == Environment::Production);
        let target_is_read_only = tgt_prof.is_some_and(|p| p.read_only);
        let target_is_local = tgt_srv.to_ascii_lowercase().contains("local")
            || tgt_prof.is_some_and(|p| {
                p.is_demo
                    || p.name.to_ascii_lowercase().contains("local")
                    || p.host.to_ascii_lowercase().contains("localhost")
            });

        self.db_sync_confirm_checked = false;
        self.db_sync_running = false;
        self.db_sync_progress = None;
        self.pending_db_sync = Some(PendingDbSync {
            title: title.into(),
            source_label,
            target_label,
            target_ws_idx: tgt_ws_idx,
            target_db: tgt_db.to_string(),
            target_is_prod,
            target_is_read_only,
            target_is_local,
            sql,
            statement_count: stmts.len(),
            reload_target_schema,
        });
    }
}
