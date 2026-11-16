use crate::commands::*;
use crate::engine::{Command, Session};
use crate::geom::{self, P2};
use crate::model::{Component, DetectorSpace, Element, Floor, Guid, Id, MAX_POINTS, Model, Room, TOL, Unbound};
use crate::model::{HeatEvidence, MAX_DETECTORS};
use crate::model::{boundary, coord, is_zero, size, text};
use bimcraft_i18n::ui_msg;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn fire_standard(s: &mut Session, p: &Value) -> R {
    let c = "fire_standard";
    only(c, p, &["set"])?;
    if let Some(value) = p.get("set") {
        let package = serde_json::from_value(value.clone()).map_err(|e| bad(c, format!("unsupported fire standard: {e}")))?;
        if s.model.fire_standard != package {
            s.model_mut().fire_standard = package;
        }
    }
    let package = s.model.fire_standard;
    Ok(json!({"package":package,"detector_rules":package.detector_rules(),"hidden_space_rules":package.hidden_space_rules()}))
}

fn hidden_fire_protection(s: &mut Session, p: &Value) -> R {
    let c = "hidden_fire_protection";
    only(c, p, &["space", "cable_load", "pipe_insulation", "plenum_aup_scope", "g2_cables_qualify", "f5_off_egress"])?;
    let plenum = match string(c, p, "space")?.as_deref() {
        Some("plenum") => true,
        Some("underfloor") => false,
        _ => return Err(bad(c, "choose space: plenum or underfloor")),
    };
    let cable = p
        .get("cable_load")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or_else(|| bad(c, "cable_load must be nonnegative litres per metre"))
        })
        .transpose()?;
    let pipe = match string(c, p, "pipe_insulation")?.as_deref() {
        None => None,
        Some("none" | "ng" | "g1") => Some(1),
        Some("g2") => Some(2),
        Some("g3" | "g4") => Some(3),
        _ => return Err(bad(c, "choose pipe_insulation: none, ng, g1, g2, g3 or g4")),
    };
    let confirmed = |key| flag(c, p, key).map(|value| (!absent(p.get(key))).then_some(value));
    let scope = confirmed("plenum_aup_scope")?;
    let g2 = confirmed("g2_cables_qualify")?;
    let alternative = flag(c, p, "f5_off_egress")?;
    let mut result = json!({"package":s.model.fire_standard,"standard":s.model.fire_standard.hidden_space_rules(),"table":2,"item":"10","status":"insufficient_data"});
    let (Some(cable), Some(pipe)) = (cable, pipe) else {
        result["missing"] = json!(
            [cable.is_none().then_some("cable_load"), pipe.is_none().then_some("pipe_insulation")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        );
        return Ok(result);
    };
    let status = if cable >= 7.0 || pipe >= 2 {
        match (plenum, scope, cable, pipe, g2) {
            (true, None, ..) => {
                result["missing"] = json!(["plenum_aup_scope"]);
                "insufficient_data"
            }
            (true, Some(false), ..) if (1.5..7.0).contains(&cable) => "sps_required",
            (true, Some(false), ..) => "outside_aup_scope",
            (_, _, 0.0, 2, None) => {
                result["missing"] = json!(["g2_cables_qualify"]);
                "insufficient_data"
            }
            (_, _, 0.0, 2, Some(true)) => "aup_exception_allowed",
            _ => "aup_required",
        }
    } else if cable >= 1.5 {
        "sps_required"
    } else {
        "no_table_2_item_10_trigger"
    };
    result["status"] = json!(status);
    if status == "aup_required" && alternative {
        result["alternative"] = json!("upa_and_sps");
    }
    Ok(result)
}

fn detector_ceiling_check(s: &mut Session, p: &Value) -> R {
    let c = "detector_ceiling_check";
    only(c, p, &["open_percent", "min_opening", "thickness"])?;
    let open = p
        .get("open_percent")
        .filter(|value| !value.is_null())
        .map(|value| {
            value.as_f64().filter(|value| (0.0..=100.0).contains(value)).ok_or_else(|| bad(c, "`open_percent` must be between 0 and 100"))
        })
        .transpose()?;
    let opening = num(c, p, "min_opening")?;
    if opening.is_some_and(|value| value <= 0.0) {
        return Err(bad(c, "`min_opening` must be positive"));
    }
    let thickness = num(c, p, "thickness")?;
    if thickness.is_some_and(|value| value <= 0.0) {
        return Err(bad(c, "`thickness` must be positive"));
    }
    let open_ok = open.map(|value| value >= 40.0);
    let opening_ok = opening.map(|value| value > 10.0);
    let thickness_ok = thickness.zip(opening).map(|(thickness, opening)| thickness <= 3.0 * opening);
    let checks = [open_ok, opening_ok, thickness_ok];
    let status = if checks.contains(&Some(false)) {
        "needs_below_ceiling_detector"
    } else if checks.contains(&None) {
        "insufficient_data"
    } else {
        "eligible"
    };
    let outcome = |check| match check {
        Some(true) => "pass",
        Some(false) => "fail",
        None => "insufficient_data",
    };
    Ok(json!({
        "package":s.model.fire_standard,
        "standard":s.model.fire_standard.detector_rules(),
        "clause":"6.6.7",
        "status":status,
        "checks":{"open_percent":outcome(open_ok), "min_opening":outcome(opening_ok), "thickness":outcome(thickness_ok)}
    }))
}

