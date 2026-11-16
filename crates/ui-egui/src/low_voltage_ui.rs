use crate::icons::Icon;
use crate::ribbon::panel;
use crate::{BimApp, Tool};
use bimcraft_i18n::msg;
use egui::Ui;

impl BimApp {
    /// The Low Voltage tab (fire alarm, voice evacuation, security): its panel, its detectors and alarm appliances, its
    /// security devices, and the circuit that runs between them.
    pub(crate) fn low_voltage_tab(&mut self, ui: &mut Ui) {
        self.tools(ui, "Select", &[(Icon::Modify, "Modify", "Modify (MD): select elements", Tool::Modify)]);
        panel(ui, msg!("Fire Alarm"), |ui| {
            self.buttons(
                ui,
                &[
                    Self::fixture_button("alarmpanel"),
                    Self::fixture_button("smokedetector"),
                    Self::fixture_button("heatdetector"),
                    Self::fixture_button("callpoint"),
                ],
            );
        });
        panel(ui, msg!("Alarm & Voice"), |ui| {
            self.buttons(ui, &[Self::fixture_button("sounder"), Self::fixture_button("exitsign"), Self::fixture_button("speaker")]);
        });
        panel(ui, msg!("Fire Zones"), |ui| {
            use bimcraft_core::model::FireZoneKind;
            let level = self.plan_level();
            ui.add_enabled_ui(level.is_some(), |ui| {
                for (label, kind) in
                    [(msg!("Detection Zone"), FireZoneKind::Detection), (msg!("Notification Zone"), FireZoneKind::Notification)]
                {
                    if ui.button(crate::lang::t(label)).clicked()
                        && let Some(level) = level
                    {
                        self.start_sketch(crate::sketch::Of::FireZone(level, kind), Vec::new());
                    }
                }
            });
        });
        panel(ui, msg!("Security"), |ui| {
            self.buttons(ui, &[Self::fixture_button("motiondetector"), Self::fixture_button("camera")]);
        });
        panel(ui, msg!("Wires"), |ui| {
            self.buttons(
                ui,
                &[(
                    Icon::Wire,
                    "Wire",
                    msg!("Wire (EW): click its start and its end; a chain of clicks makes a wire for each step"),
                    Tool::Run("wire", None),
                )],
            );
            if self.minimal {
                return;
            }
            ui.menu_button(crate::lang::t(msg!("Route Carriers")), |ui| {
                let level = self.plan_level();
                for (label, enabled, scope) in [
                    (msg!("Route Carriers on Level"), level.is_some(), serde_json::json!({"level":level})),
                    (msg!("Route Carriers in Project"), true, serde_json::json!({})),
                ] {
                    let picked = ui
                        .add_enabled_ui(enabled, |ui| {
                            ui.menu_button(crate::lang::t(label), |ui| {
                                let types = self.session.model.types.runs.get("cablecarrier");
                                if ui.button(crate::lang::t(msg!("Default"))).clicked() {
                                    return Some(None);
                                }
                                egui::ScrollArea::vertical()
                                    .max_height(220.0)
                                    .show(ui, |ui| {
                                        types
                                            .into_iter()
                                            .flat_map(|types| types.keys())
                                            .find_map(|name| ui.button(name).clicked().then(|| Some(name.clone())))
                                    })
                                    .inner
                            })
                            .inner
                            .flatten()
                        })
                        .inner;
                    if let Some(type_name) = picked {
                        self.run("cable_carrier", serde_json::json!({"from_wires":scope,"type":type_name}));
                        ui.close();
                    }
                }
            });
        });
        if !self.minimal {
            panel(ui, msg!("Electrical Lines"), |ui| self.electrical_lines(ui));
        }
    }

