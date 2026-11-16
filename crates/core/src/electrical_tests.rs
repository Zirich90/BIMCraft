//! The electrical settings: voltages, distribution systems and wire types, as the project's tables, through the commands every
//! front end drives.

use serde_json::{Value, json};

use crate::electrical;
use crate::engine::Session;
use crate::library::Kind;
use crate::nbr5410;
use crate::units::Units;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.model.clone();
    let e = s.execute(id, &p).expect_err(&format!("{id} {p} should fail"));
    assert_eq!(*s.model, *before, "a refused {id} changes nothing");
    e
}

#[test]
fn stage8_line_table_keeps_traversal_marks_and_terminal_addresses_separate() {
    let mut s = Session::new();
    let upper = run(&mut s, "level", json!({"name":"Upper","elevation":4000}))["id"].clone();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let b = run(&mut s, "component", json!({"family":"Smoke Detector","at":[2000,0],"level":upper}))["id"].clone();
    let branch = run(&mut s, "component", json!({"family":"Heat Detector","at":[1000,1000]}))["id"].clone();
    let first = run(&mut s, "electrical_terminal", json!({"device":a,"name":"A","address":"101","load_ma":0}))["id"].clone();
    let second = run(&mut s, "electrical_terminal", json!({"device":a,"name":"B","address":"102","limit_ma":20}))["id"].clone();
    run(&mut s, "electrical_terminal", json!({"device":branch,"name":"Unattached","address":"999"}));
    let line = run(
        &mut s,
        "electrical_line",
        json!({
            "source":panel,"devices":[a,b],"branches":[{"from":a,"devices":[branch]}],
            "system":"СПС","mark_prefix":"L","terminals":[second,first]
        }),
    )["id"]
        .clone();
    run(&mut s, "electrical_line_mark", json!({"id":line,"device":b,"mark":"FIXED"}));
    let before = s.model.clone();
    let history = s.history().into_iter().map(str::to_owned).collect::<Vec<_>>();
    let report = run(&mut s, "electrical_line_table", json!({"id":line}));
    let cell = |row: usize, column: &str| {
        let index = report["columns"].as_array().unwrap().iter().position(|v| v == column).unwrap();
        report["rows"][row][index].clone()
    };
    assert_eq!(report["elements"], json!([[panel], [a], [branch], [b]]));
    assert_eq!(report["device_count"], 3);
    assert_eq!(cell(0, "Role"), "source");
    assert_eq!(cell(3, "Mark"), "FIXED");
    assert_eq!(cell(3, "Order"), 4);
    assert_eq!(cell(1, "GUID"), json!(s.model.guid(a.as_u64().unwrap())));
    let terminals = cell(1, "Terminals");
    assert_eq!(terminals[0]["id"], second);
    assert_eq!(terminals[0]["address"], "102");
    assert!(terminals[0]["load_ma"].is_null());
    assert_eq!(terminals[1]["address"], "101");
    assert_eq!(terminals[1]["load_ma"].as_f64(), Some(0.0));
    assert!(terminals[1]["limit_ma"].is_null());
    assert_eq!(cell(2, "Terminals"), json!([]));
    assert_eq!(report["groups"].as_array().unwrap().len(), 2);
    let filtered = run(&mut s, "electrical_line_table", json!({"system":"СПС","level":upper}));
    assert_eq!(filtered["elements"], json!([[b]]));
    let order = filtered["columns"].as_array().unwrap().iter().position(|v| v == "Order").unwrap();
    assert_eq!(filtered["rows"][0][order], 4);
    assert_eq!(run(&mut s, "electrical_line_table", json!({"system":"СОУЭ"}))["rows"], json!([]));
    assert_eq!(s.model, before);
    assert_eq!(s.history().into_iter().map(str::to_owned).collect::<Vec<_>>(), history);
    let (back, _) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    let mut reopened = Session::new();
    reopened.model = std::sync::Arc::new(back);
    assert_eq!(run(&mut reopened, "electrical_line_table", json!({"id":s.model.guid(line.as_u64().unwrap())})), report);
}

#[test]
fn stage8_line_table_exports_without_editing_or_overwriting_protected_files() {
    use crate::engine::{Files, touches_files};
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let detector = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[detector],"system":"=СПС,\"Тест\""}));
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project.bimcraft");
    run(&mut s, "save", json!({"path":project}));
    let saved = std::fs::read(&project).unwrap();
    let before = s.model.clone();
    s.read_only = Some("test permission");
    let command = crate::commands::ALL.iter().find(|c| c.id == "electrical_line_table").unwrap();
    assert_eq!(touches_files(command, &json!({})), None);
    assert_eq!(touches_files(command, &json!({"path":"table.csv"})), Some(Files::Write));
    let csv = dir.path().join("table.csv");
    run(&mut s, "electrical_line_table", json!({"path":csv}));
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.contains("'="), "spreadsheet formula text is escaped: {text}");
    let xlsx = dir.path().join("table.XLSX");
    run(&mut s, "electrical_line_table", json!({"path":xlsx}));
    let bytes = std::fs::read(&xlsx).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    use std::io::Read;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let mut body = String::new();
        entry.read_to_string(&mut body).unwrap();
        let mut reader = quick_xml::Reader::from_str(&body);
        while reader.read_event().unwrap() != quick_xml::events::Event::Eof {}
        if entry.name().unwrap() == "xl/worksheets/sheet1.xml" {
            assert!(!body.contains("<f>"));
            assert!(body.contains(&format!("<v>{panel}</v>")));
            assert!(body.contains("=СПС,&quot;Тест&quot;"));
        }
    }
    assert!(fails(&mut s, "electrical_line_table", json!({"path":project})).contains("read-only"));
    assert_eq!(std::fs::read(&project).unwrap(), saved);
    let unsupported = dir.path().join("manual.odt");
    std::fs::write(&unsupported, b"manual text").unwrap();
    assert!(fails(&mut s, "electrical_line_table", json!({"path":unsupported})).contains(".csv or .xlsx"));
    assert_eq!(std::fs::read(&unsupported).unwrap(), b"manual text");
    let script = format!("electrical_line_table {}", json!({"path":csv}));
    assert!(s.script_dry_run(&script).is_err());
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), text);
    assert_eq!(s.model, before);
}

#[test]
fn stage8_line_table_rejects_hostile_filters_without_changes() {
    let mut s = Session::new();
    for params in [
        json!([]),
        json!("table"),
        json!({"unknown":true}),
        json!({"id":100000}),
        json!({"level":1e300}),
        json!({"level":"Missing"}),
        json!({"system":false}),
        json!({"system":"x".repeat(crate::model::MAX_TEXT+1)}),
        json!({"path":""}),
    ] {
        fails(&mut s, "electrical_line_table", params);
    }
}

#[test]
fn stage8_line_table_counts_shared_devices_once_and_keeps_line_terminal_scope() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let b = run(&mut s, "component", json!({"family":"Heat Detector","at":[2000,0]}))["id"].clone();
    let terminal = run(&mut s, "electrical_terminal", json!({"device":a,"name":"A","address":"42"}))["id"].clone();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[a,b],"mark_prefix":"A","terminals":[terminal]}));
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[a],"mark_prefix":"B"}));
    run(&mut s, "electrical_line", json!({"source":a,"devices":[b],"mark_prefix":"C"}));
    let report = run(&mut s, "electrical_line_table", json!({}));
    assert_eq!(report["device_count"], 2);
    assert_eq!(report["elements"], json!([[panel], [a], [b], [panel], [a], [a], [b]]));
    let columns = report["columns"].as_array().unwrap();
    let marks = columns.iter().position(|v| v == "Mark").unwrap();
    let terminals = columns.iter().position(|v| v == "Terminals").unwrap();
    assert_eq!(report["rows"][1][marks], "A2");
    assert_eq!(report["rows"][4][marks], "B2");
    assert_eq!(report["rows"][5][marks], "C1");
    assert_eq!(report["rows"][1][terminals][0]["address"], "42");
    assert_eq!(report["rows"][4][terminals], json!([]));
    assert_eq!(report["rows"][5][terminals], json!([]));
}

#[test]
fn stage8_line_table_refuses_lossy_xlsx_without_overwriting() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[a],"system":"A\u{1}B"}));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.xlsx");
    std::fs::write(&path, b"previous report").unwrap();
    assert!(fails(&mut s, "electrical_line_table", json!({"path":path})).contains("XML cannot preserve"));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous report");
    let csv = dir.path().join("table.csv");
    run(&mut s, "electrical_line_table", json!({"path":csv}));
    assert!(std::fs::read_to_string(csv).unwrap().contains("A\u{1}B"));
}

#[test]
fn stage8_line_table_rejects_unrepresentable_nested_xlsx_cells() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let terminal = run(&mut s, "electrical_terminal", json!({"device":a,"name":"A","address":"\u{FFFE}"}))["id"].clone();
    let line = run(&mut s, "electrical_line", json!({"source":panel,"devices":[a],"terminals":[terminal]}))["id"].clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.xlsx");
    std::fs::write(&path, b"previous table").unwrap();
    assert!(fails(&mut s, "electrical_line_table", json!({"path":path})).contains("XML cannot preserve"));
    let mut terminals = Vec::new();
    let long = "\u{10000}".repeat(crate::model::MAX_TEXT);
    for _ in 0..40 {
        terminals.push(run(&mut s, "electrical_terminal", json!({"device":a,"name":long,"address":long,"purpose":long}))["id"].clone());
    }
    run(&mut s, "electrical_line_edit", json!({"id":line,"terminals":terminals}));
    assert!(fails(&mut s, "electrical_line_table", json!({"path":path})).contains("32767"));
    assert_eq!(std::fs::read(path).unwrap(), b"previous table");
}

#[test]
fn stage8_line_table_caps_aggregate_rows_before_overwriting() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let detector = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let id = run(&mut s, "electrical_line", json!({"source":panel,"devices":[detector]}))["id"].as_u64().unwrap();
    let original = s.model.elements.get(id).unwrap().clone();
    let model = std::sync::Arc::make_mut(&mut s.model);
    for _ in 1..crate::model::MAX_POINTS / 2 {
        let id = model.fresh_id();
        model.elements.insert(id, original.clone());
    }
    let report = run(&mut s, "electrical_line_table", json!({}));
    assert_eq!(report["rows"].as_array().unwrap().len(), crate::model::MAX_POINTS);
    let model = std::sync::Arc::make_mut(&mut s.model);
    let id = model.fresh_id();
    model.elements.insert(id, original);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.csv");
    std::fs::write(&path, b"previous table").unwrap();
    assert!(fails(&mut s, "electrical_line_table", json!({"path":path})).contains("rows and copied terminals"));
    assert_eq!(std::fs::read(path).unwrap(), b"previous table");
}

