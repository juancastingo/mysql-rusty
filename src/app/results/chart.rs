use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn draw_results_chart(ui: &mut Ui, tab: &mut QueryTab, result: &QueryResult) {
        if result.columns.is_empty() || result.rows.is_empty() {
            ui.label("No query result data available to chart.");
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Chart Type:").strong());
            ui.selectable_value(&mut tab.chart_type, ChartType::Bar, "Bar");
            ui.selectable_value(&mut tab.chart_type, ChartType::Line, "Line");

            ui.separator();
            ui.label(RichText::new("X Axis (Label):").strong());
            let current_x = result
                .columns
                .get(tab.chart_x_col)
                .cloned()
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("chart_x_axis_cb")
                .selected_text(current_x)
                .show_ui(ui, |ui| {
                    for (i, col) in result.columns.iter().enumerate() {
                        ui.selectable_value(&mut tab.chart_x_col, i, col);
                    }
                });

            ui.separator();
            ui.label(RichText::new("Y Axis (Metric):").strong());
            let current_y = result
                .columns
                .get(tab.chart_y_col)
                .cloned()
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("chart_y_axis_cb")
                .selected_text(current_y)
                .show_ui(ui, |ui| {
                    for (i, col) in result.columns.iter().enumerate() {
                        ui.selectable_value(&mut tab.chart_y_col, i, col);
                    }
                });
        });
        ui.separator();

        let y_col = tab.chart_y_col;
        let x_col = tab.chart_x_col;
        let chart_type = tab.chart_type;
        let col_name = result.columns.get(y_col).cloned().unwrap_or_default();

        Plot::new("result_visual_chart")
            .legend(egui_plot::Legend::default())
            .height(ui.available_height().max(250.0))
            .show(ui, |plot_ui| {
                if chart_type == ChartType::Bar {
                    let mut bars = Vec::new();
                    for (i, row) in result.rows.iter().enumerate() {
                        if let Some(cell) = row.get(y_col) {
                            let val = match cell {
                                crate::db::CellValue::Integer(v) => *v as f64,
                                crate::db::CellValue::Float(v) => *v,
                                crate::db::CellValue::Text(s) => s.parse::<f64>().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            let label =
                                row.get(x_col).map(|c| c.display_text()).unwrap_or_default();
                            bars.push(Bar::new(i as f64, val).width(0.65).name(label));
                        }
                    }
                    plot_ui.bar_chart(
                        BarChart::new(&col_name, bars).color(Color32::from_rgb(56, 189, 248)),
                    );
                } else {
                    let mut pts = Vec::new();
                    for (i, row) in result.rows.iter().enumerate() {
                        if let Some(cell) = row.get(y_col) {
                            let val = match cell {
                                crate::db::CellValue::Integer(v) => *v as f64,
                                crate::db::CellValue::Float(v) => *v,
                                crate::db::CellValue::Text(s) => s.parse::<f64>().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            pts.push([i as f64, val]);
                        }
                    }
                    plot_ui.line(
                        Line::new(&col_name, PlotPoints::new(pts))
                            .color(Color32::from_rgb(45, 212, 191))
                            .width(2.5),
                    );
                }
            });
    }
}
