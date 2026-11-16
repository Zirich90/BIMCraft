//! Laying point fire detectors over a room (SP 484.1311500.2020): every point of the room within a detector's reach, a
//! smoke detector's 6.4 m or a heat detector's 3.5 m, and no detector nearer a wall than half a metre.

use serde_json::{Value, json};

use crate::engine::Session;
use crate::family::Category;
use crate::geom::{P2, Region, dist_to_segment, ring_edges};
use crate::model::Element;

#[test]
fn fire_standard_is_pinned_in_project_files() {
    let mut s = Session::new();
    let package = "ru_sp484_2020_a1_sp486_2020_a1";
    assert_eq!(run(&mut s, "fire_standard", json!({}))["package"], package);
    let before = s.model.clone();
    fails(&mut s, "fire_standard", json!({"set":"unknown"}));
    fails(&mut s, "fire_standard", json!({"set":null}));
    assert_eq!(s.model, before);
    assert_eq!(run(&mut s, "fire_standard", json!({"set":package}))["package"], package);

    let saved = s.model.to_json().unwrap();
    let (reopened, left_out) = crate::model::Model::read(&saved).unwrap();
    assert!(left_out.is_empty());
    assert_eq!(reopened.fire_standard, s.model.fire_standard);

    let mut file: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(file["fire_standard"], package);
    file["fire_standard"] = json!("unknown");
    assert!(crate::model::Model::read(&file.to_string()).is_err());
    file.as_object_mut().unwrap().remove("fire_standard");
    let (legacy, _) = crate::model::Model::read(&file.to_string()).unwrap();
    assert_eq!(legacy.fire_standard, reopened.fire_standard);

    assert_eq!(run(&mut s, "detector_ceiling_check", json!({}))["package"], package);
    assert_eq!(run(&mut s, "hidden_fire_protection", json!({"space":"plenum"}))["package"], package);
}

#[test]
fn hidden_fire_protection_keeps_cable_thresholds_and_plenum_scope_distinct() {
    let mut s = Session::new();
    let mut data = json!({"space":"plenum","cable_load":1.5,"pipe_insulation":"none"});
    let check = |s: &mut Session, data: &Value| run(s, "hidden_fire_protection", data.clone());
    assert_eq!(check(&mut s, &data)["status"], "sps_required");
    data["cable_load"] = json!(7);
    assert_eq!(check(&mut s, &data)["status"], "insufficient_data");
    data["plenum_aup_scope"] = json!(true);
    assert_eq!(check(&mut s, &data)["status"], "aup_required");
    data["plenum_aup_scope"] = json!(false);
    assert_eq!(check(&mut s, &data)["status"], "outside_aup_scope");
    data["space"] = json!("underfloor");
    data.as_object_mut().unwrap().remove("plenum_aup_scope");
    data["f5_off_egress"] = json!(true);
    assert_eq!(check(&mut s, &data)["alternative"], "upa_and_sps");
    data["cable_load"] = json!(0);
    data["pipe_insulation"] = json!("g2");
    assert_eq!(check(&mut s, &data)["status"], "insufficient_data");
    data["g2_cables_qualify"] = json!(true);
    assert_eq!(check(&mut s, &data)["status"], "aup_exception_allowed");
    data["pipe_insulation"] = json!("g3");
    assert_eq!(check(&mut s, &data)["status"], "aup_required");
    data["pipe_insulation"] = json!("none");
    data["cable_load"] = json!(1.499);
    assert_eq!(check(&mut s, &data)["status"], "no_table_2_item_10_trigger");
    data["cable_load"] = json!(-1);
    assert!(fails(&mut s, "hidden_fire_protection", data).contains("cable_load"));
}

#[test]
fn perforated_ceiling_requires_all_three_measured_conditions() {
    let mut s = Session::new();
    let assess = |s: &mut Session, data| run(s, "detector_ceiling_check", data);
    let at_limits = assess(&mut s, json!({"open_percent":40,"min_opening":12,"thickness":36}));
    assert_eq!(at_limits["clause"], "6.6.7");
    assert_eq!(at_limits["status"], "eligible");
    for data in [
        json!({"open_percent":39.99,"min_opening":12,"thickness":36}),
        json!({"open_percent":40,"min_opening":10,"thickness":30}),
        json!({"open_percent":40,"min_opening":12,"thickness":36.01}),
        json!({"open_percent":39.99}),
    ] {
        assert_eq!(assess(&mut s, data)["status"], "needs_below_ceiling_detector");
    }
    let missing = assess(&mut s, json!({"open_percent":40,"min_opening":12}));
    assert_eq!(missing["status"], "insufficient_data");
    assert_eq!(missing["checks"]["thickness"], "insufficient_data");
    assert!(fails(&mut s, "detector_ceiling_check", json!({"open_percent":101})).contains("open_percent"));
    assert!(fails(&mut s, "detector_ceiling_check", json!({"min_opening":0})).contains("min_opening"));
    assert!(fails(&mut s, "detector_ceiling_check", json!({"thickness":-1})).contains("thickness"));
}

#[test]
fn one_dwelling_zone_keeps_its_members_on_each_level() {
    let mut s = Session::new();
    let base = s.model.levels()[0].0;
    let upper = run(&mut s, "level", json!({"name":"Upper","elevation":4000}))["id"].as_u64().unwrap();
    let outline = json!([[0, 0], [4000, 0], [4000, 4000], [0, 4000]]);
    let upstairs = json!([[6000, 0], [10000, 0], [10000, 4000], [6000, 4000]]);
    run(&mut s, "wall", json!({"level":upper,"points":upstairs,"closed":true}));
    let upper_room = run(&mut s, "room", json!({"level":upper,"point":[8000,2000]}))["id"].as_u64().unwrap();
    run(&mut s, "wall", json!({"level":base,"points":outline,"closed":true}));
    run(&mut s, "room", json!({"level":base,"point":[2000,2000]}));
    let device = |s: &mut Session, level, at| {
        let id = run(s, "component", json!({"family":"Smoke Detector","level":level,"at":at}))["id"].as_u64().unwrap();
        let Some(Element::Component(component)) = s.model.elements.get(id) else { panic!("missing detector") };
        let offset = 2200.0 - s.model.component_contact_height(component, "bottom");
        run(s, "modify", json!({"id":id,"offset":offset}));
        id
    };
    let lower_detector = device(&mut s, base, [2000, 2000]);
    let upper_detector = device(&mut s, upper, [8000, 2000]);
    device(&mut s, base, [8000, 2000]);
    device(&mut s, upper, [2000, 2000]);
    let mut config =
        json!({"name":"Apartment 4","kind":"detection","level":base,"height":3500,"boundary":outline,"include_rooms":[upper_room]});
    assert!(fails(&mut s, "fire_zone", json!({"zone":config})).contains("level"));
    config["unit"] = json!("apartment");
    let id = run(&mut s, "fire_zone", json!({"zone":config}))["id"].as_u64().unwrap();
    let report = run(&mut s, "fire_zone", json!({"id":id}));
    assert_eq!(report["devices"], json!([lower_detector, upper_detector]));
    assert_eq!(report["assessment"]["room_group"]["status"], "unit_exception");
    let memberships =
        crate::low_voltage::zone_geometry::detection_memberships(&s.model, &[s.model.guid(upper)].into_iter().collect(), &[upper_detector])
            .unwrap();
    assert_eq!(memberships.get(&upper_detector), Some(&vec![id]));
    run(&mut s, "modify", json!({"id":upper,"graphics":{"engineering":true}}));
    let drawing = crate::render::plan(&s.model, upper, 800, 600);
    let Some(Element::Room(room)) = s.model.elements.get(upper_room) else { panic!("missing upper room") };
    let region = crate::rooms::Rooms::new(&s.model).region(room).unwrap();
    let (a, b) = ring_edges(&region.outer).next().unwrap();
    assert!(drawing.pick_all(drawing.view.px((a + b) * 0.5), 3.0).contains(&id));
    assert!(!drawing.pick_all(drawing.view.px(P2(2000.0, 0.0)), 3.0).contains(&id));
    if let Some(path) = std::env::var_os("DUMP_MULTILEVEL_FIRE_ZONE_PLAN") {
        std::fs::write(path, crate::render::png(&drawing).unwrap()).unwrap();
    }
    let (reopened, left_out) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty());
    assert_eq!(reopened.elements.get(id), s.model.elements.get(id));
}

