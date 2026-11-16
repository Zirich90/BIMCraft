//! Spatial membership and fire-to-notification links for low-voltage systems.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::family::Category;
use crate::geom::{self, P2, Region};
use crate::model::{Element, FireZone, FireZoneKind, Guid, Id, Model, TOL};
use crate::rooms::Rooms;

struct Projection<'a> {
    boundary: &'a [P2],
    included: Vec<Region>,
    excluded: Vec<Region>,
}

impl<'a> Projection<'a> {
    fn new(rooms: &Rooms<'_>, model: &Model, zone: &'a FireZone, unresolved: &mut Vec<Guid>) -> Self {
        Self::on_level(rooms, model, zone, zone.level, unresolved)
    }

    fn on_level(rooms: &Rooms<'_>, model: &Model, zone: &'a FireZone, level: Guid, unresolved: &mut Vec<Guid>) -> Self {
        let mut regions = |ids: &[Guid]| {
            ids.iter()
                .filter_map(|&guid| {
                    let region = match model.elements.get(guid) {
                        Some(Element::Room(room)) if room.level != level => return None,
                        Some(Element::Room(room)) => rooms.region(room),
                        _ => None,
                    };
                    if region.is_none() {
                        unresolved.push(guid);
                    }
                    region
                })
                .collect::<Vec<_>>()
        };
        let boundary = if level == zone.level { zone.boundary.as_slice() } else { &[] };
        Self { boundary, included: regions(&zone.include_rooms), excluded: regions(&zone.exclude_rooms) }
    }

    fn contains(&self, at: P2) -> bool {
        (geom::contains(self.boundary, at) || self.included.iter().any(|room| room.contains(at)))
            && !self.excluded.iter().any(|room| room.contains(at))
    }

    fn rings(&self) -> impl Iterator<Item = &[P2]> {
        std::iter::once(self.boundary).chain(
            self.included
                .iter()
                .chain(&self.excluded)
                .flat_map(|room| std::iter::once(room.outer.as_slice()).chain(room.holes.iter().map(Vec::as_slice))),
        )
    }

    fn area(&self) -> Result<f64, &'static str> {
        if self.included.is_empty() && self.excluded.is_empty() {
            return Ok(geom::area(self.boundary).abs());
        }
        area_where(self.rings(), |at| self.contains(at))
    }
}

fn area_where<'a>(rings: impl Iterator<Item = &'a [P2]>, contains: impl Fn(P2) -> bool) -> Result<f64, &'static str> {
    Ok(regions_where(rings, contains)?.iter().map(Region::area).sum())
}

fn regions_where<'a>(rings: impl Iterator<Item = &'a [P2]>, contains: impl Fn(P2) -> bool) -> Result<Vec<Region>, &'static str> {
    let cuts: Vec<_> = rings.flat_map(geom::ring_edges).take(509).collect();
    if cuts.len() > 508 {
        return Err("too many edges to assess the effective zone projection");
    }
    let (lo, hi) = cuts.iter().fold((P2(f64::INFINITY, f64::INFINITY), P2(f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), (p, _)| {
        (P2(lo.0.min(p.0), lo.1.min(p.1)), P2(hi.0.max(p.0), hi.1.max(p.1)))
    });
    let envelope = Region { outer: vec![lo, P2(hi.0, lo.1), hi, P2(lo.0, hi.1)], holes: Vec::new() };
    let parts = envelope.partition_by_height(&cuts, |at| contains(at).then_some(0.0))?;
    Ok(parts.into_iter().filter_map(|(label, region)| label.map(|_| region)).collect())
}

pub(crate) fn on_additional_level(model: &Model, zone: &FireZone, level: Guid) -> bool {
    zone.unit.is_some()
        && zone.level != level
        && zone.include_rooms.iter().any(|room| matches!(model.elements.get(*room), Some(Element::Room(room)) if room.level == level))
}

pub(crate) fn additional_plan_regions(model: &Model, rooms: &Rooms<'_>, zone: &FireZone, level: Guid) -> Vec<Region> {
    let mut unresolved = Vec::new();
    let projection = Projection::on_level(rooms, model, zone, level, &mut unresolved);
    if !unresolved.is_empty() || projection.included.is_empty() {
        return Vec::new();
    }
    if projection.included.len() == 1 && projection.excluded.is_empty() {
        return projection.included;
    }
    regions_where(projection.rings(), |at| projection.contains(at)).unwrap_or_default()
}

