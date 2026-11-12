//! Linked models: Manage > Link Model asks for a project or an IFC and links it; Reload Links reads the files again.

use std::path::Path;
use std::sync::Arc;

use bimcraft_core::family::Category;
use bimcraft_core::model::{Element, Guid, Id, Model};
use egui::{Context, Ui};
use serde_json::{Value, json};

use crate::BimApp;
use crate::lang::{t, tf};
use crate::popup;

pub(crate) struct IfcReview {
    link: Guid,
    model: Arc<Model>,
    report: Value,
    notice: Option<String>,
    group: Option<Value>,
    penetration_reference: String,
}

impl BimApp {
    /// Link Model (LK): the file the user picks.
    pub(crate) fn link_model_dialog(&mut self, ctx: &Context) {
        let dialog = rfd::AsyncFileDialog::new().add_filter(t("BIMCraft project or IFC"), &["bimcraft", "ifc"]);
        self.ask_file(ctx, dialog, false, |app, path| app.link_path(&path));
    }

    /// Links the project at `path`, kept relative to this project's folder where it lies under it, so the two move together.
    pub(crate) fn link_path(&mut self, path: &Path) {
        let folder = self.session.path.as_deref().and_then(Path::parent);
        let kept = folder.and_then(|dir| path.strip_prefix(dir).ok()).unwrap_or(path);
        self.run("link", json!({"path": kept.to_string_lossy()}));
    }

    pub(crate) fn review_ifc_reload(&mut self, result: &Value) {
        self.ifc_reviews = entries(&result["adaptations"])
            .iter()
            .filter_map(|report| {
                let mut report = report.as_object()?.clone();
                let id = report.remove("link")?.as_u64()?;
                let link = self.session.model.elements.guid(id)?;
                Some(IfcReview {
                    link,
                    model: self.session.model.clone(),
                    report: Value::Object(report),
                    notice: None,
                    group: None,
                    penetration_reference: String::new(),
                })
            })
            .collect();
    }

    pub(crate) fn selected_ifc_group(&self) -> Option<Value> {
        let model = &self.session.model;
        let mut room = None;
        let mut ids = Vec::new();
        for &id in &self.selection {
            let guid = model.elements.guid(id)?;
            match model.elements.get(id)? {
                Element::Room(_) if room.is_none() => room = Some(guid),
                Element::Component(component)
                    if model
                        .families
                        .get(&component.family)
                        .is_some_and(|family| matches!(family.category, Category::SmokeDetector | Category::HeatDetector)) =>
                {
                    ids.push(guid)
                }
                _ => return None,
            }
        }
        let room = room?;
        (!ids.is_empty()).then(|| json!({"room": room, "ids": ids}))
    }

    pub(crate) fn preview_ifc_group(&mut self, id: Id, group: Value) {
        let Some(link) = self.session.model.elements.guid(id) else { return };
        if let Some(report) = self.run("adapt_ifc", json!({"id":link,"groups":[group],"append":true})) {
            self.ifc_reviews.clear();
            self.ifc_reviews.push_back(IfcReview {
                link,
                model: self.session.model.clone(),
                report,
                notice: None,
                group: Some(group),
                penetration_reference: String::new(),
            });
        }
    }