#[test]
fn fire_zone_checks_dedicated_room_projection_without_merging_adjacent_rooms() {
    let mut s = Session::new();
    let (room, region) = room(&mut s, json!([[0, 0], [4000, 0], [4000, 4000], [0, 4000]]), [2000.0, 2000.0], None);
    let level = s.model.levels()[0].0;
    let mut config = json!({"name":"Dedicated","kind":"detection","level":level,"height":3500,"boundary":region.outer});
    let id = run(&mut s, "fire_zone", json!({"zone":config}))["id"].as_u64().unwrap();
    let params = json!({"id":id,"dedicated_rooms":[room]});
    let assessed = run(&mut s, "fire_zone", params.clone());
    assert_eq!(assessed["assessment"]["dedicated_projection"]["status"], "pass");
    let mut other = config.clone();
    other["name"] = json!("Overlapping detection");
    let other_id = run(&mut s, "fire_zone", json!({"zone":other}))["id"].as_u64().unwrap();
    let assessed = run(&mut s, "fire_zone", params.clone());
    let projection = &assessed["assessment"]["dedicated_projection"];
    assert_eq!(projection["status"], "fail");
    assert_eq!(projection["levels"][0]["overlapping_zones"][0]["id"], other_id);
    other["base_offset"] = json!(3500);
    run(&mut s, "fire_zone", json!({"id":other_id,"zone":other}));
    let assessed = run(&mut s, "fire_zone", params.clone());
    assert_eq!(assessed["assessment"]["dedicated_projection"]["status"], "pass");
    config["boundary"] = json!([[0, 0], [6000, 0], [6000, 4000], [0, 4000]]);
    run(&mut s, "fire_zone", json!({"id":id,"zone":config}));
    let assessed = run(&mut s, "fire_zone", params);
    let projection = &assessed["assessment"]["dedicated_projection"];
    assert_eq!(projection["status"], "fail");
    assert!(projection["levels"][0]["extra_m2"].as_f64().unwrap() > 0.0);
}

#[test]
fn fire_zone_uses_room_overrides_height_and_notification_links() {
    let mut s = Session::new();
    let (excluded, _) = room(&mut s, json!([[0, 0], [4000, 0], [4000, 4000], [0, 4000]]), [2000.0, 2000.0], None);
    let (included, _) = room(&mut s, json!([[8000, 0], [12000, 0], [12000, 4000], [8000, 4000]]), [10000.0, 2000.0], None);
    let device = |s: &mut Session, at: Value, height| {
        let id = run(s, "component", json!({"family":"Smoke Detector","at":at}))["id"].as_u64().unwrap();
        let Some(Element::Component(component)) = s.model.elements.get(id) else { panic!("missing detector") };
        let offset = f64::from(height) - s.model.component_contact_height(component, "bottom");
        run(s, "modify", json!({"id":id,"offset":offset}));
        id
    };
    device(&mut s, json!([2000, 2000]), 2200);
    let extended = device(&mut s, json!([10000, 2000]), 2200);
    device(&mut s, json!([10000, 2000]), 4000);
    let direct = device(&mut s, json!([5000, 2000]), 2200);
    run(&mut s, "component", json!({"family":"Manual Call Point","at":[5000,2000]}));
    let level = s.model.levels()[0].0;
    let boundary = json!([[0, 0], [6000, 0], [6000, 4000], [0, 4000]]);
    let notify =
        run(&mut s, "fire_zone", json!({"zone":{"name":"Evacuate","kind":"notification","level":level,"height":3000,"boundary":boundary}}))
            ["id"]
            .as_u64()
            .unwrap();
    let sounder = run(&mut s, "component", json!({"family":"Sounder","at":[5000,2000]}))["id"].as_u64().unwrap();
    run(&mut s, "component", json!({"family":"Sounder","at":[5000,2500],"offset":1000}));
    assert_eq!(run(&mut s, "fire_zone", json!({"id":notify}))["devices"], json!([sounder]));
    let config = json!({"name":"Detect","kind":"detection","level":level,"height":3000,"boundary":boundary,
        "include_rooms":[included],"exclude_rooms":[excluded],"notify":[notify]});
    let id = run(&mut s, "fire_zone", json!({"zone":config}))["id"].as_u64().unwrap();
    let guid = s.model.guid(id);
    let report = run(&mut s, "fire_zone", json!({"id":guid}));
    assert_eq!(report["devices"], json!([extended, direct]));
    assert_eq!(report["assessment"]["area_m2"], 24.0);
    assert_eq!(report["assessment"]["area"]["status"], "pass");
    assert_eq!(report["assessment"]["rooms"]["ids"], json!([included]));
    let group = json!({"same_floor":true,"same_fire_compartment":true,"isolated_rooms":[included],
        "common_exit":true,"max_exit_gap":10000,"total_exit_gap":50000});
    let grouped = run(&mut s, "fire_zone", json!({"id":id,"room_group":group}));
    assert_eq!(grouped["assessment"]["room_group"]["status"], "pass");
    let mut with_vestibule = group.clone();
    with_vestibule["isolated_rooms"] = json!([]);
    with_vestibule["vestibules"] = json!([{"room":included,"depth":800,"narrowest_door":900}]);
    let check =
        |s: &mut Session, evidence: &Value| run(s, "fire_zone", json!({"id":id,"room_group":evidence}))["assessment"]["room_group"].clone();
    let assessed = check(&mut s, &with_vestibule);
    assert_eq!(assessed["excluded_vestibules"], json!([included]));
    assert_eq!(assessed["counted_rooms"], 0);
    assert!(assessed["counted_area_m2"].as_f64().unwrap() < 24.0);
    with_vestibule["vestibules"][0]["depth"] = json!(900);
    let assessed = check(&mut s, &with_vestibule);
    assert_eq!(assessed["excluded_vestibules"], json!([]));
    assert_eq!(assessed["counted_rooms"], 1);
    let mut over = group.clone();
    over["common_exit"] = json!(false);
    assert_eq!(run(&mut s, "fire_zone", json!({"id":id,"room_group":over}))["assessment"]["room_group"]["status"], "fail");
    let incomplete = run(&mut s, "fire_zone", json!({"id":id,"room_group":{"same_floor":true}}));
    assert_eq!(incomplete["assessment"]["room_group"]["status"], "insufficient_data");
    assert_eq!(report["zone"]["notify"], json!([s.model.guid(notify)]));
    let outline = run(&mut s, "edit_boundary", json!({"id":id}))["sketch"].clone();
    assert_eq!(outline.as_array().unwrap().len(), 4);
    run(&mut s, "edit_boundary", json!({"id":id,"sketch":outline}));
    assert_eq!(s.model.guid(id), guid);
    let (reopened, left_out) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty());
    assert_eq!(reopened.guid(id), guid);
    let mut invalid = config.clone();
    invalid["notify"] = json!([guid]);
    assert!(fails(&mut s, "fire_zone", json!({"id":guid,"zone":invalid})).contains("notification"));
    run(&mut s, "delete", json!({"ids":[notify,included]}));
    let report = run(&mut s, "fire_zone", json!({"id":guid}));
    assert_eq!(report["devices"], json!([direct]));
    assert!(report["zone"].get("notify").is_none());
}

#[test]
fn fire_zone_checks_isolated_room_area_and_exit_gap_limits() {
    let mut s = Session::new();
    let (left, _) = room(&mut s, json!([[0, 0], [20000, 0], [20000, 10000], [0, 10000]]), [10000.0, 5000.0], None);
    let (right, _) = room(&mut s, json!([[30000, 0], [50000, 0], [50000, 10000], [30000, 10000]]), [40000.0, 5000.0], None);
    let level = s.model.levels()[0].0;
    let zone = run(
        &mut s,
        "fire_zone",
        json!({"zone":{
            "name":"Main","kind":"detection","level":level,"height":3000,
            "boundary":[[-1000,-1000],[51000,-1000],[51000,11000],[-1000,11000]]
        }}),
    )["id"]
        .clone();
    let mut group = json!({"same_floor":true,"same_fire_compartment":true,"isolated_rooms":[left,right],
        "common_exit":true,"max_exit_gap":10000,"total_exit_gap":10000});
    let assess =
        |s: &mut Session, group: &Value| run(s, "fire_zone", json!({"id":zone,"room_group":group}))["assessment"]["room_group"].clone();
    let report = assess(&mut s, &group);
    assert_eq!(report["status"], "pass");
    assert!((report["isolated_area_m2"].as_f64().unwrap() - 400.0).abs() < 20.0);
    group["max_exit_gap"] = json!(10001);
    group["total_exit_gap"] = json!(10001);
    assert_eq!(assess(&mut s, &group)["checks"]["max_exit_gap"], "fail");
    group["max_exit_gap"] = json!(10000);
    group["total_exit_gap"] = json!(50001);
    assert_eq!(assess(&mut s, &group)["checks"]["total_exit_gap"], "fail");
}

#[test]
fn fire_zone_assesses_projected_area_and_hidden_grouping() {
    let mut s = Session::new();
    let level = s.model.levels()[0].0;
    let boundary = json!([[0, 0], [50000, 0], [50000, 40000], [0, 40000]]);
    let config = json!({"name":"Zone", "kind":"detection", "level":level, "height":3000, "boundary":boundary});
    let main = run(&mut s, "fire_zone", json!({"zone":config}))["id"].clone();
    let report = run(&mut s, "fire_zone", json!({"id":main}));
    assert_eq!(report["assessment"]["area_m2"], 2000.0);
    assert_eq!(report["assessment"]["area"]["status"], "pass");
    let mut hidden = config.clone();
    hidden["name"] = json!("Above ceiling");
    hidden["space"] = json!("plenum");
    let id = run(&mut s, "fire_zone", json!({"zone":hidden}))["id"].clone();
    let report = run(&mut s, "fire_zone", json!({"id":id}));
    assert_eq!(report["assessment"]["hidden_grouping"]["status"], "pass");
    assert_eq!(report["assessment"]["hidden_grouping"]["main_zone"], main);
    let duplicate = run(&mut s, "fire_zone", json!({"zone":config}))["id"].clone();
    let report = run(&mut s, "fire_zone", json!({"id":id}));
    assert_eq!(report["assessment"]["hidden_grouping"]["status"], "ambiguous");
    run(&mut s, "delete", json!({"ids":[duplicate]}));

    hidden["boundary"] = json!([[0, 0], [51000, 0], [51000, 40000], [0, 40000]]);
    run(&mut s, "fire_zone", json!({"id":id, "zone":hidden}));
    let report = run(&mut s, "fire_zone", json!({"id":id}));
    assert_eq!(report["assessment"]["area_m2"], 2040.0);
    assert_eq!(report["assessment"]["area"]["status"], "limit_exceeded");
    assert_eq!(report["assessment"]["hidden_grouping"]["status"], "insufficient_data");
}