#[test]
fn stage8_specification_deduplicates_members_and_counts_nonspatial_quantities() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].as_u64().unwrap();
    let smoke = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].as_u64().unwrap();
    let heat = run(&mut s, "component", json!({"family":"Heat Detector","at":[2000,0]}))["id"].as_u64().unwrap();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[smoke,heat],"system":"СПС"}));
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[smoke],"system":"СОУЭ"}));
    for quantity in [2, 4] {
        run(
            &mut s,
            "equipment",
            json!({"name":"License","quantity":quantity,"system":"СПС","product":{"description":"Software","brand":"LV","article":"LIC","manufacturer":"Own","mass_kg":0.0}}),
        );
    }
    let (family, ty) = s.model.elements[heat].family().unwrap();
    let key = format!("{family}: {ty}");
    std::sync::Arc::make_mut(&mut s.model).library.set_placeholder(&Kind::FamilyType, &key, true).unwrap();
    let before = s.model.clone();
    let report = run(&mut s, "electrical_specification", json!({}));
    assert_eq!(report["rows"].as_array().unwrap().len(), 3);
    assert_eq!(report["excluded"], json!([heat]));
    assert_eq!(report["ready"], false);
    let items = report["items"].as_array().unwrap();
    for id in [panel, smoke] {
        let item = items.iter().find(|item| item["elements"].as_array().unwrap().contains(&json!(id))).unwrap();
        assert_eq!(item["quantity"], 1.0);
        assert_eq!(item["systems"], json!(["СОУЭ", "СПС"]));
    }
    let software = report["rows"].as_array().unwrap().iter().find(|row| row[1] == "Software").unwrap();
    assert_eq!(software[6], 6.0);
    assert_eq!(software[7], 0.0);
    let filtered = run(&mut s, "electrical_specification", json!({"system":"СОУЭ"}));
    assert_eq!(filtered["rows"].as_array().unwrap().len(), 2);
    assert_eq!(filtered["excluded"], json!([]));
    assert_eq!(*s.model, *before);
}

#[test]
fn stage8_specification_uses_the_type_name_for_a_blank_description() {
    let mut s = Session::new();
    run(&mut s, "equipment", json!({"name":"License","product":{"description":"   ","brand":"LV","article":"LIC","manufacturer":"Own"}}));
    let report = run(&mut s, "electrical_specification", json!({}));
    assert_eq!(report["rows"][0][1], "Specification Items: License");
}

#[test]
fn stage8_specification_counts_physical_wires_carriers_and_reserve_once() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let carrier = run(&mut s, "cable_carrier", json!({"points":[[0,0],[3000,0]]}))["ids"][0].clone();
    let wire = run(&mut s, "wire", json!({"points":[[0,0],[3000,0]],"carrier":carrier,"reserve":10,"cable":"КПС"}))["ids"][0].clone();
    assert!(wire.is_number() && carrier.is_number());
    for system in ["СПС", "СОУЭ"] {
        run(&mut s, "electrical_line", json!({"source":panel,"devices":[],"segments":[wire],"system":system}));
    }
    let report = run(&mut s, "electrical_specification", json!({}));
    let item = report["items"].as_array().unwrap().iter().find(|item| item["elements"].as_array().unwrap().contains(&wire)).unwrap();
    assert!((item["length_mm"].as_f64().unwrap() - 3300.0).abs() < 1e-6);
    assert!((item["quantity"].as_f64().unwrap() - 3.3).abs() < 1e-9);
    let item = report["items"].as_array().unwrap().iter().find(|item| item["elements"].as_array().unwrap().contains(&carrier)).unwrap();
    assert_eq!(item["length_mm"], 3000.0);
    run(&mut s, "wire_connect", json!({"wire":wire,"device":panel,"point":[0,0,1000]}));
    let parent = s.model.guid(wire.as_u64().unwrap());
    let members: Vec<_> = s
        .model
        .elements
        .with_guids()
        .filter_map(|(id, guid, element)| match element {
            crate::model::Element::Wire(run) if guid == parent || run.feeder.as_ref().is_some_and(|f| f.wire == parent) => {
                Some((id, s.model.run_geom(element).unwrap().length()))
            }
            _ => None,
        })
        .collect();
    assert!(members.len() > 1, "connection creates physical feeder legs");
    let expected = members.iter().map(|(_, length)| length).sum::<f64>() * 1.1;
    let report = run(&mut s, "electrical_specification", json!({}));
    let item = report["items"].as_array().unwrap().iter().find(|item| item["elements"].as_array().unwrap().contains(&wire)).unwrap();
    assert!((item["length_mm"].as_f64().unwrap() - expected).abs() < 1e-6);
    assert_eq!(item["elements"].as_array().unwrap().len(), members.len());
}

#[test]
fn stage8_specification_exports_atomically_and_refuses_hostile_parameters() {
    let mut s = Session::new();
    let dir = tempfile::tempdir().unwrap();
    run(
        &mut s,
        "equipment",
        json!({"name":"=License","quantity":2,"system":"СПС","product":{"description":"=Software","brand":"LV","article":"LIC","manufacturer":"Own"}}),
    );
    let before = s.model.clone();
    for suffix in ["csv", "xlsx"] {
        let path = dir.path().join(format!("specification.{suffix}"));
        run(&mut s, "electrical_specification", json!({"path":path}));
        let bytes = std::fs::read(&path).unwrap();
        if suffix == "csv" {
            assert!(String::from_utf8(bytes).unwrap().contains("'=Software"));
        } else {
            assert_eq!(&bytes[..2], b"PK");
        }
    }
    assert_eq!(*s.model, *before);
    run(&mut s, "equipment", json!({"name":"CR text","product":{"description":"A\rB"}}));
    let cr_path = dir.path().join("cr.xlsx");
    run(&mut s, "electrical_specification", json!({"path":cr_path}));
    let parts = crate::xlsx::unzip(&std::fs::read(cr_path).unwrap());
    assert!(parts["xl/worksheets/sheet1.xml"].contains("A&#13;B"));
    let before = s.model.clone();
    let manual = dir.path().join("manual.odt");
    std::fs::write(&manual, b"manual text").unwrap();
    for params in [json!([]), json!({"system":false}), json!({"id":100000}), json!({"path":manual}), json!({"path":false})] {
        fails(&mut s, "electrical_specification", params);
    }
    assert_eq!(std::fs::read(manual).unwrap(), b"manual text");
    assert_eq!(*s.model, *before);
}

#[test]
fn stage8_specification_caps_carrier_system_memberships_before_overwrite() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let carrier = run(&mut s, "cable_carrier", json!({"points":[[0,0],[3000,0]]}))["ids"][0].clone();
    let wire = run(&mut s, "wire", json!({"points":[[0,0],[3000,0]],"carrier":carrier}))["ids"][0].clone();
    let line = run(&mut s, "electrical_line", json!({"source":panel,"devices":[],"segments":[wire],"system":"S0"}))["id"].as_u64().unwrap();
    let crate::model::Element::ElectricalLine(template) = s.model.elements[line].clone() else {
        panic!("line expected");
    };
    let model = std::sync::Arc::make_mut(&mut s.model);
    for i in 1..4000 {
        let mut line = template.clone();
        line.system = format!("S{i}");
        model.insert(crate::model::Element::ElectricalLine(line));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("specification.csv");
    std::fs::write(&path, b"existing report").unwrap();
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("10000"));
    assert_eq!(std::fs::read(path).unwrap(), b"existing report");
}

#[test]
fn stage8_specification_refuses_lossy_and_oversized_exports_before_overwrite() {
    let mut s = Session::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("specification.xlsx");
    std::fs::write(&path, b"existing report").unwrap();
    let id =
        run(&mut s, "equipment", json!({"name":"License","quantity":2,"product":{"description":"Software\u{1}"}}))["id"].as_u64().unwrap();
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("XML"));
    assert_eq!(std::fs::read(&path).unwrap(), b"existing report");
    let element = s.model.elements[id].clone();
    let model = std::sync::Arc::make_mut(&mut s.model);
    for _ in 0..crate::model::MAX_POINTS {
        model.insert(element.clone());
    }
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("10000"));
    assert_eq!(std::fs::read(path).unwrap(), b"existing report");
}

