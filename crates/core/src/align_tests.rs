use crate::engine::Session;
use crate::geom::{self, P2};
use serde_json::json;

#[test]
fn align_rotates_and_translates_a_wall_with_its_opening_in_one_undo_step() {
    let mut s = Session::new();
    s.script("wall {\"points\": [[1000, 2000], [4000, 4000]]}\ndoor {\"wall\": 100002, \"offset\": 1500}").unwrap();
    let before = s.model.clone();
    s.execute("align", &json!({"id": 100002, "reference": [[0,0],[6000,0]], "line": [[1000,2000],[4000,4000]]})).unwrap();
    let w = s.model.wall(100002).unwrap();
    assert!(w.start.1.abs() < 0.002 && w.end.1.abs() < 0.002);
    assert!((w.start.dist(w.end) - P2(1000.0, 2000.0).dist(P2(4000.0, 4000.0))).abs() < 0.002);
    assert!(s.model.check().is_ok());
    s.execute("undo", &json!({})).unwrap();
    assert_eq!(s.model.elements, before.elements);
    s.execute("redo", &json!({})).unwrap();
    assert!(geom::parallel(P2(0.0, 0.0), P2(6000.0, 0.0), s.model.wall(100002).unwrap().start, s.model.wall(100002).unwrap().end));
}

#[test]
fn align_refuses_degenerate_lines_and_rolls_back_a_hosted_opening_that_cannot_turn() {
    let mut s = Session::new();
    s.script("wall {\"points\": [[0,0],[6000,0]]}\ndoor {\"wall\": 100002, \"offset\": 2000}").unwrap();
    let before = s.model.clone();
    for p in [
        json!({"id":100002,"reference":[[0,0],[0,0]],"line":[[0,0],[6000,0]]}),
        json!({"id":100002,"reference":[[0,0],[6000,0]],"line":[[1,1],[1,1]]}),
        json!({"id":100003,"reference":[[0,0],[6000,0]],"line":[[2000,-100],[2000,100]]}),
    ] {
        assert!(s.execute("align", &p).is_err());
        assert_eq!(s.model.elements, before.elements);
    }
}

#[test]
fn align_handles_plan_edges_of_structural_mep_and_architectural_elements() {
    let ring = "[[1000,1000],[4000,1000],[4000,4000],[1000,4000]]";
    let mut scripts = vec![
        "column {\"at\": [3000,2000], \"angle\": 30}".to_string(),
        "structural_column {\"at\": [3000,2000]}".to_string(),
        "footing {\"at\": [3000,2000]}".to_string(),
        format!("floor {{\"boundary\": {ring}}}"),
        format!("ceiling {{\"boundary\": {ring}}}"),
        format!("roof {{\"boundary\": {ring}, \"slope\": 0}}"),
        format!("shaft {{\"boundary\": {ring}}}"),
        format!("mass {{\"boundary\": {ring}, \"height\": 3000}}"),
        "stair {\"start\": [1000,2000], \"angle\": 30, \"railings\": true}".to_string(),
        "ramp {\"start\": [1000,2000], \"angle\": 30, \"height\": 300, \"railings\": true}".to_string(),
        "railing {\"path\": [[1000,2000],[4000,3000]]}".to_string(),
        "room_separator {\"points\": [[1000,2000],[4000,3000]]}".to_string(),
        "detail_line {\"points\": [[1000,2000],[4000,3000],[5000,2000]]}".to_string(),
        "grid {\"start\": [1000,2000], \"end\": [4000,3000]}".to_string(),
        "family {\"name\": \"Box\", \"types\": {\"A\": {}}, \"forms\": [{\"profile\": [[0,0],[1000,0],[1000,500],[0,500]], \"to\": 1000}]}\ncomponent {\"family\": \"Box\", \"at\": [3000,1000], \"angle\": 30}".to_string(),
    ];
    for c in ["beam", "strip_footing", "pipe", "conduit", "cable_tray", "wire", "down_conductor", "ground_conductor", "cabling"] {
        scripts.push(format!("{c} {{\"points\": [[1000,2000],[4000,3000]]}}"));
    }
    for script in scripts {
        let mut s = Session::new();
        s.script(&script).unwrap_or_else(|e| panic!("{script}: {e}"));
        let id = *s.model.elements.keys().find(|&&id| id >= 100002).unwrap();
        let line = crate::align::lines(&s.model, id).first().copied().unwrap_or_else(|| panic!("no lines: {script}"));
        let reference = (P2(-2000.0, -1000.0), P2(6000.0, 1000.0));
        let before = s.model.clone();
        s.execute("align", &json!({"id": id, "reference": reference, "line": line})).unwrap_or_else(|e| panic!("{script}: {e}"));
        let after = crate::align::lines(&s.model, id);
        let n = (reference.1 - reference.0).unit().perp();
        assert!(
            after.iter().any(|&(a, b)| {
                (a - reference.0).dot(n).abs() < 0.01
                    && (b - reference.0).dot(n).abs() < 0.01
                    && (a.dist(b) - line.0.dist(line.1)).abs() < 0.01
            }),
            "{script}: no unchanged-length edge on the reference: {after:?}"
        );
        s.execute("undo", &json!({})).unwrap();
        assert_eq!(s.model.elements, before.elements, "{script}");
    }
}