#[test]
fn fire_zone_reports_more_than_five_projected_rooms() {
    let mut s = Session::new();
    let level = s.model.levels()[0].0;
    let mut rooms = Vec::new();
    for x in [0, 4000, 8000, 12000, 16000, 20000] {
        let boundary = json!([[x, 0], [x + 3000, 0], [x + 3000, 3000], [x, 3000]]);
        rooms.push(room(&mut s, boundary, [f64::from(x) + 1500.0, 1500.0], None).0);
    }
    let config = json!({"name":"Six rooms", "kind":"detection", "level":level, "height":3000,
        "boundary":[[0,0],[24000,0],[24000,4000],[0,4000]]});
    let id = run(&mut s, "fire_zone", json!({"zone":config}))["id"].clone();
    let report = run(&mut s, "fire_zone", json!({"id":id}));
    assert_eq!(report["assessment"]["rooms"]["ids"], json!(rooms));
    assert_eq!(report["assessment"]["rooms"]["status"], "limit_exceeded");
}

#[test]
fn detectors_use_the_modeled_ceiling_for_reach_and_contact_height() {
    let mut s = Session::new();
    let (room, _) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], Some("Level 2"));
    run(&mut s, "level", json!({"elevation": 9000, "name": "High"}));
    run(&mut s, "ceiling", json!({"level":"Level 2","boundary":[[0,0],[8000,0],[8000,6000],[0,6000]],"offset":2700}));

    let placed = run(&mut s, "detectors", json!({"room":room}));
    assert_eq!(placed["ids"].as_array().unwrap().len(), 1);
    let id = placed["ids"][0].as_u64().unwrap();
    let Some(Element::Component(device)) = s.model.elements.get(id) else { panic!("missing detector") };
    assert_eq!(device.offset + s.model.component_contact_height(device, "bottom"), 2700.0);
    assert_eq!(run(&mut s, "resat", json!({"room":room}))["changed"], json!([]));
}

#[test]
fn detectors_cover_both_ceiling_heights_and_resat_keeps_device_ids() {
    let mut s = Session::new();
    let (room, _) = room(&mut s, json!([[0, 0], [12000, 0], [12000, 6000], [0, 6000]]), [3000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":[[0,0],[6000,0],[6000,6000],[0,6000]],"offset":2500}));
    let right =
        run(&mut s, "ceiling", json!({"boundary":[[6000,0],[12000,0],[12000,6000],[6000,6000]],"offset":2800}))["id"].as_u64().unwrap();

    let placed = run(&mut s, "detectors", json!({"room":room}));
    let ids = placed["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    let (left_id, right_id) = if laid(&s, &placed)[0].0.0 < 6000.0 {
        (ids[0].as_u64().unwrap(), ids[1].as_u64().unwrap())
    } else {
        (ids[1].as_u64().unwrap(), ids[0].as_u64().unwrap())
    };
    let contact = |s: &Session, id| match s.model.elements.get(id) {
        Some(Element::Component(c)) => c.offset + s.model.component_contact_height(c, "bottom"),
        _ => panic!("missing detector"),
    };
    assert_eq!((contact(&s, left_id), contact(&s, right_id)), (2500.0, 2800.0));
    let right_guid = s.model.guid(right_id);
    run(&mut s, "modify", json!({"id":right,"offset":3000}));
    assert_eq!(run(&mut s, "resat", json!({"room":room}))["changed"], json!([right_id]));
    assert_eq!((contact(&s, left_id), contact(&s, right_id)), (2500.0, 3000.0));
    assert_eq!(s.model.guid(right_id), right_guid);
}

#[test]
fn detectors_ignore_the_seam_between_equal_ceiling_heights() {
    let mut s = Session::new();
    let (room, _) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [3000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":[[0,0],[4000,0],[4000,6000],[0,6000]],"offset":2500}));
    run(&mut s, "ceiling", json!({"boundary":[[4000,0],[8000,0],[8000,6000],[4000,6000]],"offset":2500}));
    assert_eq!(run(&mut s, "detectors", json!({"room":room}))["ids"].as_array().unwrap().len(), 1);
}

#[test]
fn detectors_separate_hidden_spaces_and_follow_their_upper_slab() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    let ceiling = run(&mut s, "ceiling", json!({"boundary":boundary,"offset":2500}))["id"].as_u64().unwrap();
    let slab = run(&mut s, "floor", json!({"boundary":boundary,"offset":3500}))["id"].as_u64().unwrap();
    let base = run(&mut s, "floor", json!({"boundary":boundary,"offset":-600}))["id"].as_u64().unwrap();
    let raised = run(&mut s, "floor", json!({"boundary":[[0,0],[4000,0],[4000,6000],[0,6000]],"offset":0}))["id"].as_u64().unwrap();
    let plenum = json!({"above":ceiling,"below":slab});
    let underfloor = json!({"above":base,"below":raised});
    let main = run(&mut s, "detectors", json!({"room":room}))["ids"][0].as_u64().unwrap();
    let hidden = run(&mut s, "detectors", json!({"room":room,"space":plenum}))["ids"][0].as_u64().unwrap();
    let under = run(&mut s, "detectors", json!({"room":room,"space":underfloor}))["ids"][0].as_u64().unwrap();
    let contact = |s: &Session, id| match s.model.elements.get(id) {
        Some(Element::Component(c)) => c.offset + s.model.component_contact_height(c, "bottom"),
        _ => panic!("missing detector"),
    };
    let depth = match s.model.elements.get(slab) {
        Some(Element::Floor(f)) => s.model.floor_depth(f),
        _ => panic!("missing slab"),
    };
    assert_eq!(contact(&s, main), 2500.0);
    assert_eq!(contact(&s, hidden), 3500.0 - depth);
    assert_eq!(contact(&s, under), -depth);
    let level = s.model.levels()[0].0;
    let zone = run(
        &mut s,
        "fire_zone",
        json!({"zone":{
            "name":"Plenum","kind":"detection","level":level,"boundary":boundary,
            "base_offset":2500,"height":1000,"space":"plenum"
        }}),
    )["id"]
        .clone();
    let assess = |s: &mut Session, evidence: Value| run(s, "fire_zone", json!({"id":zone,"remote_indicators":evidence}));
    let evidence = |detector, visible| json!([{"detector":detector,"connected":true,"visible":visible}]);
    let hidden_guid = s.model.guid(hidden);
    let report = assess(&mut s, evidence(hidden_guid, true));
    assert_eq!(report["devices"], json!([hidden]));
    assert_eq!(report["assessment"]["hidden_indication"]["status"], "pass");
    assert_eq!(assess(&mut s, json!([]))["assessment"]["hidden_indication"]["status"], "insufficient_data");
    let not_visible = assess(&mut s, evidence(hidden_guid, false));
    assert_eq!(not_visible["assessment"]["hidden_indication"]["status"], "fail");
    let outside = evidence(s.model.guid(main), true);
    assert!(fails(&mut s, "fire_zone", json!({"id":zone,"remote_indicators":outside})).contains("member"));
    let Some(Element::Component(device)) = s.model.elements.get(under) else { panic!("missing underfloor detector") };
    assert!(device.at.0 < 4000.0);
    let guid = s.model.guid(hidden);
    run(&mut s, "modify", json!({"id":slab,"offset":3700}));
    assert_eq!(run(&mut s, "resat", json!({"room":room}))["ids"], json!([main]));
    assert!(fails(&mut s, "resat", json!({"room":room,"ids":[hidden]})).contains("space"));
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":[guid],"space":plenum}))["changed"], json!([hidden]));
    assert_eq!(s.model.guid(hidden), guid);
    assert_eq!(contact(&s, hidden), 3700.0 - depth);
    let (reopened, left_out) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty());
    let Some(Element::Component(device)) = reopened.elements.get(guid) else { panic!("missing reopened detector") };
    assert_eq!(device.detector_space, Some(crate::model::DetectorSpace::Plenum));
    assert!(fails(&mut s, "detectors", json!({"room":room,"space":{"above":slab,"below":raised}})).contains("positive"));
}

#[test]
fn detectors_respect_linear_beam_bays_and_cross_spacing() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    run(&mut s, "beam", json!({"points":[[4000,0],[4000,6000]],"offset":3000,"type":"Beam 200 x 500"}));
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let positions = laid(&s, &placed);
    let (left, right): (Vec<_>, Vec<_>) = positions.iter().map(|(at, _)| at.0).partition(|&x| x < 4000.0);
    let left_edge = left.iter().copied().max_by(f64::total_cmp).unwrap();
    let right_edge = right.iter().copied().min_by(f64::total_cmp).unwrap();
    assert!(left_edge <= 3400.0 && right_edge >= 4600.0);
    assert!(right_edge - left_edge <= 2300.0);
    assert_eq!(placed["package"], json!(s.model.fire_standard));
    let preview = run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}));
    assert_eq!(preview["package"], placed["package"]);
    assert_eq!(preview["status"], "ready");
}