#[test]
fn stage8_specification_pdf_paginates_without_changing_the_model() {
    let mut s = Session::new();
    for i in 0..80 {
        run(
            &mut s,
            "equipment",
            json!({"name":format!("License {i}"),"quantity":2,"system":"СПС","product":{"description":format!("Software {i}\nManual text"),"brand":"LV","article":"LIC","manufacturer":"Own","mass_kg":0.0}}),
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("specification.pdf");
    let before = s.model.clone();
    let report = run(&mut s, "electrical_specification", json!({"path":path}));
    assert!(report["pages"].as_u64().unwrap() > 1);
    assert_eq!(report["rows"].as_array().unwrap().len(), 80);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert_eq!(std::sync::Arc::as_ptr(&s.model), std::sync::Arc::as_ptr(&before));
    assert!(s.script_dry_run(&format!("electrical_specification {}", json!({"path":path}))).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn stage8_specification_pdf_refuses_unprintable_cells_and_empty_selection_before_overwrite() {
    let mut s = Session::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("specification.pdf");
    std::fs::write(&path, b"existing report").unwrap();
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("no items"));
    run(&mut s, "equipment", json!({"name":"License","product":{"description":"Software\u{1}"}}));
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("control"));
    let mut s = Session::new();
    run(&mut s, "equipment", json!({"name":"License","product":{"description":"X\n".repeat(100)}}));
    assert!(fails(&mut s, "electrical_specification", json!({"path":path})).contains("fit"));
    assert_eq!(std::fs::read(path).unwrap(), b"existing report");
}

#[test]
fn stage8_diagram_groups_types_per_floor_and_line_with_one_head() {
    let mut s = Session::new();
    let upper = run(&mut s, "level", json!({"name":"Upper","elevation":4000}))["id"].clone();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0],"level":upper}))["id"].clone();
    let b = run(&mut s, "component", json!({"family":"Smoke Detector","at":[2000,0],"level":upper}))["id"].clone();
    let c = run(&mut s, "component", json!({"family":"Heat Detector","at":[3000,0],"level":upper}))["id"].clone();
    let first = run(&mut s, "electrical_line", json!({"source":panel,"devices":[a,b,c],"system":"СПС","mark_prefix":"L2."}))["id"].clone();
    run(&mut s, "electrical_line_mark", json!({"id":first,"device":b,"mark":"LOCKED"}));
    let second = run(&mut s, "electrical_line", json!({"source":panel,"devices":[a],"system":"СПС","mark_prefix":"L10."}))["id"].clone();
    let before = s.model.clone();
    let report = run(&mut s, "electrical_line_diagram", json!({"mode":"grouped"}));
    let diagram = &report["diagram"];
    assert_eq!(diagram["heads"].as_array().unwrap().len(), 1);
    assert_eq!(diagram["heads"][0]["id"], panel);
    assert_eq!(diagram["bands"][0]["name"], "Upper");
    let rows = diagram["bands"][0]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["line"], first);
    assert_eq!(rows[1]["line"], second);
    assert_eq!(rows[0]["groups"][0]["count"], 2);
    assert_eq!(rows[0]["groups"][0]["ids"], json!([a, b]));
    assert_eq!(rows[0]["groups"][0]["marks"], json!(["L2.2", "LOCKED"]));
    assert_eq!(rows[0]["groups"][1]["count"], 1);
    assert_eq!(rows[1]["groups"][0]["count"], 1);
    assert_eq!(s.model, before);
    assert_eq!(run(&mut s, "electrical_line_diagram", json!({"system":"Missing","mode":"grouped"}))["pages"], 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("diagram.pdf");
    s.read_only = Some("reports only");
    let exported = run(&mut s, "electrical_line_diagram", json!({"path":path,"mode":"grouped"}));
    assert_eq!(exported["diagram"], *diagram);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(s.script_dry_run(&format!("electrical_line_diagram {}", json!({"path":path}))).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(s.model, before);
}

#[test]
fn stage8_diagram_full_and_simplified_keep_branch_boundaries_and_marks() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let mut devices = Vec::new();
    for i in 1..=4 {
        devices.push(run(&mut s, "component", json!({"family":"Smoke Detector","at":[i*1000,0]}))["id"].clone());
    }
    let branch = run(&mut s, "component", json!({"family":"Heat Detector","at":[1000,1000]}))["id"].clone();
    let line = run(
        &mut s,
        "electrical_line",
        json!({"source":panel,"devices":devices,"branches":[{"from":devices[0],"devices":[branch]}],"mark_prefix":"L"}),
    )["id"]
        .clone();
    let full = run(&mut s, "electrical_line_diagram", json!({"id":line,"mode":"full"}));
    let chains = full["segments"].as_array().unwrap();
    let ids: Vec<_> = chains
        .iter()
        .flat_map(|chain| chain["items"].as_array().unwrap().iter())
        .flat_map(|item| item["ids"].as_array().unwrap().iter().cloned())
        .collect();
    assert_eq!(ids, [devices[0].clone(), branch, devices[1].clone(), devices[2].clone(), devices[3].clone()]);
    assert!(chains.iter().any(|chain| chain["from"] == devices[0]));
    let simplified = run(&mut s, "electrical_line_diagram", json!({"id":line,"mode":"simplified"}));
    let gaps: Vec<_> = simplified["segments"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|chain| chain["items"].as_array().unwrap().iter())
        .filter(|item| item["kind"] == "gap")
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0]["ids"], json!([devices[2]]));
    assert_eq!(gaps[0]["omitted"], 1);
    assert_eq!(gaps[0]["marks"], json!(["L5"]));
    run(&mut s, "electrical_line_mark", json!({"id":line,"device":devices[2],"mark":"MANUAL"}));
    let locked = run(&mut s, "electrical_line_diagram", json!({"id":line,"mode":"simplified"}));
    assert!(
        locked["segments"].as_array().unwrap().iter().all(|chain| chain["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["kind"] != "gap"))
    );
}

#[test]
fn stage8_diagram_keeps_explicit_branch_start_after_trunk_leaf() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let mut devices = Vec::new();
    for i in 1..=6 {
        devices.push(run(&mut s, "component", json!({"family":"Smoke Detector","at":[i*1000,0]}))["id"].clone());
    }
    let line = run(
        &mut s,
        "electrical_line",
        json!({
            "source":panel,"devices":devices[..3],
            "branches":[{"from":devices[2],"devices":devices[3..]}],"mark_prefix":"L"
        }),
    )["id"]
        .clone();
    for mode in ["full", "simplified"] {
        let report = run(&mut s, "electrical_line_diagram", json!({"id":line,"mode":mode}));
        let segments = report["segments"].as_array().unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1]["from"], devices[2]);
        assert_eq!(segments[0]["items"].as_array().unwrap().last().unwrap()["ids"], json!([devices[2]]));
        assert_eq!(segments[1]["items"][0]["ids"], json!([devices[3]]));
    }
}

#[test]
fn stage8_diagram_nested_forks_have_distinct_nonconnecting_routes() {
    use crate::render::{Part, Prim};
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let mut devices = Vec::new();
    for i in 1..=5 {
        devices.push(run(&mut s, "component", json!({"family":"Smoke Detector","at":[i*1000,0]}))["id"].clone());
    }
    let line = run(
        &mut s,
        "electrical_line",
        json!({
            "source":panel,"devices":[devices[0],devices[4]],
            "branches":[{"from":devices[0],"devices":[devices[1],devices[3]]},
                        {"from":devices[1],"devices":[devices[2]]}]
        }),
    )["id"]
        .as_u64()
        .unwrap();
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, Some(line), None, None).unwrap();
    let segments = crate::low_voltage::diagram_devices::segments(&s.model, Some(line), None, None, false).unwrap();
    let pages = crate::low_voltage::diagram_layout::draw_devices(&s.model, &diagram, &segments, false).unwrap();
    assert_eq!(pages.len(), 1);
    let mut gutters: Vec<_> = pages[0]
        .prims
        .iter()
        .filter_map(|prim| {
            let Prim::Line { pts, .. } = prim else { return None };
            pts.windows(2).find_map(|pair| {
                let [(x, y), (xx, yy)] = pair else { return None };
                (*x == *xx && (254.0 * 5.0..270.0 * 5.0).contains(x) && (y - yy).abs() > 20.0).then_some(*x)
            })
        })
        .collect();
    gutters.sort_by(f32::total_cmp);
    gutters.dedup();
    assert!(gutters.len() >= 2, "different parents must not share one visible vertical connection");
    let verticals: Vec<_> = pages[0]
        .prims
        .iter()
        .filter_map(|prim| {
            let Prim::Line { pts, .. } = prim else { return None };
            if pts.len() != 2 {
                return None;
            }
            let (a, b) = (pts[0], pts[1]);
            (a.0 == b.0 && (254.0 * 5.0..270.0 * 5.0).contains(&a.0)).then_some((a, b))
        })
        .collect();
    for (prim, part) in pages[0].prims.iter().zip(&pages[0].parts) {
        if *part == Part::Text {
            continue;
        }
        let Prim::Line { pts, .. } = prim else { continue };
        if pts.len() != 2 || pts[0].1 != pts[1].1 {
            continue;
        }
        let (a, b) = (pts[0], pts[1]);
        for &(v, w) in &verticals {
            let crossing = v.0 > a.0.min(b.0) && v.0 < a.0.max(b.0) && a.1 > v.1.min(w.1) && a.1 < v.1.max(w.1);
            assert!(!crossing, "crossing routes need a visible nonconnecting gap: {a:?}–{b:?} crosses {v:?}–{w:?}");
        }
    }
}

#[test]
fn stage8_diagram_source_taps_do_not_join_neighboring_risers() {
    use crate::render::{Part, Prim};
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let b = run(&mut s, "component", json!({"family":"Heat Detector","at":[2000,0]}))["id"].clone();
    let first = run(&mut s, "electrical_line", json!({"source":panel,"devices":[a],"mark_prefix":"L1"}))["id"].as_u64().unwrap();
    let second = run(&mut s, "electrical_line", json!({"source":panel,"devices":[b],"mark_prefix":"L2"}))["id"].as_u64().unwrap();
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, None, None, None).unwrap();
    let segments = crate::low_voltage::diagram_devices::segments(&s.model, None, None, None, false).unwrap();
    for pages in [
        crate::low_voltage::diagram_layout::draw_devices(&s.model, &diagram, &segments, false).unwrap(),
        crate::low_voltage::diagram_layout::draw_devices(&s.model, &diagram, &segments, true).unwrap(),
        crate::low_voltage::diagram_layout::draw(&s.model, &diagram).unwrap(),
    ] {
        for ((prim, owner), part) in pages[0].prims.iter().zip(&pages[0].owners).zip(&pages[0].parts) {
            if *part == Part::Text {
                continue;
            }
            let other_x = if *owner == Some(first) {
                130.0 * 5.0
            } else if *owner == Some(second) {
                118.0 * 5.0
            } else {
                continue;
            };
            let Prim::Line { pts, .. } = prim else { continue };
            for pair in pts.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                assert!(
                    !(a.1 == b.1 && (52.0 * 5.0..=244.0 * 5.0).contains(&a.1) && a.0.min(b.0) < other_x && a.0.max(b.0) > other_x),
                    "source tap must visibly cross the other line's riser without joining it"
                );
            }
        }
    }
}

#[test]
fn stage8_diagram_continuation_references_source_and_keeps_heading_on_paper() {
    use crate::render::{Part, Prim};
    let mut s = Session::new();
    let level = run(&mut s, "level", json!({"name":"A".repeat(120),"elevation":4000}))["id"].clone();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0],"level":level}))["id"].clone();
    let mut devices = Vec::new();
    for i in 1..=36 {
        devices.push(run(&mut s, "component", json!({"family":"Smoke Detector","at":[i*1000,0],"level":level}))["id"].clone());
    }
    let line = run(&mut s, "electrical_line", json!({"source":panel,"devices":devices}))["id"].as_u64().unwrap();
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, Some(line), None, None).unwrap();
    let segments = crate::low_voltage::diagram_devices::segments(&s.model, Some(line), None, None, false).unwrap();
    let pages = crate::low_voltage::diagram_layout::draw_devices(&s.model, &diagram, &segments, false).unwrap();
    assert!(pages.len() > 1);
    for (i, page) in pages.iter().enumerate() {
        let source_links: Vec<_> = page
            .prims
            .iter()
            .filter_map(|p| match p {
                Prim::Line { pts, dash, .. } if pts == &[(150.0, 270.0), (590.0, 270.0)] => Some(dash),
                _ => None,
            })
            .collect();
        assert_eq!(source_links.len(), 1);
        assert_eq!(source_links[0].is_some(), i > 0);
        for (prim, part) in page.prims.iter().zip(&page.parts) {
            if *part == Part::Text && prim.points().iter().any(|p| p.1 >= 22.0 * 5.0 && p.1 <= 36.0 * 5.0) {
                assert!(prim.points().iter().all(|p| p.0 <= 408.0 * 5.0), "heading clips outside A3");
            }
        }
    }
}