    fn electrical_lines(&mut self, ui: &mut Ui) {
        use bimcraft_core::model::Element;
        use serde_json::json;

        ui.menu_button(crate::lang::t(msg!("Electrical Lines")), |ui| {
            let model = &self.session.model;
            let devices: Vec<_> = self
                .selection
                .iter()
                .copied()
                .filter(|id| matches!(model.elements.get(*id), Some(Element::Component(c)) if c.is_placed()))
                .collect();
            let segments: Vec<_> = self
                .selection
                .iter()
                .copied()
                .filter(|id| matches!(model.elements.get(*id), Some(Element::Wire(w)) if w.feeder.is_none()))
                .collect();
            if let Some((source, rest)) = devices.split_first()
                && ui.button(crate::lang::t(msg!("Create Line from Selection"))).clicked()
            {
                self.run("electrical_line", json!({"source":source,"devices":rest,"segments":segments}));
                ui.close();
                return;
            }
            if let [device] = devices.as_slice() {
                self.electrical_terminals(ui, *device);
            }
            let lines: Vec<_> = self
                .session
                .model
                .elements
                .iter()
                .filter_map(|(&id, element)| {
                    let Element::ElectricalLine(line) = element else { return None };
                    Some((id, line.system.clone()))
                })
                .collect();
            for (id, system) in lines {
                ui.menu_button(format!("{id} {system}"), |ui| {
                    let model = &self.session.model;
                    let Some(Element::ElectricalLine(line)) = model.elements.get(id) else { return };
                    let rows: Vec<_> = line
                        .order(model)
                        .into_iter()
                        .zip(line.marks(model))
                        .map(|(guid, mark)| {
                            let locked = line.locked_marks.iter().any(|entry| entry.device == guid);
                            (guid, model.elements.id(guid), mark, locked)
                        })
                        .collect();
                    for (guid, device, mark, locked) in rows {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}  {mark}", device.map_or_else(String::new, |id| id.to_string())));
                            let label = if locked { msg!("Unlock Mark") } else { msg!("Lock Mark") };
                            if ui.button(crate::lang::t(label)).clicked() {
                                self.run("electrical_line_mark", json!({"id":id,"device":guid,"mark":(!locked).then_some(mark)}));
                            }
                        });
                    }
                    if ui.button(crate::lang::t(msg!("Renumber Electrical Line"))).clicked() {
                        self.run("electrical_line_renumber", json!({"id":id}));
                    }
                });
            }
        });
    }

    fn electrical_terminals(&mut self, ui: &mut Ui, device: bimcraft_core::ids::Id) {
        use bimcraft_core::model::Element;

        ui.menu_button(crate::lang::t(msg!("Electrical Terminals")), |ui| {
            let model = &self.session.model;
            let owner = model.guid(device);
            let terminals: Vec<_> = model
                .elements
                .iter()
                .filter_map(|(&id, element)| {
                    let Element::ElectricalTerminal(terminal) = element else { return None };
                    (terminal.device == owner).then(|| (id, terminal.name.clone()))
                })
                .collect();
            for (id, name) in terminals {
                self.electrical_terminal_name(ui, device, Some(id), name);
            }
            self.electrical_terminal_name(ui, device, None, String::new());
        });
    }

    fn electrical_terminal_name(&mut self, ui: &mut Ui, device: bimcraft_core::ids::Id, id: Option<bimcraft_core::ids::Id>, name: String) {
        use serde_json::json;

        let key = ui.make_persistent_id(("electrical-terminal-name", device, id));
        let mut draft = ui.data_mut(|data| data.get_temp::<String>(key)).unwrap_or(name);
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut draft);
            let label = if id.is_some() { msg!("Rename Terminal") } else { msg!("Add Terminal") };
            if ui.add_enabled(!draft.trim().is_empty(), egui::Button::new(crate::lang::t(label))).clicked() {
                let params = id.map_or_else(|| json!({"device":device,"name":draft}), |id| json!({"id":id,"name":draft}));
                if self.run("electrical_terminal", params).is_some() {
                    ui.data_mut(|data| data.remove::<String>(key));
                    return;
                }
            }
            ui.data_mut(|data| data.insert_temp(key, draft));
        });
    }
}