#[test]
fn detectors_cover_three_bays_beneath_a_tee_of_ceiling_beams() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[0, 3000], [8000, 3000]]), json!([[4000, 0], [4000, 3000]])] {
        run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let points = laid(&s, &placed);
    assert!(points.iter().any(|(at, _)| at.1 < 2900.0 && at.0 < 3900.0));
    assert!(points.iter().any(|(at, _)| at.1 < 2900.0 && at.0 > 4100.0));
    assert!(points.iter().any(|(at, _)| at.1 > 3100.0));
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
    let level = s.model.levels()[0].0;
    run(&mut s, "modify", json!({"id":level,"graphics":{"engineering":true}}));
    run(&mut s, "detail", json!({"view":level,"detail":"medium"}));
    let drawing = crate::render::ceiling_plan(&s.model, level, 1100, 800);
    for detector in placed["ids"].as_array().unwrap() {
        assert!(drawing.owners.contains(&detector.as_u64()));
    }
    if let Some(path) = std::env::var_os("DUMP_TEE_DETECTOR_CEILING_PLAN") {
        std::fs::write(path, crate::render::png(&drawing).unwrap()).unwrap();
    }
}

#[test]
fn detectors_cover_around_the_free_ends_of_a_ceiling_beam() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    run(&mut s, "beam", json!({"points":[[4000,1500],[4000,4500]],"offset":3000,"type":"Beam 200 x 500"}));
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let points = laid(&s, &placed);
    assert!(points.iter().any(|(at, _)| at.0 < 3900.0));
    assert!(points.iter().any(|(at, _)| at.0 > 4100.0));
    for (at, _) in points {
        let dx = (at.0 - 4000.0).abs() - 100.0;
        let dy = (1500.0 - at.1).max(at.1 - 4500.0);
        assert!(dx.max(0.0).hypot(dy.max(0.0)) >= 500.0 - 1e-6, "{at:?}");
    }
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

#[test]
fn detectors_cover_around_a_free_beam_end_in_a_crossed_layout() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[0, 3000], [8000, 3000]]), json!([[4000, 0], [4000, 2400]])] {
        run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let points = laid(&s, &placed);
    assert!(points.iter().any(|(at, _)| at.1 < 2900.0 && at.0 < 3900.0));
    assert!(points.iter().any(|(at, _)| at.1 < 2900.0 && at.0 > 4100.0));
    assert!(points.iter().any(|(at, _)| at.1 > 3100.0));
    for (at, _) in points {
        let dx = ((at.0 - 4000.0).abs() - 100.0).max(0.0);
        let dy = (at.1 - 2400.0).max(0.0);
        assert!(dx.hypot(dy) >= 500.0 - 1e-6, "{at:?}");
        assert!((at.1 - 3000.0).abs() >= 600.0 - 1e-6, "{at:?}");
    }
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

#[test]
fn detectors_use_the_middle_of_a_narrow_bay_between_deep_beams() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for x in [3500, 4500] {
        run(&mut s, "beam", json!({"points":[[x,0],[x,6000]],"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let positions = laid(&s, &placed);
    let middle: Vec<_> = positions.iter().filter(|(at, _)| (3600.0..4400.0).contains(&at.0)).collect();
    assert!(!middle.is_empty());
    assert!(middle.iter().all(|(at, _)| (at.0 - 4000.0).abs() < 1e-6));
    assert!(positions.iter().any(|(at, _)| at.0 <= 2900.0));
    assert!(positions.iter().any(|(at, _)| at.0 >= 5100.0));
}

#[test]
fn detectors_center_a_narrow_bay_between_a_finished_wall_and_beam() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, region) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    let (lo, _) = crate::geom::bounds(&region.outer).unwrap();
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    run(&mut s, "beam", json!({"points":[[800,0],[800,6000]],"offset":3000,"type":"Beam 200 x 500"}));
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let positions = laid(&s, &placed);
    let middle = (lo.0 + 700.0) * 0.5;
    let bay: Vec<_> = positions.iter().filter(|(at, _)| at.0 < 700.0).collect();
    assert!(!bay.is_empty());
    assert!(bay.iter().all(|(at, _)| (at.0 - middle).abs() < 1e-6));
    assert!(positions.iter().any(|(at, _)| at.0 >= 1400.0));
}

#[test]
fn detectors_center_a_narrow_corridor_between_finished_walls() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [16000, 0], [16000, 900], [0, 900]]);
    let (room, region) = room(&mut s, boundary.clone(), [8000.0, 450.0], None);
    let (lo, hi) = crate::geom::bounds(&region.outer).unwrap();
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    let placed = run(&mut s, "detectors", json!({"room":room,"radius":2500}));
    let positions = laid(&s, &placed);
    assert!(positions.len() > 1);
    assert!(positions.iter().all(|(at, _)| (at.1 - (lo.1 + hi.1) * 0.5).abs() < 1e-6));
    for corner in region.outer {
        assert!(positions.iter().any(|(at, _)| at.dist(corner) <= 2500.0));
    }
}

#[test]
fn crossed_beam_cells_choose_beam_bottom_or_ceiling_by_cell_width() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [6500, 0], [6500, 6500], [0, 6500]]);
    let (room, _) = room(&mut s, boundary.clone(), [3500.0, 3500.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[1800, 0], [1800, 6500]]), json!([[0, 1800], [6500, 1800]])] {
        run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let mut bottom = false;
    let mut ceiling = false;
    for id in placed["ids"].as_array().unwrap() {
        let Some(Element::Component(device)) = s.model.elements.get(id.as_u64().unwrap()) else { panic!("missing detector") };
        let contact = device.offset + s.model.component_contact_height(device, "bottom");
        if (contact - 2500.0).abs() < 1e-6 {
            assert!((device.at.0 - 1800.0).abs() <= 100.0 || (device.at.1 - 1800.0).abs() <= 100.0);
            bottom = true;
        } else {
            assert!((contact - 3000.0).abs() < 1e-6);
            assert!(device.at.0 > 1900.0 && device.at.1 > 1900.0);
            ceiling = true;
        }
    }
    assert!(bottom && ceiling);
}

#[test]
fn crossed_beams_with_different_depths_mount_at_the_selected_underside() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [5900, 0], [5900, 5900], [0, 5900]]);
    let (room, _) = room(&mut s, boundary.clone(), [3500.0, 3500.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    run(&mut s, "beam", json!({"points":[[1800,0],[1800,5900]],"offset":3000,"type":"Beam 200 x 500"}));
    run(&mut s, "beam", json!({"points":[[0,1800],[5900,1800]],"offset":3200,"type":"Beam 200 x 500"}));
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let mut underside = false;
    let mut ceiling = false;
    for id in placed["ids"].as_array().unwrap() {
        let Some(Element::Component(device)) = s.model.elements.get(id.as_u64().unwrap()) else { panic!("missing detector") };
        let contact = device.offset + s.model.component_contact_height(device, "bottom");
        if (contact - 2500.0).abs() < 1e-6 {
            assert!((device.at.0 - 1800.0).abs() < 1e-6);
            underside = true;
        } else {
            assert!((contact - 3000.0).abs() < 1e-6);
            ceiling = true;
        }
    }
    assert!(underside && ceiling);
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

#[test]
fn oblique_beams_keep_detectors_inside_each_wide_ceiling_cell() {
    assert_oblique_beam_ceiling_cells(json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]));
}

#[test]
fn oblique_beams_cover_a_wide_cell_with_a_wall_inset() {
    assert_oblique_beam_ceiling_cells(json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000], [0, 4750], [750, 4750], [750, 4200], [0, 4200]]));
}

#[test]
fn oblique_beams_cover_a_wide_cell_with_insets_on_two_sides() {
    assert_oblique_beam_ceiling_cells(json!([
        [0, 0],
        [8000, 0],
        [8000, 6000],
        [3300, 6000],
        [3300, 5650],
        [2700, 5650],
        [2700, 6000],
        [0, 6000],
        [0, 4750],
        [750, 4750],
        [750, 4200],
        [0, 4200]
    ]));
}

#[test]
fn oblique_beams_report_the_uncovered_point_when_an_inset_blocks_visibility() {
    let mut s = Session::new();
    let room = oblique_beam_room(
        &mut s,
        json!([
            [0, 0],
            [8000, 0],
            [8000, 6000],
            [3300, 6000],
            [3300, 5650],
            [2700, 5650],
            [2700, 6000],
            [0, 6000],
            [0, 5400],
            [750, 5400],
            [750, 4500],
            [0, 4500]
        ]),
    );
    let error = fails(&mut s, "detectors", json!({"room":room}));
    assert!(error.contains("no visible detector position reaches (") && error.contains(") with the requested clearances"), "{error}");
}

#[test]
fn oblique_beams_report_a_deep_wall_inset_that_leaves_a_narrow_cell() {
    let mut s = Session::new();
    let room =
        oblique_beam_room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000], [0, 5400], [2800, 5400], [2800, 4500], [0, 4500]]));
    assert!(fails(&mut s, "detectors", json!({"room":room})).contains("separately designed mounting surface"));
}