#[test]
fn align_slides_openings_along_their_host_and_keeps_reversed_parallel_lines_pointing_the_same_way() {
    let mut s = Session::new();
    s.script("wall {\"points\": [[0,0],[6000,0]]}\ndoor {\"wall\": 100002, \"offset\": 2000}\nwindow {\"wall\": 100002, \"offset\": 4500}")
        .unwrap();
    for (id, x, to) in [(100003, 2000.0, 1500.0), (100004, 4500.0, 4000.0)] {
        let reference = [[to, -1000.0], [to, 1000.0]];
        let out = s.execute("align", &json!({"id":s.model.guid(id), "reference":reference,"line":[[x,-100],[x,100]]})).unwrap();
        assert_eq!(out["angle"], 0.0);
        let e = crate::element_json(&s.model, id);
        assert_eq!(e["offset"], to);
    }
    s.execute("grid", &json!({"start":[6000,1000],"end":[1000,1000]})).unwrap();
    let out = s.execute("align", &json!({"id":100005,"reference":[[0,0],[7000,0]],"line":[[6000,1000],[1000,1000]]})).unwrap();
    assert_eq!(out["angle"], 0.0);
    let e = crate::element_json(&s.model, 100005);
    assert_eq!(e["start"], json!([6000.0, 0.0]));
    assert_eq!(e["end"], json!([1000.0, 0.0]));
}

#[test]
fn align_uses_real_straight_segments_and_not_chords_of_curves() {
    let mut s = Session::new();
    s.execute("detail_line", &json!({"points":[[1000,1000],[3000,1000],[4000,2000]],"arcs":[0,90]})).unwrap();
    assert_eq!(crate::align::lines(&s.model, 100002), [(P2(1000.0, 1000.0), P2(3000.0, 1000.0))]);
    s.execute("detail_line", &json!({"centre":[6000,2000],"radius":1000})).unwrap();
    assert!(crate::align::lines(&s.model, 100003).is_empty());
    s.execute("pipe", &json!({"points":[[1000,1000],[1000,1000]],"offset":0,"end_offset":3000})).unwrap();
    assert_eq!(crate::align::lines(&s.model, 100004).len(), 2, "upright round runs offer centre axes, not facets");
    s.execute("align", &json!({"id":100004,"reference":[[0,0],[0,5000]],"line":[[1000,990],[1000,1010]]})).unwrap();
    let before = s.model.clone();
    assert!(s.execute("align", &json!({"id":100004,"reference":[[0,0],[5000,5000]],"line":[[0,990],[0,1010]]})).is_err());
    assert_eq!(s.model.elements, before.elements);
}

#[test]
fn sketch_alignment_refuses_a_collapsed_join_without_changing_the_draft() {
    use crate::sketch::SketchLine;
    let lines = vec![
        SketchLine::new(P2(1000.0, 1000.0), P2(4000.0, 1000.0)),
        SketchLine::new(P2(4000.0, 1000.0), P2(4000.0, 2000.0)),
        SketchLine::new(P2(4000.0, 2000.0), P2(1000.0, 2000.0)),
        SketchLine::new(P2(1000.0, 2000.0), P2(1000.0, 1000.0)),
    ];
    let before = lines.clone();
    let reference = (P2(0.0, 2000.0), P2(6000.0, 2000.0));
    assert!(crate::align::sketch_lines(&lines, 0, reference, (lines[0].start(), lines[0].end()), Some(2)).is_err());
    assert_eq!(lines, before);
}