#[test]
fn stage8_diagram_preserves_conflicting_source_marks_and_orders_lanes() {
    use crate::render::Prim;
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let l10 = run(&mut s, "electrical_line", json!({"source":panel,"devices":[],"mark_prefix":"L10."}))["id"].clone();
    let l2 = run(&mut s, "electrical_line", json!({"source":panel,"devices":[],"mark_prefix":"L2."}))["id"].clone();
    run(&mut s, "electrical_line_mark", json!({"id":l10,"device":panel,"mark":"PANEL-10"}));
    run(&mut s, "electrical_line_mark", json!({"id":l2,"device":panel,"mark":"PANEL-2"}));
    let report = run(&mut s, "electrical_line_diagram", json!({"mode":"grouped"}));
    assert_eq!(report["diagram"]["heads"].as_array().unwrap().len(), 1);
    assert_eq!(report["diagram"]["heads"][0]["line_marks"][l10.to_string()], "PANEL-10");
    assert_eq!(report["diagram"]["heads"][0]["line_marks"][l2.to_string()], "PANEL-2");
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, None, None, None).unwrap();
    let pages = crate::low_voltage::diagram_layout::draw(&s.model, &diagram).unwrap();
    let lane = pages[0].prims.iter().zip(&pages[0].owners).find_map(|(p, owner)| match p {
        Prim::Line { pts, .. } if pts == &[(590.0, 260.0), (590.0, 1220.0)] => *owner,
        _ => None,
    });
    assert_eq!(lane, l2.as_u64());
    run(&mut s, "electrical_line_mark", json!({"id":l10,"device":panel,"mark":"PANEL-2"}));
    let single = run(&mut s, "electrical_line_diagram", json!({"id":l10}));
    assert_eq!(single["diagram"]["heads"][0]["mark"], "PANEL-2");
}

#[test]
fn stage8_diagram_draws_family_lines_dots_and_labels_in_all_modes() {
    use crate::render::{Part, Prim};
    let mut s = Session::new();
    for name in ["Fire Alarm Panel", "Smoke Detector"] {
        run(
            &mut s,
            "family",
            json!({"name":name,"symbols":[
                {"line":[[0,0],[10,0],[5,10],[0,0]]},
                {"dot":{"centre":[1,1],"radius":0.5}},
                {"text":{"at":[5,5],"height":4,"value":"A","align":"center"}}
            ]}),
        );
    }
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].as_u64().unwrap();
    let device = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].as_u64().unwrap();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[device]}));
    let before = s.model.clone();
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, None, None, None).unwrap();
    for mode in ["full", "simplified", "grouped"] {
        let report = run(&mut s, "electrical_line_diagram", json!({"mode":mode}));
        assert!(
            report["diagnostics"].as_array().unwrap().iter().all(|note| !note.as_str().unwrap().contains("generic rectangle")),
            "{mode}"
        );
        let pages = if mode == "grouped" {
            crate::low_voltage::diagram_layout::draw(&s.model, &diagram).unwrap()
        } else {
            let segments = crate::low_voltage::diagram_devices::segments(&s.model, None, None, None, mode == "simplified").unwrap();
            crate::low_voltage::diagram_layout::draw_devices(&s.model, &diagram, &segments, mode == "simplified").unwrap()
        };
        assert_eq!(pages.len(), 1, "{mode}");
        let page = &pages[0];
        for id in [panel, device] {
            let owned: Vec<_> = page
                .prims
                .iter()
                .zip(&page.owners)
                .zip(&page.parts)
                .filter_map(|((p, owner), part)| (*owner == Some(id)).then_some((p, part)))
                .collect();
            assert!(
                owned
                    .iter()
                    .any(|(p, part)| **part != Part::Text && matches!(p, Prim::Line { pts, .. } if pts.len() == 4 && pts[0] == pts[3])),
                "{mode}: line {id}"
            );
            assert!(
                owned
                    .iter()
                    .any(|(p, part)| **part != Part::Text
                        && matches!(p, Prim::Fill { rings, .. } if rings.len() == 1 && rings[0].len() == 24)),
                "{mode}: dot {id}"
            );
            assert!(owned.iter().any(|(_, part)| **part == Part::Text), "{mode}: label {id}");
        }
    }
    assert_eq!(*s.model, *before);
}

#[test]
fn stage8_diagram_reports_unsupported_symbol_glyphs() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    run(&mut s, "electrical_line", json!({"source":panel,"devices":[]}));
    std::sync::Arc::make_mut(&mut s.model).families.get_mut("Fire Alarm Panel").unwrap().symbols =
        vec![serde_json::from_value(json!({"text":{"at":[0,0],"height":5,"value":"火"}})).unwrap()];
    let diagram = crate::low_voltage::diagram_layout::build(&s.model, None, None, None).unwrap();
    let pages = crate::low_voltage::diagram_layout::draw(&s.model, &diagram).unwrap();
    assert_eq!(pages[0].notes.iter().any(|n| n.contains("unsupported symbol label glyph")), !crate::font::supports("火"));
}

#[test]
fn stage8_diagram_rejects_invalid_modes_and_preserves_existing_files() {
    let mut s = Session::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manual.odt");
    std::fs::write(&path, b"manual paragraphs").unwrap();
    for params in
        [json!([]), json!({"mode":"unknown"}), json!({"mode":false}), json!({"id":100000}), json!({"level":100000}), json!({"path":path})]
    {
        fails(&mut s, "electrical_line_diagram", params);
    }
    assert_eq!(std::fs::read(path).unwrap(), b"manual paragraphs");
}

#[test]
fn fire_battery_capacity_checks_both_operating_modes() {
    let mut s = Session::new();
    let mut params = json!({
        "standby_a": 0.25, "fire_a": 1.5, "source_a": 0.25,
        "end_capacity_percent": 80, "load_factor": 1.2, "installed_ah": 20
    });
    let result = run(&mut s, "fire_battery", params.clone());
    assert!((result["required_ah"].as_f64().unwrap() - 20.625).abs() < 1e-9);
    assert_eq!(result["status"], "insufficient_capacity");
    params["installed_ah"] = json!(21);
    let result = run(&mut s, "fire_battery", params.clone());
    assert_eq!(result["status"], "sufficient_capacity");
    params.as_object_mut().unwrap().remove("load_factor");
    let result = run(&mut s, "fire_battery", params.clone());
    assert_eq!(result["missing"], json!(["load_factor"]));
    assert!(result["required_ah"].is_null());
    params["end_capacity_percent"] = json!(0);
    assert!(fails(&mut s, "fire_battery", params).contains("end_capacity_percent"));
}

#[test]
fn fire_battery_uses_mode_specific_factors_and_reports_fire_runtime() {
    let mut s = Session::new();
    let mut params = json!({
        "standby_a": 0.25, "fire_a": 1.5, "source_a": 0.25,
        "end_capacity_percent": 80, "standby_load_factor": 1.0,
        "fire_load_factor": 2.0, "installed_ah": 30
    });
    let report = run(&mut s, "fire_battery", params.clone());
    assert_eq!(report["standard"], "SP 6.13130.2025 Appendix B, B.2");
    assert_eq!(report["required_ah"], 19.375);
    assert_eq!(report["fire_runtime_h"], 16.0);
    assert_eq!(report["status"], "sufficient_capacity");

    params["fire_load_factor"] = Value::Null;
    let report = run(&mut s, "fire_battery", params.clone());
    assert_eq!(report["missing"], json!(["fire_load_factor"]));
    assert!(report["required_ah"].is_null());
    assert_eq!(report["fire_runtime_h"], 16.0);

    params["load_factor"] = json!(1.2);
    assert!(fails(&mut s, "fire_battery", params.clone()).contains("load_factor"));
    params.as_object_mut().unwrap().remove("load_factor");
    params["fire_load_factor"] = json!(2.0);
    params["fire_a"] = json!(0.0);
    assert!(run(&mut s, "fire_battery", params)["fire_runtime_h"].is_null());
}

#[test]
fn stage7_soue_and_okl_reports_distinguish_failures_and_missing_data() {
    let mut s = Session::new();
    let amp = run(&mut s, "soue_amplifier_load", json!({"rated_w":50,"reserve_percent":20,"taps_w":[25,20,null]}));
    assert_eq!(amp["allowed_w"], 40.0);
    assert_eq!(amp["status"], "overload");
    assert_eq!(amp["missing_taps"], json!([3]));
    let sound = run(
        &mut s,
        "soue_sound_check",
        json!({"room":"other","points":[{"noise_dba":60,"signal_dba":75},{"noise_dba":86,"signal_dba":103,"supplementary_alarm":false},{}]}),
    );
    assert_eq!(sound["points"][0]["violations"], json!([]));
    assert_eq!(sound["points"][1]["violations"], json!(["supplementary_alarm_required", "below_minimum"]));
    assert_eq!(sound["points"][2]["missing"], json!(["noise_dba", "signal_dba"]));
    let installed = json!({"cable":"A","carrier":"B","fastener":"C","mounting":"concrete ceiling","pitch_mm":300});
    let mut tested = installed.clone();
    tested["document"] = json!("test report 01");
    tested["duration_min"] = json!(60);
    let mut params = json!({"required_min":60,"installed":installed,"tested":tested});
    assert_eq!(run(&mut s, "fire_okl_check", params.clone())["status"], "matches_declared_evidence");
    params["installed"]["fastener"] = json!("D");
    params["tested"]["duration_min"] = json!(45);
    let report = run(&mut s, "fire_okl_check", params);
    assert_eq!(report["mismatches"], json!(["fastener"]));
    assert_eq!(report["insufficient_duration"], true);
    assert_eq!(run(&mut s, "fire_okl_check", json!({"required_min":60}))["status"], "missing_evidence");
}

#[test]
fn stage7_parts_estimate_counts_end_supports_and_section_joints() {
    let mut s = Session::new();
    let report = run(
        &mut s,
        "fire_carrier_parts",
        json!({"runs":[{"length_m":6,"support_pitch_m":1.5,"section_m":2},{"length_m":2.5,"support_pitch_m":1}],
            "bends":1,"tees":2,"unlicensed_workstations":1}),
    );
    assert_eq!(report, json!({"supports":9,"connectors":2,"bends":1,"tees":2,"software_licenses":1}));
}