fn detectors(s: &mut Session, p: &Value) -> R {
    let c = "detectors";
    only(c, p, &["room", "kind", "family", "type", "radius", "margin", "space", "architecture", "heat"])?;
    let m = &s.model;
    let room = element_id(c, m, p.get("room").ok_or_else(|| bad(c, "`room` (the room's id) is required"))?)?;
    let architecture = detector_architecture(c, p, m)?;
    let geometry = architecture.as_ref().unwrap_or(m);
    let (r, region) = detector_room(c, geometry, room)?;
    let kind = match string(c, p, "kind")?.as_deref() {
        None | Some("smoke") => crate::family::Category::SmokeDetector,
        Some("heat") => crate::family::Category::HeatDetector,
        Some(k) => return Err(bad(c, format!("`kind` is smoke or heat, not `{k}`"))),
    };
    let family = match string(c, p, "family")? {
        Some(f) => f,
        None => m
            .families
            .iter()
            .find(|(_, f)| f.category == kind)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| bad(c, "the project has no family of that kind of detector: give `family`"))?,
    };
    let f = m.families.get(&family).ok_or_else(|| bad(c, format!("family `{family}` doesn't exist")))?;
    let Some(limits) = detector_limits(f.category) else {
        return Err(bad(c, format!("`{family}` isn't a smoke or heat detector")));
    };
    let type_name = match string(c, p, "type")? {
        Some(t) if f.types.contains_key(&t) => t,
        Some(t) => return Err(bad(c, format!("family `{family}` has no type `{t}`"))),
        None => f.types.keys().next().cloned().ok_or_else(|| bad(c, format!("family `{family}` has no type")))?,
    };
    let space = hidden_detector_space(c, p, geometry)?;
    let points = detector_grid(c, p, geometry, r, &region, limits, space.as_ref())?;
    let device = Component {
        family,
        type_name,
        level: r.level,
        at: r.point,
        angle: 0.0,
        offset: 0.0,
        mirrored: false,
        params: BTreeMap::new(),
        provision: None,
        mount: None,
        detector_space: space.as_ref().map(|space| space.kind),
        connection_sides: Vec::new(),
    };
    let contact = m.component_contact_height(&device, "bottom");
    if let Some(space) = &space {
        space.check_fit(c, m, &device)?;
    }
    let model = s.model_mut();
    let ids: Vec<Id> = points
        .into_iter()
        .map(|(at, ceiling)| {
            let offset = ceiling.map_or(device.offset, |height| height - contact);
            model.insert(Element::Component(Component { at, offset, ..device.clone() }))
        })
        .collect();
    Ok(json!({"ids": ids, "package": model.fire_standard}))
}