fn assert_oblique_beam_ceiling_cells(boundary: Value) {
    let mut s = Session::new();
    let room = oblique_beam_room(&mut s, boundary);
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let mut cells = std::collections::BTreeSet::new();
    for id in placed["ids"].as_array().unwrap() {
        let Some(Element::Component(device)) = s.model.elements.get(id.as_u64().unwrap()) else { panic!("missing detector") };
        let contact = device.offset + s.model.component_contact_height(device, "bottom");
        let from_cross = device.at.1 - 3000.0;
        let from_oblique = (device.at - P2(3500.0, 0.0)).dot(P2(6000.0, -1000.0).unit());
        assert!((contact - 3000.0).abs() < 1e-6);
        assert!(from_cross.abs() >= 600.0 - 1e-6 && from_oblique.abs() >= 600.0 - 1e-6);
        cells.insert((from_cross > 0.0, from_oblique > 0.0));
    }
    assert_eq!(cells.len(), 4);
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

fn oblique_beam_room(s: &mut Session, boundary: Value) -> u64 {
    let (id, _) = room(s, boundary.clone(), [4000.0, 3000.0], None);
    run(s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[0, 3000], [8000, 3000]]), json!([[3500, 0], [4500, 6000]])] {
        run(s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    id
}

#[test]
fn oblique_beams_cover_a_wide_cell_with_a_returning_inset() {
    let mut s = Session::new();
    let boundary =
        json!([[0, 0], [14000, 0], [14000, 9000], [0, 9000], [0, 5300], [1500, 5300], [1500, 6800], [2400, 6800], [2400, 4600], [0, 4600]]);
    let (room, _) = room(&mut s, boundary.clone(), [5000.0, 2000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[0, 3000], [14000, 3000]]), json!([[6500, 0], [7600, 9000]])] {
        run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    assert!(laid(&s, &placed).iter().any(|(at, _)| at.0 < 1500.0 && (5300.0..6800.0).contains(&at.1)));
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

#[test]
fn oblique_beams_report_uncovered_point_in_a_tight_returning_inset() {
    let mut s = Session::new();
    let boundary =
        json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000], [0, 5300], [450, 5300], [450, 5500], [850, 5500], [850, 4600], [0, 4600]]);
    let room = oblique_beam_room(&mut s, boundary);
    let before = s.model.clone();

    let error = fails(&mut s, "detectors", json!({"room":room}));

    assert!(error.contains("(219, 5550)"), "{error}");
    assert_eq!(s.model, before);
}

#[test]
fn oblique_beams_check_clear_passages_around_a_room_hole() {
    for (x, wide) in [(2800, true), (1200, false)] {
        let mut s = Session::new();
        let boundary = json!([[0, 0], [14000, 0], [14000, 12000], [0, 12000]]);
        run(&mut s, "wall", json!({"closed":true,"points":[[x,2700],[x+600,2700],[x+600,3300],[x,3300]]}));
        let (room, region) = room(&mut s, boundary.clone(), [5000.0, 2000.0], None);
        assert_eq!(region.holes.len(), 1);
        run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
        for points in [json!([[0, 6000], [14000, 6000]]), json!([[6500, 0], [7500, 12000]])] {
            run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
        }
        if wide {
            let placed = run(&mut s, "detectors", json!({"room":room}));
            let ids = placed["ids"].as_array().unwrap();
            assert!(ids.len() >= 4);
            for id in ids {
                let Some(Element::Component(device)) = s.model.elements.get(id.as_u64().unwrap()) else { panic!("missing detector") };
                assert!(region.contains(device.at) && region.edge_distance(device.at) >= 500.0);
                let contact = device.offset + s.model.component_contact_height(device, "bottom");
                assert!((contact - 3000.0).abs() < 1e-6);
            }
            assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
        } else {
            let before = s.model.clone();
            assert!(fails(&mut s, "detectors", json!({"room":room})).contains("mounting surface"));
            assert_eq!(s.model, before);
        }
    }
}

#[test]
fn oblique_beams_check_the_passage_between_two_room_holes() {
    for (x, y, wide) in [(7000, 5000, true), (4400, 2900, false)] {
        let mut s = Session::new();
        let boundary = json!([[0, 0], [24000, 0], [24000, 20000], [0, 20000]]);
        for (x, y) in [(2800, 2700), (x, y)] {
            run(&mut s, "wall", json!({"closed":true,"points":[[x,y],[x+600,y],[x+600,y+600],[x,y+600]]}));
        }
        let (room, region) = room(&mut s, boundary.clone(), [5000.0, 2000.0], None);
        assert_eq!(region.holes.len(), 2);
        run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
        for points in [json!([[0, 10000], [24000, 10000]]), json!([[12000, 0], [14000, 20000]])] {
            run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
        }
        if wide {
            let placed = run(&mut s, "detectors", json!({"room":room}));
            let positions = laid(&s, &placed);
            assert!(positions.len() >= 4);
            assert!(positions.iter().all(|(at, _)| region.contains(*at) && region.edge_distance(*at) >= 500.0));
            assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
        } else {
            let before = s.model.clone();
            assert!(fails(&mut s, "detectors", json!({"room":room})).contains("mounting surface"));
            assert_eq!(s.model, before);
        }
    }
}

#[test]
fn narrow_oblique_beam_cells_mount_on_the_shared_beam_underside() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    run(&mut s, "ceiling", json!({"boundary":boundary,"offset":3000}));
    for points in [json!([[0, 1800], [8000, 1800]]), json!([[3500, 0], [4500, 6000]])] {
        run(&mut s, "beam", json!({"points":points,"offset":3000,"type":"Beam 200 x 500"}));
    }
    let placed = run(&mut s, "detectors", json!({"room":room}));
    let mut below = [false; 2];
    let mut on_ceiling = [false; 2];
    for id in placed["ids"].as_array().unwrap() {
        let Some(Element::Component(device)) = s.model.elements.get(id.as_u64().unwrap()) else { panic!("missing detector") };
        let contact = device.offset + s.model.component_contact_height(device, "bottom");
        let from_oblique = (device.at - P2(3500.0, 0.0)).dot(P2(6000.0, -1000.0).unit());
        let side = usize::from(from_oblique > 0.0);
        if (contact - 2500.0).abs() < 1e-6 {
            assert!((device.at.1 - 1800.0).abs() < 1e-6);
            below[side] = true;
        } else {
            assert!((contact - 3000.0).abs() < 1e-6 && device.at.1 > 1900.0);
            on_ceiling[side] = true;
        }
    }
    assert_eq!(below, [true; 2]);
    assert_eq!(on_ceiling, [true; 2]);
    assert_eq!(run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"preview":true}))["status"], "ready");
}

#[test]
fn hidden_smoke_detector_radius_depends_on_clear_height() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    let (room, _) = room(&mut s, boundary.clone(), [4000.0, 3000.0], None);
    let lower = run(&mut s, "floor", json!({"boundary":boundary,"offset":-2000}))["id"].as_u64().unwrap();
    let upper = run(&mut s, "floor", json!({"boundary":boundary,"offset":0}))["id"].as_u64().unwrap();
    let depth = match s.model.elements.get(upper) {
        Some(Element::Floor(f)) => s.model.floor_depth(f),
        _ => panic!("missing upper slab"),
    };
    let space = json!({"above":lower,"below":upper});
    run(&mut s, "modify", json!({"id":upper,"offset":-2000.0 + 1699.0 + depth}));
    let ids = run(&mut s, "detectors", json!({"room":room,"space":space,"radius":9600}))["ids"].clone();
    assert!(fails(&mut s, "detectors", json!({"room":room,"space":space,"radius":9601})).contains("9600"));
    assert!(fails(&mut s, "detectors", json!({"room":room,"radius":9600})).contains("6400"));
    assert!(fails(&mut s, "detectors", json!({"room":room,"space":space,"radius":3501,"family":"Heat Detector"})).contains("3500"));
    run(&mut s, "modify", json!({"id":upper,"offset":-2000.0 + 1700.0 + depth}));
    assert!(fails(&mut s, "resat", json!({"room":room,"space":space,"radius":9600,"ids":ids})).contains("6400"));
}