#[test]
fn stage7_radial_voltage_uses_downstream_current_and_both_conductors() {
    let mut s = Session::new();
    let mut params = json!({"source_v":24,"segments":[
        {"length_m":100,"ohm_per_km":10,"load_ma":100,"min_v":23},
        {"length_m":150,"ohm_per_km":10,"load_ma":200,"min_v":23}
    ]});
    let report = run(&mut s, "fire_voltage_drop", params.clone());
    let points = &report["segments"];
    assert!((points[0]["voltage_v"].as_f64().unwrap() - 23.4).abs() < 1e-9);
    assert!((points[1]["voltage_v"].as_f64().unwrap() - 22.8).abs() < 1e-9);
    assert_eq!(points[0]["below_minimum"], false);
    assert_eq!(points[1]["below_minimum"], true);
    params["segments"][1].as_object_mut().unwrap().remove("load_ma");
    let report = run(&mut s, "fire_voltage_drop", params);
    assert_eq!(report["missing"], json!([{"segment":2,"fields":["load_ma"]}]));
    assert!(report["segments"].is_null());
}

#[test]
fn stage7_line_load_reports_each_mode_and_missing_device_data() {
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let a = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let b = run(&mut s, "component", json!({"family":"Smoke Detector","at":[2000,0]}))["id"].clone();
    let line = run(&mut s, "electrical_line", json!({"source":panel,"devices":[a,b]}))["id"].clone();
    let mut params = json!({
        "id":line, "limit_ma":100, "reserve_percent":20,
        "loads":[{"id":a,"standby_ma":10,"fire_ma":35},{"id":b,"standby_ma":15,"fire_ma":50}]
    });
    let report = run(&mut s, "fire_line_load", params.clone());
    assert_eq!(report["allowed_ma"], 80.0);
    assert_eq!(report["standby"]["total_ma"], 25.0);
    assert_eq!(report["standby"]["over_limit"], false);
    assert_eq!(report["fire"]["total_ma"], 85.0);
    assert_eq!(report["fire"]["over_limit"], true);
    params["loads"][1].as_object_mut().unwrap().remove("fire_ma");
    let report = run(&mut s, "fire_line_load", params.clone());
    assert_eq!(report["fire"]["missing_ids"], json!([b]));
    assert!(report["fire"]["over_limit"].is_null());
    params["loads"][1]["id"] = panel;
    assert!(fails(&mut s, "fire_line_load", params).contains("line load"));
}

#[test]
fn fire_line_searches_three_isolators_across_four_detection_zones() {
    assert_fire_line_isolator_set(3);
}

#[test]
fn fire_line_searches_four_isolators_across_five_detection_zones() {
    assert_fire_line_isolator_set(4);
}

fn assert_fire_line_isolator_set(required: usize) {
    let mut s = Session::new();
    let positions = &[[0, 0], [4000, 0], [8000, 0], [8000, 4000], [4000, 4000], [0, 4000]][..required + 2];
    let ids: Vec<_> = positions
        .iter()
        .enumerate()
        .map(|(i, at)| {
            run(&mut s, "component", json!({"family":if i == 0 {"Fire Alarm Panel"} else {"Smoke Detector"},"at":at}))["id"].clone()
        })
        .collect();
    let terminals: Vec<_> = ["Outgoing", "Return"]
        .into_iter()
        .map(|name| run(&mut s, "electrical_terminal", json!({"device":ids[0],"name":name}))["id"].clone())
        .collect();
    let level = s.model.levels()[0].0;
    for (i, &[x, y]) in positions.iter().enumerate().skip(1) {
        let boundary = [[x - 500, y - 500], [x + 500, y - 500], [x + 500, y + 500], [x - 500, y + 500]];
        run(
            &mut s,
            "fire_zone",
            json!({"zone":{"name":format!("Zone {i}"),"kind":"detection","level":level,"height":3500,"boundary":boundary}}),
        );
    }
    let mut wires = Vec::new();
    for i in 0..positions.len() {
        let next = (i + 1) % positions.len();
        let wire = run(&mut s, "wire", json!({"points":[positions[i],positions[next]],"offset":1000}))["ids"][0].clone();
        for (end, device) in [i, next].into_iter().enumerate() {
            let mut params = json!({"wire":wire,"device":ids[device],"point":[0,0,1000],"end":end});
            if device == 0 {
                params["terminal"] = terminals[end].clone();
            }
            run(&mut s, "wire_connect", params);
        }
        wires.push(wire);
    }
    let line =
        run(&mut s, "electrical_line", json!({"source":ids[0],"devices":ids[1..],"segments":wires,"terminals":terminals}))["id"].clone();
    let before = s.model.clone();
    let smaller =
        run(&mut s, "fire_line_faults", json!({"id":line,"propose":true,"max_isolators":required-1,"source_isolates_ports":true}));
    let status_key = if required == 3 { "pair_status_after" } else { "set_status_after" };
    assert_eq!(smaller[status_key], "fail", "{smaller}");
    let report = run(&mut s, "fire_line_faults", json!({"id":line,"propose":true,"max_isolators":required,"source_isolates_ports":true}));
    assert_eq!(report["placement_status"], "proposed_set", "{report}");
    assert_eq!(report["set_status_after"], "pass", "{report}");
    let placements = report["placements"].as_array().unwrap();
    assert_eq!(placements.len(), required);
    assert_eq!(s.model, before);
    let script = placements
        .iter()
        .map(|place| {
            format!(
                "fire_isolator {}",
                json!({
                    "id":line,"wire":place["wire"],"fraction":place["fraction"],"family":"Sounder","apply":true
                })
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.script_atomic(&script).unwrap();
    let applied = run(&mut s, "fire_line_faults", json!({"id":line,"source_isolates_ports":true}));
    assert_eq!(applied["status"], "pass", "{applied}");
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before);
}

#[test]
fn fire_line_faults_follow_wired_loop_and_transit_isolator() {
    let mut s = Session::new();
    let positions = [[0, 0], [4000, 0], [4000, 4000], [0, 4000]];
    let ids: Vec<_> = ["Fire Alarm Panel", "Smoke Detector", "Sounder", "Manual Call Point"]
        .into_iter()
        .zip(positions)
        .map(|(family, at)| run(&mut s, "component", json!({"family":family,"at":at}))["id"].clone())
        .collect();
    let first = run(&mut s, "electrical_terminal", json!({"device":ids[0],"name":"Outgoing"}))["id"].clone();
    let last = run(&mut s, "electrical_terminal", json!({"device":ids[0],"name":"Return"}))["id"].clone();
    let mut wires = Vec::new();
    for i in 0..positions.len() {
        let next = (i + 1) % positions.len();
        let wire = run(&mut s, "wire", json!({"points":[positions[i],positions[next]],"offset":1000}))["ids"][0].clone();
        for (end, at) in [i, next].into_iter().enumerate() {
            let mut p = json!({"wire":wire,"device":ids[at],"point":[0,0,1000],"end":if i == 1 { 0 } else { end }});
            if at == 0 {
                p["terminal"] = if end == 0 { first.clone() } else { last.clone() };
            }
            run(&mut s, "wire_connect", p);
        }
        wires.push(wire);
    }
    let level = s.model.levels()[0].0;
    let zone = run(
        &mut s,
        "fire_zone",
        json!({"zone":{"name":"Detection","kind":"detection","level":level,"height":3500,
        "boundary":[[3000,-1000],[5000,-1000],[5000,1000],[3000,1000]]}}),
    )["id"]
        .clone();
    let line =
        run(&mut s, "electrical_line", json!({"source":ids[0],"devices":ids[1..],"segments":wires,"terminals":[first,last]}))["id"].clone();
    let before = s.model.clone();
    let unprotected = run(&mut s, "fire_line_faults", json!({"id":line}));
    assert_eq!(unprotected["status"], "fail");
    let short = &unprotected["faults"][1];
    assert_eq!(short["kind"], "short");
    assert_eq!(short["automatic_lost"], 1);
    assert_eq!(short["manual_lost"], 1);
    let protected = run(&mut s, "fire_line_faults", json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true}));
    assert_eq!(protected["status"], "pass", "{protected}");
    assert_eq!(protected["faults"][0]["automatic_lost"], 0, "the return port feeds an open loop");
    assert_eq!(protected["faults"][1]["automatic_lost"], 1);
    assert_eq!(protected["faults"][1]["manual_lost"], 0);
    assert_eq!(s.model, before);
    let already_protected =
        run(&mut s, "fire_line_faults", json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true,"propose":true}));
    assert_eq!(already_protected["placement_status"], "already_passes");
    let common_port = run(&mut s, "fire_line_faults", json!({"id":line,"propose":true}));
    assert_eq!(common_port["placement_status"], "no_improving_placement");
    let paired = run(&mut s, "fire_line_faults", json!({"id":line,"propose":true,"allow_pair":true}));
    assert_eq!(paired["placement_status"], "proposed_pair", "{paired}");
    assert_eq!(paired["pair_status_after"], "fail", "the short at the shared source remains: {paired}");
    let placements = paired["placements"].as_array().unwrap();
    assert_eq!(placements.len(), 2);
    assert_ne!(placements[0]["wire"], placements[1]["wire"]);
    let script = placements
        .iter()
        .map(|place| {
            format!(
                "fire_isolator {}",
                json!({"id":line,"wire":place["wire"],"fraction":place["fraction"],"family":"Sounder","apply":true})
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.script_atomic(&script).unwrap();
    let after = run(&mut s, "fire_line_faults", json!({"id":line}));
    let failed = |result: &Value| result["faults"].as_array().unwrap().iter().filter(|fault| fault["status"] == "fail").count();
    assert_eq!(paired["pair_failing_faults_after"], failed(&after));
    assert!(failed(&after) < failed(&common_port));
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before);
    let recommendation = run(&mut s, "fire_line_faults", json!({"id":line,"source_isolates_ports":true,"propose":true}));
    assert_eq!(recommendation["status"], "fail");
    let placement = &recommendation["placement"];
    assert!(wires[1..3].contains(&placement["wire"]), "{recommendation}");
    assert_eq!(placement["status_after"], "pass");
    assert_eq!(s.model, before);
    run(
        &mut s,
        "fire_isolator",
        json!({
            "id":line,"wire":placement["wire"],"fraction":placement["fraction"],"family":"Sounder","apply":true
        }),
    );
    assert_eq!(run(&mut s, "fire_line_faults", json!({"id":line,"source_isolates_ports":true}))["status"], "pass");
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before);
    let supply = run(&mut s, "component", json!({"family":"Distribution Board","at":[-4000,0]}))["id"].clone();
    let supply_wire = run(&mut s, "wire", json!({"points":[[-4000,0],[0,0]],"offset":1000}))["ids"][0].clone();
    for (end, device) in [&supply, &ids[0]].into_iter().enumerate() {
        run(&mut s, "wire_connect", json!({"wire":supply_wire,"device":device,"point":[0,0,1000],"end":end}));
    }
    let power = run(&mut s, "electrical_line", json!({"source":supply,"devices":[ids[0]],"segments":[supply_wire]}))["id"].clone();
    let with_power = s.model.clone();
    let mut params = json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true,"power":{"id":power}});
    let incomplete = run(&mut s, "fire_line_faults", params.clone());
    assert_eq!(incomplete["status"], "insufficient_data");
    assert_eq!(incomplete["communication_status"], "pass");
    params["power"]["independent"] = json!(ids[1..]);
    let assessed = run(&mut s, "fire_line_faults", params);
    assert_eq!(assessed["scope"], "communication_and_power");
    assert_eq!(assessed["status"], "fail");
    assert_eq!(assessed["communication_status"], "pass");
    assert_eq!(assessed["power"]["status"], "fail");
    let power_faults = assessed["power"]["faults"].as_array().unwrap();
    assert_eq!(power_faults.len(), 2);
    for fault in power_faults {
        assert_eq!(fault["wire"], supply_wire);
        assert_eq!(fault["automatic_lost"], 1);
        assert_eq!(fault["manual_lost"], 1);
    }
    assert_eq!(s.model, with_power);
    let relay_wire = run(&mut s, "wire", json!({"points":[[-4000,0],[4000,4000]],"offset":1000}))["ids"][0].clone();
    for (end, device) in [&supply, &ids[2]].into_iter().enumerate() {
        run(&mut s, "wire_connect", json!({"wire":relay_wire,"device":device,"point":[0,0,1000],"end":end}));
    }
    let relay_power = run(&mut s, "electrical_line", json!({"source":supply,"devices":[ids[2]],"segments":[relay_wire]}))["id"].clone();
    let both = run(
        &mut s,
        "fire_line_faults",
        json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true,
            "power":{"lines":[{"id":power},{"id":relay_power}],"independent":[ids[1],ids[3]]}}),
    );
    assert_eq!(both["communication_status"], "pass");
    assert_eq!(both["power"]["status"], "fail");
    let faults = both["power"]["faults"].as_array().unwrap();
    assert_eq!(faults.len(), 4);
    assert!(faults.iter().any(|fault| fault["wire"] == supply_wire && fault["automatic_lost"] == 1));
    assert!(faults.iter().any(|fault| fault["wire"] == relay_wire && fault["automatic_lost"] == 0));
    let dependent_wire = run(&mut s, "wire", json!({"points":[[4000,4000],[4000,0]],"offset":1000}))["ids"][0].clone();
    for (end, device) in [&ids[2], &ids[1]].into_iter().enumerate() {
        run(&mut s, "wire_connect", json!({"wire":dependent_wire,"device":device,"point":[0,0,1000],"end":end}));
    }
    let dependent = run(&mut s, "electrical_line", json!({"source":ids[2],"devices":[ids[1]],"segments":[dependent_wire]}))["id"].clone();
    let cascade = run(
        &mut s,
        "fire_line_faults",
        json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true,
            "power":{"lines":[{"id":dependent},{"id":relay_power}],"independent":[ids[0],ids[3]]}}),
    );
    let cascade_faults = cascade["power"]["faults"].as_array().unwrap();
    assert_eq!(cascade_faults.iter().filter(|fault| fault["wire"] == relay_wire && fault["automatic_lost"] == 1).count(), 2);
    let relay_only = run(
        &mut s,
        "fire_line_faults",
        json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true,
            "power":{"id":relay_power,"independent":[ids[0],ids[1],ids[3]]}}),
    );
    assert_eq!(relay_only["status"], "pass", "{relay_only}");
    assert_eq!(relay_only["power"]["faults"][0]["automatic_lost"], 0);
    assert_eq!(relay_only["power"]["faults"][0]["manual_lost"], 0);
    run(&mut s, "delete", json!({"ids":[zone]}));
    let no_zone = run(&mut s, "fire_line_faults", json!({"id":line,"isolators":[ids[2]],"source_isolates_ports":true}));
    assert_eq!(no_zone["status"], "insufficient_data");
}