fn fire_zone(s: &mut Session, p: &Value) -> R {
    let c = "fire_zone";
    only(c, p, &["id", "zone", "remote_indicators", "room_group", "dedicated_rooms"])?;
    let m = &s.model;
    let id = p.get("id").filter(|v| !v.is_null()).map(|v| element_id(c, m, v)).transpose()?;
    if let Some(id) = id
        && !matches!(m.elements.get(id), Some(Element::FireZone(_)))
    {
        return Err(bad(c, format!("{id} isn't a fire zone")));
    }
    if let Some(value) = p.get("zone").filter(|v| !v.is_null()) {
        if p.get("dedicated_rooms").is_some() {
            return Err(bad(c, "inspect an existing detection zone to assess dedicated rooms"));
        }
        if p.get("room_group").is_some() {
            return Err(bad(c, "inspect an existing main detection zone to assess a room group"));
        }
        if p.get("remote_indicators").is_some() {
            return Err(bad(c, "inspect an existing hidden detection zone to assess remote indicators"));
        }
        let mut zone: crate::model::FireZone = serde_json::from_value(value.clone()).map_err(|e| bad(c, e))?;
        zone.boundary = ring(zone.boundary);
        let m = s.model_mut();
        let id = match id {
            Some(id) => {
                m.elements.insert(id, Element::FireZone(zone));
                id
            }
            None => m.insert(Element::FireZone(zone)),
        };
        return Ok(json!({"id": id, "guid": m.guid(id)}));
    }
    let Some(id) = id else {
        if p.get("dedicated_rooms").is_some() {
            return Err(bad(c, "select a detection zone to assess dedicated rooms"));
        }
        if p.get("room_group").is_some() {
            return Err(bad(c, "select a main detection zone to assess a room group"));
        }
        if p.get("remote_indicators").is_some() {
            return Err(bad(c, "select a hidden detection zone to assess remote indicators"));
        }
        return Ok(json!({"ids": m.rare("fire_zone")}));
    };
    let Some(Element::FireZone(zone)) = m.elements.get(id) else { return Err(bad(c, "select a fire zone")) };
    let confirmed = if let Some(value) = p.get("remote_indicators") {
        if zone.space.is_none() || zone.kind != crate::model::FireZoneKind::Detection {
            return Err(bad(c, "select a hidden detection zone to assess remote indicators"));
        }
        let evidence = value
            .as_array()
            .filter(|entries| entries.len() <= MAX_POINTS)
            .ok_or_else(|| bad(c, format!("`remote_indicators` must be an array of up to {MAX_POINTS} records")))?;
        let mut confirmed = BTreeMap::new();
        for item in evidence {
            only(c, item, &["detector", "connected", "visible"])?;
            let detector = element_id(c, m, &item["detector"])?;
            let fact = |key| match item.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Bool(value)) => Ok(Some(*value)),
                Some(_) => Err(bad(c, format!("`{key}` must be true or false"))),
            };
            let facts = [fact("connected")?, fact("visible")?];
            let state = if facts.contains(&Some(false)) {
                "fail"
            } else if facts.contains(&None) {
                "insufficient_data"
            } else {
                "pass"
            };
            if confirmed.insert(detector, state).is_some() {
                return Err(bad(c, format!("give only one remote indicator record for detector {detector}")));
            }
        }
        Some(confirmed)
    } else {
        None
    };
    let group = p
        .get("room_group")
        .map(|value| {
            if zone.space.is_some() || zone.kind != crate::model::FireZoneKind::Detection {
                return Err(bad(c, "select a main detection zone to assess a room group"));
            }
            if zone.unit.is_some() {
                return Err(bad(c, "a whole-unit zone uses the unit exception instead of ordinary room grouping"));
            }
            if value["isolated_rooms"].as_array().is_some_and(|rooms| rooms.len() > MAX_POINTS) {
                return Err(bad(c, format!("give up to {MAX_POINTS} isolated rooms")));
            }
            if value["vestibules"].as_array().is_some_and(|rooms| rooms.len() > 256) {
                return Err(bad(c, "give up to 256 vestibules"));
            }
            serde_json::from_value(value.clone()).map_err(|error| bad(c, format!("`room_group`: {error}")))
        })
        .transpose()?;
    let mut report = crate::low_voltage::zone_geometry::report(m, id, zone, confirmed, group).map_err(|error| bad(c, error))?;
    report["package"] = json!(m.fire_standard);
    if let Some(value) = p.get("dedicated_rooms") {
        if zone.space.is_some() || zone.kind != crate::model::FireZoneKind::Detection {
            return Err(bad(c, "select a main detection zone to assess dedicated rooms"));
        }
        if !value.as_array().is_some_and(|rooms| (1..=32).contains(&rooms.len())) {
            return Err(bad(c, "give between 1 and 32 dedicated rooms"));
        }
        let dedicated = guids_param(c, m, value, "dedicated_rooms")?;
        report["assessment"]["dedicated_projection"] =
            crate::low_voltage::zone_geometry::dedicated_projection(m, id, zone, &dedicated).map_err(|error| bad(c, error))?;
    }
    Ok(report)
}

fn detector_architecture(c: &str, p: &Value, m: &Model) -> Result<Option<Model>, String> {
    p.get("architecture")
        .filter(|v| !v.is_null())
        .map(|v| crate::link::with_architecture(m, element_id(c, m, v)?).map_err(|e| bad(c, e)))
        .transpose()
}

fn detector_room<'a>(c: &str, m: &'a Model, id: Id) -> Result<(&'a Room, geom::Region), String> {
    let Some(Element::Room(room)) = m.elements.get(id) else { return Err(bad(c, format!("{id} isn't a room"))) };
    let region = m.room_bound(room, m.room_area).map_err(|reason| match reason {
        Unbound::NotEnclosed => bad(c, format!("room {} isn't bound by walls any more", room.number)),
        Unbound::Shared(other) => {
            bad(c, format!("room {} shares its boundary with {}; resolve the room anchors", room.number, m.shown(other)))
        }
    })?;
    Ok((room, region))
}

struct HiddenDetectorSpace<'a> {
    kind: DetectorSpace,
    lower: SlabPlan<'a>,
    upper: SlabPlan<'a>,
    bottom: f64,
    top: f64,
}

impl HiddenDetectorSpace<'_> {
    fn check_fit(&self, c: &str, m: &Model, device: &Component) -> Result<(), String> {
        let depth = m.component_contact_height(device, "bottom") - m.component_contact_height(device, "top");
        if depth > self.top - self.bottom + TOL {
            return Err(bad(c, "the detector does not fit between the hidden space surfaces"));
        }
        Ok(())
    }
}

struct SlabPlan<'a> {
    outer: &'a [P2],
    holes: &'a [Vec<P2>],
}

impl SlabPlan<'_> {
    fn contains(&self, at: P2) -> bool {
        geom::contains(self.outer, at) && !self.holes.iter().any(|hole| geom::contains(hole, at))
    }
}

