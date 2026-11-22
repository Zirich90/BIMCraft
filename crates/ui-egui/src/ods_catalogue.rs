use super::{BimApp, Ui, cowork, json, t};

impl BimApp {
    pub(super) fn ods_catalogue_menu(&mut self, ui: &mut Ui) {
        if self.session.path.is_some() && !matches!(self.cowork, Some(cowork::Live::Guest(_))) {
            ui.menu_button(t("ODS Catalogue"), |ui| {
                if ui.button(t("Link ODS Catalogue")).clicked() {
                    let project = self.session.path.clone();
                    let dialog = rfd::AsyncFileDialog::new().add_filter("ODS", &["ods"]);
                    self.ask_file(ui.ctx(), dialog, false, move |app, path| {
                        if app.session.path == project {
                            app.run("catalogue_link", json!({"path":path.to_string_lossy()}));
                        }
                    });
                    ui.close();
                }
                if ui.button(t("Read Linked ODS Catalogue")).clicked() {
                    self.run("catalogue_sync", json!({}));
                    ui.close();
                }
                if ui.button(t("Unlink ODS Catalogue")).clicked() {
                    self.run("catalogue_link", json!({"path":null}));
                    ui.close();
                }
            });
        }
    }
}