fn hidden_grouping(model: &Model, rooms: &Rooms<'_>, zone: &FireZone, hidden: &Projection<'_>, area: f64) -> Value {
    // Check the vertical projection path of clause 6.2.13.
    let mut candidates = model.rare("fire_zone").into_iter().filter_map(|id| {
        let Element::FireZone(main) = model.elements.get(id)? else { return None };
        (main.kind == FireZoneKind::Detection && main.space.is_none() && zone_levels(model, main).contains(&zone.level))
            .then_some((id, main))
    });
    let mut matched = Vec::new();
    if area > 0.0 {
        for (id, main) in candidates.by_ref().take(128) {
            let mut unresolved = Vec::new();
            let main = Projection::on_level(rooms, model, main, zone.level, &mut unresolved);
            if !unresolved.is_empty() {
                continue;
            }
            if area_where(hidden.rings().chain(main.rings()), |at| hidden.contains(at) != main.contains(at))
                .is_ok_and(|difference| difference < 1.0)
            {
                matched.push(id);
            }
        }
        if candidates.next().is_some() {
            return json!({"clause":"6.2.13", "status":"insufficient_data", "reason":"Too many main zones to verify the hidden zone projection"});
        }
    }
    match matched.as_slice() {
        [id] => json!({"clause":"6.2.13", "status":"pass", "main_zone":id}),
        [] => {
            json!({"clause":"6.2.13", "status":"insufficient_data", "reason":"Supply a matching main zone projection or evidence of connected visible remote indicators"})
        }
        _ => json!({"clause":"6.2.13", "status":"ambiguous", "main_zones":matched}),
    }
}

pub(crate) fn dedicated_projection(model: &Model, id: Id, zone: &FireZone, dedicated: &[Guid]) -> Result<Value, String> {
    let rooms = Rooms::new(model);
    let mut required = BTreeMap::<Guid, Vec<Region>>::new();
    let mut unresolved = Vec::new();
    for &guid in dedicated {
        let Some(Element::Room(room)) = model.elements.get(guid) else { return Err(format!("{guid} is not a room")) };
        if let Some(region) = rooms.region(room) {
            required.entry(room.level).or_default().push(region);
        } else {
            unresolved.push(guid);
        }
    }
    let actual_levels = zone_levels(model, zone);
    let mut levels = actual_levels.clone();
    levels.extend(required.keys().copied());
    if levels.len() > 32 {
        return Ok(json!({"clause":"6.3.3","status":"insufficient_data","reason":"Too many levels to verify the dedicated projection"}));
    }
    let neighbors: Vec<_> = model
        .rare("fire_zone")
        .into_iter()
        .filter(|&other| other != id)
        .filter_map(|other| {
            let Element::FireZone(candidate) = model.elements.get(other)? else { return None };
            let bottom = zone.base_offset.max(candidate.base_offset);
            let top = (zone.base_offset + zone.height).min(candidate.base_offset + candidate.height);
            if candidate.kind != FireZoneKind::Detection || candidate.space.is_some() || top - bottom <= TOL {
                return None;
            }
            let shared: Vec<_> = zone_levels(model, candidate).intersection(&actual_levels).copied().collect();
            (!shared.is_empty()).then_some((other, candidate, shared))
        })
        .take(129)
        .collect();
    if neighbors.len() > 128 {
        return Ok(
            json!({"clause":"6.3.3","status":"insufficient_data","reason":"Too many nearby zones to verify the dedicated projection"}),
        );
    }
    let mut results = Vec::new();
    let mut mismatch = false;
    let mut incomplete = false;
    for level in levels {
        let projection = Projection::on_level(&rooms, model, zone, level, &mut unresolved);
        let expected = required.get(&level).map_or(&[][..], Vec::as_slice);
        let rings = || {
            projection
                .rings()
                .chain(expected.iter().flat_map(|room| std::iter::once(room.outer.as_slice()).chain(room.holes.iter().map(Vec::as_slice))))
        };
        let area = |extra| {
            area_where(rings(), |at| {
                let inside = projection.contains(at);
                let expected = expected.iter().any(|room| room.contains(at));
                if extra { inside && !expected } else { expected && !inside }
            })
        };
        match (area(true), area(false)) {
            (Ok(extra), Ok(missing)) => {
                mismatch |= extra >= 1.0 || missing >= 1.0;
                let mut overlaps = Vec::new();
                let mut unverified = Vec::new();
                for (other, candidate, shared) in &neighbors {
                    if !shared.contains(&level) {
                        continue;
                    }
                    let neighbor = Projection::on_level(&rooms, model, candidate, level, &mut unresolved);
                    match area_where(projection.rings().chain(neighbor.rings()), |at| projection.contains(at) && neighbor.contains(at)) {
                        Ok(area) if area >= 1.0 => overlaps.push(json!({"id":other,"area_m2":area/1_000_000.0})),
                        Ok(_) => {}
                        Err(reason) => unverified.push(json!({"id":other,"reason":reason})),
                    }
                }
                mismatch |= !overlaps.is_empty();
                incomplete |= !unverified.is_empty();
                results.push(json!({"level":level,"extra_m2":extra/1_000_000.0,"missing_m2":missing/1_000_000.0,
                    "overlapping_zones":overlaps,"unverified_zones":unverified}));
            }
            (Err(reason), _) | (_, Err(reason)) => {
                incomplete = true;
                results.push(json!({"level":level,"reason":reason}));
            }
        }
    }
    let status = if incomplete || !unresolved.is_empty() {
        "insufficient_data"
    } else if mismatch {
        "fail"
    } else {
        "pass"
    };
    Ok(json!({"clause":"6.3.3","status":status,"levels":results,"unresolved_rooms":unresolved}))
}

