//! Manage > Technical Standards (Normas Técnicas): configure national and international drawing standards
//! (NBR, ISO, ANSI/AIA, EN, JIS), adapting line weights, sheet formats, margins, object styles, and dimensioning.

use bimcraft_core::model::Model;
use bimcraft_core::standards::TechnicalStandard;
use egui::{Align, ComboBox, Context, Grid, Id as UiId, Layout, RichText, ScrollArea, Ui, WidgetInfo, WidgetType};

use crate::popup::{self, Modal};
use serde_json::json;

use crate::BimApp;
use crate::lang::{category, t, tf};

const TABS: [&str; 5] = ["Overview", "Line Weights", "Sheets & Margins", "Object Styles", "Dimensions & Annotations"];

/// The Technical Standards dialog.
pub(crate) struct StandardsDialog {
    pub(crate) selected: TechnicalStandard,
    pub(crate) tab: usize,
    pub(crate) adapt_line_weights: bool,
    pub(crate) adapt_object_styles: bool,
    pub(crate) adapt_units: bool,
}

impl StandardsDialog {
    pub(crate) fn new(m: &Model) -> StandardsDialog {
        StandardsDialog { selected: m.standard(), tab: 0, adapt_line_weights: true, adapt_object_styles: true, adapt_units: true }
    }
}

/// Overview tab: standards summary, projection angle, governing bodies, and reference documents.
fn overview_tab(ui: &mut Ui, std: TechnicalStandard) {
    ui.label(RichText::new(t(std.name())).strong());
    ui.add_space(2.0);
    ui.label(t(std.description()));
    ui.add_space(6.0);

    Grid::new("std-overview-grid").striped(true).num_columns(2).min_col_width(180.0).show(ui, |ui| {
        ui.label(RichText::new(t("Region / Country:")).strong());
        ui.label(t(std.region()));
        ui.end_row();

        ui.label(RichText::new(t("Governing Body:")).strong());
        ui.label(std.governing_body());
        ui.end_row();

        ui.label(RichText::new(t("Projection Method:")).strong());
        ui.label(t(std.projection_method().name()));
        ui.end_row();

        ui.label(RichText::new(t("Suggested Units:")).strong());
        ui.label(t(std.default_units().label()));
        ui.end_row();

        ui.label(RichText::new(t("Default Sheet Size:")).strong());
        ui.label(std.default_sheet_size());
        ui.end_row();
    });

    ui.add_space(8.0);
    ui.label(RichText::new(t("Standards Included in this Preset:")).strong());
    ui.add_space(3.0);
    for r in std.reference_standards() {
        ui.horizontal(|ui| {
            ui.label("•");
            ui.label(*r);
        });
    }
}

/// Line weights tab: standard pen widths and architectural hierarchy.
fn line_weights_tab(ui: &mut Ui, std: TechnicalStandard) {
    ui.label(t("Pen widths conforming to the selected standard: fine, medium, and heavy line hierarchies on paper."));
    ui.add_space(4.0);

    match std {
        TechnicalStandard::Nbr => {
            ui.label(RichText::new(t("NBR 16861 / NBR 6492 Architectural Line Hierarchy:")).strong());
            ui.label(t("• Thick line (0.50 mm — Pen 10): Walls and structural elements cut in plan and section."));
            ui.label(t("• Very thick line (0.70 mm — Pen 12): Prominent structural reinforced concrete columns and beams cut."));
            ui.label(t("• Medium line (0.35 mm — Pen 7): Doors, windows, millwork, and stair risers."));
            ui.label(t("• Fine line (0.18 mm — Pen 3 / 4): Projections, dimension lines, witness lines, grid lines, and axes."));
            ui.label(t("• Extra fine line (0.13 mm — Pen 1): Material hatching and floor tile patterns."));
        }
        TechnicalStandard::Iso | TechnicalStandard::Eu => {
            ui.label(RichText::new(t("ISO 128 / EN ISO 128 Line Series:")).strong());
            ui.label(t("• Standard line width ratio 1 : √2 (0.18, 0.25, 0.35, 0.50, 0.70, 1.00 mm)."));
            ui.label(t("• Cut lines: 0.50 mm (Pen 10) for main building elements."));
            ui.label(t("• Outlines in view: 0.25 mm (Pen 5) or 0.35 mm (Pen 7)."));
            ui.label(t("• Annotation and hatching: 0.18 mm (Pen 3 / 4)."));
        }
        TechnicalStandard::Ansi => {
            ui.label(RichText::new(t("ASME Y14.2 & AIA CAD National CAD Standard (NCS):")).strong());
            ui.label(t("• Wide / Heavy (0.50 mm — 0.70 mm): Cut walls and structural sections."));
            ui.label(t("• Medium (0.35 mm): Secondary elements, fixtures, and door swings."));
            ui.label(t("• Thin / Fine (0.18 mm — 0.25 mm): Dimension lines, witness lines, text notes, and crosshatching."));
        }
        TechnicalStandard::Jis => {
            ui.label(RichText::new(t("JIS Z 8310 / JIS A 0150 Line Standards:")).strong());
            ui.label(t("• Standard line groups: 0.18, 0.25, 0.35, 0.50, 0.70 mm."));
        }
    }

    ui.add_space(8.0);
    let lw = std.line_weights(std.default_units());
    ui.label(RichText::new(t("Annotation Pens (mm on paper):")).strong());
    ui.add_space(2.0);
    ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
        Grid::new("std-lw-grid").striped(true).min_col_width(70.0).show(ui, |ui| {
            ui.label(RichText::new(t("Pen")).strong());
            ui.label(RichText::new(t("Width (mm)")).strong());
            ui.end_row();
            for (i, w) in lw.annotation.iter().enumerate() {
                ui.label((i + 1).to_string());
                ui.label(format!("{w:.2} mm"));
                ui.end_row();
            }
        });
    });
}

