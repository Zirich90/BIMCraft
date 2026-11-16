//! Linked models, driven as a user drives them: Manage > Link Model (its file picked for it), the Project Browser's Linked Models
//! and what a plan lets be picked.

use super::*;
use std::path::PathBuf;

use egui_kittest::kittest::NodeT;

/// A box of walls saved to a file of its own: the path.
fn shell(name: &str) -> PathBuf {
    let mut s = Session::new();
    s.script(BOX).unwrap();
    let path = std::env::temp_dir().join(format!("bimcraft-ui-link-{name}-{}.bimcraft", std::process::id()));
    s.execute("save", &json!({"path": path})).unwrap();
    path
}

fn in_browser<'a>(h: &'a Harness<BimApp>, label: &'a str) -> egui_kittest::Node<'a> {
    let entries = h.get_all_by_label(label).filter(|n| n.rect().center().x < 290.0 && n.accesskit_node().role() == Role::Button);
    entries.min_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y)).unwrap()
}

const FIND: &str = "Search the Project Browser";

fn links(h: &Harness<BimApp>) -> Vec<Id> {
    ids(h, "link")
}

#[test]
fn a_linked_model_is_drawn_in_the_plan_and_the_plan_cannot_pick_it() {
    let path = shell("plan");
    let mut h = app("");
    h.state_mut().link_path(&path);
    h.run();
    let _ = std::fs::remove_file(&path);
    assert_eq!(links(&h).len(), 1, "{}", h.state().status.text);
    // The model the window draws has the walls; the project has none.
    assert!(h.state().session.model.elements.values().all(|e| e.category() != "wall"));
    let drawn = h.state_mut().view_model();
    assert_eq!(drawn.elements.values().filter(|e| e.category() == "wall").count(), 4);
    // A click on a linked wall selects nothing.
    click(&mut h, P2(4000.0, 0.0));
    assert!(h.state().selection.is_empty(), "{:?}", h.state().selection);
}

#[test]
fn the_project_browser_lists_linked_models_and_a_click_selects_one_for_properties() {
    let path = shell("browser");
    let mut h = app("");
    assert!(h.query_by_label("Linked Models").is_none(), "no group where nothing is linked");
    h.state_mut().link_path(&path);
    h.run();
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    assert!(h.query_by_label("Linked Models").is_some());
    type_into(&mut h, FIND, &name);
    in_browser(&h, &name).click();
    h.run();
    assert_eq!(h.state().selection, links(&h));
    assert!(h.query_by_label("Origin").is_some() && h.query_by_label("Angle").is_some(), "its parameters show");
}

#[test]
fn a_right_click_on_a_linked_model_removes_it() {
    let path = shell("remove");
    let mut h = app("");
    h.state_mut().link_path(&path);
    h.run();
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    type_into(&mut h, FIND, &name);
    in_browser(&h, &name).click_secondary();
    h.run();
    press(&mut h, "Remove");
    assert!(links(&h).is_empty());
    assert_eq!(h.state_mut().view_model().elements.values().filter(|e| e.category() == "wall").count(), 0);
}

#[test]
fn a_link_whose_file_is_missing_is_listed_as_not_loaded_and_reload_says_so() {
    let path = shell("lost");
    let mut h = app("");
    h.state_mut().link_path(&path);
    h.run();
    let name = path.file_stem().unwrap().to_string_lossy().into_owned();
    // The file is gone when the project opens: the link stays, and says it isn't loaded.
    std::fs::remove_file(&path).unwrap();
    let r = h.state_mut().session.execute("reload", &json!({})).unwrap();
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(1), "reload says which could not be read: {r}");
    h.state_mut().session.model_mut().loaded.clear();
    h.run();
    assert!(h.query_by_label(&format!("{name} (not loaded)")).is_some());
    assert_eq!(links(&h).len(), 1);
}

