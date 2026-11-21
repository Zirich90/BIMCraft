use serde_json::json;

use crate::engine::Session;
use crate::library::Kind;

#[test]
fn catalogue_binding_uses_session_approval_and_open_only_reads() {
    let files = tempfile::tempdir().unwrap();
    let project = files.path().join("project.bim");
    let workbook = files.path().join("approved.ods");
    let mut s = Session::new();
    let conduit = |width| json!({"category":"conduit","name":"Approved","width":width,"depth":width});
    s.execute("type", &conduit(20)).unwrap();
    s.execute("save", &json!({"path":project})).unwrap();
    s.execute("type", &conduit(30)).unwrap();
    s.execute("catalogue_export", &json!({"path":workbook})).unwrap();
    let request = crate::bridge::Request::Execute { command: "catalogue_link".into(), params: json!({"path":workbook}), read_only: false };
    assert!(crate::bridge::run(&mut s, &request, crate::bridge::Access::Edit).is_err());
    s.execute("catalogue_link", &json!({"path":workbook})).unwrap();
    let original = std::fs::read(&workbook).unwrap();

    let mut project_json: serde_json::Value = serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    project_json["catalogue_path"] = json!(workbook);
    std::fs::write(&project, project_json.to_string()).unwrap();
    let mut untrusted = Session::new();
    let opened = untrusted.execute("open", &json!({"path":project})).unwrap();
    assert_eq!(opened["left_out"], json!(["/catalogue_path"]));
    assert_eq!(untrusted.model.types.runs["conduit"]["Approved"].width, 20.0);
    assert!(untrusted.execute("catalogue_sync", &json!({})).is_err());

    s.execute("open", &json!({"path":project})).unwrap();
    assert_eq!(s.model.types.runs["conduit"]["Approved"].width, 30.0);
    assert_eq!(std::fs::read(&workbook).unwrap(), original);
    assert!(s.script_dry_run("catalogue_sync {}").is_err());
    s.no_files = true;
    assert!(s.execute("catalogue_sync", &json!({})).is_err());
    s.no_files = false;
    s.read_only = Some("test");
    assert!(s.execute("catalogue_link", &json!({"path":null})).is_err());
    assert!(s.execute("catalogue_sync", &json!({})).is_err());
    assert_eq!(std::fs::read(&workbook).unwrap(), original);
    s.read_only = None;
    assert!(s.script_atomic("catalogue_link {\"path\":null}\nunknown_command").is_err());
    assert!(s.execute("catalogue_sync", &json!({})).is_ok());

    std::fs::write(&workbook, b"broken").unwrap();
    let opened = s.execute("open", &json!({"path":project})).unwrap();
    assert!(opened["catalogue_warning"].is_string());
    assert_eq!(s.model.types.runs["conduit"]["Approved"].width, 20.0);
    assert_eq!(std::fs::read(&workbook).unwrap(), b"broken");
}

#[test]
fn ods_catalogue_round_trips_upstream_carrier_types() {
    let mut s = Session::new();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalogue.ods");
    for category in ["conduit", "cabletray"] {
        s.execute("type", &json!({"category":category,"name":"Custom","width":30,"depth":30})).unwrap();
    }
    let entries: Vec<_> = ["conduit", "cabletray"]
        .map(|category| Kind::Type(category.into()))
        .into_iter()
        .map(|kind| (s.model.library.guid(&kind, "Custom").unwrap(), kind))
        .collect();
    s.execute("catalogue_export", &json!({"path":path})).unwrap();

    s.execute("type", &json!({"category":"conduit","name":"Custom","width":50,"depth":50})).unwrap();
    s.execute("catalogue_import", &json!({"path":path})).unwrap();
    assert_eq!(s.model.types.runs["conduit"]["Custom"].width, 30.0);
    assert_eq!(s.model.types.runs["cabletray"]["Custom"].width, 30.0);
    for (guid, kind) in entries {
        assert_eq!(s.model.library.guid(&kind, "Custom"), Some(guid));
    }
}