/// Sheets and margins tab: binding margin, title block, and folding rules.
fn sheets_tab(ui: &mut Ui, std: TechnicalStandard) {
    ui.label(t("Paper sizes, sheet borders, filing margins, title blocks, and folding specifications."));
    ui.add_space(6.0);

    let margins_a1 = std.sheet_margins("A1");
    let margins_a3 = std.sheet_margins("A3");

    Grid::new("std-sheets-grid").striped(true).num_columns(2).min_col_width(220.0).show(ui, |ui| {
        ui.label(RichText::new(t("Left Filing / Binding Margin:")).strong());
        ui.label(format!(
            "{:.1} mm ({})",
            margins_a1.left,
            match std {
                TechnicalStandard::Nbr => t("NBR 16752: 25 mm on all formats"),
                TechnicalStandard::Iso | TechnicalStandard::Eu | TechnicalStandard::Jis => t("ISO 5457: 20 mm filing margin"),
                TechnicalStandard::Ansi => t("0.75 in (19.05 mm) binding margin"),
            }
        ));
        ui.end_row();

        ui.label(RichText::new(t("Other Borders (Top, Right, Bottom):")).strong());
        ui.label(match std {
            TechnicalStandard::Nbr => format!(
                "{} / {}",
                tf("{} mm for A0/A1", &[&margins_a1.top.to_string()]),
                tf("{} mm for A2-A4", &[&margins_a3.top.to_string()])
            ),
            TechnicalStandard::Iso | TechnicalStandard::Eu | TechnicalStandard::Jis => format!("{:.1} mm (ISO 5457)", margins_a1.top),
            TechnicalStandard::Ansi => "12.7 mm (0.5 in)".to_string(),
        });
        ui.end_row();

        ui.label(RichText::new(t("Title Block Width:")).strong());
        ui.label(format!(
            "{:.0} mm ({})",
            margins_a1.title_width,
            match std {
                TechnicalStandard::Nbr => t("NBR 16752: max 178 mm"),
                TechnicalStandard::Iso | TechnicalStandard::Eu | TechnicalStandard::Jis => t("ISO 7200: 180 mm"),
                TechnicalStandard::Ansi => t("ASME / AIA: 180 mm"),
            }
        ));
        ui.end_row();

        ui.label(RichText::new(t("Folding Marks:")).strong());
        ui.label(match std {
            TechnicalStandard::Nbr => t("Active (NBR 17067: A4 module folding with ear)"),
            _ => t("Inactive (flat storage)"),
        });
        ui.end_row();
    });

    if std == TechnicalStandard::Nbr {
        ui.add_space(6.0);
        ui.label(RichText::new(t("NBR 17067 Folding Rules (Dobramento de Folha):")).strong());
        ui.label(t("• Final folded format is A4 (210 mm x 297 mm) for vertical filing and binder archival."));
        ui.label(t("• Title block (legenda) is located at bottom right and remains visible on the folded cover."));
        ui.label(t("• 25 mm left binding margin is preserved without punching through drawing contents."));
        ui.label(t("• Large formats (A0, A1, A2) include a 105 mm triangular ear fold on top-left."));
    }
}

/// Object styles tab: model category pens.
fn object_styles_tab(ui: &mut Ui, std: TechnicalStandard) {
    ui.label(t("Pen assignments by model category: how walls, columns, doors, windows, and floors draw."));
    ui.add_space(6.0);

    let styles = std.object_styles();
    ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        Grid::new("std-obj-styles-grid").striped(true).num_columns(3).min_col_width(120.0).show(ui, |ui| {
            ui.label(RichText::new(t("Category")).strong());
            ui.label(RichText::new(t("Projection Pen")).strong());
            ui.label(RichText::new(t("Cut Pen")).strong());
            ui.end_row();

            for (cat, style) in styles {
                ui.label(category(cat, true));
                ui.label(format!("{} (Pen {})", t("Projection"), style.projection.0));
                ui.label(format!("{} (Pen {})", t("Cut"), style.cut.0));
                ui.end_row();
            }
        });
    });
}