fn hidden_detector_space<'a>(c: &str, p: &Value, m: &'a Model) -> Result<Option<HiddenDetectorSpace<'a>>, String> {
    let Some(space) = p.get("space").filter(|v| !v.is_null()) else { return Ok(None) };
    only(c, space, &["above", "below"])?;
    let bound = |name| {
        let id = element_id(c, m, space.get(name).ok_or_else(|| bad(c, format!("`space.{name}` is required")))?)?;
        m.elements.get(id).ok_or_else(|| bad(c, format!("{id} doesn't exist")))
    };
    let floor = |f: &'a Floor| {
        if f.slope != 0.0 || !f.shape.is_empty() {
            return Err(bad(c, "hidden detector space needs flat, unshaped floor slabs"));
        }
        Ok((SlabPlan { outer: &f.boundary, holes: &f.openings }, m.elevation(f.level) + f.offset))
    };
    let (kind, lower, bottom) = match bound("above")? {
        Element::Ceiling(v) => (
            DetectorSpace::Plenum,
            SlabPlan { outer: &v.boundary, holes: &v.openings },
            m.elevation(v.level) + v.offset + m.types.ceiling.get(&v.type_name).map_or(0.0, |t| t.thickness),
        ),
        Element::Floor(f) => {
            let (plan, top) = floor(f)?;
            (DetectorSpace::Underfloor, plan, top)
        }
        _ => return Err(bad(c, "`space.above` must name a ceiling or flat floor")),
    };
    let Element::Floor(f) = bound("below")? else { return Err(bad(c, "`space.below` must name a flat floor")) };
    let (upper, top) = floor(f)?;
    let top = top - m.floor_depth(f);
    if top <= bottom + TOL {
        return Err(bad(c, "hidden detector space must have a positive clear height between slab faces"));
    }
    Ok(Some(HiddenDetectorSpace { kind, lower, upper, bottom, top }))
}

struct DetectorLimits {
    radius: f64,
    height: f64,
    clause: &'static str,
    heat: bool,
}

impl HeatEvidence {
    /// SP 484 Amendment 1, 6.2.3; A1 / A1R temperatures in GOST 34698-2020, table 5.1.
    fn check(&self) -> Result<(), &'static str> {
        let allowed = |temperature| (-273.15..=1000.0).contains(&temperature);
        if ![self.trip_c, self.normal_c, self.max_c].into_iter().all(allowed) || self.normal_c > self.max_c {
            return Err("supply ordered and finite ambient temperatures in degrees Celsius");
        }
        let (rated_normal, rated_max) = match self.class.as_str() {
            "A1" | "A1R" if (54.0..=65.0).contains(&self.trip_c) => (25.0, 50.0),
            "PR" if self.trip_c >= 54.0 => match (self.rated_normal_c, self.rated_max_c) {
                (Some(normal), Some(max)) if allowed(normal) && allowed(max) && normal <= max && max < self.trip_c => (normal, max),
                _ => {
                    return Err(
                        "insufficient PR temperature evidence; supply valid rated_normal_c and rated_max_c from the device documentation",
                    );
                }
            },
            _ => return Err("above 6000 mm use A1, A1R or PR with a compatible trip temperature of at least 54 °C"),
        };
        if self.normal_c > rated_normal || self.max_c > rated_max {
            return Err("ambient temperatures exceed the detector's normal or maximum normal rating");
        }
        Ok(())
    }
}

/// SP 484.1311500.2020 Amendment 1, approved by MChS order 252 of 2025-03-27.
fn detector_limits(category: crate::family::Category) -> Option<DetectorLimits> {
    use crate::family::Category::{HeatDetector, SmokeDetector};

    match category {
        HeatDetector => Some(DetectorLimits { radius: 3500.0, height: 9000.0, clause: "6.6.15", heat: true }),
        SmokeDetector => Some(DetectorLimits { radius: 6400.0, height: 12000.0, clause: "6.6.16", heat: false }),
        _ => None,
    }
}