#[test]
fn ifc_adaptation_keeps_hidden_detector_and_slab_guids_across_reloads() {
    let mut architecture = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    run(&mut architecture, "wall", json!({"points":boundary,"closed":true}));
    let ceiling = run(&mut architecture, "ceiling", json!({"boundary":boundary,"offset":2500}))["id"].as_u64().unwrap();
    let slab = run(&mut architecture, "floor", json!({"boundary":boundary,"offset":3500}))["id"].as_u64().unwrap();
    let depth = match architecture.model.elements.get(slab) {
        Some(Element::Floor(floor)) => architecture.model.floor_depth(floor),
        _ => panic!("missing upper slab"),
    };
    let space = json!({"above":architecture.model.guid(ceiling),"below":architecture.model.guid(slab)});
    let path = std::env::temp_dir().join(format!("bimcraft-hidden-ifc-detectors-{}.ifc", std::process::id()));
    run(&mut architecture, "save", json!({"path":path}));
    let mut s = Session::new();
    let link = run(&mut s, "link", json!({"path":path}))["id"].as_u64().unwrap();
    let room = run(&mut s, "room", json!({"point":[4000,3000]}))["id"].as_u64().unwrap();
    let placed = run(&mut s, "detectors", json!({"room":room,"architecture":link,"space":space}));
    let id = placed["ids"][0].as_u64().unwrap();
    let guid = s.model.guid(id);
    run(&mut architecture, "modify", json!({"id":slab,"offset":3700}));
    run(&mut architecture, "save", json!({"path":path}));
    let groups = json!([{"room":room,"ids":[guid],"space":space}]);
    let before = s.model.clone();
    let preview = run(&mut s, "adapt_ifc", json!({"id":link,"groups":groups}));
    assert_eq!(preview["status"], "ready");
    assert_eq!(*s.model, *before);
    let applied = run(&mut s, "adapt_ifc", json!({"id":link,"groups":groups,"apply":true,"expected":preview}));
    assert_eq!(applied["status"], "applied");
    let Some(Element::Component(device)) = s.model.elements.get(id) else { panic!("missing detector") };
    assert_eq!(s.model.guid(id), guid);
    assert_eq!(device.detector_space, Some(crate::model::DetectorSpace::Plenum));
    assert!((device.offset + s.model.component_contact_height(device, "bottom") - (3700.0 - depth)).abs() < 0.01);
    let (reopened, left_out) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty());
    s.model = std::sync::Arc::new(reopened);
    run(&mut architecture, "modify", json!({"id":slab,"offset":3900}));
    run(&mut architecture, "save", json!({"path":path}));
    let reload = run(&mut s, "reload", json!({"id":link}));
    let next = &reload["adaptations"][0];
    assert_eq!(next["status"], "ready");
    assert!((next["groups"][0]["targets"][0]["contact_offset"].as_f64().unwrap() - (3900.0 - depth)).abs() < 0.01);
    assert_eq!(s.model.guid(id), guid);
    let _ = std::fs::remove_file(path);
}

#[test]
fn detectors_follow_reloaded_ifc_architecture_and_keep_the_connected_device() {
    let mut architecture = Session::new();
    let boundary = json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]);
    run(&mut architecture, "wall", json!({"points":boundary,"closed":true}));
    let ceiling = run(&mut architecture, "ceiling", json!({"boundary":boundary,"offset":2500}))["id"].as_u64().unwrap();
    let path = std::env::temp_dir().join(format!("bimcraft-detectors-architecture-{}.ifc", std::process::id()));
    run(&mut architecture, "save", json!({"path":path}));

    let mut s = Session::new();
    let link = run(&mut s, "link", json!({"path":path,"at":[1000,2000]}))["id"].as_u64().unwrap();
    let link = s.model.guid(link);
    let room = run(&mut s, "room", json!({"point":[4000,5000]}))["id"].as_u64().unwrap();
    let placed = run(&mut s, "detectors", json!({"room":room,"architecture":link}));
    let id = placed["ids"][0].as_u64().unwrap();
    let guid = s.model.guid(id);
    let at = laid(&s, &placed)[0].0;
    let contact = |s: &Session| match s.model.elements.get(id) {
        Some(Element::Component(c)) => c.offset + s.model.component_contact_height(c, "bottom"),
        _ => panic!("missing detector"),
    };
    assert!((contact(&s) - 2500.0).abs() < 0.01);
    let Element::Component(device) = s.model.elements.get(id).unwrap() else { panic!("missing detector") };
    let local_contact = s.model.component_contact_height(device, "bottom");
    let wire = run(&mut s, "wire", json!({"points":[[2000,3000],[2500,3000]],"offset":1500}))["ids"][0].as_u64().unwrap();
    run(&mut s, "wire_connect", json!({"wire":wire,"device":guid,"point":[0,0,local_contact]}));
    let feeder = s
        .model
        .elements
        .iter()
        .find_map(|(&id, element)| {
            let f = element.run()?.feeder.as_ref()?;
            (f.device == guid && f.leg == 0).then_some(id)
        })
        .unwrap();
    let feeder_guid = s.model.guid(feeder);

    run(&mut architecture, "wall", json!({"points":[[7000,0],[7000,6000]]}));
    run(&mut architecture, "modify", json!({"id":ceiling,"offset":2800}));
    run(&mut architecture, "save", json!({"path":path}));
    let before = s.model.clone();
    let mut params = json!({"id":link,"groups":[{"room":room,"ids":[guid]}]});
    let preview = run(&mut s, "adapt_ifc", params.clone());
    assert_eq!(preview["status"], "ready");
    assert!((preview["groups"][0]["targets"][0]["contact_offset"].as_f64().unwrap() - 2800.0).abs() < 0.01);
    assert!(preview["routes"].as_array().unwrap().iter().any(|route| route["wire"] == wire));
    assert_eq!(*s.model, *before);
    let blocked = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true,"groups":[{"room":room,"ids":[guid],"radius":2000}]}));
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["groups"][0]["status"], "count_mismatch");
    assert_eq!(*s.model, *before);
    let old_preview = run(&mut s, "resat", json!({"room":room,"ids":[guid],"architecture":link,"preview":true}));
    assert!((old_preview["targets"][0]["contact_offset"].as_f64().unwrap() - 2500.0).abs() < 0.01);
    assert!(
        fails(
            &mut s,
            "adapt_ifc",
            json!({"id":link,"apply":true,"groups":[
                {"room":room,"ids":[guid]}, {"room":room,"ids":[guid]}
            ]})
        )
        .contains("more than one")
    );
    params["apply"] = json!(true);
    let result = run(&mut s, "adapt_ifc", params);
    assert_eq!(result["status"], "applied");
    assert_eq!(result["changed"], json!([id]));
    assert!(laid(&s, &placed)[0].0.0 < at.0 - 100.0);
    assert!((contact(&s) - 2800.0).abs() < 0.01);
    assert_eq!(s.model.guid(id), guid);
    assert_eq!(s.model.elements.get(wire).unwrap().run().unwrap().connections[0].device, guid);
    assert_eq!(s.model.guid(feeder), feeder_guid);
    assert!(s.model.elements.get(feeder).is_some_and(|element| {
        element.run().and_then(|r| r.feeder.as_ref()).is_some_and(|f| f.device == guid && f.leg == 0)
            && s.model.run_geom(element).is_some_and(|g| (g.a[2] - 2800.0).abs() < 0.01)
    }));
    assert!(s.model.elements.values().all(|e| !matches!(e, Element::Wall(_) | Element::Ceiling(_))));
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before);
    let undone = run(&mut s, "resat", json!({"room":room,"ids":[guid],"architecture":link,"preview":true}));
    assert!((undone["targets"][0]["contact_offset"].as_f64().unwrap() - 2500.0).abs() < 0.01);
    run(&mut s, "redo", json!({}));
    let (loaded, left_out) = crate::model::Model::read(&s.model.to_json().unwrap()).unwrap();
    assert!(left_out.is_empty());
    s.model = std::sync::Arc::new(loaded);
    run(&mut architecture, "modify", json!({"id":ceiling,"offset":2900}));
    run(&mut architecture, "save", json!({"path":path}));
    let reloaded = run(&mut s, "reload", json!({"id":link}));
    assert_eq!(reloaded["adaptations"][0]["status"], "ready");
    assert_eq!(reloaded["adaptations"][0]["link"], s.model.elements.id(link).unwrap());
    assert_eq!(reloaded["adaptations"][0]["changed"], json!([id]));
    assert!((contact(&s) - 2800.0).abs() < 0.01, "the ordinary reload only assesses");
    let result = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    assert_eq!(result["status"], "applied");
    assert!((contact(&s) - 2900.0).abs() < 0.01);
    assert_eq!(s.model.guid(id), guid);

    let added = run(&mut architecture, "wall", json!({"points":[[1250,500],[1250,1500]],"height":2500}));
    let obstacle = architecture.model.guid(added["ids"][0].as_u64().unwrap());
    run(&mut architecture, "save", json!({"path":path}));
    let before_clash = s.model.clone();
    let blocked = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    assert_eq!(blocked["status"], "blocked");
    assert!(
        blocked["route_clashes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|clash| { clash["segment"] == wire && clash["wire"] == wire && clash["obstacle"] == json!(obstacle) })
    );
    assert_eq!(s.model, before_clash);
    let reloaded = run(&mut s, "reload", json!({"id":link}));
    assert_eq!(reloaded["adaptations"][0]["status"], "blocked");
    let blocked_after_reload = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    assert_eq!(blocked_after_reload["status"], "blocked");

    run(&mut architecture, "delete", json!({"ids":[obstacle]}));
    let beam =
        run(&mut architecture, "beam", json!({"points":[[1250,500],[1250,1500]],"offset":1500,"type":"Beam 200 x 500"}))["ids"][0].clone();
    run(&mut architecture, "save", json!({"path":path}));
    let before_penetration = s.model.clone();
    let review = run(&mut s, "adapt_ifc", json!({"id":link}));
    assert_eq!(review["package"], json!(s.model.fire_standard));
    assert_eq!(review["status"], "blocked");
    assert_eq!(review["can_approve_crossings"], true, "{review}");
    assert!(fails(&mut s, "adapt_ifc", json!({"id":link,"apply":true,"penetration_reference":"MEP-17"})).contains("expected"));
    let mut other_edition = review.clone();
    other_edition["package"] = json!("another_edition");
    let stale_package = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true,"expected":other_edition,"penetration_reference":"MEP-17"}));
    assert_eq!(stale_package["status"], "review_required");
    assert_eq!(stale_package["preview"], review);
    assert_eq!(s.model, before_penetration);
    run(&mut architecture, "modify", json!({"id":beam,"end":[1250,1600]}));
    run(&mut architecture, "save", json!({"path":path}));
    let stale = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true,"expected":review,"penetration_reference":"MEP-17"}));
    assert_eq!(stale["status"], "review_required", "{stale}");
    assert_eq!(stale["preview"]["route_clashes"], review["route_clashes"]);
    assert_ne!(stale["preview"]["ifc_revision"], review["ifc_revision"]);
    assert_eq!(s.model, before_penetration);
    let accepted = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true,"expected":stale["preview"],"penetration_reference":"MEP-17"}));
    assert_eq!(accepted["status"], "applied", "{accepted}");
    assert_eq!(accepted["penetration_reference"], "MEP-17");
    assert_eq!(s.model.guid(id), guid);
    assert_eq!(s.model.guid(feeder), feeder_guid);
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model, before_penetration);
    run(&mut architecture, "delete", json!({"ids":[beam]}));

    let enclosing = run(&mut architecture, "wall", json!({"points":[[500,1000],[1900,1000]],"height":2500}));
    let enclosing = architecture.model.guid(enclosing["ids"][0].as_u64().unwrap());
    run(&mut architecture, "save", json!({"path":path}));
    let before_enclosed = s.model.clone();
    let blocked = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    assert_eq!(blocked["status"], "blocked");
    assert!(
        blocked["route_clashes"]
            .as_array()
            .is_some_and(|clashes| { clashes.iter().any(|clash| clash["segment"] == wire && clash["obstacle"] == json!(enclosing)) }),
        "{blocked}"
    );
    assert_eq!(s.model, before_enclosed);

    run(&mut architecture, "delete", json!({"ids":[enclosing]}));
    // The 4 mm cable axis is 1 mm outside this wall's near face.
    let grazed = run(&mut architecture, "wall", json!({"points":[[500,1101],[1900,1101]],"height":2500}));
    let grazed = architecture.model.guid(grazed["ids"][0].as_u64().unwrap());
    run(&mut architecture, "save", json!({"path":path}));
    let blocked = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    assert!(
        blocked["route_clashes"]
            .as_array()
            .is_some_and(|clashes| { clashes.iter().any(|clash| clash["segment"] == wire && clash["obstacle"] == json!(grazed)) }),
        "{blocked}"
    );

    run(&mut architecture, "delete", json!({"ids":[grazed]}));
    let carrier = run(&mut s, "cable_carrier", json!({"points":[[2000,3000],[2500,3000]],"offset":1500,"type":"Tray 50 x 50"}))["ids"][0]
        .as_u64()
        .unwrap();
    let hosted = run(&mut s, "wire", json!({"points":[[2000,3000],[2500,3000]],"offset":1500,"carrier":carrier}))["ids"][0].clone();
    run(&mut s, "wire_connect", json!({"wire":hosted,"device":guid,"point":[0,0,local_contact],"carrier":carrier}));
    let tray_only = run(&mut architecture, "wall", json!({"points":[[500,1110],[1900,1110]],"height":2500}));
    let tray_only = architecture.model.guid(tray_only["ids"][0].as_u64().unwrap());
    run(&mut architecture, "save", json!({"path":path}));
    let before_carrier = s.model.clone();
    let blocked = run(&mut s, "adapt_ifc", json!({"id":link,"apply":true}));
    let clashes = blocked["route_clashes"].as_array().unwrap();
    assert_eq!(clashes.iter().filter(|clash| clash["segment"] == carrier && clash["obstacle"] == json!(tray_only)).count(), 1, "{blocked}");
    assert!(clashes.iter().all(|clash| clash["segment"] != hosted || clash["obstacle"] != json!(tray_only)), "{blocked}");
    assert_eq!(s.model, before_carrier);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn detectors_report_when_no_position_meets_wall_clearance() {
    let mut s = Session::new();
    let (room, _) = room(&mut s, json!([[0, 0], [10000, 0], [10000, 750], [0, 750]]), [5000.0, 375.0], None);

    assert!(fails(&mut s, "detectors", json!({"room":room,"margin":600})).contains("wall clearance"));
    let placed = run(&mut s, "detectors", json!({"room":room,"margin":100}));
    assert!(!placed["ids"].as_array().unwrap().is_empty());
}