#[test]
fn fire_isolator_splits_a_wired_segment_in_one_undo() {
    use crate::model::{Element, Id, Model};

    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let detector = run(&mut s, "component", json!({"family":"Smoke Detector","at":[4000,0]}))["id"].clone();
    let wire = run(&mut s, "wire", json!({"points":[[0,0],[4000,0]],"offset":1000}))["ids"][0].clone();
    // Connect in reverse order and use the same legacy end index on different devices.
    for device in [&detector, &panel] {
        run(&mut s, "wire_connect", json!({"wire":wire,"device":device,"point":[0,0,1000],"end":0}));
    }
    let line = run(&mut s, "electrical_line", json!({"source":panel,"devices":[detector],"segments":[wire]}))["id"].clone();
    let ids = |value: &Value| serde_json::from_value::<Id>(value.clone()).unwrap();
    let original_guid = s.model.guid(ids(&wire));
    let before = s.model.clone();
    let table_before = run(&mut s, "electrical_line_table", json!({"id":line}));
    let mut params = json!({"id":line,"wire":wire,"family":"Sounder","fraction":0.25});
    let preview = run(&mut s, "fire_isolator", params.clone());
    assert_eq!(preview["at"], json!([1000.0, 0.0, 1000.0]));
    assert_eq!(s.model, before);
    assert_eq!(run(&mut s, "electrical_line_table", json!({"id":line})), table_before);
    fails(&mut s, "fire_isolator", json!({"id":line,"wire":wire,"family":"Sounder","fraction":1,"apply":true}));
    params["apply"] = json!(true);
    let result = run(&mut s, "fire_isolator", params);
    let device = s.model.guid(ids(&result["device"]));
    let second = s.model.guid(ids(&result["segments"][1]));
    assert_eq!(s.model.guid(ids(&wire)), original_guid);
    let Some(Element::ElectricalLine(circuit)) = s.model.elements.get(ids(&line)) else { panic!("missing line") };
    assert_eq!(circuit.segments, [original_guid, second]);
    assert_eq!(circuit.devices, [s.model.guid(ids(&detector))]);
    assert_eq!(circuit.isolators, [device]);
    for (segment, endpoint) in [(original_guid, &panel), (second, &detector)] {
        let Some(Element::Wire(run)) = s.model.elements.get(segment) else { panic!("missing cable") };
        assert_eq!(run.connections.len(), 2);
        assert!(run.connections.iter().any(|end| end.device == s.model.guid(ids(endpoint))));
        assert!(run.connections.iter().any(|end| end.device == device));
    }
    assert_eq!(run(&mut s, "fire_line_faults", json!({"id":line}))["faults"].as_array().unwrap().len(), 4);
    let table = run(&mut s, "electrical_line_table", json!({"id":line}));
    assert_eq!(table["device_count"], 2);
    assert_eq!(table["elements"][2], json!([result["device"]]));
    for (name, value) in [("Role", json!("isolator")), ("Order", Value::Null), ("Mark", Value::Null)] {
        let column = table["columns"].as_array().unwrap().iter().position(|v| v == name).unwrap();
        assert_eq!(table["rows"][2][column], value);
    }
    let (loaded, left_out) = Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty(), "{left_out:?}");
    assert_eq!(loaded.elements.get(ids(&line)), s.model.elements.get(ids(&line)));
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before);
    assert_eq!(run(&mut s, "electrical_line_table", json!({"id":line})), table_before);
}

/// The `name` entry of `kind` is in the library, with an id and a GUID.
fn in_library(s: &Session, kind: Kind, name: &str) -> bool {
    s.model.library.entries().any(|(_, e)| e.kind == kind && e.name == name)
}

#[test]
fn a_new_project_has_the_starter_settings() {
    let s = Session::new();
    let m = &s.model;
    assert_eq!(m.check(), Ok(()));
    let v = &m.voltages["230 V"];
    assert_eq!((v.nominal, v.min, v.max), (230.0, 207.0, 253.0));
    let sys = &m.distribution_systems["230/400 V"];
    assert_eq!((sys.phases, sys.wires), (3, 4));
    assert_eq!((sys.l_l.as_deref(), sys.l_n.as_deref(), sys.ground.as_deref()), (Some("400 V"), Some("230 V"), Some("TN-S")));
    for system in ["127/220 V", "220/380 V", "277/480 V"] {
        assert!(m.distribution_systems.contains_key(system), "{system}");
    }
    for wire in ["Cu PVC 70 °C", "Cu XLPE/EPR 90 °C", "Cu THHN 90 °C", "Cu THW 75 °C"] {
        assert!(m.wire_types.contains_key(wire), "{wire}");
        assert!(in_library(&s, Kind::WireType, wire), "{wire} is an entry of the library");
    }
    assert!(m.library_in_step(), "each has an id and a GUID");
    assert!(in_library(&s, Kind::Voltage, "230 V") && in_library(&s, Kind::DistributionSystem, "230/400 V"));
}

#[test]
fn the_starter_data_agrees_with_the_sizing_tables() {
    let s = Session::new();
    let m = &s.model;
    // The voltage systems the circuit sizing knows are voltages the project has.
    for (name, ln, ll) in nbr5410::SYSTEMS {
        let sys = m.distribution_systems.get(&format!("{name} V")).unwrap_or_else(|| panic!("{name}"));
        let volts = |v: &Option<String>| v.as_deref().and_then(|n| m.voltages.get(n)).map(|v| v.nominal);
        assert_eq!((volts(&sys.l_n), volts(&sys.l_l)), (Some(*ln), Some(*ll)), "{name}");
    }
    // The wire types seeded from NBR 5410 hold its tables' reference method B1, two loaded conductors (the fifth column).
    for (wire, table) in [("Cu PVC 70 °C", nbr5410::CU_PVC), ("Cu XLPE/EPR 90 °C", nbr5410::CU_EPR)] {
        let sizes = &m.wire_types[wire].sizes;
        assert_eq!(sizes.len(), table.len(), "{wire}");
        for (size, (section, row)) in sizes.iter().zip(table) {
            assert_eq!((size.section, size.ampacity), (*section, row[4]), "{wire} {section} mm²");
        }
    }
    // The American ones are sized in AWG and kept in mm², their AWG name beside.
    let thhn = m.wire_types["Cu THHN 90 °C"].sizes.iter().find(|x| x.label.as_deref() == Some("12 AWG")).expect("12 AWG");
    assert!((thhn.section - 3.31).abs() < 0.01 && thhn.ampacity == 30.0, "{thhn:?}");
    let thw = m.wire_types["Cu THW 75 °C"].sizes.iter().find(|x| x.label.as_deref() == Some("12 AWG")).expect("12 AWG");
    assert_eq!(thw.ampacity, 25.0);
    assert!(m.wire_types["Cu THW 75 °C"].sizes.iter().any(|x| x.label.as_deref() == Some("250 kcmil")));
}