fn detector_grid(
    c: &str,
    p: &Value,
    m: &Model,
    room: &Room,
    region: &geom::Region,
    limits: DetectorLimits,
    space: Option<&HiddenDetectorSpace<'_>>,
) -> Result<Vec<(P2, Option<f64>)>, String> {
    let reach = num(c, p, "radius")?.unwrap_or(limits.radius);
    if reach <= 0.0 {
        return Err(bad(c, "`radius` is more than 0"));
    }
    let (max_reach, clause) = if !limits.heat && space.is_some_and(|space| space.top - space.bottom < 1700.0) {
        (limits.radius * 1.5, "6.6.41")
    } else {
        (limits.radius, limits.clause)
    };
    if reach > max_reach {
        return Err(bad(c, format!("`radius` exceeds {max_reach} mm for this detector (SP 484 Amendment 1, {clause})")));
    }
    let margin = num(c, p, "margin")?.unwrap_or(500.0).max(0.0);
    let heat: Option<HeatEvidence> = p
        .get("heat")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| bad(c, format!("`heat`: {e}"))))
        .transpose()?;
    if heat.is_some() && !limits.heat {
        return Err(bad(c, "`heat` applies only to heat detectors"));
    }
    let height_ok = |height| {
        if height <= 0.0 {
            return Err(bad(c, "the room must have a positive height above its base"));
        }
        if height > limits.height {
            return Err(bad(
                c,
                format!("room height {height} mm exceeds {} mm for this detector (SP 484 Amendment 1, {})", limits.height, limits.clause),
            ));
        }
        if limits.heat && height > 6000.0 {
            heat.as_ref()
                .ok_or("insufficient heat detector class and temperature evidence; supply `heat`")
                .and_then(HeatEvidence::check)
                .map_err(|e| bad(c, format!("SP 484 Amendment 1, 6.2.3: {e}")))?;
        }
        Ok(())
    };
    let Some((lo, hi)) = geom::bounds(&region.outer) else { return Err(bad(c, "the room has no boundary")) };
    let grid = |region: &geom::Region, elevations, remaining| {
        detector_ceiling_grid(c, m, (region, room.level), elevations, reach, margin, limits.heat, remaining)
    };
    if let Some(space) = space {
        height_ok(space.top - space.bottom)?;
        let shafts = m.shaft_table();
        let shafts: Vec<_> = shafts
            .iter()
            .filter(|s| {
                s.bottom < space.top && s.top > space.bottom && s.plan[0] < hi.0 && s.plan[1] < hi.1 && s.plan[2] > lo.0 && s.plan[3] > lo.1
            })
            .take(513)
            .collect();
        let cut_rings = [&space.lower, &space.upper]
            .into_iter()
            .flat_map(|plan| std::iter::once(plan.outer).chain(plan.holes.iter().map(Vec::as_slice)))
            .chain(shafts.iter().map(|s| s.boundary.as_slice()));
        let cuts: Vec<_> = cut_rings.flat_map(geom::ring_edges).take(513).collect();
        let top = space.top - m.elevation(room.level);
        let height_at = |at| {
            (space.lower.contains(at) && space.upper.contains(at) && !shafts.iter().any(|s| geom::contains(&s.boundary, at))).then_some(top)
        };
        let compartments = region.partition_by_height(&cuts, height_at).map_err(|e| bad(c, e))?;
        let mut out = Vec::new();
        for (ceiling, region) in compartments.into_iter().filter(|(ceiling, _)| ceiling.is_some()) {
            out.extend(
                grid(&region, (space.bottom, space.top), MAX_DETECTORS - out.len())?
                    .into_iter()
                    .map(|(at, depth)| (at, depth.map(|depth| top - depth).or(ceiling))),
            );
        }
        if out.is_empty() {
            return Err(bad(c, "the two space bounds don't enclose any area in the room"));
        }
        return Ok(out);
    }
    let ceilings: Vec<_> = m
        .elements
        .values()
        .filter_map(|e| match e {
            Element::Ceiling(v)
                if v.level == room.level
                    && v.offset > room.base_offset
                    && geom::bounds(&v.boundary).is_some_and(|(a, b)| a.0 < hi.0 && a.1 < hi.1 && b.0 > lo.0 && b.1 > lo.1) =>
            {
                Some(v)
            }
            _ => None,
        })
        .take(513)
        .collect();
    let ceiling_at = |at| {
        ceilings
            .iter()
            .filter(|v| geom::contains(&v.boundary, at) && !v.openings.iter().any(|hole| geom::contains(hole, at)))
            .map(|v| v.offset)
            .min_by(f64::total_cmp)
    };
    let cuts: Vec<_> = ceilings
        .iter()
        .flat_map(|v| std::iter::once(&v.boundary).chain(&v.openings))
        .flat_map(|ring| geom::ring_edges(ring))
        .take(513)
        .collect();
    let compartments = if cuts.is_empty() {
        vec![(None, region.clone())]
    } else {
        region.partition_by_height(&cuts, ceiling_at).map_err(|e| bad(c, e))?
    };
    let level_z = m.elevation(room.level);
    let fallback = m.level_above(room.level).map_or(crate::model::DEFAULT_HEIGHT, |up| m.elevation(up) - level_z);
    let mut out = Vec::new();
    for (ceiling, region) in compartments {
        let top = ceiling.unwrap_or(fallback);
        height_ok(top - room.base_offset)?;
        let remaining = MAX_DETECTORS - out.len();
        out.extend(
            grid(&region, (level_z + room.base_offset, level_z + top), remaining)?
                .into_iter()
                .map(|(at, depth)| (at, depth.map(|depth| top - depth).or(ceiling))),
        );
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn detector_ceiling_grid(
    c: &str,
    m: &Model,
    (region, level): (&geom::Region, Guid),
    elevations: (f64, f64),
    reach: f64,
    margin: f64,
    heat: bool,
    remaining: usize,
) -> Result<Vec<(P2, Option<f64>)>, String> {
    let walls = std::cell::OnceCell::new();
    let wall_face = |a, b| {
        walls
            .get_or_init(|| m.face_lines(level))
            .iter()
            .any(|&(start, end)| geom::dist_to_segment(a, start, end) < 0.001 && geom::dist_to_segment(b, start, end) < 0.001)
    };
    let Some(layout) = crate::detector_beams::layout(m, region, elevations, reach, heat).map_err(|e| bad(c, e))? else {
        let centered = crate::detector_beams::wall_bay(region, reach, margin, remaining, wall_face).map_err(|e| bad(c, e))?;
        let points = match centered {
            Some(points) => points,
            None => detector_compartment_grid(c, region, reach, Some(margin), remaining, |at| region.edge_distance(at) >= margin, |at| at)?,
        };
        return Ok(points.into_iter().map(|at| (at, None)).collect());
    };
    let mut out: Vec<(P2, Option<f64>)> = Vec::new();
    for bay in &layout.bays {
        let remaining = remaining - out.len();
        if let Some((points, depth)) = layout.on_beam(bay, margin.max(500.0), remaining).map_err(|e| bad(c, e))? {
            for at in points {
                if !out.iter().any(|(point, other)| *other == Some(depth) && point.dist(at) < 1e-6) {
                    out.push((at, Some(depth)));
                }
            }
            continue;
        }
        if let Some(points) = layout.centered(bay, reach, margin, remaining, wall_face).map_err(|e| bad(c, e))? {
            out.extend(points.into_iter().map(|at| (at, None)));
            continue;
        }
        let clear = |at| {
            let at = layout.to_world(at);
            layout.clear(at) && region.edge_distance(at) >= margin
        };
        let points = detector_compartment_grid(c, bay, layout.reach, None, remaining, clear, |at| layout.to_world(at))?;
        out.extend(points.into_iter().map(|at| (layout.to_world(at), None)));
    }
    Ok(out)
}

fn detector_compartment_grid(
    c: &str,
    region: &geom::Region,
    reach: f64,
    rectangle_margin: Option<f64>,
    remaining: usize,
    clear: impl Fn(P2) -> bool,
    world: impl Fn(P2) -> P2,
) -> Result<Vec<P2>, String> {
    let Some((lo, hi)) = geom::bounds(&region.outer) else { return Err(bad(c, "the ceiling compartment has no boundary")) };
    let step = reach * std::f64::consts::SQRT_2;
    let cells = |span: f64| (span / step).ceil().max(1.0);
    let cell_count = cells(hi.0 - lo.0) * cells(hi.1 - lo.1);
    if cell_count > remaining as f64 {
        return Err(bad(c, format!("this `radius` would need over {MAX_DETECTORS} positions over the room: ask for a bigger one")));
    }
    let edge_count = region.holes.iter().fold(region.outer.len(), |n, hole| n.saturating_add(hole.len()));
    if (cell_count as usize).saturating_mul(edge_count) > 100_000 {
        return Err(bad(c, "the room exceeds the coverage grid limit; divide the area into smaller rooms"));
    }
    let mut points = region.cover_where(reach, &clear);
    if points.is_empty() {
        let message = if rectangle_margin.is_some() {
            "the grid found no detector position meeting the wall clearance"
        } else {
            "the grid found no detector position meeting wall and beam clearances"
        };
        return Err(bad(c, message));
    }
    region.complete_visible_cover(&mut points, reach, rectangle_margin, remaining, clear, world).map_err(|e| bad(c, e))?;
    Ok(points)
}

fn resat(s: &mut Session, p: &Value) -> R {
    resat_impl(s, p, false)
}

pub(crate) fn resat_impl(s: &mut Session, p: &Value, adapting_link: bool) -> R {
    use crate::family::Category::{HeatDetector, SmokeDetector};

    let c = "resat";
    only(c, p, &["room", "ids", "id", "kind", "radius", "margin", "space", "architecture", "preview", "heat"])?;
    let preview = flag(c, p, "preview")?;
    let m = &s.model;
    let room = element_id(c, m, p.get("room").ok_or_else(|| bad(c, "`room` is required"))?)?;
    let architecture = detector_architecture(c, p, m)?;
    let geometry = architecture.as_ref().unwrap_or(m);
    let (r, region) = detector_room(c, geometry, room)?;
    let space = hidden_detector_space(c, p, geometry)?;
    let kind = match string(c, p, "kind")?.as_deref() {
        None => None,
        Some("smoke") => Some(SmokeDetector),
        Some("heat") => Some(HeatDetector),
        Some(k) => return Err(bad(c, format!("`kind` is smoke or heat, not `{k}`"))),
    };
    let category = |component: &Component| m.families.get(&component.family).map(|f| f.category);
    let ids = if p.get("ids").is_some() || p.get("id").is_some() {
        ids_param(c, m, p)?
    } else {
        if space.is_some() || architecture.is_some() {
            return Err(bad(c, "give the detector ids for the selected hidden space or linked architecture"));
        }
        m.elements
            .iter()
            .filter_map(|(id, e)| {
                let Element::Component(device) = e else { return None };
                (device.level == r.level
                    && device.is_placed()
                    && device.detector_space.is_none()
                    && region.contains(device.at)
                    && category(device).is_some_and(|k| matches!(k, SmokeDetector | HeatDetector) && kind.is_none_or(|want| want == k)))
                .then_some(*id)
            })
            .take(MAX_DETECTORS + 1)
            .collect()
    };
    if ids.len() > MAX_DETECTORS {
        return Err(bad(c, format!("select no more than {MAX_DETECTORS} detectors")));
    }
    let mut selected = Vec::with_capacity(ids.len());
    let mut kind = kind;
    for &id in &ids {
        let Some(Element::Component(device)) = m.elements.get(id) else { return Err(bad(c, format!("{id} isn't a detector"))) };
        let family = m.families.get(&device.family).ok_or_else(|| bad(c, format!("{id} has no family")))?;
        if !matches!(family.category, SmokeDetector | HeatDetector) {
            return Err(bad(c, format!("{id} isn't a smoke or heat detector")));
        }
        if device.level != r.level || !device.is_placed() || device.mount.is_some() {
            return Err(bad(c, format!("{id} must be a placed, unmounted detector on the room's level")));
        }
        if device.detector_space != space.as_ref().map(|space| space.kind) {
            return Err(bad(c, format!("{id} belongs to a different detector space")));
        }
        if let Some(space) = &space {
            space.check_fit(c, m, device)?;
        }
        if *kind.get_or_insert(family.category) != family.category {
            return Err(bad(c, "select detectors of the same kind"));
        }
        selected.push((id, device));
    }
    if ids.is_empty() && !preview {
        if num(c, p, "radius")?.is_some_and(|radius| radius <= 0.0) {
            return Err(bad(c, "`radius` is more than 0"));
        }
        num(c, p, "margin")?;
        return Ok(json!({"ids": [], "changed": [], "package": m.fire_standard}));
    }
    if !absent(p.get("heat"))
        && selected.windows(2).any(|pair| pair[0].1.family != pair[1].1.family || pair[0].1.type_name != pair[1].1.type_name)
    {
        return Err(bad(c, "apply the heat evidence to detectors of a single family and type"));
    }
    let limits = kind.and_then(detector_limits).ok_or_else(|| bad(c, "select smoke or heat detectors"))?;
    let mut points = detector_grid(c, p, geometry, r, &region, limits, space.as_ref())?;
    let proposal = if preview || adapting_link {
        let status = if points.len() == ids.len() { "ready" } else { "count_mismatch" };
        let targets: Vec<_> = points.iter().map(|(at, contact_offset)| json!({"at": at, "contact_offset": contact_offset})).collect();
        let report = json!({"ids": ids, "status": status, "targets": targets, "package": m.fire_standard});
        if preview || status != "ready" {
            return Ok(report);
        }
        Some(report)
    } else {
        None
    };
    if points.len() != ids.len() {
        return Err(bad(c, format!("the room's grid needs {} positions but {} detectors were selected", points.len(), ids.len())));
    }
    let order = |a: P2, b: P2| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1));
    selected.sort_by(|(a_id, a), (b_id, b)| order(a.at, b.at).then(a_id.cmp(b_id)));
    points.sort_by(|a, b| order(a.0, b.0));
    let changes: Vec<_> = selected
        .into_iter()
        .zip(points)
        .filter_map(|((id, device), (at, ceiling))| {
            let offset = ceiling.map_or(device.offset, |height| height - m.component_contact_height(device, "bottom"));
            (device.at != at || device.offset != offset).then(|| (id, Component { at, offset, ..device.clone() }))
        })
        .collect();
    let changed: Vec<_> = changes.iter().map(|(id, _)| *id).collect();
    for (id, device) in changes {
        s.model_mut().elements.insert(id, Element::Component(device));
    }
    let mut result = proposal.unwrap_or_else(|| json!({"ids": ids, "package": s.model.fire_standard}));
    result["changed"] = json!(changed);
    Ok(result)
}