#[test]
fn resat_moves_the_same_connected_detector_after_room_and_ceiling_edits() {
    let mut s = Session::new();
    let (room, _) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], None);
    let ceiling = run(&mut s, "ceiling", json!({"boundary":[[0,0],[9000,0],[9000,6000],[0,6000]],"offset":2500}))["id"].as_u64().unwrap();
    let made = run(&mut s, "detectors", json!({"room":room}));
    let id = made["ids"][0].as_u64().unwrap();
    let guid = s.model.guid(id);
    let before = laid(&s, &made)[0].0;
    let Some(Element::Component(c)) = s.model.elements.get(id) else { panic!("missing detector") };
    let contact = s.model.component_contact_height(c, "bottom");
    let wire = run(&mut s, "wire", json!({"points":[[1000,1000],[1500,1000]],"offset":1500}))["ids"][0].as_u64().unwrap();
    run(&mut s, "wire_connect", json!({"wire":wire,"device":guid,"point":[0,0,contact]}));
    run(&mut s, "wall", json!({"points":[[7000,0],[7000,6000]]}));
    run(&mut s, "modify", json!({"id":ceiling,"offset":2800}));
    let original = s.model.to_json().unwrap();
    let result = run(&mut s, "resat", json!({"room":room,"ids":[guid]}));
    assert_eq!(result["changed"], json!([id]));
    assert_ne!(laid(&s, &result)[0].0, before);
    assert_eq!(s.model.guid(id), guid);
    let Some(Element::Component(c)) = s.model.elements.get(id) else { panic!("missing detector") };
    assert_eq!(c.offset + s.model.component_contact_height(c, "bottom"), 2800.0);
    let connections = &s.model.elements.get(wire).unwrap().run().unwrap().connections;
    assert_eq!(connections[0].device, guid);
    assert!(s.model.elements.values().any(|element| {
        element.run().and_then(|r| r.feeder.as_ref()).is_some_and(|f| f.device == guid && f.leg == 0)
            && s.model.run_geom(element).is_some_and(|g| g.a == [c.at.0, c.at.1, s.model.elevation(c.level) + 2800.0])
    }));
    assert_eq!(run(&mut s, "resat", json!({"room":room}))["changed"], json!([]));
    run(&mut s, "undo", json!({}));
    assert_eq!(s.model.to_json().unwrap(), original);
}

#[test]
fn resat_refuses_a_grid_requiring_more_detectors_without_moving_any() {
    let mut s = Session::new();
    let (id, _) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], None);
    run(&mut s, "detectors", json!({"room":id}));
    assert!(fails(&mut s, "resat", json!({"room":id,"radius":2000})).contains("positions"));
    let before = s.model.clone();
    let preview = run(&mut s, "resat", json!({"room":id,"radius":2000,"preview":true}));
    assert_eq!(preview["status"], "count_mismatch");
    assert!(preview["targets"].as_array().unwrap().len() > 1);
    assert_eq!(*s.model, *before);
    assert!(fails(&mut s, "resat", json!({"room":id,"radius":1})).contains("2000"));
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.model.clone();
    let e = s.execute(id, &p).expect_err(&format!("{id} {p} should fail"));
    assert_eq!(*s.model, *before, "a refused {id} changes nothing");
    e
}

/// A room bounded by walls round `points`, and its region; `level` when the walls and the room are to stand on a level
/// other than the model's lowest.
fn room(s: &mut Session, points: Value, inside: [f64; 2], level: Option<&str>) -> (u64, Region) {
    run(
        s,
        "wall",
        level.map(|l| json!({"points": points, "closed": true, "level": l})).unwrap_or(json!({"points": points, "closed": true})),
    );
    let id =
        run(s, "room", level.map(|l| json!({"point": inside, "level": l})).unwrap_or(json!({"point": inside})))["id"].as_u64().unwrap();
    let region = match s.model.elements.get(id) {
        Some(Element::Room(r)) => s.model.room_region(r).unwrap(),
        _ => panic!("{id} is no room"),
    };
    (id, region)
}

/// The detectors laid, each its point and its family's category.
fn laid(s: &Session, r: &Value) -> Vec<(P2, Category)> {
    r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| match s.model.elements.get(v.as_u64().unwrap()) {
            Some(Element::Component(c)) => (c.at, s.model.families[&c.family].category),
            e => panic!("{v} is {e:?}"),
        })
        .collect()
}