#[test]
fn a_new_project_starts_in_the_settings_of_its_units() {
    let s = Session::new();
    for units in Units::ALL {
        assert!(s.model.distribution_systems.contains_key(electrical::start_system(units)), "{units:?}");
        assert!(s.model.wire_types.contains_key(electrical::start_wire(units)), "{units:?}");
    }
    assert_eq!(electrical::start_system(Units::Millimetres), "230/400 V");
    assert_eq!(electrical::start_system(Units::FeetInches), "277/480 V");
}

#[test]
fn voltages_are_made_edited_renamed_and_deleted() {
    let mut s = Session::new();
    let r = run(&mut s, "voltage", json!({"name": "690 V", "nominal": 690}));
    assert_eq!(r["name"], json!("690 V"));
    // The band is 10 per cent either way unless it is given.
    let v = &s.model.voltages["690 V"];
    assert_eq!((v.nominal, v.min, v.max), (690.0, 621.0, 759.0));
    assert!(in_library(&s, Kind::Voltage, "690 V"));
    run(&mut s, "voltage", json!({"name": "690 V", "min": 600, "max": 760}));
    let v = &s.model.voltages["690 V"];
    assert_eq!((v.nominal, v.min, v.max), (690.0, 600.0, 760.0), "what is not given stays");
    // Without a name, they are listed.
    assert_eq!(run(&mut s, "voltage", json!({}))["voltages"]["690 V"]["max"], json!(760.0));
    run(&mut s, "voltage", json!({"name": "690 V", "rename": "690 V IT"}));
    assert!(s.model.voltages.contains_key("690 V IT") && !s.model.voltages.contains_key("690 V"));
    assert!(in_library(&s, Kind::Voltage, "690 V IT") && !in_library(&s, Kind::Voltage, "690 V"));
    run(&mut s, "voltage", json!({"name": "690 V IT", "delete": true}));
    assert!(!s.model.voltages.contains_key("690 V IT") && !in_library(&s, Kind::Voltage, "690 V IT"));
    // One undo step each.
    assert_eq!(s.history().iter().filter(|l| **l == "Voltage Definitions").count(), 4);
    run(&mut s, "undo", json!({}));
    assert!(s.model.voltages.contains_key("690 V IT"));
}

#[test]
fn a_voltage_that_is_not_one_is_refused() {
    let mut s = Session::new();
    for (p, says) in [
        (json!({"name": "X"}), "nominal"),
        (json!({"name": "X", "nominal": 0}), "nominal"),
        (json!({"name": "X", "nominal": -230}), "nominal"),
        (json!({"name": "X", "nominal": 1e300}), "nominal"),
        (json!({"name": "X", "nominal": "high"}), "nominal"),
        (json!({"name": "X", "nominal": 230, "min": 250}), "min"),
        (json!({"name": "X", "nominal": 230, "max": 200}), "max"),
        (json!({"name": "X", "nominal": 230, "min": -5}), "min"),
        (json!({"name": "230 V", "nominal": 230, "rename": "400 V"}), "400 V"),
        (json!({"name": "nope", "delete": true}), "nope"),
        (json!({"name": "230 V", "volts": 230}), "volts"),
    ] {
        let e = fails(&mut s, "voltage", p.clone());
        assert!(e.contains(says), "{p}: {e}");
    }
}

#[test]
fn a_distribution_system_names_voltages_the_project_has() {
    let mut s = Session::new();
    let r = run(
        &mut s,
        "distribution_system",
        json!({"name": "Bifásico", "phases": 2, "wires": 3, "l_l": "220 V", "l_n": "127 V", "ground": "TT"}),
    );
    assert_eq!(r["name"], json!("Bifásico"));
    let sys = &s.model.distribution_systems["Bifásico"];
    assert_eq!(
        (sys.phases, sys.wires, sys.l_l.as_deref(), sys.l_n.as_deref(), sys.l_g.as_deref()),
        (2, 3, Some("220 V"), Some("127 V"), None)
    );
    assert!(in_library(&s, Kind::DistributionSystem, "Bifásico"));
    // What is not given stays; "" takes a voltage or the earthing away.
    run(&mut s, "distribution_system", json!({"name": "Bifásico", "wires": 2, "l_n": "", "ground": ""}));
    let sys = &s.model.distribution_systems["Bifásico"];
    assert_eq!((sys.wires, sys.l_l.as_deref(), sys.l_n.as_deref(), sys.ground.as_deref()), (2, Some("220 V"), None, None));
    assert_eq!(run(&mut s, "distribution_system", json!({}))["distribution_systems"]["Bifásico"]["l_l"], json!("220 V"));
    // The voltages a system supplies, which a circuit's has to be one of.
    assert_eq!(s.model.system_voltages("230/400 V"), [230.0, 400.0]);
    assert_eq!(s.model.system_voltages("Bifásico"), [220.0]);
    assert!(s.model.system_voltages("nope").is_empty());
    assert_eq!(s.model.voltage_range("230 V"), Some((207.0, 253.0)));
    assert_eq!(s.model.voltage_range("nope"), None);
    run(&mut s, "distribution_system", json!({"name": "Bifásico", "delete": true}));
    assert!(!s.model.distribution_systems.contains_key("Bifásico") && !in_library(&s, Kind::DistributionSystem, "Bifásico"));
}

#[test]
fn a_distribution_system_with_an_unknown_voltage_is_refused() {
    let mut s = Session::new();
    for key in ["l_l", "l_n", "l_g"] {
        let e = fails(&mut s, "distribution_system", json!({"name": "S", "phases": 3, "wires": 4, "l_l": "400 V", key: "nope"}));
        assert!(e.contains("nope") && e.contains(key) && e.contains("230 V"), "the refusal names the voltage and the ones there are: {e}");
    }
}

#[test]
fn a_distribution_system_that_is_not_one_is_refused() {
    let mut s = Session::new();
    let ok = json!({"name": "S", "phases": 3, "wires": 4, "l_l": "400 V", "l_n": "230 V"});
    let with = |k: &str, v: Value| {
        let mut p = ok.clone();
        p[k] = v;
        p
    };
    for (p, says) in [
        (with("phases", json!(4)), "phases"),
        (with("phases", json!(0)), "phases"),
        (with("phases", json!("three")), "phases"),
        (with("wires", json!(2)), "wires"),
        (with("wires", json!(6)), "wires"),
        (with("wires", json!(1.5)), "wires"),
        (with("ground", json!("PE")), "TN-S"),
        // Three phases at 230 V line to neutral are 400 V apart, not 300.
        (with("l_l", json!("380 V")), "230 V"),
        (with("l_l", json!("")), "l_l"),
        (json!({"name": "S", "phases": 1, "wires": 2}), "voltage"),
        (json!({"name": "S", "phases": 3, "wires": 4}), "voltage"),
        (json!({"name": "S", "phases": 1}), "wires"),
        (json!({"name": "S", "wires": 2}), "phases"),
        (json!({"name": "230/400 V", "rename": "127/220 V"}), "127/220 V"),
        (json!({"name": "nope", "delete": true}), "nope"),
        (with("volts", json!(230)), "volts"),
    ] {
        let e = fails(&mut s, "distribution_system", p.clone());
        assert!(e.contains(says), "{p}: {e}");
    }
}

#[test]
fn a_voltage_a_system_names_stays_and_follows_its_rename() {
    let mut s = Session::new();
    let e = fails(&mut s, "voltage", json!({"name": "230 V", "delete": true}));
    assert!(e.contains("230 V") && e.contains("230/400 V"), "the refusal names the system that holds it: {e}");
    run(&mut s, "voltage", json!({"name": "230 V", "rename": "230 V (EU)"}));
    let sys = &s.model.distribution_systems["230/400 V"];
    assert_eq!((sys.l_n.as_deref(), sys.l_g.as_deref()), (Some("230 V (EU)"), Some("230 V (EU)")));
    assert_eq!(s.model.check(), Ok(()));
}

#[test]
fn a_wire_type_holds_sections_in_square_millimetres_and_their_ampacity() {
    let mut s = Session::new();
    let r = run(
        &mut s,
        "wire_type",
        json!({"name": "Al XLPE 90 °C", "material": "aluminium", "temperature": 90, "insulation": "XLPE", "note": "Method C",
               "sizes": [{"section": 16, "ampacity": 61}, {"section": 25, "ampacity": 79, "label": "AA 25"}, {"section": 35, "ampacity": 96}]}),
    );
    assert_eq!(r["name"], json!("Al XLPE 90 °C"));
    let w = &s.model.wire_types["Al XLPE 90 °C"];
    assert_eq!((w.material.as_str(), w.temperature, w.insulation.as_str(), w.note.as_str()), ("aluminium", 90.0, "XLPE", "Method C"));
    assert_eq!(w.sizes.len(), 3);
    assert_eq!(s.model.wire_ampacity("Al XLPE 90 °C", 25.0), Some(79.0));
    assert_eq!(s.model.wire_ampacity("Al XLPE 90 °C", 50.0), None, "a section the table lacks has none");
    assert_eq!(s.model.wire_ampacity("nope", 25.0), None);
    // The sizes are the table: given again, they replace it; what else is not given stays.
    run(&mut s, "wire_type", json!({"name": "Al XLPE 90 °C", "sizes": [{"section": 50, "ampacity": 115}]}));
    let w = &s.model.wire_types["Al XLPE 90 °C"];
    assert_eq!((w.sizes.len(), w.insulation.as_str()), (1, "XLPE"));
    assert_eq!(run(&mut s, "wire_type", json!({}))["wire_types"]["Al XLPE 90 °C"]["sizes"][0]["section"], json!(50.0));
    run(&mut s, "wire_type", json!({"name": "Al XLPE 90 °C", "rename": "Al 90"}));
    assert!(in_library(&s, Kind::WireType, "Al 90") && !in_library(&s, Kind::WireType, "Al XLPE 90 °C"));
    run(&mut s, "wire_type", json!({"name": "Al 90", "delete": true}));
    assert!(!s.model.wire_types.contains_key("Al 90"));
}