#[test]
fn the_manage_tab_offers_link_model_and_the_linked_models_group_reloads_them_all() {
    let path = shell("manage");
    let mut h = app("");
    press(&mut h, "Manage");
    if h.query_by_label("Link Model").is_none() {
        press(&mut h, "Project Location ▾");
    }
    assert!(h.query_by_label("Link Model").is_some());
    h.key_press(Key::Escape);
    h.run();
    h.state_mut().link_path(&path);
    h.run();
    let walls = |h: &mut Harness<BimApp>| h.state_mut().view_model().elements.values().filter(|e| e.category() == "wall").count();
    assert_eq!(walls(&mut h), 4);
    // The file gains a wall on disk; a right click on the group reads every link again.
    let mut other = Session::new();
    other.script(BOX).unwrap();
    other.script(r#"wall {"points": [[0,0],[0,-3000]]}"#).unwrap();
    other.execute("save", &json!({"path": path})).unwrap();
    type_into(&mut h, FIND, "linked");
    h.get_by_label("Linked Models").click_secondary();
    h.run();
    press(&mut h, "Reload All");
    let _ = std::fs::remove_file(&path);
    assert_eq!(walls(&mut h), 5);
}

#[test]
fn lk_asks_for_the_file_on_the_next_frame_and_is_listed_in_ks() {
    let mut app = BimApp::new(Session::new());
    app.typed = "LK".into();
    assert!(app.shortcut());
    assert!(app.link_dialog, "the dialog is asked for, not opened where the shortcut is typed");
    assert!(crate::shortcuts().iter().any(|(letters, name)| *letters == "LK" && *name == "Link Model"));
}

#[test]
fn an_ifc_link_can_preview_the_selected_room_and_detectors() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("architecture.ifc");
    let mut architecture = Session::new();
    architecture.script(BOX).unwrap();
    architecture.execute("ceiling", &json!({"boundary":[[0,0],[8000,0],[8000,6000],[0,6000]],"offset":2500})).unwrap();
    architecture.execute("save", &json!({"path":path})).unwrap();

    let mut h = app("");
    h.state_mut().link_path(&path);
    let link = links(&h)[0];
    let session = &mut h.state_mut().session;
    let room = session.execute("room", &json!({"point":[4000,3000]})).unwrap()["id"].as_u64().unwrap();
    let detector = session.execute("detectors", &json!({"room":room,"architecture":link})).unwrap()["ids"][0].as_u64().unwrap();
    let guid = session.model.guid(detector);
    h.state_mut().selection = vec![room, detector];
    h.run();
    type_into(&mut h, FIND, "architecture");
    h.get_by_label("architecture").click_secondary();
    h.run();
    press(&mut h, "Preview IFC Detector Group");
    assert!(h.query_by_label("Review IFC Adaptation").is_some());
    assert!(h.state_mut().session.execute("adapt_ifc", &json!({"id":link})).is_err());
    press(&mut h, "Refresh IFC Preview");
    press(&mut h, "Apply IFC Adaptation");
    assert!(h.query_by_label("Review IFC Adaptation").is_none());
    assert_eq!(h.state().session.model.guid(detector), guid);
    let saved = h.state_mut().session.execute("adapt_ifc", &json!({"id":link})).unwrap();
    assert_eq!(saved["status"], "ready");
    let duplicate = json!({"id":link,"groups":[{"room":room,"ids":[guid]}],"append":true,"apply":true});
    let error = h.state_mut().session.execute("adapt_ifc", &duplicate).unwrap_err();
    assert!(error.contains("selected in more than one group"), "{error}");
    assert_eq!(h.state_mut().session.execute("adapt_ifc", &json!({"id":link})).unwrap(), saved);
}