pub(crate) const FIRE_ZONE: Command = edit(
    "fire_zone",
    "Fire Zone",
    &[],
    "{id?:id|GUID, zone?:{name, kind:detection|notification, level:id|GUID, boundary:[[x,y],…], base_offset?, height, space?:plenum|underfloor, unit?:dwelling|apartment|hotel_room, include_rooms?:[id|GUID,…], exclude_rooms?:[id|GUID,…], notify?:[id|GUID,…]}, remote_indicators?:[{detector:id|GUID, connected?:bool, visible?:bool},…], room_group?:{same_floor?, same_fire_compartment?, isolated_rooms?:[id|GUID,…], common_exit?, max_exit_gap?, total_exit_gap?, vestibules?:[{room:id|GUID,depth,narrowest_door},…]}, dedicated_rooms?:[room id|GUID,…]} create or replace a fire zone; omit zone to inspect its equipment and detection checks or list zones. For inspection, remote_indicators supplies field evidence for 6.2.13 in hidden zones; room_group supplies general 6.3.4 evidence for main zones, with exit gaps and vestibule measurements in mm. Use dedicated_rooms to compare the projection with a designer-confirmed group requiring its own zone under 6.3.3. Set unit only for all rooms of one confirmed unit. Include its other levels explicitly by room; base_offset and height apply relative to each level",
    fire_zone,
);
pub(crate) const FIRE_STANDARD: Command = edit(
    "fire_standard",
    "Fire Standard",
    &[],
    "{set?} reads the project's fire rules edition or explicitly selects a supported edition. Supported: ru_sp484_2020_a1_sp486_2020_a1",
    fire_standard,
);
pub(crate) const DETECTOR_CEILING_CHECK: Command = other(
    "detector_ceiling_check",
    "Check Perforated Ceiling",
    "{open_percent?, min_opening?, thickness?} checks SP 484 Amendment 1, 6.6.7 for one point detector above a suspended ceiling. +         Give the percentage of its projected coverage area open through the perforations, the smallest opening dimension in any +         section, and the perforation thickness (both mm). Returns the three checks and whether below-ceiling coverage may use this +         detector. Missing measurements are reported. Changes no elements",
    detector_ceiling_check,
);
pub(crate) const HIDDEN_FIRE_PROTECTION: Command = other(
    "hidden_fire_protection",
    "Check Hidden Space Fire Protection",
    "{space:plenum|underfloor, cable_load?:litres per metre, pipe_insulation?:none|ng|g1|g2|g3|g4, plenum_aup_scope?:bool, g2_cables_qualify?:bool, f5_off_egress?:bool} checks SP 486 Amendment 1, Table 2 item 10. Enter the cable load after excluding verified note 2 exemptions. Use the highest installed pipe insulation group. Confirm whether note 4 puts the plenum within AUP scope. For G2, confirm that cables are absent or all comply with note 2(a). Confirm F5 outside evacuation routes to show the UPA and SPS alternative. Changes no elements",
    hidden_fire_protection,
);
pub(crate) const DETECTORS: Command = edit(
    "detectors",
    ui_msg!("Lay Detectors"),
    &[],
    "{room, kind?: smoke|heat, family?, type?, radius?, margin?} lays point fire detectors over a room (SP 484): a square \
         grid with reach `radius` mm (maximum and default 6400 for smoke, 3500 for heat). SP 484 Amendment 1 limits room heights to \
         12000 for smoke and 9000 for heat. `margin` defaults to 500 mm from walls. `family` is by default the \
         project's first of the `kind`'s (smoke), `type` its first. Fits the detectors to the modeled ceiling underside; \
         without one, uses the next level or the default height to check the height limit. Covers each ceiling height \
         compartment separately. Above 6000 mm, heat detectors require `heat: {class, trip_c, normal_c, max_c, rated_normal_c?, rated_max_c?}` \
         (SP 484 Amendment 1, 6.2.3). `architecture?: id|GUID` adds the geometry of the selected loaded link. \
         Optional `space: {above: id|GUID, below: id|GUID}` uses the void above a ceiling or floor \
         and below a flat floor, clipped to the room in plan. Returns the new ids",
    detectors,
);
pub(crate) const RESAT: Command = edit(
    "resat",
    ui_msg!("Adapt Devices"),
    &[],
    "{room, ids?: [id|GUID], kind?: smoke|heat, radius?, margin?} repositions existing point detectors on the room's level \
         on the current coverage grid. Without ids, takes detectors inside the room (optionally filtered by kind). Requires \
         unmounted detectors of one kind and one per grid position. In each ceiling compartment, fits each detector to the \
         underside; without a ceiling, keeps its offset. For a hidden `space: {above, below}` or a selected \
         `architecture: id|GUID` link, supply the detector ids explicitly. For high heat layouts supply the same `heat` data as \
         `detectors` for a single detector family/type. `preview: true` returns candidate points and whether \
         their count matches the selection, with no changes. Applying keeps ids, GUIDs and connections. Returns {ids, changed}",
    resat,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireZoneKind {
    Detection,
    Notification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireZoneUnit {
    Dwelling,
    Apartment,
    HotelRoom,
}

/// A volume and room overrides for an alarm or notification zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FireZone {
    pub name: String,
    pub kind: FireZoneKind,
    pub level: Guid,
    pub boundary: Vec<P2>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub base_offset: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<DetectorSpace>,
    /// One complete dwelling, apartment or hotel room under SP 484, 6.3.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<FireZoneUnit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_rooms: Vec<Guid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_rooms: Vec<Guid>,
    /// Notification zones activated by a fire event in this detection zone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify: Vec<Guid>,
}

pub(super) fn check_zone(model: &Model, who: &str, zone: &FireZone) -> Result<(), String> {
    model.level(zone.level).ok_or_else(|| format!("{who}: missing zone level"))?;
    text(who, "name", &zone.name)?;
    if zone.name.trim().is_empty() {
        return Err(format!("{who}: name the fire zone"));
    }
    coord(&format!("{who} base offset"), zone.base_offset)?;
    size(&format!("{who} height"), zone.height)?;
    coord(&format!("{who} top offset"), zone.base_offset + zone.height)?;
    boundary(who, &zone.boundary)?;
    if zone.boundary.len() > 512 {
        return Err(format!("{who}: a fire zone boundary has at most 512 points"));
    }
    if crate::sketch::solve(&crate::sketch::lines_of(&zone.boundary, &[])).is_err() {
        return Err(format!("{who}: give a simple fire zone boundary"));
    }
    if zone.unit.is_some() && (zone.kind != FireZoneKind::Detection || zone.space.is_some()) {
        return Err(format!("{who}: a whole unit needs a main detection zone"));
    }
    let mut rooms = BTreeSet::new();
    for room in zone.include_rooms.iter().chain(&zone.exclude_rooms) {
        if !rooms.insert(room)
            || !matches!(model.elements.get(*room), Some(Element::Room(r)) if r.level == zone.level || zone.unit.is_some())
        {
            return Err(format!("{who}: name each room once, on the zone's level unless it covers one whole unit"));
        }
    }
    if rooms.len() > 512 || zone.notify.len() > 512 {
        return Err(format!("{who}: at most 512 room overrides and 512 notification links"));
    }
    if zone.kind == FireZoneKind::Notification && (zone.space.is_some() || !zone.notify.is_empty()) {
        return Err(format!("{who}: only detection zones can specify a hidden space or trigger notification"));
    }
    let mut targets = BTreeSet::new();
    for g in &zone.notify {
        if !targets.insert(g)
            || !matches!(model.elements.get(*g), Some(Element::FireZone(target)) if target.kind == FireZoneKind::Notification)
        {
            return Err(format!("{who}: fire events must target distinct notification zones"));
        }
    }
    Ok(())
}

macro_rules! visit_zone {
    ($zone:expr, $f:expr, $($m:tt)?) => {{
        let zone = $zone;
        let f = $f;
                f(& $($m)? zone.level);
                for g in & $($m)? zone.include_rooms { f(g); }
                for g in & $($m)? zone.exclude_rooms { f(g); }
                for g in & $($m)? zone.notify { f(g); }
    }};
}
pub(crate) use visit_zone;
