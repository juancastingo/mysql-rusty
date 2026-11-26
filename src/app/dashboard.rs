use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_connections_dashboard(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_duplicate: Option<ConnectionProfile> = None;
        let mut to_delete = None;
        let mut toggle_fav_id: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt("connections_dashboard_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(pal.accent_bg)
                        .stroke(Stroke::new(1.0, pal.accent.gamma_multiply(0.4)))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(rust_i18n::t!("dashboard.badge"))
                                    .size(11.0)
                                    .color(pal.accent_text)
                                    .strong(),
                            );
                        });

                    ui.add_space(8.0);
                    ui.heading(
                        RichText::new(rust_i18n::t!("dashboard.heading"))
                            .size(24.0)
                            .color(pal.text_primary)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(rust_i18n::t!("dashboard.subtitle"))
                            .size(13.5)
                            .color(pal.muted_text),
                    );
                    ui.add_space(14.0);
                });

                let max_w = 960.0f32.min(ui.available_width() - 40.0).max(320.0);
                let left_pad = ((ui.available_width() - max_w) / 2.0).max(16.0);

                // Action & Filter Toolbar
                ui.horizontal(|ui| {
                    ui.add_space(left_pad);
                    ui.vertical(|ui| {
                        ui.set_width(max_w);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(pal.primary_button("+ New Connection"))
                                .clicked()
                            {
                                self.editing_connection = ConnectionProfile::default();
                                self.show_connection_modal = true;
                            }
                            if ui
                                .add(pal.subtle_button("Import Workbench"))
                                .on_hover_text(
                                    "Import connections from MySQL Workbench (connections.xml)",
                                )
                                .clicked()
                            {
                                self.import_workbench_connections();
                            }
                            if !self.config.trashed_connections.is_empty()
                                && ui
                                    .add(pal.subtle_button(format!(
                                        "Recycle Bin ({})",
                                        self.config.trashed_connections.len()
                                    )))
                                    .on_hover_text(
                                        "Restore or permanently delete trashed connections",
                                    )
                                    .clicked()
                            {
                                self.show_connection_trash_modal = true;
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let is_grid = self.config.ui.dashboard_grid_view;
                                    if ui
                                        .add(pal.icon_toggle_button("☰", !is_grid))
                                        .on_hover_text("List view")
                                        .clicked()
                                    {
                                        self.config.ui.dashboard_grid_view = false;
                                        let _ = self.config.save();
                                    }
                                    if ui
                                        .add(pal.icon_toggle_button("▦", is_grid))
                                        .on_hover_text("Grid view")
                                        .clicked()
                                    {
                                        self.config.ui.dashboard_grid_view = true;
                                        let _ = self.config.save();
                                    }
                                },
                            );
                        });

                        if !self.config.connections.is_empty() {
                            ui.add_space(10.0);
                            pal.compact_card_frame().show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.dashboard_search)
                                            .hint_text("Search connections (name, host, user, DB)...")
                                            .desired_width(240.0),
                                    );
                                    if !self.dashboard_search.is_empty()
                                        && ui
                                            .small_button("×")
                                            .on_hover_text("Clear search")
                                            .clicked()
                                    {
                                        self.dashboard_search.clear();
                                    }

                                    ui.separator();

                                    let all_active = self.dashboard_env_filter.is_none()
                                        && !self.dashboard_favorites_only;
                                    if ui.add(pal.pill_button("All", all_active)).clicked() {
                                        self.dashboard_env_filter = None;
                                        self.dashboard_favorites_only = false;
                                    }
                                    if ui
                                        .add(pal.pill_button(
                                            "★ Favorites",
                                            self.dashboard_favorites_only,
                                        ))
                                        .clicked()
                                    {
                                        self.dashboard_favorites_only =
                                            !self.dashboard_favorites_only;
                                    }
                                    for env in [
                                        Environment::Development,
                                        Environment::Staging,
                                        Environment::Production,
                                    ] {
                                        let sel = self.dashboard_env_filter == Some(env);
                                        if ui.add(pal.pill_button(env.label(), sel)).clicked() {
                                            self.dashboard_env_filter =
                                                if sel { None } else { Some(env) };
                                        }
                                    }
                                });
                            });
                        }
                    });
                });

                ui.add_space(16.0);

                if self.config.connections.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(left_pad);
                        ui.vertical(|ui| {
                            ui.set_width(max_w);
                            pal.card_frame().inner_margin(28.0).show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("No connection profiles yet")
                                            .size(16.0)
                                            .strong()
                                            .color(pal.text_primary),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(
                                            "Create a new MySQL, MariaDB, PostgreSQL, or SQLite connection, or try the built-in sandbox.",
                                        )
                                        .size(12.5)
                                        .color(pal.muted_text),
                                    );
                                    ui.add_space(14.0);
                                    if ui
                                        .add(pal.subtle_button("Restore Sandbox Demo Profile"))
                                        .clicked()
                                    {
                                        self.config
                                            .connections
                                            .push(ConnectionProfile::demo_profile());
                                        let _ = self.config.save();
                                    }
                                });
                            });
                        });
                    });
                } else {
                    let q = self.dashboard_search.trim().to_lowercase();
                    let mut profiles: Vec<ConnectionProfile> = self
                        .config
                        .connections
                        .iter()
                        .filter(|p| {
                            if self.dashboard_favorites_only && !p.is_favorite {
                                return false;
                            }
                            if let Some(env) = self.dashboard_env_filter
                                && p.environment != env
                            {
                                return false;
                            }
                            if !q.is_empty() {
                                let hay = format!(
                                    "{} {} {} {} {} {}",
                                    p.name,
                                    p.host,
                                    p.user,
                                    p.default_database,
                                    p.sqlite_path,
                                    p.dialect.display_name()
                                )
                                .to_lowercase();
                                if !hay.contains(&q) {
                                    return false;
                                }
                            }
                            true
                        })
                        .cloned()
                        .collect();

                    // Sort favorites to top while preserving relative order
                    profiles.sort_by_key(|p| !p.is_favorite);

                    if profiles.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(left_pad);
                            ui.vertical(|ui| {
                                ui.set_width(max_w);
                                pal.card_frame().inner_margin(24.0).show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("No connections match your filters")
                                                .size(15.0)
                                                .strong()
                                                .color(pal.text_primary),
                                        );
                                        ui.add_space(8.0);
                                        if ui.add(pal.subtle_button("Clear Filters")).clicked() {
                                            self.dashboard_search.clear();
                                            self.dashboard_env_filter = None;
                                            self.dashboard_favorites_only = false;
                                        }
                                    });
                                });
                            });
                        });
                    } else {
                        let use_grid = self.config.ui.dashboard_grid_view && max_w >= 680.0;
                        if use_grid {
                            let col_gap = 12.0;
                            let card_w = (max_w - col_gap) / 2.0;
                            for chunk in profiles.chunks(2) {
                                ui.horizontal(|ui| {
                                    ui.add_space(left_pad);
                                    for (i, prof) in chunk.iter().enumerate() {
                                        if i > 0 {
                                            ui.add_space(col_gap);
                                        }
                                        ui.vertical(|ui| {
                                            ui.set_width(card_w);
                                            self.render_dashboard_connection_card(
                                                ui,
                                                &pal,
                                                prof,
                                                true,
                                                &mut to_connect,
                                                &mut to_edit,
                                                &mut to_duplicate,
                                                &mut to_delete,
                                                &mut toggle_fav_id,
                                            );
                                        });
                                    }
                                });
                                ui.add_space(12.0);
                            }
                        } else {
                            for prof in &profiles {
                                ui.horizontal(|ui| {
                                    ui.add_space(left_pad);
                                    ui.vertical(|ui| {
                                        ui.set_width(max_w);
                                        self.render_dashboard_connection_card(
                                            ui,
                                            &pal,
                                            prof,
                                            false,
                                            &mut to_connect,
                                            &mut to_edit,
                                            &mut to_duplicate,
                                            &mut to_delete,
                                            &mut toggle_fav_id,
                                        );
                                    });
                                });
                                ui.add_space(10.0);
                            }
                        }
                    }
                }
                ui.add_space(30.0);
            });

        if let Some(fav_id) = toggle_fav_id
            && let Some(p) = self.config.connections.iter_mut().find(|c| c.id == fav_id)
        {
            p.is_favorite = !p.is_favorite;
            let _ = self.config.save();
        }
        if let Some(mut dup) = to_duplicate {
            dup.id = Uuid::new_v4().to_string();
            dup.name = format!("{} (Copy)", dup.name);
            dup.is_demo = false;
            self.config.connections.push(dup);
            let _ = self.config.save();
            self.status_banner = "Duplicated connection profile.".to_string();
        }
        if let Some(prof) = to_connect {
            self.connect_to_profile(prof);
        }
        if let Some(prof) = to_edit {
            self.editing_connection = prof;
            self.show_connection_modal = true;
        }
        if let Some(id) = to_delete {
            let name = self
                .config
                .connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "this connection".to_string());
            self.pending_delete_connection = Some((id, name));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_dashboard_connection_card(
        &self,
        ui: &mut egui::Ui,
        pal: &crate::theme::Palette,
        prof: &ConnectionProfile,
        compact_grid: bool,
        to_connect: &mut Option<ConnectionProfile>,
        to_edit: &mut Option<ConnectionProfile>,
        to_duplicate: &mut Option<ConnectionProfile>,
        to_delete: &mut Option<String>,
        toggle_fav_id: &mut Option<String>,
    ) {
        let is_demo = prof.is_demo;
        let card_accent = if is_demo {
            pal.success
        } else {
            self.parse_hex_color(&prof.color_hex)
        };

        let card_id = ui.make_persistent_id(("dashboard_card", &prof.id));
        let prev_hovered = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(card_id))
            .unwrap_or(false);

        let frame_resp = egui::Frame::new()
            .fill(if prev_hovered {
                pal.card_hover_bg
            } else {
                pal.card_bg
            })
            .stroke(Stroke::new(
                1.0,
                if prev_hovered {
                    pal.border_active
                } else {
                    pal.card_border
                },
            ))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(14.0)
            .show(ui, |ui| {
                let inner_resp = ui.horizontal(|ui| {
                    let bar_x = ui.cursor().min.x;
                    let bar_top = ui.cursor().min.y;
                    ui.add_space(10.0);

                    ui.vertical(|ui| {
                        // Title row with favorite star & badges
                        ui.horizontal_wrapped(|ui| {
                            let star_txt = if prof.is_favorite { "★" } else { "☆" };
                            let star_color = if prof.is_favorite {
                                pal.warning
                            } else {
                                pal.muted_text
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(star_txt).size(14.0).color(star_color),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(if prof.is_favorite {
                                    "Remove from favorites"
                                } else {
                                    "Pin to favorites"
                                })
                                .clicked()
                            {
                                *toggle_fav_id = Some(prof.id.clone());
                            }

                            ui.label(
                                RichText::new(&prof.name)
                                    .size(15.5)
                                    .color(pal.text_primary)
                                    .strong(),
                            );
                        });

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            pal.dialect_badge(ui, prof.dialect);
                            let (er, eg, eb) = prof.environment.color_rgb();
                            let (env_fg, env_bg) = pal.env_badge_colors(er, eg, eb);
                            pal.badge(
                                ui,
                                &prof.environment.label().to_uppercase(),
                                env_fg,
                                env_bg,
                            );
                            if is_demo {
                                pal.badge(ui, "SANDBOX", pal.success, pal.success_bg);
                            }
                            if prof.ssh.enabled {
                                pal.badge(ui, "SSH", pal.accent_text, pal.accent_bg);
                            }
                            if prof.read_only {
                                pal.badge(ui, "READ-ONLY", pal.warning, pal.warning_bg);
                            }
                        });

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if prof.dialect == crate::dialect::Dialect::Sqlite {
                                let path_str = if prof.sqlite_path.is_empty() {
                                    ":memory:"
                                } else {
                                    &prof.sqlite_path
                                };
                                ui.label(
                                    RichText::new(path_str)
                                        .size(11.5)
                                        .monospace()
                                        .color(pal.text_secondary),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!(
                                        "{}@{}:{}",
                                        prof.user, prof.host, prof.port
                                    ))
                                    .size(11.5)
                                    .monospace()
                                    .color(pal.text_secondary),
                                );
                                if !prof.default_database.is_empty() {
                                    ui.label(RichText::new("•").color(pal.muted_text));
                                    ui.label(
                                        RichText::new(&prof.default_database)
                                            .size(11.5)
                                            .color(pal.accent_text),
                                    );
                                }
                            }
                        });

                        if prof.ssh.enabled {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!(
                                    "SSH: {}@{}:{}",
                                    prof.ssh.ssh_user, prof.ssh.ssh_host, prof.ssh.ssh_port
                                ))
                                .size(11.0)
                                .color(pal.muted_text),
                            );
                        }

                        if compact_grid {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let connect_btn = if is_demo {
                                    egui::Button::new(
                                        RichText::new("Connect")
                                            .size(12.5)
                                            .color(Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(Color32::from_rgb(5, 150, 105))
                                    .corner_radius(egui::CornerRadius::same(6))
                                } else {
                                    pal.primary_button("Connect")
                                };
                                if ui.add(connect_btn).clicked() {
                                    *to_connect = Some(prof.clone());
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(RichText::new("Delete").color(pal.danger))
                                            .clicked()
                                        {
                                            *to_delete = Some(prof.id.clone());
                                        }
                                        if ui
                                            .small_button("Clone")
                                            .on_hover_text("Duplicate connection profile")
                                            .clicked()
                                        {
                                            *to_duplicate = Some(prof.clone());
                                        }
                                        if ui.small_button("Edit").clicked() {
                                            *to_edit = Some(prof.clone());
                                        }
                                    },
                                );
                            });
                        }
                    });

                    if !compact_grid {
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let connect_btn = if is_demo {
                                    egui::Button::new(
                                        RichText::new("Connect")
                                            .size(13.0)
                                            .color(Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(Color32::from_rgb(5, 150, 105))
                                    .corner_radius(egui::CornerRadius::same(6))
                                } else {
                                    pal.primary_button("Connect")
                                };
                                if ui.add(connect_btn).clicked() {
                                    *to_connect = Some(prof.clone());
                                }
                                if ui.button("Edit").clicked() {
                                    *to_edit = Some(prof.clone());
                                }
                                if ui
                                    .button("Clone")
                                    .on_hover_text("Duplicate connection profile")
                                    .clicked()
                                {
                                    *to_duplicate = Some(prof.clone());
                                }
                                if ui
                                    .button(RichText::new("Delete").color(pal.danger))
                                    .clicked()
                                {
                                    *to_delete = Some(prof.id.clone());
                                }
                            },
                        );
                    }

                    (bar_x, bar_top)
                });

                let (bar_x, bar_top) = inner_resp.inner;
                let bar_bottom = inner_resp.response.rect.bottom();
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_x, bar_top),
                    egui::pos2(bar_x + 4.0, bar_bottom.max(bar_top + 40.0)),
                );
                ui.painter().rect_filled(
                    bar_rect,
                    egui::CornerRadius::same(2),
                    card_accent,
                );
            });

        let hovered = frame_resp.response.hovered();
        ui.ctx().data_mut(|d| d.insert_temp(card_id, hovered));

        if frame_resp
            .response
            .interact(egui::Sense::click())
            .double_clicked()
        {
            *to_connect = Some(prof.clone());
        }
    }
}