fn projected_rooms(model: &Model, rooms: &Rooms<'_>, zone: &FireZone, projection: &Projection<'_>) -> Result<Vec<Id>, &'static str> {
    let Some((lo, hi)) = projection
        .rings()
        .filter_map(geom::bounds)
        .reduce(|(lo, hi), (a, b)| (P2(lo.0.min(a.0), lo.1.min(a.1)), P2(hi.0.max(b.0), hi.1.max(b.1))))
    else {
        return Ok(Vec::new());
    };
    let lookup = model.lookup();
    let mut candidates = lookup.referrers(&zone.level).filter_map(|id| {
        let Element::Room(room) = model.elements.get(id)? else { return None };
        (room.level == zone.level && room.base_offset < zone.base_offset + zone.height).then_some((id, room))
    });
    let mut ids = Vec::new();
    for (id, room) in candidates.by_ref().take(256) {
        let Some(region) = rooms.region(room) else {
            if projection.contains(room.point) {
                return Err("a room in the zone has an unresolved boundary");
            }
            continue;
        };
        let Some((a, b)) = geom::bounds(&region.outer) else { continue };
        if a.0 >= hi.0 || a.1 >= hi.1 || b.0 <= lo.0 || b.1 <= lo.1 {
            continue;
        }
        let rings = projection.rings().chain(std::iter::once(region.outer.as_slice())).chain(region.holes.iter().map(Vec::as_slice));
        if area_where(rings, |at| projection.contains(at) && region.contains(at))? > 1.0 {
            ids.push(id);
        }
    }
    if candidates.next().is_some() {
        return Err("too many rooms on the level to assess zone membership");
    }
    ids.sort_unstable();
    Ok(ids)
}

