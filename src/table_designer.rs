use crate::db::{ColumnInfo, ForeignKeyInfo, IndexInfo, TableSchema};
use egui::{Color32, RichText, Ui};

#[derive(Debug, Clone)]
pub struct TableDraft {
    pub original_table: Option<TableSchema>,
    pub table_name: String,
    pub engine: String,
    pub charset: String,
    pub columns: Vec<ColumnInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub indexes: Vec<IndexInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableDesignerAction {
    None,
    ApplyDdl(String),
    OpenInQueryTab(String),
    Close,
}

impl TableDraft {
    pub fn new_empty(table_name: impl Into<String>) -> Self {
        Self {
            original_table: None,
            table_name: table_name.into(),
            engine: "InnoDB".to_string(),
            charset: "utf8mb4".to_string(),
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
                    name: "created_at".to_string(),
                    data_type: "DATETIME".to_string(),
                    is_nullable: false,
                    is_primary_key: false,
                    default_value: Some("CURRENT_TIMESTAMP".to_string()),
                    extra: String::new(),
                },
            ],
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
        }
    }

    pub fn from_table(table: &TableSchema) -> Self {
        Self {
            original_table: Some(table.clone()),
            table_name: table.name.clone(),
            engine: if table.engine.is_empty() {
                "InnoDB".to_string()
            } else {
                table.engine.clone()
            },
            charset: "utf8mb4".to_string(),
            columns: table.columns.clone(),
            foreign_keys: table.foreign_keys.clone(),
            indexes: table.indexes.clone(),
        }
    }

    pub fn is_create_mode(&self) -> bool {
        self.original_table.is_none()
    }

    pub fn generate_ddl_diff(&self) -> String {
        match &self.original_table {
            None => self.generate_create_table_ddl(),
            Some(old) => self.generate_alter_table_ddl(old),
        }
    }

    fn format_col_def(c: &ColumnInfo) -> String {
        let mut def = format!("`{}` {}", c.name, c.data_type);
        if c.is_nullable {
            def.push_str(" NULL");
        } else {
            def.push_str(" NOT NULL");
        }

        if let Some(def_val) = &c.default_value {
            let val = def_val.trim();
            if !val.is_empty() {
                if val.eq_ignore_ascii_case("NULL")
                    || val.eq_ignore_ascii_case("CURRENT_TIMESTAMP")
                    || val.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                    || (val.starts_with('\'') && val.ends_with('\''))
                {
                    def.push_str(&format!(" DEFAULT {val}"));
                } else {
                    def.push_str(&format!(" DEFAULT '{val}'"));
                }
            }
        }

        if !c.extra.trim().is_empty() {
            def.push_str(&format!(" {}", c.extra.trim().to_uppercase()));
        }

        def
    }

    fn generate_create_table_ddl(&self) -> String {
        let mut elements: Vec<String> = Vec::new();

        for col in &self.columns {
            elements.push(format!("  {}", Self::format_col_def(col)));
        }

        let pks: Vec<String> = self
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| format!("`{}`", c.name))
            .collect();

        if !pks.is_empty() {
            elements.push(format!("  PRIMARY KEY ({})", pks.join(", ")));
        }

        for idx in &self.indexes {
            if idx.index_name.eq_ignore_ascii_case("PRIMARY") {
                continue;
            }
            if !idx.non_unique {
                elements.push(format!(
                    "  UNIQUE KEY `{}` (`{}`)",
                    idx.index_name, idx.column_name
                ));
            } else {
                elements.push(format!(
                    "  KEY `{}` (`{}`)",
                    idx.index_name, idx.column_name
                ));
            }
        }

        for fk in &self.foreign_keys {
            elements.push(format!(
                "  CONSTRAINT `{}` FOREIGN KEY (`{}`) REFERENCES `{}` (`{}`)",
                fk.constraint_name, fk.column_name, fk.referenced_table, fk.referenced_column
            ));
        }

        format!(
            "CREATE TABLE `{}` (\n{}\n) ENGINE={} DEFAULT CHARSET={};",
            self.table_name,
            elements.join(",\n"),
            if self.engine.is_empty() {
                "InnoDB"
            } else {
                &self.engine
            },
            if self.charset.is_empty() {
                "utf8mb4"
            } else {
                &self.charset
            }
        )
    }

    fn generate_alter_table_ddl(&self, old: &TableSchema) -> String {
        let mut stmts: Vec<String> = Vec::new();
        let target_tbl = &self.table_name;

        // Table rename
        if old.name != self.table_name {
            stmts.push(format!(
                "ALTER TABLE `{}` RENAME TO `{}`;",
                old.name, target_tbl
            ));
        }

        // Columns diff
        let old_col_map: std::collections::HashMap<&str, &ColumnInfo> =
            old.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let new_col_map: std::collections::HashMap<&str, &ColumnInfo> =
            self.columns.iter().map(|c| (c.name.as_str(), c)).collect();

        // Added columns
        for col in &self.columns {
            if !old_col_map.contains_key(col.name.as_str()) {
                stmts.push(format!(
                    "ALTER TABLE `{}` ADD COLUMN {};",
                    target_tbl,
                    Self::format_col_def(col)
                ));
            }
        }

        // Modified columns
        for col in &self.columns {
            if let Some(old_col) = old_col_map.get(col.name.as_str()) {
                let changed = old_col.data_type != col.data_type
                    || old_col.is_nullable != col.is_nullable
                    || old_col.default_value != col.default_value
                    || old_col.extra != col.extra;
                if changed {
                    stmts.push(format!(
                        "ALTER TABLE `{}` MODIFY COLUMN {};",
                        target_tbl,
                        Self::format_col_def(col)
                    ));
                }
            }
        }

        // Dropped columns
        for old_col in &old.columns {
            if !new_col_map.contains_key(old_col.name.as_str()) {
                stmts.push(format!(
                    "ALTER TABLE `{}` DROP COLUMN `{}`;",
                    target_tbl, old_col.name
                ));
            }
        }

        // Indexes diff
        let old_idx_map: std::collections::HashMap<&str, &IndexInfo> = old
            .indexes
            .iter()
            .map(|i| (i.index_name.as_str(), i))
            .collect();
        let new_idx_map: std::collections::HashMap<&str, &IndexInfo> = self
            .indexes
            .iter()
            .map(|i| (i.index_name.as_str(), i))
            .collect();

        // Dropped indexes
        for old_idx in &old.indexes {
            if old_idx.index_name.eq_ignore_ascii_case("PRIMARY") {
                continue;
            }
            if !new_idx_map.contains_key(old_idx.index_name.as_str()) {
                stmts.push(format!(
                    "ALTER TABLE `{}` DROP INDEX `{}`;",
                    target_tbl, old_idx.index_name
                ));
            }
        }

        // Added indexes
        for idx in &self.indexes {
            if idx.index_name.eq_ignore_ascii_case("PRIMARY") {
                continue;
            }
            if !old_idx_map.contains_key(idx.index_name.as_str()) {
                let uniq = if !idx.non_unique { "UNIQUE " } else { "" };
                stmts.push(format!(
                    "ALTER TABLE `{}` ADD {}INDEX `{}` (`{}`);",
                    target_tbl, uniq, idx.index_name, idx.column_name
                ));
            }
        }

        // Foreign keys diff
        let old_fk_map: std::collections::HashMap<&str, &ForeignKeyInfo> = old
            .foreign_keys
            .iter()
            .map(|f| (f.constraint_name.as_str(), f))
            .collect();
        let new_fk_map: std::collections::HashMap<&str, &ForeignKeyInfo> = self
            .foreign_keys
            .iter()
            .map(|f| (f.constraint_name.as_str(), f))
            .collect();

        // Dropped FKs
        for old_fk in &old.foreign_keys {
            if !new_fk_map.contains_key(old_fk.constraint_name.as_str()) {
                stmts.push(format!(
                    "ALTER TABLE `{}` DROP FOREIGN KEY `{}`;",
                    target_tbl, old_fk.constraint_name
                ));
            }
        }

        // Added FKs
        for fk in &self.foreign_keys {
            if !old_fk_map.contains_key(fk.constraint_name.as_str()) {
                stmts.push(format!(
                    "ALTER TABLE `{}` ADD CONSTRAINT `{}` FOREIGN KEY (`{}`) REFERENCES `{}` (`{}`);",
                    target_tbl, fk.constraint_name, fk.column_name, fk.referenced_table, fk.referenced_column
                ));
            }
        }

        if stmts.is_empty() {
            format!("-- No changes detected for table `{target_tbl}`")
        } else {
            stmts.join("\n")
        }
    }

    pub fn generate_up_down_migration(&self) -> (String, String) {
        let up = self.generate_ddl_diff();
        let down = if let Some(old) = &self.original_table {
            let mut stmts = Vec::new();
            let target_tbl = &self.table_name;
            let old_col_map: std::collections::HashMap<&str, &ColumnInfo> =
                old.columns.iter().map(|c| (c.name.as_str(), c)).collect();
            let new_col_map: std::collections::HashMap<&str, &ColumnInfo> =
                self.columns.iter().map(|c| (c.name.as_str(), c)).collect();

            // Reverse added columns -> DROP COLUMN
            for col in &self.columns {
                if !old_col_map.contains_key(col.name.as_str()) {
                    stmts.push(format!(
                        "ALTER TABLE `{target_tbl}` DROP COLUMN `{}`;",
                        col.name
                    ));
                }
            }
            // Reverse modified columns -> revert to old col definition
            for col in &self.columns {
                if let Some(old_col) = old_col_map.get(col.name.as_str()) {
                    let changed = old_col.data_type != col.data_type
                        || old_col.is_nullable != col.is_nullable
                        || old_col.default_value != col.default_value
                        || old_col.extra != col.extra;
                    if changed {
                        stmts.push(format!(
                            "ALTER TABLE `{target_tbl}` MODIFY COLUMN {};",
                            Self::format_col_def(old_col)
                        ));
                    }
                }
            }
            // Reverse dropped columns -> ADD COLUMN back
            for old_col in &old.columns {
                if !new_col_map.contains_key(old_col.name.as_str()) {
                    stmts.push(format!(
                        "ALTER TABLE `{target_tbl}` ADD COLUMN {};",
                        Self::format_col_def(old_col)
                    ));
                }
            }
            if old.name != self.table_name {
                stmts.push(format!(
                    "ALTER TABLE `{target_tbl}` RENAME TO `{}`;",
                    old.name
                ));
            }
            if stmts.is_empty() {
                "-- No rollback operations required".to_string()
            } else {
                stmts.join("\n")
            }
        } else {
            format!("DROP TABLE IF EXISTS `{}`;", self.table_name)
        };

        (up, down)
    }

    pub fn generate_migration_file_content(&self) -> String {
        let (up, down) = self.generate_up_down_migration();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        format!(
            "-- ========================================================\n-- Migration for `{}` generated at {}\n-- ========================================================\n\n-- >>> UP MIGRATION (APPLY)\n{}\n\n-- <<< DOWN MIGRATION (ROLLBACK)\n{}\n",
            self.table_name, now, up, down
        )
    }

    pub fn render_editor(&mut self, ui: &mut Ui) -> TableDesignerAction {
        let mut action = TableDesignerAction::None;

        ui.horizontal(|ui| {
            let mode_str = if self.is_create_mode() {
                "CREATE TABLE"
            } else {
                "ALTER TABLE"
            };
            ui.heading(
                RichText::new(format!("{mode_str}: `{}`", self.table_name))
                    .color(Color32::from_rgb(56, 189, 248))
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    action = TableDesignerAction::Close;
                }
            });
        });
        ui.separator();

        // Table metadata
        ui.horizontal(|ui| {
            ui.label(RichText::new("Table Name:").strong());
            ui.add(egui::TextEdit::singleline(&mut self.table_name).desired_width(180.0));

            ui.add_space(16.0);
            ui.label(RichText::new("Engine:").strong());
            egui::ComboBox::from_id_salt("designer_engine_select")
                .selected_text(&self.engine)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.engine, "InnoDB".to_string(), "InnoDB");
                    ui.selectable_value(&mut self.engine, "MyISAM".to_string(), "MyISAM");
                    ui.selectable_value(&mut self.engine, "MEMORY".to_string(), "MEMORY");
                });

            ui.add_space(16.0);
            ui.label(RichText::new("Charset:").strong());
            egui::ComboBox::from_id_salt("designer_charset_select")
                .selected_text(&self.charset)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.charset, "utf8mb4".to_string(), "utf8mb4");
                    ui.selectable_value(&mut self.charset, "utf8".to_string(), "utf8");
                    ui.selectable_value(&mut self.charset, "latin1".to_string(), "latin1");
                });
        });
        ui.separator();

        // Tabs or sections for Columns, Indexes, FKs
        ui.collapsing(
            RichText::new(format!("Columns ({})", self.columns.len())).strong(),
            |ui| {
                let mut remove_idx = None;
                egui::Grid::new("designer_columns_grid")
                    .striped(true)
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Type");
                        ui.strong("PK");
                        ui.strong("Nullable");
                        ui.strong("Default Value");
                        ui.strong("Extra");
                        ui.strong("Actions");
                        ui.end_row();

                        for (idx, col) in self.columns.iter_mut().enumerate() {
                            ui.add(egui::TextEdit::singleline(&mut col.name).desired_width(140.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut col.data_type).desired_width(120.0),
                            );
                            ui.checkbox(&mut col.is_primary_key, "");
                            ui.checkbox(&mut col.is_nullable, "");

                            let mut def_buf = col.default_value.clone().unwrap_or_default();
                            if ui
                                .add(egui::TextEdit::singleline(&mut def_buf).desired_width(100.0))
                                .changed()
                            {
                                col.default_value = if def_buf.trim().is_empty() {
                                    None
                                } else {
                                    Some(def_buf)
                                };
                            }

                            ui.add(egui::TextEdit::singleline(&mut col.extra).desired_width(100.0));
                            if ui
                                .small_button("Del")
                                .on_hover_text("Remove column")
                                .clicked()
                            {
                                remove_idx = Some(idx);
                            }
                            ui.end_row();
                        }
                    });

                if let Some(idx) = remove_idx {
                    if self.columns.len() > 1 {
                        self.columns.remove(idx);
                    }
                }

                if ui.button("+ Add Column").clicked() {
                    let n = self.columns.len() + 1;
                    self.columns.push(ColumnInfo {
                        name: format!("col_{n}"),
                        data_type: "VARCHAR(255)".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        extra: String::new(),
                    });
                }
            },
        );

        ui.add_space(4.0);
        ui.collapsing(
            RichText::new(format!("Indexes ({})", self.indexes.len())).strong(),
            |ui| {
                let mut remove_idx = None;
                egui::Grid::new("designer_indexes_grid")
                    .striped(true)
                    .min_col_width(100.0)
                    .show(ui, |ui| {
                        ui.strong("Index Name");
                        ui.strong("Column");
                        ui.strong("Unique");
                        ui.strong("Actions");
                        ui.end_row();

                        for (idx, item) in self.indexes.iter_mut().enumerate() {
                            ui.add(
                                egui::TextEdit::singleline(&mut item.index_name)
                                    .desired_width(160.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut item.column_name)
                                    .desired_width(140.0),
                            );
                            let mut is_unique = !item.non_unique;
                            if ui.checkbox(&mut is_unique, "").changed() {
                                item.non_unique = !is_unique;
                            }
                            if ui.small_button("Del").clicked() {
                                remove_idx = Some(idx);
                            }
                            ui.end_row();
                        }
                    });

                if let Some(idx) = remove_idx {
                    self.indexes.remove(idx);
                }

                if ui.button("+ Add Index").clicked() {
                    let n = self.indexes.len() + 1;
                    self.indexes.push(IndexInfo {
                        index_name: format!("idx_{n}"),
                        column_name: self
                            .columns
                            .first()
                            .map(|c| c.name.clone())
                            .unwrap_or_default(),
                        non_unique: true,
                        index_type: "BTREE".to_string(),
                    });
                }
            },
        );

        ui.add_space(4.0);
        ui.collapsing(
            RichText::new(format!("Foreign Keys ({})", self.foreign_keys.len())).strong(),
            |ui| {
                let mut remove_idx = None;
                egui::Grid::new("designer_fks_grid")
                    .striped(true)
                    .min_col_width(100.0)
                    .show(ui, |ui| {
                        ui.strong("Constraint Name");
                        ui.strong("Source Column");
                        ui.strong("Ref Table");
                        ui.strong("Ref Column");
                        ui.strong("Actions");
                        ui.end_row();

                        for (idx, fk) in self.foreign_keys.iter_mut().enumerate() {
                            ui.add(
                                egui::TextEdit::singleline(&mut fk.constraint_name)
                                    .desired_width(140.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut fk.column_name)
                                    .desired_width(120.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut fk.referenced_table)
                                    .desired_width(120.0),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut fk.referenced_column)
                                    .desired_width(120.0),
                            );
                            if ui.small_button("Del").clicked() {
                                remove_idx = Some(idx);
                            }
                            ui.end_row();
                        }
                    });

                if let Some(idx) = remove_idx {
                    self.foreign_keys.remove(idx);
                }

                if ui.button("+ Add Foreign Key").clicked() {
                    let n = self.foreign_keys.len() + 1;
                    self.foreign_keys.push(ForeignKeyInfo {
                        constraint_name: format!("fk_{}_{n}", self.table_name),
                        column_name: "user_id".to_string(),
                        referenced_table: "users".to_string(),
                        referenced_column: "id".to_string(),
                    });
                }
            },
        );

        ui.separator();

        // Live DDL preview section
        let ddl = self.generate_ddl_diff();
        ui.label(
            RichText::new("Live DDL Preview:")
                .strong()
                .color(Color32::from_rgb(163, 230, 53)),
        );
        egui::ScrollArea::vertical()
            .id_salt("designer_ddl_preview_scroll")
            .max_height(140.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut ddl.as_str())
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let apply_btn = egui::Button::new(
                RichText::new("Apply DDL")
                    .color(Color32::from_rgb(16, 185, 129))
                    .strong(),
            );
            if ui.add(apply_btn).clicked() {
                action = TableDesignerAction::ApplyDdl(ddl.clone());
            }

            let open_tab_btn = egui::Button::new(
                RichText::new("Open in Query Tab").color(Color32::from_rgb(56, 189, 248)),
            );
            if ui.add(open_tab_btn).clicked() {
                action = TableDesignerAction::OpenInQueryTab(ddl.clone());
            }

            let migration_btn = egui::Button::new(
                RichText::new("Generate Migration (Up/Down)")
                    .color(Color32::from_rgb(168, 85, 247)),
            );
            if ui
                .add(migration_btn)
                .on_hover_text(
                    "Generate structured UP/DOWN migration script in a new SQL query tab",
                )
                .clicked()
            {
                action =
                    TableDesignerAction::OpenInQueryTab(self.generate_migration_file_content());
            }

            if ui.button("Cancel").clicked() {
                action = TableDesignerAction::Close;
            }
        });

        action
    }
}
