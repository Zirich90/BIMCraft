//! Connectors with a place: where a pipe, a data cable or an earthing conductor meets a family, which way it faces and how
//! big it is, worked out for a component where it stands. What a network makes of them is a later piece; these are the
//! connectors themselves, through the commands every front end drives.

use serde_json::{Value, json};

use crate::element_json;
use crate::engine::Session;
use crate::language::Language;
use crate::model::Model;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.model.clone();
    let e = s.execute(id, &p).expect_err(&format!("{id} {p} should fail"));
    assert_eq!(*s.model, *before, "a refused {id} changes nothing");
    e
}

fn made(s: &mut Session, p: Value) -> u64 {
    run(s, "component", p)["id"].as_u64().unwrap()
}

fn connectors(s: &Session, id: u64) -> Vec<Value> {
    element_json(&s.model, id)["connectors"].as_array().cloned().unwrap_or_default()
}

fn near(a: &Value, b: [f64; 3]) -> bool {
    a.as_array().is_some_and(|v| v.len() == 3 && v.iter().zip(b).all(|(x, y)| x.as_f64().is_some_and(|x| (x - y).abs() < 1e-6)))
}

#[test]
fn cable_ends_reference_the_instance_and_its_data_port() {
    let mut s = Session::new();
    run(
        &mut s,
        "family",
        json!({"name":"Detector","types":{"A":{}},
        "connectors":[{"name":"Loop","kind":"data","at":[100,20,50]},
            {"name":"Supply","kind":"power","system":"power-balanced","voltage":127,"load_va":12}]}),
    );
    let left = made(&mut s, json!({"family":"Detector","at":[1000,1000]}));
    let right = made(&mut s, json!({"family":"Detector","at":[4000,1000]}));
    let board = crate::circuit_tests::board(&mut s);
    let circuit = crate::circuit_tests::make(&mut s, json!({"devices":[left,right],"panel":board}));
    let power = element_json(&s.model, circuit);
    assert_eq!(power["installed_va"], json!(24.0));
    let port = connectors(&s, left)[0]["guid"].clone();
    assert_eq!(connectors(&s, right)[0]["guid"], port);
    let cable = run(&mut s, "cabling", json!({"points":[[1100,1020],[4100,1020]],"offset":50}))["ids"][0].as_u64().unwrap();
    let mut legacy: Value = serde_json::from_str(&s.model.to_json().unwrap()).unwrap();
    legacy["format"] = json!(20);
    let (legacy, left_out) = Model::read(&legacy.to_string()).unwrap();
    assert!(left_out.is_empty());
    assert_eq!(legacy.elements.get(cable), s.model.elements.get(cable));
    let endpoints = json!([{"component":left,"port":port},{"component":right,"port":port}]);
    run(&mut s, "cable_ports", json!({"id":cable,"ends":endpoints}));
    let mut bounds = crate::bounds::BoundsCache::default();
    let before = bounds.get(&s.model, cable);
    fails(&mut s, "move", json!({"id":right,"by":[-3000,0]}));
    let points = run(&mut s, "cable_ports", json!({"id":cable}));
    assert!(near(&points["ends"][0]["at"], [1100.0, 1020.0, 50.0]));
    assert!(near(&points["ends"][1]["at"], [4100.0, 1020.0, 50.0]));
    run(&mut s, "rotate", json!({"ids":[right],"about":[4000,1000],"angle":90}));
    assert_ne!(bounds.get(&s.model, cable), before);
    let points = run(&mut s, "cable_ports", json!({"id":cable}));
    assert!(near(&points["ends"][0]["at"], [1100.0, 1020.0, 50.0]));
    assert!(near(&points["ends"][1]["at"], [3980.0, 1100.0, 50.0]));
    let g = s.model.run_geom(s.model.elements.get(cable).unwrap()).unwrap();
    assert_eq!(g.a, [1100.0, 1020.0, 50.0]);
    assert!(near(&json!(g.b), [3980.0, 1100.0, 50.0]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("detectors.bimc");
    run(&mut s, "save", json!({"path":path}));
    run(&mut s, "open", json!({"path":path}));
    assert_eq!(run(&mut s, "cable_ports", json!({"id":cable})), points);
    assert_eq!(element_json(&s.model, circuit), power);
    let mut other = Session::new();
    let brought = run(&mut other, "insert", json!({"path":path,"ids":[cable]}));
    let brought_cable = brought["ids"][cable.to_string()].as_u64().unwrap();
    let ends = run(&mut other, "cable_ports", json!({"id":brought_cable}));
    assert_eq!(ends["ends"], points["ends"]);
    let wrong = s.model.guid(right);
    fails(&mut s, "cable_ports", json!({"id":cable,"ends":[{"component":left,"port":wrong},null]}));
    let copied = run(&mut s, "copy", json!({"ids":[cable,left],"by":[0,2000]}))["ids"][0].as_u64().unwrap();
    let copy = run(&mut s, "cable_ports", json!({"id":copied}));
    assert_ne!(copy["ends"][0]["component"], points["ends"][0]["component"]);
    assert_eq!(copy["ends"][0]["port"], port);
    assert!(near(&copy["ends"][0]["at"], [1100.0, 3020.0, 50.0]));
    assert!(copy["ends"][1].is_null());
    run(&mut s, "delete", json!({"id":right}));
    assert!(run(&mut s, "cable_ports", json!({"id":cable}))["ends"][1].is_null());
    let detached = s.model.run_geom(s.model.elements.get(cable).unwrap()).unwrap();
    assert_eq!(detached, g);
}

/// A trap with its waste out through the floor and an inlet from the wall, sized by its type.
fn trap(s: &mut Session) {
    run(
        s,
        "family",
        json!({
            "name": "Trap", "category": "plumbing", "kind": "sink",
            "parameters": [{"name": "DN", "kind": "length", "value": 50}],
            "types": {"50": {}, "75": {"DN": 75}},
            "connectors": [
                {"name": "Waste", "kind": "pipe", "domain": "waste", "flow": "out", "at": [100, -200, 0], "direction": [0, 0, -3], "size": "DN"},
                {"kind": "pipe", "domain": "cold", "flow": "in", "at": [0, 0, "DN * 10"], "direction": [0, 1, 0], "size": 20}
            ]
        }),
    );
}

#[test]
fn a_pipe_connector_stands_where_its_component_does_and_faces_with_it() {
    let mut s = Session::new();
    trap(&mut s);
    let plain = made(&mut s, json!({"family": "Trap", "at": [1000, 2000]}));
    let c = connectors(&s, plain);
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!((c[0]["name"].as_str(), c[0]["kind"].as_str(), c[0]["domain"].as_str()), (Some("Waste"), Some("pipe"), Some("waste")));
    assert_eq!((c[0]["flow"].as_str(), c[0]["size"].as_f64()), (Some("out"), Some(50.0)));
    assert!(near(&c[0]["at"], [1100.0, 1800.0, 0.0]), "{}", c[0]);
    assert!(near(&c[0]["direction"], [0.0, 0.0, -1.0]), "a direction is a unit vector: {}", c[0]);
    assert!(c[0].get("load_va").is_none() && c[0].get("system").is_none(), "a pipe connector has no load: {}", c[0]);
    assert!(c[0]["guid"].is_string(), "a connector has its GUID: {}", c[0]);
    // The type sets what its formulas read.
    let big = made(&mut s, json!({"family": "Trap", "type": "75", "at": [0, 0]}));
    let c = connectors(&s, big);
    assert_eq!(c[0]["size"].as_f64(), Some(75.0));
    assert!(near(&c[1]["at"], [0.0, 0.0, 750.0]), "{}", c[1]);
    // Turned a quarter, raised and mirrored, its connectors turn, rise and mirror with it.
    let turned = made(&mut s, json!({"family": "Trap", "at": [1000, 2000], "angle": 90, "offset": 50}));
    let c = connectors(&s, turned);
    assert!(near(&c[0]["at"], [1200.0, 2100.0, 50.0]), "{}", c[0]);
    assert!(near(&c[1]["direction"], [-1.0, 0.0, 0.0]), "{}", c[1]);
    let mirrored = made(&mut s, json!({"family": "Trap", "at": [1000, 2000], "mirrored": true}));
    let c = connectors(&s, mirrored);
    assert!(near(&c[0]["at"], [1100.0, 2200.0, 0.0]), "{}", c[0]);
    assert!(near(&c[1]["direction"], [0.0, -1.0, 0.0]), "{}", c[1]);
}

#[test]
fn an_unsound_connector_is_refused_and_says_what_to_fix() {
    let mut s = Session::new();
    let pipe = json!({"kind": "pipe", "domain": "waste", "at": [0, 0, 0], "direction": [0, 0, -1], "size": 40});
    let with = |k: &str, v: Value| {
        let mut c = pipe.clone();
        if v.is_null() {
            c.as_object_mut().unwrap().remove(k);
        } else {
            c[k] = v;
        }
        json!({"name": "Bad", "category": "plumbing", "types": {"A": {}}, "connectors": [c]})
    };
    for (k, v, says) in [
        ("domain", Value::Null, "domain"),
        ("at", Value::Null, "`at`"),
        ("direction", Value::Null, "direction"),
        ("size", Value::Null, "size"),
        ("size", json!(0), "size"),
        ("size", json!(5000), "size"),
        ("size", json!("Nothing"), "size"),
        ("direction", json!([0, 0, 0]), "no length"),
        ("at", json!([0, 0, 1e300]), "place"),
        ("system", json!("power-balanced"), "system"),
        ("load_va", json!(100), "load"),
        ("domain", json!("steam"), "`cold`"),
        ("flow", json!("sideways"), "`both`"),
    ] {
        let e = fails(&mut s, "family", with(k, v.clone()));
        assert!(e.contains(says), "{k} = {v}: {e}");
    }
    // An electrical device's connector takes no domain and no size; it may have a place.
    let power = json!({"kind": "power", "system": "power-balanced", "voltage": 127, "load_va": 10});
    for (k, v) in [("domain", json!("cold")), ("size", json!(20)), ("flow", json!("in"))] {
        let mut c = power.clone();
        c[k] = v;
        let e = fails(&mut s, "family", json!({"name": "Bad", "category": "outlet", "types": {"A": {}}, "connectors": [c]}));
        assert!(e.contains(k), "{e}");
    }
    let mut placed = power.clone();
    placed["at"] = json!([0, -20, 300]);
    run(&mut s, "family", json!({"name": "Box", "category": "outlet", "types": {"A": {}}, "connectors": [placed]}));
    let id = made(&mut s, json!({"family": "Box", "at": [500, 0]}));
    let c = &connectors(&s, id)[0];
    assert!(near(&c["at"], [500.0, -20.0, 300.0]) && c["load_va"].as_f64() == Some(10.0), "{c}");
    // A power or a switch connector still needs its system.
    let e = fails(&mut s, "family", json!({"name": "Bad", "types": {"A": {}}, "connectors": [{"kind": "power", "voltage": 127}]}));
    assert!(e.contains("system"), "{e}");
    // A data or an earthing connector needs its place, and nothing more.
    let e = fails(&mut s, "family", json!({"name": "Bad", "types": {"A": {}}, "connectors": [{"kind": "data"}]}));
    assert!(e.contains("`at`"), "{e}");
    let e = fails(
        &mut s,
        "family",
        json!({"name": "Bad", "types": {"A": {}}, "connectors": [{"kind": "data", "at": [0, 0, 0], "domain": "cold"}]}),
    );
    assert!(e.contains("domain"), "{e}");
    run(&mut s, "family", json!({"name": "Jack", "types": {"A": {}}, "connectors": [{"kind": "data", "at": [0, 0, 300]}]}));
}

/// A family's connectors, as (domain, flow, DN).
type Want = &'static [(&'static str, &'static str, f64)];

#[test]
fn the_starter_fixtures_carry_their_waste_and_water_in_every_language() {
    // (family, its connectors as (domain, flow, DN))
    let cases: [(&str, Want); 6] = [
        ("WC", &[("waste", "out", 100.0), ("cold", "in", 20.0)]),
        ("Washbasin", &[("waste", "out", 40.0), ("cold", "in", 20.0)]),
        ("Bathtub", &[("waste", "out", 40.0), ("cold", "in", 20.0), ("hot", "in", 20.0)]),
        ("Kitchen Sink", &[("waste", "out", 50.0), ("cold", "in", 20.0)]),
        ("Shower", &[("waste", "out", 40.0), ("cold", "in", 20.0), ("hot", "in", 20.0)]),
        ("Floor Drain", &[("waste", "out", 40.0)]),
    ];
    for l in Language::ALL {
        let m = Model::new_in("x", l);
        assert_eq!(m.check(), Ok(()), "{l:?}");
        for (family, want) in cases {
            let f = m.families.get(l.word(family)).unwrap_or_else(|| panic!("{l:?}: no {family}"));
            for t in f.types.keys() {
                let got: Vec<(String, String, f64)> = f
                    .connector_values(t, &Default::default())
                    .unwrap()
                    .iter()
                    .map(|c| {
                        let domain = serde_json::to_value(c.domain).unwrap().as_str().unwrap_or_default().to_string();
                        let flow = serde_json::to_value(c.flow).unwrap().as_str().unwrap_or_default().to_string();
                        assert!(c.at.is_some() && c.direction.is_some(), "{l:?} {family} {t}");
                        (domain, flow, c.size.unwrap_or_default())
                    })
                    .collect();
                let want: Vec<(String, String, f64)> = want.iter().map(|(d, f, n)| (d.to_string(), f.to_string(), *n)).collect();
                assert_eq!(got, want, "{l:?} {family} {t}");
            }
        }
    }
}

#[test]
fn a_waste_outlet_through_the_floor_is_under_the_fixture_and_faces_down() {
    let mut s = Session::new();
    let wc = made(&mut s, json!({"family": "WC", "at": [2000, 1000], "angle": 180}));
    let c = connectors(&s, wc);
    let waste = c.iter().find(|c| c["domain"] == "waste").unwrap();
    // 300 mm out from the wall it stands against, which a turn of 180° puts to its north.
    assert!(near(&waste["at"], [2000.0, 1300.0, 0.0]), "{waste}");
    assert!(near(&waste["direction"], [0.0, 0.0, -1.0]), "{waste}");
}

#[test]
fn a_plumbing_fixture_with_pipe_connectors_is_no_electrical_device_in_ifc() {
    let mut s = Session::new();
    made(&mut s, json!({"family": "WC", "at": [0, 0]}));
    made(&mut s, json!({"family": "Outlet", "at": [1000, 0]}));
    let f = crate::ifc::export(&s.model);
    assert_eq!(f.matches("'Pset_ElectricalDeviceCommon'").count(), 1, "the outlet's only");
}

#[test]
fn a_family_with_pipe_connectors_saves_and_opens_as_it_was() {
    let mut s = Session::new();
    trap(&mut s);
    let before = s.model.families["Trap"].clone();
    let back = Model::from_json(&s.model.to_json().unwrap()).unwrap();
    assert_eq!(back.families["Trap"], before);
    // A file's connectors of before keep their system, and read as they did.
    let outlet = &back.families[Language::English.word("Outlet")].connectors[0];
    assert_eq!(serde_json::to_value(outlet.system).unwrap(), json!("power-balanced"));
}

#[test]
fn a_fitting_has_a_connector_at_each_end_of_its_diameter_and_takes_its_pipes_domain() {
    // (family, its ends' directions)
    let cases: [(&str, &[[f64; 3]]); 3] = [
        ("Elbow 90", &[[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        ("Tee", &[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        ("Gate Valve", &[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]),
    ];
    for l in Language::ALL {
        let m = Model::new_in("x", l);
        for (family, ends) in cases {
            let f = &m.families[l.word(family)];
            for t in f.types.keys() {
                let dn: f64 = t.parse().unwrap();
                let c = f.connector_values(t, &Default::default()).unwrap();
                assert_eq!(c.iter().map(|c| c.direction.unwrap()).collect::<Vec<_>>(), ends.to_vec(), "{l:?} {family} {t}");
                for c in &c {
                    assert_eq!((c.size, c.domain), (Some(dn), None), "{l:?} {family} {t}");
                    // Each end is where its pipe goes in: a diameter and a half and 30 mm out from the middle.
                    let at = c.at.unwrap();
                    let out = at[0].abs().max(at[1].abs());
                    assert!((out - (dn * 1.5 + 30.0)).abs() < 1e-9 && at[2] == 0.0, "{l:?} {family} {t}: {at:?}");
                }
            }
        }
    }
    // Only a fitting's or an accessory's pipe connector may leave its domain to the pipe.
    let mut s = Session::new();
    let open = json!({"kind": "pipe", "at": [0, 0, 0], "direction": [1, 0, 0], "size": 50});
    let e = fails(&mut s, "family", json!({"name": "Bad", "category": "plumbing", "types": {"A": {}}, "connectors": [open]}));
    assert!(e.contains("domain") && e.contains("fitting"), "{e}");
    run(&mut s, "family", json!({"name": "Union", "category": "pipefitting", "types": {"A": {}}, "connectors": [open]}));
}

#[test]
fn cable_span_follows_its_carrier_and_survives_detachment() {
    let mut s = Session::new();
    let carrier =
        run(&mut s, "conduit", json!({"points":[[1000,1000],[3000,1000]],"offset":100,"end_offset":300}))["ids"][0].as_u64().unwrap();
    let cable = run(&mut s, "cabling", json!({"points":[[1500,1600],[2500,1600]]}))["ids"][0].as_u64().unwrap();
    let original = s.model.guid(carrier);
    fails(&mut s, "cable_carrier", json!({"id":cable,"span":{"carrier":cable,"start":0,"end":1}}));
    fails(&mut s, "cable_carrier", json!({"id":cable,"span":{"carrier":carrier,"start":0.5,"end":0.5}}));
    run(&mut s, "cable_carrier", json!({"id":cable,"span":{"carrier":carrier,"start":0.25,"end":0.75}}));
    let span = run(&mut s, "cable_carrier", json!({"id":cable}));
    assert_eq!(span["span"]["carrier"], json!(original));
    assert!(near(&span["axis"][0], [1500.0, 1000.0, 150.0]));
    assert!(near(&span["axis"][1], [2500.0, 1000.0, 250.0]));
    let mut bounds = crate::bounds::BoundsCache::default();
    let before = bounds.get(&s.model, cable);
    run(&mut s, "move", json!({"ids":[carrier],"by":[0,500]}));
    assert_ne!(bounds.get(&s.model, cable), before);
    let moved = run(&mut s, "cable_carrier", json!({"id":cable}));
    assert!(near(&moved["axis"][0], [1500.0, 1500.0, 150.0]));
    assert!(near(&moved["axis"][1], [2500.0, 1500.0, 250.0]));
    fails(&mut s, "move", json!({"ids":[cable],"by":[0,200]}));
    let independent = run(&mut s, "copy", json!({"ids":[cable],"by":[0,2000]}))["ids"][0].as_u64().unwrap();
    let copied = run(&mut s, "cable_carrier", json!({"id":independent}));
    assert!(copied["span"].is_null());
    assert!(near(&copied["axis"][0], [1500.0, 3500.0, 150.0]));
    let pair = run(&mut s, "copy", json!({"ids":[carrier,cable],"by":[0,3000]}));
    let carrier_copy = pair["ids"][0].as_u64().unwrap();
    let cable_copy = pair["ids"][1].as_u64().unwrap();
    let copied = run(&mut s, "cable_carrier", json!({"id":cable_copy}));
    assert_eq!(copied["span"]["carrier"], json!(s.model.guid(carrier_copy)));
    assert!(near(&copied["axis"][0], [1500.0, 4500.0, 150.0]));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("route.bimc");
    run(&mut s, "save", json!({"path":path}));
    let mut reopened = Session::new();
    run(&mut reopened, "open", json!({"path":path}));
    assert_eq!(run(&mut reopened, "cable_carrier", json!({"id":cable})), moved);
    let mut other = Session::new();
    let brought = run(&mut other, "insert", json!({"path":path,"ids":[cable]}));
    let brought_cable = brought["ids"][cable.to_string()].as_u64().unwrap();
    let brought = run(&mut other, "cable_carrier", json!({"id":brought_cable}));
    assert_eq!(brought["span"], moved["span"]);
    assert_eq!(brought["axis"], moved["axis"]);
    run(&mut reopened, "delete", json!({"ids":[carrier]}));
    let detached = run(&mut reopened, "cable_carrier", json!({"id":cable}));
    assert!(detached["span"].is_null());
    assert_eq!(detached["axis"], moved["axis"]);
}

#[test]
fn cable_route_places_ordered_spans_atomically() {
    let mut s = Session::new();
    let first =
        run(&mut s, "conduit", json!({"points":[[1000,1000],[3000,1000]],"offset":100,"end_offset":200}))["ids"][0].as_u64().unwrap();
    let next = run(&mut s, "cable_tray", json!({"points":[[3000,3000],[3000,1000]],"offset":200}))["ids"][0].as_u64().unwrap();
    let spans = json!([{"carrier":first,"start":0.5,"end":1},{"carrier":next,"start":1,"end":0.5}]);
    let mut gap = spans.clone();
    gap[1]["start"] = json!(0.9);
    assert!(fails(&mut s, "cable_route", json!({"spans":gap})).contains("meet"));

    let route = run(&mut s, "cable_route", json!({"spans":spans,"system":"Data"}));
    let ids = route["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    let a = run(&mut s, "cable_carrier", json!({"id":ids[0]}));
    let b = run(&mut s, "cable_carrier", json!({"id":ids[1]}));
    assert!(near(&a["axis"][0], [2000.0, 1000.0, 150.0]));
    assert_eq!(a["axis"][1], b["axis"][0]);
    assert!(near(&b["axis"][1], [3000.0, 2000.0, 200.0]));
    assert!((route["length"].as_f64().unwrap() - (1000.0_f64.hypot(50.0) + 1000.0)).abs() < 1e-6);
    let check = json!({"ids":ids});
    assert_eq!(run(&mut s, "cable_route_check", check.clone())["connected"], true);
    run(&mut s, "modify", json!({"id":next,"offset":300}));
    let broken = run(&mut s, "cable_route_check", check);
    assert_eq!(broken["connected"], false);
    assert_eq!(broken["gaps"], json!([{"from":ids[0],"to":ids[1],"distance":100.0}]));
}

#[test]
fn cable_route_adds_leads_to_placed_data_ports() {
    let mut s = Session::new();
    run(&mut s, "family", json!({"name":"Detector","types":{"A":{}},"connectors":[{"name":"Data","kind":"data","at":[0,0,50]}]}));
    let left = made(&mut s, json!({"family":"Detector","at":[500,1000]}));
    let right = made(&mut s, json!({"family":"Detector","at":[3500,1000]}));
    let port = connectors(&s, left)[0]["guid"].clone();
    let carrier = run(&mut s, "conduit", json!({"points":[[1000,1000],[3000,1000]],"offset":50}))["ids"][0].as_u64().unwrap();
    let route = run(
        &mut s,
        "cable_route",
        json!({
            "spans":[{"carrier":carrier,"start":0,"end":1}],
            "ends":[{"component":left,"port":port},{"component":right,"port":port}]
        }),
    );
    let ids = route["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(route["length"], 3000.0);
    assert_eq!(run(&mut s, "cable_route_check", json!({"ids":ids}))["connected"], true);
    assert!(near(&run(&mut s, "cable_ports", json!({"id":ids[0]}))["ends"][0]["at"], [500.0, 1000.0, 50.0]));
    assert!(near(&run(&mut s, "cable_ports", json!({"id":ids[2]}))["ends"][1]["at"], [3500.0, 1000.0, 50.0]));
}

#[test]
fn shrinking_a_carrier_revalidates_its_cable_span() {
    let mut s = Session::new();
    let carrier = run(&mut s, "conduit", json!({"points":[[1000,1000],[3000,1000]]}))["ids"][0].as_u64().unwrap();
    let cable = run(&mut s, "cabling", json!({"points":[[1500,1600],[2500,1600]]}))["ids"][0].as_u64().unwrap();
    run(&mut s, "cable_carrier", json!({"id":cable,"span":{"carrier":carrier,"start":0.0,"end":0.01}}));

    run(&mut s, "modify", json!({"id":carrier,"end":[2000,1000]}));
    let before = run(&mut s, "cable_carrier", json!({"id":cable}));
    assert!((before["length"].as_f64().unwrap() - 10.0).abs() < 1e-6);
    let error = fails(&mut s, "modify", json!({"id":carrier,"end":[1050,1000]}));
    assert!(error.contains("too short"), "{error}");
    assert_eq!(run(&mut s, "cable_carrier", json!({"id":cable})), before);
}

#[test]
fn carrier_fill_uses_the_cables_present_at_each_section() {
    let mut s = Session::new();
    run(&mut s, "type", json!({"category":"conduit","name":"Measured","width":32,"round":true,"clear_section":{"diameter":20}}));
    run(&mut s, "type", json!({"category":"cable","name":"Data","width":8,"round":true}));
    let carrier = run(&mut s, "conduit", json!({"points":[[1000,1000],[3000,1000]],"type":"Measured"}))["ids"][0].as_u64().unwrap();
    let add_cable = |s: &mut Session, start, end| {
        let cable = run(s, "cabling", json!({"points":[[1500,1600],[2500,1600]],"type":"Data"}))["ids"][0].as_u64().unwrap();
        run(s, "cable_carrier", json!({"id":cable,"span":{"carrier":carrier,"start":start,"end":end}}));
        cable
    };
    let params = json!({"id":carrier,"profile":"nbr5410_2004","routed":true});
    assert!(run(&mut s, "carrier_fill", params.clone())["ok"].as_bool().unwrap());
    add_cable(&mut s, 0.0, 0.5);
    add_cable(&mut s, 1.0, 0.5);
    let separate = run(&mut s, "carrier_fill", params.clone());
    assert_eq!(separate["count"], 1);
    assert_eq!(separate["ok"], true);
    let overlap = add_cable(&mut s, 0.25, 0.75);
    let filled = run(&mut s, "carrier_fill", params.clone());
    assert_eq!(filled["count"], 2);
    assert_eq!(filled["limit_percent"], 31.0);
    assert_eq!(filled["ok"], false);
    run(&mut s, "cable_carrier", json!({"id":overlap,"span":null}));
    assert_eq!(run(&mut s, "carrier_fill", params)["ok"], true);
    fails(&mut s, "carrier_fill", json!({"id":carrier,"profile":"nbr5410_2004","routed":true,"areas":[25]}));
}

#[test]
fn a_format_21_project_keeps_its_conduit_tray_and_unattached_cable() {
    use crate::model::Model;

    let mut s = Session::new();
    let ids = ["conduit", "cable_tray", "cabling"]
        .map(|command| run(&mut s, command, json!({"points":[[1000,1000],[3000,1000]]}))["ids"][0].as_u64().unwrap());
    let mut file: Value = serde_json::from_str(&s.model.to_json().unwrap()).unwrap();
    file["format"] = json!(21);
    let (opened, left_out) = Model::read(&file.to_string()).unwrap();
    assert!(left_out.is_empty(), "{left_out:?}");
    for id in ids {
        let before = s.model.elements.get(id).unwrap();
        let after = opened.elements.get(id).unwrap();
        assert_eq!(opened.guid(id), s.model.guid(id));
        assert_eq!(before.category(), after.category());
        assert_eq!(opened.run_geom(after), s.model.run_geom(before));
    }
    opened.check().unwrap();
}