#[test]
fn ods_catalogue_respects_file_restrictions() {
    let mut s = Session::new();
    let path = tempfile::tempdir().unwrap();
    let file = path.path().join("catalogue.ods");
    s.execute("catalogue_export", &json!({"path":file})).unwrap();
    s.read_only = Some("read-only test");
    let error = s.execute("catalogue_import", &json!({"path":file})).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    let new_file = path.path().join("new.ods");
    let params = json!({"path":new_file});
    let error = s.execute("catalogue_export", &params).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    let error = s.script_dry_run(&format!("catalogue_export {params}")).unwrap_err();
    assert!(error.contains("dry run"), "{error}");
    assert!(!new_file.exists());
}

#[test]
fn ods_catalogue_invalid_guid_rolls_back_the_entire_import() {
    let mut s = Session::new();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalogue.ods");
    s.execute("type", &json!({"category":"conduit","name":"Custom","width":30,"depth":30})).unwrap();
    let before = s.model.clone();
    let rows = [
        ["GUID", "Category", "Name", "Width_mm", "Depth_mm", "Round", "Material"],
        ["", "cabletray", "New tray", "50", "30", "false", ""],
        ["not-a-guid", "conduit", "Custom", "90", "90", "false", ""],
    ];
    let rows: Vec<Vec<String>> = rows.into_iter().map(|row| row.into_iter().map(String::from).collect()).collect();
    std::fs::write(&path, crate::ods::write(&rows).unwrap()).unwrap();
    assert!(s.execute("catalogue_import", &json!({"path":path})).is_err());
    assert_eq!(s.model, before);
}

#[test]
fn ods_catalogue_round_trips_clear_sections_and_reads_legacy_workbooks() {
    let mut s = Session::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measured.ods");
    let conduit = json!({"category":"conduit","name":"Measured conduit","width":40,"round":true});
    let tray = json!({"category":"cabletray","name":"Measured box","width":100,"depth":50});
    s.execute("type", &conduit).unwrap();
    s.execute("type", &tray).unwrap();
    s.execute("catalogue_export", &json!({"path":path})).unwrap();
    let mut legacy_rows = crate::ods::read(&std::fs::read(&path).unwrap()).unwrap();
    for row in &mut legacy_rows {
        row.truncate(7);
    }
    let legacy = dir.path().join("legacy.ods");
    std::fs::write(&legacy, crate::ods::write(&legacy_rows).unwrap()).unwrap();

    let mut measured_conduit = conduit.clone();
    measured_conduit["clear_section"] = json!({"diameter":32});
    let mut measured_tray = tray.clone();
    measured_tray["clear_section"] = json!({"width":90,"depth":42,"enclosure":"lidded_box"});
    s.execute("type", &measured_conduit).unwrap();
    s.execute("type", &measured_tray).unwrap();
    let measured_types = s.model.types.runs.clone();
    s.execute("catalogue_import", &json!({"path":legacy})).unwrap();
    assert_eq!(s.model.types.runs, measured_types);
    s.execute("catalogue_export", &json!({"path":path})).unwrap();

    let mut plain_conduit = conduit;
    plain_conduit["clear_section"] = serde_json::Value::Null;
    let mut plain_tray = tray;
    plain_tray["clear_section"] = serde_json::Value::Null;
    s.execute("type", &plain_conduit).unwrap();
    s.execute("type", &plain_tray).unwrap();
    s.execute("catalogue_import", &json!({"path":path})).unwrap();
    assert_eq!(s.model.types.runs, measured_types);

    let mut rows = crate::ods::read(&std::fs::read(&path).unwrap()).unwrap();
    let conduit_row = rows.iter_mut().find(|row| row.get(2).is_some_and(|name| name == "Measured conduit")).unwrap();
    conduit_row[7] = "41".into();
    std::fs::write(&path, crate::ods::write(&rows).unwrap()).unwrap();
    let before = s.model.clone();
    assert!(s.execute("catalogue_import", &json!({"path":path})).is_err());
    assert_eq!(s.model, before);

    let conduit_row = rows.iter_mut().find(|row| row.get(2).is_some_and(|name| name == "Measured conduit")).unwrap();
    conduit_row.truncate(7);
    std::fs::write(&path, crate::ods::write(&rows).unwrap()).unwrap();
    s.execute("catalogue_import", &json!({"path":path})).unwrap();
    assert!(s.model.types.runs["conduit"]["Measured conduit"].clear_section.is_none());
    assert_eq!(s.model.types.runs["cabletray"]["Measured box"], measured_types["cabletray"]["Measured box"]);
}