#[test]
fn reloading_an_ifc_link_reviews_detector_changes_and_rechecks_before_applying() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("architecture.ifc");
    let mut architecture = Session::new();
    architecture.script(BOX).unwrap();
    let ceiling = architecture.execute("ceiling", &json!({"boundary":[[0,0],[8000,0],[8000,6000],[0,6000]],"offset":2500})).unwrap()["id"]
        .as_u64()
        .unwrap();
    architecture.execute("save", &json!({"path":path})).unwrap();

    #[cfg(not(feature = "visual-tests"))]
    let mut h = app("");
    #[cfg(feature = "visual-tests")]
    let output = std::env::var_os("IFC_REVIEW_DUMP").map(PathBuf::from);
    #[cfg(feature = "visual-tests")]
    let mut h = if output.is_some() {
        let mut state = BimApp::new(Session::new());
        (state.profile, state.docking) = (crate::style::Profile::Rev, crate::dock::Docking::revit());
        Harness::builder()
            .with_size([1280.0, 800.0])
            .with_max_steps(32)
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::new())
            .build_ui_state(|ui, state: &mut BimApp| state.show(ui), state)
    } else {
        app("")
    };
    h.state_mut().link_path(&path);
    let link = links(&h)[0];
    let session = &mut h.state_mut().session;
    let room = session.execute("room", &json!({"point":[4000,3000]})).unwrap()["id"].as_u64().unwrap();
    let detector = session.execute("detectors", &json!({"room":room,"architecture":link})).unwrap()["ids"][0].as_u64().unwrap();
    let guid = session.model.guid(detector);
    let saved = session.execute("adapt_ifc", &json!({"id":link,"groups":[{"room":room,"ids":[guid]}],"apply":true})).unwrap();
    assert_eq!(saved["status"], "applied");
    let link_path = match session.model.elements.get(link) {
        Some(Element::Link(link)) => link.path.clone(),
        _ => panic!("link is missing"),
    };
    let original = session.model.loaded[&link_path].clone();
    let contact = |h: &Harness<BimApp>| {
        let model = &h.state().session.model;
        let Some(Element::Component(c)) = model.elements.get(detector) else { panic!("detector is missing") };
        c.offset + model.component_contact_height(c, "bottom")
    };
    let mut raise = |offset| {
        architecture.execute("modify", &json!({"id":ceiling,"offset":offset})).unwrap();
        architecture.execute("save", &json!({"path":path})).unwrap();
    };
    raise(2800);
    h.run();
    type_into(&mut h, FIND, "linked");
    h.get_by_label("Linked Models").click_secondary();
    h.run();
    press(&mut h, "Reload All");
    assert!(h.query_by_label("Review IFC Adaptation").is_some());
    assert!((contact(&h) - 2500.0).abs() < 0.01);
    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.model.loaded[&link_path]));

    #[cfg(feature = "visual-tests")]
    if let Some(output) = output {
        std::fs::create_dir_all(&output).unwrap();
        let Some(Element::Room(room)) = h.state().session.model.elements.get(room) else { panic!("room is missing") };
        let room_label = format!("{} {}", room.number, room.name);
        press(&mut h, &room_label);
        for (name, theme, light_canvas) in [
            ("light", egui::ThemePreference::Light, false),
            ("dark", egui::ThemePreference::Dark, false),
            ("dark-light-canvas", egui::ThemePreference::Dark, true),
        ] {
            let state = h.state_mut();
            state.theme_wanted = Some(theme);
            state.light_canvas = light_canvas;
            h.run();
            h.render().unwrap().save(output.join(format!("ifc-review-{name}.png"))).unwrap();
        }
    }

    raise(2900);
    press(&mut h, "Apply IFC Adaptation");
    assert!(h.query_by_label("The IFC preview changed. Review it again before applying.").is_some());
    assert!((contact(&h) - 2500.0).abs() < 0.01);
    press(&mut h, "Apply IFC Adaptation");
    assert!(h.query_by_label("Review IFC Adaptation").is_none());
    assert!((contact(&h) - 2900.0).abs() < 0.01);
    assert_eq!(h.state().session.model.guid(detector), guid);
    assert!(!std::sync::Arc::ptr_eq(&original, &h.state().session.model.loaded[&link_path]));
    h.state_mut().session.execute("undo", &json!({})).unwrap();
    assert!((contact(&h) - 2500.0).abs() < 0.01);
    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.model.loaded[&link_path]));
}

#[test]
fn a_walls_levels_and_a_plan_views_level_and_parent_are_named_in_properties() {
    let mut h = app(BOX);
    let w = ids(&h, "wall")[0];
    h.state_mut().selection = vec![w];
    h.run();
    h.run();
    let shown = |h: &Harness<BimApp>, label: &str| h.get_by_role_and_label(Role::ComboBox, label).value();
    assert_eq!(shown(&h, "Base Constraint").as_deref(), Some("Level 1"));
    assert_eq!(shown(&h, "Top Constraint").as_deref(), Some("Level 2"));
}

#[test]
fn link_model_asks_how_to_position_the_file_and_links_it_by_shared_coordinates() {
    let path = shell("shared");
    let mut h = app("");
    h.state_mut().link_box = Some(crate::links::LinkBox::for_test(path.clone()));
    h.run();
    assert!(h.query_by_label("Auto - Origin to Origin").is_some(), "the box asks");
    pick(&mut h, Role::RadioButton, "Auto - By Shared Coordinates");
    press(&mut h, "OK");
    let _ = std::fs::remove_file(&path);
    let made = links(&h);
    assert_eq!(made.len(), 1, "{}", h.state().status.text);
    let Some(Element::Link(l)) = h.state().session.model.elements.get(made[0]) else { panic!() };
    assert_eq!(l.by, bimcraft_core::link::By::Shared);
    assert!(h.state().link_box.is_none(), "the box closed");
}