#[test]
fn a_wire_type_that_is_not_one_is_refused() {
    let mut s = Session::new();
    let ok =
        json!({"name": "W", "material": "copper", "temperature": 70, "insulation": "PVC", "sizes": [{"section": 1.5, "ampacity": 17.5}]});
    let with = |k: &str, v: Value| {
        let mut p = ok.clone();
        p[k] = v;
        p
    };
    let sizes = |v: Value| with("sizes", v);
    for (p, says) in [
        (with("material", json!("gold")), "copper"),
        (with("temperature", json!(0)), "temperature"),
        (with("temperature", json!(1e300)), "temperature"),
        (with("insulation", json!("x".repeat(300))), "insulation"),
        (sizes(json!([])), "sizes"),
        (sizes(json!("x")), "sizes"),
        (sizes(json!([{"section": 0, "ampacity": 10}])), "section"),
        (sizes(json!([{"section": -1.5, "ampacity": 10}])), "section"),
        (sizes(json!([{"section": 1.5, "ampacity": 0}])), "ampacity"),
        (sizes(json!([{"section": 1.5, "ampacity": 1e300}])), "ampacity"),
        (sizes(json!([{"section": 1.5}])), "ampacity"),
        (sizes(json!([{"section": 1.5, "ampacity": 10, "gauge": 5}])), "gauge"),
        // The sections go up, and a larger one never carries less.
        (sizes(json!([{"section": 2.5, "ampacity": 24}, {"section": 1.5, "ampacity": 17.5}])), "section"),
        (sizes(json!([{"section": 1.5, "ampacity": 17.5}, {"section": 1.5, "ampacity": 17.5}])), "section"),
        (sizes(json!([{"section": 1.5, "ampacity": 17.5}, {"section": 2.5, "ampacity": 10}])), "ampacity"),
        (json!({"name": "W2", "temperature": 70, "insulation": "PVC", "sizes": [{"section": 1.5, "ampacity": 17.5}]}), "material"),
        (json!({"name": "Cu PVC 70 °C", "rename": "Cu THHN 90 °C"}), "Cu THHN 90 °C"),
        (json!({"name": "nope", "delete": true}), "nope"),
        (with("amps", json!(1)), "amps"),
    ] {
        let e = fails(&mut s, "wire_type", p.clone());
        assert!(e.contains(says), "{p}: {e}");
    }
}

#[test]
fn the_tables_hold_so_many_and_no_more() {
    let mut s = Session::new();
    let rows: Vec<Value> = (1..=electrical::MAX_SIZES + 1).map(|i| json!({"section": i, "ampacity": i})).collect();
    let e = fails(&mut s, "wire_type", json!({"name": "W", "material": "copper", "temperature": 70, "insulation": "PVC", "sizes": rows}));
    assert!(e.contains(&electrical::MAX_SIZES.to_string()), "{e}");
    for i in s.model.voltages.len()..electrical::MAX_VOLTAGES {
        run(&mut s, "voltage", json!({"name": format!("V{i}"), "nominal": 100 + i}));
    }
    let e = fails(&mut s, "voltage", json!({"name": "one too many", "nominal": 50}));
    assert!(e.contains(&electrical::MAX_VOLTAGES.to_string()), "{e}");
}

#[test]
fn the_settings_are_saved_with_the_project_and_come_back() {
    let mut s = Session::new();
    run(&mut s, "voltage", json!({"name": "690 V", "nominal": 690}));
    run(&mut s, "distribution_system", json!({"name": "Mina", "phases": 3, "wires": 3, "l_l": "690 V"}));
    run(
        &mut s,
        "wire_type",
        json!({"name": "W", "material": "copper", "temperature": 70, "insulation": "PVC", "sizes": [{"section": 4, "ampacity": 32}]}),
    );
    let guid = |s: &Session, k: Kind, n: &str| s.model.library.entries().find(|(_, e)| e.kind == k && e.name == n).map(|(_, e)| e.guid);
    let path = std::env::temp_dir().join(format!("bimcraft-electrical-{}.bimcraft", std::process::id()));
    run(&mut s, "save", json!({"path": path.to_str().unwrap()}));
    // A file holds them by GUID, and a system names its voltages by theirs.
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let g = guid(&s, Kind::Voltage, "690 V").unwrap().to_string();
    assert_eq!(file["distribution_systems"]["Mina"]["l_l"], json!(g), "{}", file["distribution_systems"]["Mina"]);
    let mut t = Session::new();
    run(&mut t, "open", json!({"path": path.to_str().unwrap()}));
    assert_eq!(t.model.voltages, s.model.voltages);
    assert_eq!(t.model.distribution_systems, s.model.distribution_systems);
    assert_eq!(t.model.wire_types, s.model.wire_types);
    for (k, n) in [(Kind::Voltage, "690 V"), (Kind::DistributionSystem, "Mina"), (Kind::WireType, "W")] {
        assert_eq!(guid(&t, k.clone(), n), guid(&s, k, n), "{n} keeps its GUID");
    }
    // A file from before the tables opens with the starter set, and nothing left out.
    let mut old = file;
    for k in ["voltages", "distribution_systems", "wire_types"] {
        old.as_object_mut().unwrap().remove(k);
    }
    std::fs::write(&path, old.to_string()).unwrap();
    let mut u = Session::new();
    let r = run(&mut u, "open", json!({"path": path.to_str().unwrap()}));
    assert!(r["left_out"].as_array().is_none_or(Vec::is_empty), "nothing is left out: {r}");
    assert!(u.model.voltages.contains_key("230 V") && u.model.distribution_systems.contains_key("230/400 V"));
    assert_eq!(u.model.check(), Ok(()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn inspect_lists_the_settings() {
    let mut s = Session::new();
    let r = run(&mut s, "inspect", json!({}));
    assert_eq!(r["voltages"]["230 V"]["nominal"], json!(230.0));
    assert_eq!(r["distribution_systems"]["230/400 V"]["l_l"], json!("400 V"));
    assert_eq!(r["wire_types"]["Cu PVC 70 °C"]["sizes"][0]["section"], json!(0.5));
}

#[test]
fn the_settings_come_in_with_insert_from_file() {
    let mut from = Session::new();
    run(&mut from, "voltage", json!({"name": "690 V", "nominal": 690}));
    run(&mut from, "distribution_system", json!({"name": "Mina", "phases": 3, "wires": 3, "l_l": "690 V"}));
    run(
        &mut from,
        "wire_type",
        json!({"name": "W", "material": "copper", "temperature": 70, "insulation": "PVC", "sizes": [{"section": 4, "ampacity": 32}]}),
    );
    run(&mut from, "wall", json!({"points": [[0, 0], [3000, 0]]}));
    let path = std::env::temp_dir().join(format!("bimcraft-electrical-insert-{}.bimcraft", std::process::id()));
    run(&mut from, "save", json!({"path": path.to_str().unwrap()}));
    let mut to = Session::new();
    let r = run(&mut to, "insert", json!({"path": path.to_str().unwrap()}));
    let came: Vec<&str> = r["library"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for what in ["voltage: 690 V", "distribution system: Mina", "wire type: W"] {
        assert!(came.contains(&what), "{what} in {came:?}");
    }
    assert_eq!(to.model.distribution_systems["Mina"].l_l.as_deref(), Some("690 V"));
    assert_eq!(to.model.check(), Ok(()));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_starter_set_is_data_that_passes_its_own_checks() {
    let (v, d, w) = (electrical::voltages(), electrical::distribution_systems(), electrical::wire_types());
    assert!(!v.is_empty() && !d.is_empty() && !w.is_empty());
    for (n, x) in &v {
        assert_eq!(x.check(n), Ok(()));
    }
    for (n, x) in &d {
        assert_eq!(x.check(n, &v), Ok(()));
    }
    for (n, x) in &w {
        assert_eq!(x.check(n), Ok(()));
    }
}

#[test]
fn typed_low_voltage_release_keeps_system_sections_and_rolls_back_bad_output() {
    use std::io::Read;
    let mut s = Session::new();
    let panel = run(&mut s, "component", json!({"family":"Fire Alarm Panel","at":[0,0]}))["id"].clone();
    let smoke = run(&mut s, "component", json!({"family":"Smoke Detector","at":[1000,0]}))["id"].clone();
    let other = run(&mut s, "component", json!({"family":"Smoke Detector","at":[2000,0]}))["id"].clone();
    let first = run(&mut s, "electrical_line", json!({"source":panel,"devices":[smoke],"profile":"sps"}))["id"].clone();
    let second = run(&mut s, "electrical_line", json!({"source":panel,"devices":[other],"profile":"sot"}))["id"].clone();
    let a = run(&mut s, "electrical_line_inspect", json!({"id":first}));
    let b = run(&mut s, "electrical_line_inspect", json!({"id":second}));
    assert_eq!(a["marks"], json!(["SPS-1", "SPS-2"]));
    assert_eq!(b["marks"], json!(["SOT-1", "SOT-2"]));
    assert_eq!(a["requires_okl"], true);
    assert_eq!(run(&mut s, "fire_okl_check", json!({"profile":"sot"}))["status"], "optional");
    fails(&mut s, "electrical_line_edit", json!({"id":first,"profile":"unknown"}));
    let dir = tempfile::tempdir().unwrap();
    let ods = dir.path().join("equipment.ods");
    let pdf = dir.path().join("design.pdf");
    let before = s.model.clone();
    let spec = run(&mut s, "electrical_specification", json!({"path":ods}));
    for name in ["sps", "sot", "shared"] {
        assert_eq!(spec["sections"][name].as_array().unwrap().len(), 1);
    }
    let mut workbook = zip::ZipArchive::new(std::fs::File::open(ods).unwrap()).unwrap();
    let mut content = String::new();
    workbook.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
    assert!(
        content.contains("table:name=\"1 shared\"") && content.contains("table:name=\"2 sot\"") && content.contains("table:name=\"3 sps\"")
    );
    let report = run(&mut s, "electrical_design_pdf", json!({"path":pdf,"code":"IOS5","title":"Fire and security systems"}));
    assert_eq!(report["pages"], 5);
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(String::from_utf8_lossy(&bytes).contains("/Count 5"));
    fails(&mut s, "electrical_design_pdf", json!({"path":pdf,"code":"IOS5","title":"Fire","sheets":[first]}));
    assert_eq!(std::fs::read(pdf).unwrap(), bytes);
    assert_eq!(s.model, before);
}