fn assessment(model: &Model, rooms: &Rooms<'_>, zone: &FireZone, projection: &Projection<'_>, unresolved: &[Guid]) -> Value {
    let area = if unresolved.is_empty() { projection.area() } else { Err("room override boundaries could not be resolved") };
    let mut report = match area {
        Ok(area) => {
            let area_m2 = area / 1_000_000.0;
            let status = if area_m2 > 2000.0 {
                "limit_exceeded"
            } else if area_m2 <= 0.0 {
                "empty"
            } else {
                "pass"
            };
            json!({"area_m2":area_m2, "area":{"clause":"6.3.4", "status":status, "limit_m2":2000.0}})
        }
        Err(reason) => json!({"area":{"clause":"6.3.4", "status":"insufficient_data", "reason":reason}}),
    };
    if zone.space.is_some() {
        report["hidden_grouping"] = match area {
            Ok(area) => hidden_grouping(model, rooms, zone, projection, area),
            Err(reason) => json!({"clause":"6.2.13", "status":"insufficient_data", "reason":reason}),
        };
    } else {
        report["rooms"] = match area.and_then(|_| projected_rooms(model, rooms, zone, projection)) {
            Ok(ids) => {
                let status = if ids.len() > 5 { "limit_exceeded" } else { "pass" };
                json!({"clause":"6.3.4", "status":status, "limit":5, "ids":ids})
            }
            Err(reason) => json!({"clause":"6.3.4", "status":"insufficient_data", "reason":reason}),
        };
    }
    report
}

fn zone_levels(model: &Model, zone: &FireZone) -> BTreeSet<Guid> {
    if zone.unit.is_none() {
        return BTreeSet::from([zone.level]);
    }
    std::iter::once(zone.level)
        .chain(zone.include_rooms.iter().filter_map(|room| {
            let Element::Room(room) = model.elements.get(*room)? else { return None };
            Some(room.level)
        }))
        .collect()
}