/// Every point of the region, every 100 mm, within `reach` of a detector; every detector inside and half a metre off the walls.
fn covers(region: &Region, at: &[P2], reach: f64) {
    let rings = || std::iter::once(&region.outer).chain(&region.holes);
    let wall = |p: P2| rings().flat_map(|r| ring_edges(r)).map(|(a, b)| dist_to_segment(p, a, b)).fold(f64::MAX, f64::min);
    let inside = |p: P2| crate::geom::contains(&region.outer, p) && !region.holes.iter().any(|h| crate::geom::contains(h, p));
    for p in at {
        assert!(inside(*p), "{p:?} is in the room");
        assert!(wall(*p) >= 500.0 - 1e-6, "{p:?} is {} from a wall", wall(*p));
    }
    let (lo, hi) = region.outer.iter().fold((P2(f64::MAX, f64::MAX), P2(f64::MIN, f64::MIN)), |(lo, hi), p| {
        (P2(lo.0.min(p.0), lo.1.min(p.1)), P2(hi.0.max(p.0), hi.1.max(p.1)))
    });
    let mut y = lo.1;
    while y <= hi.1 {
        let mut x = lo.0;
        while x <= hi.0 {
            let p = P2(x, y);
            if inside(p) {
                let near = at.iter().map(|d| d.dist(p)).fold(f64::MAX, f64::min);
                assert!(near <= reach + 1e-6, "{p:?} is {near} from the nearest detector");
            }
            x += 100.0;
        }
        y += 100.0;
    }
}

#[test]
fn detectors_cover_a_room_within_their_reach_and_stand_off_its_walls() {
    let mut s = Session::new();
    let (id, region) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], None);
    // A smoke detector reaches 6.4 m: one in the middle of a room under 9 m across.
    let r = run(&mut s, "detectors", json!({"room": id}));
    let smoke = laid(&s, &r);
    assert_eq!(smoke.len(), 1);
    assert_eq!(smoke[0].1, Category::SmokeDetector);
    covers(&region, &smoke.iter().map(|d| d.0).collect::<Vec<_>>(), 6400.0);
    // A heat detector reaches 3.5 m, a grid under 4.95 m: two by two.
    let r = run(&mut s, "detectors", json!({"room": id, "kind": "heat"}));
    let heat = laid(&s, &r);
    assert_eq!(heat.len(), 4);
    assert!(heat.iter().all(|d| d.1 == Category::HeatDetector));
    covers(&region, &heat.iter().map(|d| d.0).collect::<Vec<_>>(), 3500.0);
    // On the room's level, at the family's first type.
    let level = match s.model.elements.get(id) {
        Some(Element::Room(r)) => r.level,
        e => panic!("{id} is {e:?}"),
    };
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    for i in ids {
        match s.model.elements.get(i) {
            Some(Element::Component(c)) => {
                assert_eq!(c.level, level);
                assert_eq!(c.type_name, "3000");
            }
            e => panic!("{e:?}"),
        }
    }
    // A reach of one's own: a tighter grid.
    let r = run(&mut s, "detectors", json!({"room": id, "radius": 2000}));
    let tight = laid(&s, &r);
    covers(&region, &tight.iter().map(|d| d.0).collect::<Vec<_>>(), 2000.0);
    assert!(tight.len() > heat.len());
}

#[test]
fn detectors_cover_an_l_shaped_room_and_leave_out_the_corner_it_does_not_have() {
    let mut s = Session::new();
    let (id, region) =
        room(&mut s, json!([[0, 0], [20000, 0], [20000, 8000], [10000, 8000], [10000, 16000], [0, 16000]]), [5000.0, 5000.0], None);
    for (kind, reach) in [("smoke", 6400.0), ("heat", 3500.0)] {
        let r = run(&mut s, "detectors", json!({"room": id, "kind": kind}));
        covers(&region, &laid(&s, &r).iter().map(|d| d.0).collect::<Vec<_>>(), reach);
    }
}

#[test]
fn detectors_cover_both_arms_when_the_wall_blocks_the_direct_path() {
    let mut s = Session::new();
    let (id, region) = room(
        &mut s,
        json!([[0, 0], [8000, 0], [8000, 8000], [5000, 8000], [5000, 2500], [3000, 2500], [3000, 8000], [0, 8000]]),
        [1500.0, 4000.0],
        None,
    );
    let result = run(&mut s, "detectors", json!({"room":id}));
    let points: Vec<_> = laid(&s, &result).into_iter().map(|(at, _)| at).collect();
    assert!(points.iter().any(|&P2(x, y)| x < 3000.0 && y > 2500.0));
    assert!(points.iter().any(|&P2(x, y)| x > 5000.0 && y > 2500.0));
    covers(&region, &points, 6400.0);
    assert_eq!(run(&mut s, "resat", json!({"room":id}))["changed"], json!([]));
}

#[test]
fn detectors_want_a_room_and_a_detector_family() {
    let mut s = Session::new();
    let (id, _) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], None);
    assert!(fails(&mut s, "detectors", json!({})).contains("room"));
    let wall = s.model.elements.iter().find(|(_, e)| matches!(e, Element::Wall(_))).map(|(k, _)| *k).unwrap();
    assert!(fails(&mut s, "detectors", json!({"room": wall})).contains("room"));
    assert!(fails(&mut s, "detectors", json!({"room": id, "kind": "flame"})).contains("smoke or heat"));
    assert!(fails(&mut s, "detectors", json!({"room": id, "family": "Camera"})).contains("detector"));
    assert!(fails(&mut s, "detectors", json!({"room": id, "radius": 0})).contains("radius"));
    // A reach finer than a whole grid of detectors is refused, not laid.
    assert!(fails(&mut s, "detectors", json!({"room": id, "radius": 1})).contains("detectors"));
    // A room its walls no longer bound has nothing to cover.
    run(&mut s, "delete", json!({"ids": [wall]}));
    assert!(fails(&mut s, "detectors", json!({"room": id})).contains("bound"));
}

#[test]
fn sp484_amendment_one_uses_fixed_reach_up_to_the_height_limit() {
    let mut s = Session::new();
    let (id, region) = room(&mut s, json!([[0, 0], [8000, 0], [8000, 6000], [0, 6000]]), [4000.0, 3000.0], Some("Level 2"));
    let ceiling = run(&mut s, "ceiling", json!({"level":"Level 2","boundary":[[0,0],[8000,0],[8000,6000],[0,6000]],"offset":9000}))["id"]
        .as_u64()
        .unwrap();
    let heat = json!({"class":"A1R","trip_c":54,"normal_c":25,"max_c":50});
    for (kind, reach, laid_count) in [("smoke", 6400.0, 1), ("heat", 3500.0, 4)] {
        let mut p = json!({"room":id,"kind":kind});
        if kind == "heat" {
            p["heat"] = heat.clone();
        }
        let r = run(&mut s, "detectors", p);
        let at = laid(&s, &r).iter().map(|d| d.0).collect::<Vec<_>>();
        assert_eq!(at.len(), laid_count);
        covers(&region, &at, reach);
    }
    run(&mut s, "modify", json!({"id":ceiling,"offset":9001}));
    assert!(fails(&mut s, "detectors", json!({"room":id,"kind":"heat"})).contains("6.6.15"));
    assert!(fails(&mut s, "resat", json!({"room":id,"kind":"heat"})).contains("6.6.15"));
    run(&mut s, "modify", json!({"id":ceiling,"offset":12000}));
    let r = run(&mut s, "detectors", json!({"room": id}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    assert!(fails(&mut s, "detectors", json!({"room":id,"radius":6401})).contains("6.6.16"));
    run(&mut s, "modify", json!({"id":ceiling,"offset":12001}));
    assert!(fails(&mut s, "detectors", json!({"room":id,"radius":6400})).contains("6.6.16"));
}

#[test]
fn sp484_high_heat_requires_a_supported_class_and_temperature_evidence() {
    let mut s = Session::new();
    let boundary = json!([[0, 0], [6000, 0], [6000, 5000], [0, 5000]]);
    let (room, _) = room(&mut s, boundary.clone(), [3000.0, 2500.0], None);
    let ceiling = run(&mut s, "ceiling", json!({"boundary":boundary,"offset":6000}))["id"].as_u64().unwrap();
    let placed = run(&mut s, "detectors", json!({"room":room,"kind":"heat"}));
    run(&mut s, "modify", json!({"id":ceiling,"offset":6001}));
    assert!(fails(&mut s, "detectors", json!({"room":room,"kind":"heat"})).contains("insufficient"));
    assert!(fails(&mut s, "resat", json!({"room":room,"ids":placed["ids"]})).contains("6.2.3"));
    for evidence in [
        json!({"class":"A2","trip_c":54,"normal_c":25,"max_c":50}),
        json!({"class":"A1","trip_c":54,"normal_c":25,"max_c":51}),
        json!({"class":"A1","trip_c":53,"normal_c":25,"max_c":50}),
        json!({"class":"PR","trip_c":70,"normal_c":27,"max_c":52}),
    ] {
        assert!(fails(&mut s, "detectors", json!({"room":room,"kind":"heat","heat":evidence})).contains("6.2.3"));
    }
    let evidence = json!({"class":"PR","trip_c":70,"normal_c":27,"max_c":52,"rated_normal_c":30,"rated_max_c":55});
    let result = run(&mut s, "resat", json!({"room":room,"ids":placed["ids"],"heat":evidence}));
    assert_eq!(result["ids"], placed["ids"]);
    assert_eq!(result["changed"].as_array().unwrap().len(), placed["ids"].as_array().unwrap().len());
}