/// Dimensioning tab: dimension terminators, text placement, witness lines, and level symbols.
fn dimensions_tab(ui: &mut Ui, std: TechnicalStandard) {
    ui.label(t("Dimension lines, witness lines, terminators, text placement, and level marker symbols."));
    ui.add_space(6.0);

    let dim = std.dimension_style();

    Grid::new("std-dim-grid").striped(true).num_columns(2).min_col_width(200.0).show(ui, |ui| {
        ui.label(RichText::new(t("Dimension Terminator:")).strong());
        ui.label(match dim.terminator {
            bimcraft_core::standards::Terminator::Tick45 => t("45° Oblique Stroke (Tick)"),
            bimcraft_core::standards::Terminator::FilledArrow => t("Filled Arrowhead"),
        });
        ui.end_row();

        ui.label(RichText::new(t("Dimension Text Placement:")).strong());
        ui.label(match dim.text_placement {
            bimcraft_core::standards::TextPlacement::AboveLine => t("Above dimension line (continuous)"),
            bimcraft_core::standards::TextPlacement::InlineBreak => t("Centered in dimension line break"),
        });
        ui.end_row();

        ui.label(RichText::new(t("Witness Line Overshoot:")).strong());
        ui.label(format!(
            "{:.1} mm ({})",
            dim.witness_overshoot_mm,
            match std {
                TechnicalStandard::Nbr => t("NBR 17068: 2 to 3 mm extension"),
                _ => t("standard overshoot"),
            }
        ));
        ui.end_row();

        ui.label(RichText::new(t("Witness Line Gap:")).strong());
        ui.label(format!(
            "{:.1} mm ({})",
            dim.witness_gap_mm,
            match std {
                TechnicalStandard::Nbr => t("NBR 17068: 1 to 2 mm clearance"),
                _ => t("standard gap"),
            }
        ));
        ui.end_row();

        ui.label(RichText::new(t("Nominal Text Height:")).strong());
        ui.label(format!("{:.1} mm (NBR 16861 / ISO 3098)", dim.text_height_mm));
        ui.end_row();

        ui.label(RichText::new(t("Level Marker Symbol:")).strong());
        ui.label(match std {
            TechnicalStandard::Nbr => t("NBR 6492: Target in plan (±0.00), equilateral triangle in section"),
            _ => t("Standard elevation marker triangle"),
        });
        ui.end_row();
    });
}

impl BimApp {
    /// The Technical Standards modal dialog.
    pub(crate) fn standards_window(&mut self, ctx: &Context) {
        let Some(mut d) = self.standards_dialog.take() else { return };
        let (mut ok, mut apply, mut cancel) = (false, false, false);

        let r = Modal::new(UiId::new("technical-standards-dialog"), t("Technical Standards")).width(740.0).show(ctx, |ui| {
          popup::body(ui, |ui| {
            ui.add_space(4.0);
            ui.label(t(
                "Configure national and international technical drawing standards (NBR, ISO, ANSI, EU). Adapting the program applies standard line weights, sheet formats, object styles, and dimensioning conventions.",
            ));
            ui.add_space(8.0);

            // Standard selector:
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("Standard Preset:")).strong());
                let cur = d.selected;
                let c = ComboBox::from_id_salt("standards-selector")
                    .selected_text(t(cur.label()))
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        for s in TechnicalStandard::ALL {
                            if ui.selectable_value(&mut d.selected, s, t(s.label())).clicked() {
                                d.selected = s;
                            }
                        }
                    });
                c.response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, t("Standard Preset")));
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (i, label) in TABS.iter().enumerate() {
                    if ui.selectable_label(d.tab == i, t(label)).clicked() {
                        d.tab = i;
                    }
                }
            });
            ui.separator();

            ScrollArea::vertical().max_height(340.0).show(ui, |ui| match d.tab {
                0 => overview_tab(ui, d.selected),
                1 => line_weights_tab(ui, d.selected),
                2 => sheets_tab(ui, d.selected),
                3 => object_styles_tab(ui, d.selected),
                _ => dimensions_tab(ui, d.selected),
            });

            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.adapt_line_weights, t("Adapt Line Weights"));
                ui.checkbox(&mut d.adapt_object_styles, t("Adapt Object Styles"));
                ui.checkbox(&mut d.adapt_units, t("Adapt Project Units"));
            });

          });
          popup::footer(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(t("Reset to NBR Defaults")).clicked() {
                    d.selected = TechnicalStandard::Nbr;
                    d.adapt_line_weights = true;
                    d.adapt_object_styles = true;
                    d.adapt_units = true;
                }

                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    apply = ui.button(t("Apply")).clicked();
                    cancel = ui.button(t("Cancel")).clicked();
                    ok = ui.button(t("OK")).clicked();
                });
            });
          });
        });

        if ok || apply {
            self.run(
                "standard",
                json!({
                    "set": d.selected.code(),
                    "adapt_line_weights": d.adapt_line_weights,
                    "adapt_object_styles": d.adapt_object_styles,
                    "adapt_units": d.adapt_units,
                }),
            );
        }

        if !(ok || cancel || r.should_close()) {
            self.standards_dialog = Some(d);
        }
    }
}