fn members(model: &Model, zone: &FireZone, level: Guid, projection: &Projection<'_>, candidates: impl Iterator<Item = Id>) -> Vec<Id> {
    candidates
        .filter(|&id| {
            let Some(Element::Component(device)) = model.elements.get(id) else { return false };
            if device.level != level || !device.is_placed() || device.detector_space != zone.space {
                return false;
            }
            let Some(family) = model.families.get(&device.family) else { return false };
            let eligible = match zone.kind {
                FireZoneKind::Detection => matches!(family.category, Category::SmokeDetector | Category::HeatDetector),
                FireZoneKind::Notification => matches!(family.category, Category::Sounder | Category::ExitSign | Category::Speaker),
            };
            if !eligible {
                return false;
            }
            let face = match family.category {
                Category::SmokeDetector | Category::HeatDetector | Category::Speaker => "bottom",
                _ => "interior",
            };
            let contact = model.component_contact_height(device, face);
            let z = device.offset + contact;
            (zone.base_offset - TOL..=zone.base_offset + zone.height + TOL).contains(&z) && projection.contains(device.at)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn detection_memberships(model: &Model, levels: &BTreeSet<Guid>, detectors: &[Id]) -> Option<BTreeMap<Id, Vec<Id>>> {
    let rooms = Rooms::new(model);
    let mut by_device = BTreeMap::<Id, Vec<Id>>::new();
    let mut zones = model.rare("fire_zone").into_iter().filter_map(|id| {
        let Element::FireZone(zone) = model.elements.get(id)? else { return None };
        if zone.kind != FireZoneKind::Detection {
            return None;
        }
        let shared: Vec<_> = zone_levels(model, zone).intersection(levels).copied().collect();
        (!shared.is_empty()).then_some((id, zone, shared))
    });
    for (id, zone, shared) in zones.by_ref().take(256) {
        for level in shared {
            let mut unresolved = Vec::new();
            let projection = Projection::on_level(&rooms, model, zone, level, &mut unresolved);
            if !unresolved.is_empty() {
                return None;
            }
            for device in members(model, zone, level, &projection, detectors.iter().copied()) {
                by_device.entry(device).or_default().push(id);
            }
        }
    }
    zones.next().is_none().then_some(by_device)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoomGroupEvidence {
    same_floor: Option<bool>,
    same_fire_compartment: Option<bool>,
    isolated_rooms: Option<Vec<Guid>>,
    common_exit: Option<bool>,
    max_exit_gap: Option<f64>,
    total_exit_gap: Option<f64>,
    #[serde(default)]
    vestibules: Vec<VestibuleEvidence>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VestibuleEvidence {
    room: Guid,
    depth: f64,
    narrowest_door: f64,
}

impl RoomGroupEvidence {
    fn assess(&self, model: &Model, rooms: &Rooms<'_>, projection: &Projection<'_>, assessment: &Value) -> Result<Value, String> {
        for (key, value) in [("max_exit_gap", self.max_exit_gap), ("total_exit_gap", self.total_exit_gap)] {
            if value.is_some_and(|value| !(0.0..=crate::model::MAX_COORD).contains(&value)) {
                return Err(format!("`{key}` must be a nonnegative distance in mm"));
            }
        }
        if self.max_exit_gap.zip(self.total_exit_gap).is_some_and(|(max, total)| max > total) {
            return Err("the total exit gap cannot be smaller than the maximum gap".into());
        }
        let projected = assessment["rooms"]["ids"].as_array();
        let mut seen_vestibules = BTreeSet::new();
        let mut excluded_vestibules = BTreeMap::new();
        for evidence in &self.vestibules {
            let Some(id) = model.elements.id(evidence.room) else { return Err(format!("unknown vestibule {}", evidence.room)) };
            if !seen_vestibules.insert(id) {
                return Err(format!("give vestibule {id} only once"));
            }
            if [evidence.depth, evidence.narrowest_door]
                .into_iter()
                .any(|value| !(0.0..=crate::model::MAX_COORD).contains(&value) || value == 0.0)
            {
                return Err(format!("vestibule {id} needs positive depth and door width in mm"));
            }
            let Some(Element::Room(room)) = model.elements.get(id) else { return Err(format!("{id} is not a vestibule room")) };
            if projected.is_some_and(|ids| !ids.iter().any(|value| value.as_u64() == Some(id))) {
                return Err(format!("vestibule {id} is outside this zone's room group"));
            }
            if evidence.depth < evidence.narrowest_door
                && let Some(region) = rooms.region(room)
            {
                excluded_vestibules.insert(id, region);
            }
        }
        let counted_rooms = projected.map(|ids| ids.len().saturating_sub(excluded_vestibules.len()));
        let counted_area = assessment["area_m2"].as_f64().and_then(|area| {
            if excluded_vestibules.is_empty() {
                return Some(area);
            }
            projected?;
            let rings = excluded_vestibules
                .values()
                .flat_map(|region| std::iter::once(region.outer.as_slice()).chain(region.holes.iter().map(Vec::as_slice)));
            area_where(projection.rings().chain(rings), |at| {
                projection.contains(at) && !excluded_vestibules.values().any(|region| region.contains(at))
            })
            .ok()
            .map(|area| area / 1_000_000.0)
        });
        let mut area = None;
        if let Some(isolated) = &self.isolated_rooms {
            let mut seen = BTreeSet::new();
            let mut total = 0.0;
            for &guid in isolated {
                let Some(id) = model.elements.id(guid) else { return Err(format!("unknown isolated room {guid}")) };
                if !seen.insert(id) {
                    return Err(format!("give isolated room {id} only once"));
                }
                if projected.is_some_and(|ids| !ids.iter().any(|value| value.as_u64() == Some(id))) {
                    return Err(format!("isolated room {id} is outside this zone's room group"));
                }
                let Some(Element::Room(room)) = model.elements.get(id) else { return Err(format!("{id} is not a room")) };
                if let Some(region) = rooms.region(room) {
                    total += region.area();
                }
            }
            if projected.is_some() {
                area = Some(total / 1_000_000.0);
            }
        }
        let isolated = self.isolated_rooms.as_deref();
        let exit_required = isolated.map(|ids| !ids.is_empty());
        let gap_required = isolated.map(|ids| ids.len() > 1);
        let check = |required, result| match required {
            Some(false) => "not_applicable",
            _ => match result {
                Some(true) => "pass",
                Some(false) => "fail",
                None => "insufficient_data",
            },
        };
        let checks = [
            (
                "zone_area",
                match counted_area {
                    Some(_) if assessment["area"]["status"] == "empty" => "empty",
                    Some(area) if area > 2000.0 => "limit_exceeded",
                    Some(_) => "pass",
                    None => "insufficient_data",
                },
            ),
            (
                "room_count",
                match counted_rooms {
                    Some(count) if count > 5 => "limit_exceeded",
                    Some(_) => "pass",
                    None => "insufficient_data",
                },
            ),
            ("same_floor", check(Some(true), self.same_floor)),
            ("same_fire_compartment", check(Some(true), self.same_fire_compartment)),
            ("isolated_area", check(Some(true), area.map(|area| area <= 500.0))),
            ("common_exit", check(exit_required, self.common_exit)),
            ("max_exit_gap", check(gap_required, self.max_exit_gap.map(|gap| gap <= 10000.0))),
            ("total_exit_gap", check(gap_required, self.total_exit_gap.map(|gap| gap <= 50000.0))),
        ];
        let status = if checks.iter().any(|(_, status)| matches!(*status, "fail" | "limit_exceeded" | "empty")) {
            "fail"
        } else if projected.is_none_or(Vec::is_empty) || checks.iter().any(|(_, status)| *status == "insufficient_data") {
            "insufficient_data"
        } else {
            "pass"
        };
        Ok(json!({"clause":"6.3.4","status":status,"isolated_area_m2":area,
            "counted_area_m2":counted_area,"counted_rooms":counted_rooms,
            "excluded_vestibules":excluded_vestibules.keys().collect::<Vec<_>>(),
            "checks":checks.into_iter().collect::<BTreeMap<_, _>>()}))
    }
}

pub(crate) fn report(
    model: &Model,
    id: Id,
    zone: &FireZone,
    remote: Option<BTreeMap<Id, &'static str>>,
    group: Option<RoomGroupEvidence>,
) -> Result<Value, String> {
    let rooms = Rooms::new(model);
    let mut unresolved = Vec::new();
    let projection = Projection::new(&rooms, model, zone, &mut unresolved);
    let lookup = model.lookup();
    let levels = zone_levels(model, zone);
    let mut devices = members(model, zone, zone.level, &projection, lookup.referrers(&zone.level));
    for &level in levels.iter().filter(|&&level| level != zone.level) {
        let projection = Projection::on_level(&rooms, model, zone, level, &mut unresolved);
        devices.extend(members(model, zone, level, &projection, lookup.referrers(&level)));
    }
    devices.sort_unstable();
    let status = if unresolved.is_empty() { "ready" } else { "insufficient_data" };
    let mut report =
        json!({"id":id, "guid":model.guid(id), "zone":zone, "status":status, "devices":devices, "unresolved_rooms":unresolved});
    if let Some(unit) = zone.unit {
        report["assessment"]["room_group"] = json!({
            "clause":"6.3.4", "status":if unresolved.is_empty() { "unit_exception" } else { "insufficient_data" },
            "unit":unit, "levels":levels
        });
    } else if zone.kind == FireZoneKind::Detection {
        report["assessment"] = assessment(model, &rooms, zone, &projection, &unresolved);
    }
    if let Some(group) = group {
        let mut result = group.assess(model, &rooms, &projection, &report["assessment"])?;
        if zone.include_rooms.iter().any(|id| matches!(model.elements.get(id), Some(Element::Room(room)) if room.level != zone.level)) {
            result["checks"]["same_floor"] = json!("fail");
            result["status"] = json!("fail");
        }
        report["assessment"]["room_group"] = result;
    }
    if let Some(remote) = remote {
        if let Some(id) = remote.keys().find(|id| devices.binary_search(id).is_err()) {
            return Err(format!("detector {id} is not a member of this hidden zone"));
        }
        let mut missing = Vec::new();
        let mut failed = Vec::new();
        for &id in &devices {
            match remote.get(&id).copied() {
                Some("pass") => {}
                Some("fail") => failed.push(id),
                _ => missing.push(id),
            }
        }
        let status = if !failed.is_empty() {
            "fail"
        } else if devices.is_empty() || !missing.is_empty() || !unresolved.is_empty() {
            "insufficient_data"
        } else {
            "pass"
        };
        let projected = report["assessment"]["hidden_grouping"]["status"] == "pass";
        report["assessment"]["hidden_indication"] = json!({
            "clause":"6.2.13", "status":status, "missing_detectors":missing, "failed_detectors":failed
        });
        report["assessment"]["hidden_compliance"] = if projected || status == "pass" { json!("pass") } else { json!("unverified") };
    }
    Ok(report)
}