    pub(crate) fn ifc_review_window(&mut self, ctx: &Context) {
        let Some(review) = self.ifc_reviews.front() else { return };
        let current = Arc::ptr_eq(&review.model, &self.session.model);
        let pending = self.ifc_reviews.len();
        let approval = review.report["can_approve_crossings"] == true;
        let mut reference = review.penetration_reference.clone();
        let mut action = (false, false, false, false);
        let response = popup::Modal::new(egui::Id::new("ifc-review"), t("Review IFC Adaptation")).show(ctx, |ui| {
            popup::body(ui, |ui| {
                review.show(ui);
                if current && approval {
                    ui.label(t("Penetration approval reference"));
                    ui.add(egui::TextEdit::singleline(&mut reference).char_limit(256));
                }
                if !current {
                    ui.label(t("The project changed. Refresh the IFC preview before applying."));
                }
            });
            action = popup::footer(ui, |ui| {
                ui.horizontal(|ui| {
                    let refresh = ui.button(t("Refresh IFC Preview")).clicked();
                    let can_apply = review.report["status"] == "ready" || (approval && !reference.trim().is_empty());
                    let label = if approval { t("Apply Approved Penetrations") } else { t("Apply IFC Adaptation") };
                    let apply = ui.add_enabled(current && can_apply, egui::Button::new(label)).clicked();
                    let next = pending > 1 && ui.button(t("Next Link")).clicked();
                    let close = ui.button(t("Close")).clicked();
                    (refresh, apply, next, close)
                })
                .inner
            });
        });
        let (refresh, apply, next, close) = action;
        if close || response.should_close() {
            self.ifc_reviews.clear();
        } else if next {
            self.ifc_reviews.pop_front();
        } else if refresh || apply {
            let mut params = json!({"id": review.link});
            if let Some(group) = &review.group {
                params["groups"] = json!([group]);
                params["append"] = json!(true);
            }
            if apply {
                params["apply"] = json!(true);
                params["expected"] = review.report.clone();
                if approval {
                    params["penetration_reference"] = json!(reference.trim());
                }
            }
            let result = self.run("adapt_ifc", params);
            let Some(review) = self.ifc_reviews.front_mut() else { return };
            match result {
                Some(report) if report["status"] == "applied" => {
                    self.ifc_reviews.pop_front();
                }
                Some(mut report) => {
                    let changed = report["status"] == "review_required";
                    review.report = if changed { report["preview"].take() } else { report };
                    review.model = self.session.model.clone();
                    review.penetration_reference.clear();
                    review.notice = changed.then(|| t("The IFC preview changed. Review it again before applying.").to_owned());
                }
                None => review.notice = Some(self.status.text.clone()),
            }
        } else if let Some(review) = self.ifc_reviews.front_mut() {
            review.penetration_reference = reference;
        }
    }
}

impl IfcReview {
    fn show(&self, ui: &mut Ui) {
        if let Some(Element::Link(link)) = self.model.elements.get(self.link) {
            ui.heading(&link.name);
        }
        if let Some(notice) = &self.notice {
            ui.label(notice);
        }
        let routes = entries(&self.report["routes"]);
        if self.report["status"] == "ready" {
            let devices = entries(&self.report["changed"]).len().to_string();
            ui.label(tf("Detectors to move: {}. Feeder segments to change: {}.", &[&devices, &routes.len().to_string()]));
        } else {
            ui.colored_label(crate::style::pal().error, t("Resolve the issues below before applying."));
        }
        for (index, group) in entries(&self.report["groups"]).iter().enumerate() {
            let room = group["room"].as_str().and_then(Guid::parse).and_then(|id| self.model.elements.get(id));
            let title = match room {
                Some(Element::Room(room)) => format!("{} {}", room.number, room.name),
                _ => t("Room").to_owned(),
            };
            let targets = entries(&group["targets"]);
            ui.push_id((self.link, index), |ui| {
                ui.collapsing(title, |ui| {
                    if let Some(error) = group["error"].as_str() {
                        ui.label(error);
                    }
                    for target in targets {
                        let at = position(&self.model, &target["at"]);
                        if let Some(offset) = target["contact_offset"].as_f64() {
                            ui.label(tf("Position {} · contact offset {}", &[&at, &self.model.units.length(offset)]));
                        } else {
                            ui.label(at);
                        }
                    }
                });
            });
        }
        if !routes.is_empty() {
            ui.collapsing(t("Feeder Route Changes"), |ui| {
                for route in routes {
                    let points =
                        |key| entries(&route[key]).iter().map(|point| position(&self.model, point)).collect::<Vec<_>>().join(" → ");
                    ui.label(tf("Segment {} · wire {}", &[&route["id"].to_string(), &route["wire"].to_string()]));
                    ui.label(tf("Before {} · after {}", &[&points("before"), &points("after")]));
                }
            });
        }
        for key in ["route_error", "route_clash_error"] {
            if let Some(error) = self.report[key].as_str() {
                ui.label(error);
            }
        }
        for clash in entries(&self.report["route_clashes"]) {
            let args = [
                clash["wire"].to_string(),
                clash["segment"].to_string(),
                clash["obstacle"].as_str().unwrap_or_default().to_owned(),
                position(&self.model, &clash["point"]),
            ];
            let args = args.each_ref().map(String::as_str);
            let message = if clash["carrier"] == true {
                tf("Wire {} · cable carrier {} crosses IFC element {} at {}", &args)
            } else {
                tf("Wire {} · segment {} crosses IFC element {} at {}", &args)
            };
            ui.label(message);
        }
    }
}

fn entries(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn position(model: &Model, point: &Value) -> String {
    entries(point).iter().filter_map(Value::as_f64).map(|mm| model.units.length(mm)).collect::<Vec<_>>().join(", ")
}
