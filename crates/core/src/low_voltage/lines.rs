use crate::commands::*;
use crate::engine::{Command, Session};
use crate::geom::P2;
use crate::library::Kind as LibKind;
use crate::model::IfcSlab;
use crate::model::text;
use crate::model::{Element, Guid, Id, MAX_COORD, MAX_POINTS, Model, TOL};
use crate::view3d;
use bimcraft_i18n::ui_msg;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn electrical_terminal(s: &mut Session, p: &Value) -> R {
    let c = "electrical_terminal";
    only(c, p, &["id", "device", "name", "purpose", "address", "load_ma", "limit_ma"])?;
    let name = string(c, p, "name")?;
    let purpose = string(c, p, "purpose")?;
    let address = string(c, p, "address")?;
    let current = |key| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|n| (0.0..=MAX_COORD).contains(n))
            .map(Some)
            .ok_or_else(|| bad(c, format!("{key} must be nonnegative milliamps or null"))),
    };
    let load_ma = current("load_ma")?;
    let limit_ma = current("limit_ma")?;
    let id = if let Some(id) = p.get("id") {
        if p.get("device").is_some() {
            return Err(bad(c, "terminal device is fixed"));
        }
        let id = element_id(c, &s.model, id)?;
        let Some(Element::ElectricalTerminal(t)) = s.model_mut().elements.get_mut(id) else {
            return Err(bad(c, "select an electrical terminal"));
        };
        if let Some(name) = name {
            t.name = name;
        }
        if let Some(purpose) = purpose {
            t.purpose = purpose;
        }
        if let Some(address) = address {
            t.address = address;
        }
        if p.get("load_ma").is_some() {
            t.load_ma = load_ma;
        }
        if p.get("limit_ma").is_some() {
            t.limit_ma = limit_ma;
        }
        id
    } else {
        let device = s.model.guid(element_id(c, &s.model, &p["device"])?);
        let name = name.ok_or_else(|| bad(c, "name is required"))?;
        s.model_mut().insert(Element::ElectricalTerminal(crate::model::ElectricalTerminal {
            device,
            name,
            purpose: purpose.unwrap_or_default(),
            address: address.unwrap_or_default(),
            load_ma,
            limit_ma,
        }))
    };
    Ok(json!({"id":id}))
}

fn electrical_branches(c: &str, model: &Model, value: &Value) -> Result<Vec<crate::model::ElectricalBranch>, String> {
    let branches =
        value.as_array().filter(|entries| entries.len() <= MAX_POINTS).ok_or_else(|| bad(c, "branches must be a bounded array"))?;
    let mut remaining = MAX_POINTS;
    branches
        .iter()
        .map(|branch| {
            only(c, branch, &["from", "devices"])?;
            let from = model.guid(element_id(c, model, &branch["from"])?);
            let devices = guids_param(c, model, &branch["devices"], "devices")?;
            remaining = remaining.checked_sub(devices.len()).ok_or_else(|| bad(c, "too many branch devices"))?;
            Ok(crate::model::ElectricalBranch { from, devices })
        })
        .collect()
}

fn electrical_line(s: &mut Session, p: &Value) -> R {
    let c = "electrical_line";
    only(c, p, &["source", "devices", "branches", "segments", "terminals", "system", "profile", "mark_prefix"])?;
    let source = s.model.guid(element_id(c, &s.model, &p["source"])?);
    let devices = guids_param(c, &s.model, &p["devices"], "devices")?;
    let segments = p.get("segments").map(|v| guids_param(c, &s.model, v, "segments")).transpose()?.unwrap_or_default();
    let segments = expand_route_segments(&s.model, segments);
    let mark_prefix = string(c, p, "mark_prefix")?.unwrap_or_default();
    let terminals = p.get("terminals").map(|v| guids_param(c, &s.model, v, "terminals")).transpose()?.unwrap_or_default();
    let branches = p.get("branches").map(|v| electrical_branches(c, &s.model, v)).transpose()?.unwrap_or_default();
    let system = string(c, p, "system")?.unwrap_or_default();
    let line = crate::model::ElectricalLine {
        source,
        system,
        profile: super::systems::param(&p["profile"]).map_err(|e| bad(c, e))?,
        devices,
        branches,
        segments,
        terminals,
        mark_prefix,
        locked_marks: Vec::new(),
        isolators: Vec::new(),
    };
    let id = s.model_mut().insert(Element::ElectricalLine(line));
    Ok(json!({"id":id}))
}

fn electrical_line_edit(s: &mut Session, p: &Value) -> R {
    let c = "electrical_line_edit";
    only(c, p, &["id", "devices", "branches", "segments", "terminals", "system", "profile", "mark_prefix"])?;
    let id = element_id(c, &s.model, &p["id"])?;
    let Some(Element::ElectricalLine(line)) = s.model.elements.get(id) else { return Err(bad(c, "select an electrical line")) };
    let mut line = line.clone();
    if let Some(v) = p.get("devices") {
        line.devices = guids_param(c, &s.model, v, "devices")?;
    }
    if let Some(v) = p.get("branches") {
        line.branches = electrical_branches(c, &s.model, v)?;
    }
    if p.get("devices").is_some() || p.get("branches").is_some() {
        let members: BTreeSet<_> = line.members().collect();
        line.locked_marks.retain(|entry| members.contains(&entry.device));
        line.terminals.retain(|g| matches!(s.model.elements.get(*g), Some(Element::ElectricalTerminal(t)) if members.contains(&t.device)));
    }
    if let Some(v) = p.get("segments") {
        line.segments = expand_route_segments(&s.model, guids_param(c, &s.model, v, "segments")?);
    }
    if let Some(v) = p.get("terminals") {
        line.terminals = guids_param(c, &s.model, v, "terminals")?;
    }
    if let Some(system) = string(c, p, "system")? {
        line.system = system;
    }
    if let Some(value) = p.get("profile") {
        line.profile = super::systems::param(value).map_err(|e| bad(c, e))?;
    }
    if let Some(prefix) = string(c, p, "mark_prefix")? {
        line.mark_prefix = prefix;
    }
    s.model_mut().elements.insert(id, Element::ElectricalLine(line));
    Ok(json!({"id":id}))
}

fn electrical_line_inspect(s: &mut Session, p: &Value) -> R {
    let c = "electrical_line_inspect";
    only(c, p, &["id"])?;
    let id = element_id(c, &s.model, &p["id"])?;
    let Some(Element::ElectricalLine(line)) = s.model.elements.get(id) else { return Err(bad(c, "select an electrical line")) };
    let ordered_devices: Vec<_> = line.order(&s.model).into_iter().filter_map(|guid| s.model.elements.id(guid)).collect();
    let terminal_checks: Vec<_> = line
        .terminals
        .iter()
        .filter_map(|guid| {
            let Element::ElectricalTerminal(t) = s.model.elements.get(*guid)? else { return None };
            Some(json!({"guid":guid,"address":t.address,"load_ma":t.load_ma,"limit_ma":t.limit_ma,
            "overload":t.load_ma.zip(t.limit_ma).map(|(load, limit)| load > limit)}))
        })
        .collect();
    Ok(
        json!({"id":id,"ordered_devices":ordered_devices,"system":line.system,"profile":line.system_profile(),"requires_okl":line.system_profile().map(super::systems::System::requires_okl),"branches":line.branches,"source":line.source,"devices":line.devices,"segments":line.segments,"isolators":line.isolators,"terminals":line.terminals,"terminal_checks":terminal_checks,"marks":line.marks(&s.model),"locked_marks":line.locked_marks}),
    )
}

fn fire_line_faults(s: &mut Session, p: &Value) -> R {
    let c = "fire_line_faults";
    only(c, p, &["id", "isolators", "source_isolates_ports", "propose", "allow_pair", "max_isolators", "power"])?;
    let m = &s.model;
    let id = element_id(c, m, &p["id"])?;
    let Some(Element::ElectricalLine(line)) = m.elements.get(id) else { return Err(bad(c, "select an electrical line")) };
    let extra = p.get("isolators").map(|v| guids_param(c, m, v, "isolators")).transpose()?.unwrap_or_default();
    let isolators = line.isolators.iter().copied().chain(extra).collect();
    let power = if let Some(power) = p.get("power") {
        only(c, power, &["id", "independent", "source_isolates_ports", "lines"])?;
        let parse_line = |entry: &Value| {
            let id = element_id(c, m, &entry["id"])?;
            let Some(Element::ElectricalLine(line)) = m.elements.get(id) else { return Err(bad(c, "select an electrical line for power")) };
            Ok((line, flag(c, entry, "source_isolates_ports")?))
        };
        let lines = if let Some(value) = power.get("lines") {
            if power.get("id").is_some() || power.get("source_isolates_ports").is_some() {
                return Err(bad(c, "use either power.id or power.lines with per-line source_isolates_ports"));
            }
            value
                .as_array()
                .filter(|lines| (1..=16).contains(&lines.len()))
                .ok_or_else(|| bad(c, "give between 1 and 16 power lines"))?
                .iter()
                .map(|entry| {
                    only(c, entry, &["id", "source_isolates_ports"])?;
                    parse_line(entry)
                })
                .collect::<Result<Vec<_>, String>>()?
        } else {
            vec![parse_line(power)?]
        };
        let independent = power.get("independent").map(|v| guids_param(c, m, v, "independent")).transpose()?.unwrap_or_default();
        Some(crate::low_voltage::faults::PowerScope { lines, independent: independent.into_iter().collect() })
    } else {
        None
    };
    let candidates = flag(c, p, "propose")?.then(|| {
        if line.segments.len() > crate::low_voltage::faults::MAX_PLACEMENT_CABLES {
            return BTreeMap::new();
        }
        line.segments
            .iter()
            .filter_map(|&guid| {
                let wire_id = m.elements.id(guid)?;
                let (fraction, at) = fire_isolator_point(m, id, line, wire_id, None).ok()?;
                Some((wire_id, (fraction, at)))
            })
            .collect()
    });
    let max_isolators = count_opt(c, p, "max_isolators", if flag(c, p, "allow_pair")? { 2 } else { 1 })?;
    if !(1..=4).contains(&max_isolators) {
        return Err(bad(c, "give a max_isolators between 1 and 4"));
    }
    let mut report = crate::low_voltage::faults::report(
        m,
        line,
        &isolators,
        flag(c, p, "source_isolates_ports")?,
        candidates.as_ref(),
        max_isolators,
        power,
    );
    report["package"] = json!(m.fire_standard);
    Ok(report)
}

fn fire_isolator(s: &mut Session, p: &Value) -> R {
    let c = "fire_isolator";
    only(c, p, &["id", "wire", "family", "type", "fraction", "apply"])?;
    let m = &s.model;
    let id = element_id(c, m, &p["id"])?;
    let wire_id = element_id(c, m, &p["wire"])?;
    let Some(Element::ElectricalLine(line)) = m.elements.get(id) else { return Err(bad(c, "select an electrical line")) };
    let (fraction, at) = fire_isolator_point(m, id, line, wire_id, num(c, p, "fraction")?).map_err(|reason| bad(c, reason))?;
    let family = string(c, p, "family")?.ok_or_else(|| bad(c, "family is required"))?;
    let types = &m.families.get(&family).ok_or_else(|| bad(c, "choose an existing isolator family"))?.types;
    let type_name = string(c, p, "type")?
        .or_else(|| types.keys().next().cloned())
        .filter(|name| types.contains_key(name))
        .ok_or_else(|| bad(c, "choose an existing isolator type"))?;
    if !flag(c, p, "apply")? {
        return Ok(json!({"at":at,"family":family,"type":type_name,"wire":wire_id,"fraction":fraction}));
    }
    let Some(Element::Wire(wire)) = m.elements.get(wire_id) else { return Err(bad(c, "select a main wire")) };
    let level = wire.level;
    let offset = at[2] - m.elevation(level);
    let device = component(s, &json!({"family":family,"type":type_name,"level":level,"at":[at[0],at[1]],"offset":offset}))?["id"].clone();
    let segments = split(s, &json!({"id":wire_id,"fraction":fraction}))?["ids"].clone();
    for (i, end) in [1, 0].into_iter().enumerate() {
        wire_connect(s, &json!({"wire":segments[i],"device":device,"end":end,"point":[0,0,0]}))?;
    }
    let device_guid = s.model.guid(element_id(c, &s.model, &device)?);
    let Some(Element::ElectricalLine(line)) = s.model_mut().elements.get_mut(id) else { return Err(bad(c, "the line is missing")) };
    line.isolators.push(device_guid);
    Ok(json!({"device":device,"segments":segments,"at":at}))
}

fn fire_isolator_point(
    m: &Model,
    id: Id,
    line: &crate::model::ElectricalLine,
    wire_id: Id,
    fraction: Option<f64>,
) -> Result<(f64, [f64; 3]), &'static str> {
    use crate::view3d::{add, dot, mul, sub};
    let wire_guid = m.guid(wire_id);
    if !line.segments.contains(&wire_guid)
        || m.elements.iter().any(|(&other, e)| other != id && matches!(e, Element::ElectricalLine(l) if l.segments.contains(&wire_guid)))
    {
        return Err("the main wire must belong only to the selected line");
    }
    let Some(element @ Element::Wire(wire)) = m.elements.get(wire_id) else { return Err("select a main wire") };
    if wire.feeder.is_some() || wire.source.is_some() || !wire.devices.is_empty() || wire.mount.is_some() || wire.carrier.is_some() {
        return Err("select an unmounted main wire without a carrier or legacy device links");
    }
    let [a, b] = wire.connections.as_slice() else { return Err("the cable needs two explicit device connections") };
    let geom = m.run_geom(element).ok_or("the cable has no physical axis")?;
    let direction = sub(geom.b, geom.a);
    let length = geom.length();
    if length <= TOL {
        return Err("the cable has no physical axis");
    }
    let projected = |end| {
        wire_landing(m, wire_guid, end)
            .map(|at| (dot(sub(at, geom.a), direction) / dot(direction, direction)).clamp(0.0, 1.0))
            .ok_or("the cable connections must have resolved landings")
    };
    let (a, b) = (projected(a)?, projected(b)?);
    let fraction = fraction.unwrap_or(f64::midpoint(a, b));
    if (fraction - a.min(b)) * length <= TOL || (a.max(b) - fraction) * length <= TOL {
        return Err("place the isolator between the two cable connection landings");
    }
    Ok((fraction, add(geom.a, mul(direction, fraction))))
}

fn cable_journal(s: &mut Session, p: &Value) -> R {
    let c = "cable_journal";
    only(c, p, &["id", "mode"])?;
    let m = &s.model;
    let id = element_id(c, m, &p["id"])?;
    let Some(Element::ElectricalLine(line)) = m.elements.get(id) else { return Err(bad(c, "select an electrical line")) };
    let mode = string(c, p, "mode")?.unwrap_or_else(|| "segments".into());
    if !matches!(mode.as_str(), "segments" | "line") {
        return Err(bad(c, "mode must be segments or line"));
    }
    // One row per route: the segments of a drawn cable share its head, its length is their sum.
    let mut lengths: BTreeMap<Guid, f64> = BTreeMap::new();
    let mut heads: Vec<Guid> = Vec::new();
    for guid in &line.segments {
        let head = route_head(m, *guid);
        if let std::collections::btree_map::Entry::Vacant(entry) = lengths.entry(head) {
            entry.insert(0.0);
            heads.push(head);
        }
    }
    for (_, guid, element) in m.elements.with_guids() {
        let Element::Wire(wire) = element else { continue };
        let owner = wire.feeder.as_ref().map_or(guid, |lead| lead.wire);
        if let Some(length) = lengths.get_mut(&route_head(m, owner))
            && let Some(geometry) = m.run_geom(element)
        {
            *length += geometry.length();
        }
    }
    for (head, length) in &mut lengths {
        if let Some(Element::Wire(wire)) = m.elements.get(*head) {
            *length *= 1.0 + wire.reserve / 100.0;
        }
    }
    let rows = if mode == "line" {
        vec![json!({"id":id,"length":hundredth(lengths.values().sum())})]
    } else {
        heads.iter().map(|head| json!({"id":m.elements.id(*head),"guid":head,"length":hundredth(lengths[head])})).collect()
    };
    Ok(json!({"line":id,"rows":rows}))
}

fn electrical_mark_template(s: &mut Session, p: &Value) -> R {
    let c = "electrical_mark_template";
    only(c, p, &["system", "prefix"])?;
    let system = string(c, p, "system")?.filter(|name| !name.trim().is_empty()).ok_or_else(|| bad(c, "system is required"))?;
    let templates = &mut s.model_mut().electrical_mark_templates;
    match p.get("prefix") {
        Some(Value::Null) => {
            templates.remove(&system);
        }
        Some(Value::String(prefix)) => {
            templates.insert(system.clone(), prefix.clone());
        }
        _ => return Err(bad(c, "prefix must be a string or null")),
    }
    Ok(json!({"system":system,"prefix":p["prefix"]}))
}

fn electrical_line_renumber(s: &mut Session, p: &Value) -> R {
    only("electrical_line_renumber", p, &["id", "mark_prefix"])?;
    if p.get("mark_prefix").is_some() {
        electrical_line_edit(s, p)?;
    }
    electrical_line_inspect(s, &json!({"id":p["id"]}))
}

fn electrical_line_mark(s: &mut Session, p: &Value) -> R {
    let c = "electrical_line_mark";
    only(c, p, &["id", "device", "mark"])?;
    let id = element_id(c, &s.model, &p["id"])?;
    let device = s.model.guid(element_id(c, &s.model, &p["device"])?);
    let mark = match p.get("mark") {
        Some(Value::Null) => None,
        Some(Value::String(mark)) => Some(mark.clone()),
        _ => return Err(bad(c, "mark must be a string or null")),
    };
    let Some(Element::ElectricalLine(line)) = s.model_mut().elements.get_mut(id) else { return Err(bad(c, "select an electrical line")) };
    if !line.members().any(|guid| guid == device) {
        return Err(bad(c, "device must belong to the line"));
    }
    line.locked_marks.retain(|entry| entry.device != device);
    if let Some(mark) = mark {
        line.locked_marks.push(crate::model::ElectricalMark { device, mark });
    }
    electrical_line_inspect(s, &json!({"id":id}))
}

fn wire(s: &mut Session, p: &Value) -> R {
    let (start, end, route) = (p.get("start_connection"), p.get("end_connection"), p.get("route"));
    let mut params = p.clone();
    if let Some(params) = params.as_object_mut() {
        params.remove("start_connection");
        params.remove("end_connection");
        params.remove("route");
    }
    let result = chain(s, &params, "wire")?;
    link_route(s, &result, route)?;
    if let Some(connection) = start {
        only("wire", connection, &["device", "point"])?;
        wire_connect(s, &json!({"wire":result["ids"][0],"device":connection["device"],"point":connection["point"]}))?;
    }
    if let Some(connection) = end {
        only("wire", connection, &["device", "point"])?;
        let last = result["ids"].as_array().and_then(|ids| ids.last()).cloned().ok_or_else(|| bad("wire", "missing created wire"))?;
        wire_connect(s, &json!({"wire":last,"device":connection["device"],"end":1,"point":connection["point"]}))?;
    }
    Ok(result)
}

/// Join the fresh segments into one cable: under `route`'s head when given, else under the first of a multi-segment chain.
/// The head names itself so a split keeps both halves on the same route; segments inherit the head's system and cable.
fn link_route(s: &mut Session, result: &Value, route: Option<&Value>) -> Result<(), String> {
    let c = "wire";
    let ids: Vec<Id> = result["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let head = match route {
        Some(value) => {
            let id = element_id(c, &s.model, value)?;
            match s.model.elements.get(id) {
                Some(Element::Wire(w)) if w.feeder.is_none() => {}
                _ => return Err(bad(c, "`route` names a main wire")),
            }
            let guid = s.model.guid(id);
            s.model.elements.get(id).and_then(Element::run).and_then(|r| r.route).unwrap_or(guid)
        }
        None if ids.len() > 1 => s.model.guid(ids[0]),
        None => return Ok(()),
    };
    let (system, cable) = s
        .model
        .elements
        .id(head)
        .and_then(|id| s.model.elements.get(id))
        .and_then(Element::run)
        .map(|r| (r.system.clone(), r.cable.clone()))
        .unwrap_or_default();
    let m = s.model_mut();
    for id in m.elements.id(head).into_iter().chain(ids) {
        if let Some(Element::Wire(w)) = m.elements.get_mut(id) {
            w.route = Some(head);
            if w.system.is_empty() {
                w.system = system.clone();
            }
            if w.cable.is_empty() {
                w.cable = cable.clone();
            }
        }
    }
    Ok(())
}

/// The route whose last segment ends at `at` on `level` at `height`: a chain click continues that cable, without a gap.
pub fn route_ending_at(m: &Model, level: Guid, at: P2, height: f64) -> Option<Guid> {
    let mut found = None;
    for (_, guid, element) in m.elements.with_guids() {
        let Element::Wire(w) = element else { continue };
        if w.feeder.is_some() || w.level != level || w.end.dist(at) > TOL {
            continue;
        }
        if (m.elevation(level) + w.end_offset.unwrap_or(w.offset) - height).abs() > TOL {
            continue;
        }
        found = Some(w.route.unwrap_or(guid));
    }
    found
}

/// The head guid of the route a wire belongs to: its `route`, else itself.
pub fn route_head(m: &Model, guid: Guid) -> Guid {
    match m.elements.id(guid).and_then(|id| m.elements.get(id)) {
        Some(Element::Wire(w)) => w.route.unwrap_or(guid),
        _ => guid,
    }
}

/// Every main wire of the route headed by `head`, in model order; the head itself included.
pub fn route_members(m: &Model, head: Guid) -> Vec<Guid> {
    m.elements
        .with_guids()
        .filter(|(_, guid, e)| matches!(e, Element::Wire(w) if w.feeder.is_none() && w.route.unwrap_or(*guid) == head))
        .map(|(_, guid, _)| guid)
        .collect()
}

/// A route's guid stands for every segment of its cable; other guids stay as they are.
fn expand_route_segments(m: &Model, segments: Vec<Guid>) -> Vec<Guid> {
    let mut out: Vec<Guid> = Vec::new();
    for guid in segments {
        let members = route_members(m, route_head(m, guid));
        if members.is_empty() {
            out.push(guid);
        }
        for member in members {
            if !out.contains(&member) {
                out.push(member);
            }
        }
    }
    out
}

fn wire_connect(s: &mut Session, p: &Value) -> R {
    let c = "wire_connect";
    only(c, p, &["wire", "device", "end", "point", "surface", "carrier", "in_carrier", "min_bend_radius", "terminal"])?;
    let end = count_opt(c, p, "end", 0)?;
    if end > 1 {
        return Err(bad(c, "end must be 0 or 1"));
    }
    let end = end as u8;
    let wire_id = element_id(c, &s.model, p.get("wire").ok_or_else(|| bad(c, "wire is required"))?)?;
    let device_id = element_id(c, &s.model, p.get("device").ok_or_else(|| bad(c, "device is required"))?)?;
    let surface = match p.get("surface") {
        None => None,
        Some(value) => {
            only(c, value, &["host", "face"])?;
            if p.get("point").is_some() {
                return Err(bad(c, "choose a fixed point or a free surface, not both"));
            }
            let host = element_id(c, &s.model, value.get("host").ok_or_else(|| bad(c, "surface.host is required"))?)?;
            let face = string(c, value, "face")?.ok_or_else(|| bad(c, "surface.face is required"))?;
            Some(crate::model::ConnectionSurface { host: Some(s.model.guid(host)), face, origin: [0.0; 3], normal: [0.0, 1.0, 0.0] })
        }
    };
    let mut point = [0.0; 3];
    if surface.is_none() {
        let values = p
            .get("point")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 3)
            .ok_or_else(|| bad(c, "point must be a local [x,y,z] in mm, or provide surface"))?;
        for (i, v) in values.iter().enumerate() {
            point[i] = v
                .as_f64()
                .filter(|v| v.is_finite() && v.abs() <= MAX_COORD)
                .ok_or_else(|| bad(c, "point must be finite and within coordinate limits"))?;
        }
    }
    let carrier = match p.get("carrier") {
        None | Some(Value::Null) => None,
        Some(value) => Some(s.model.guid(element_id(c, &s.model, value)?)),
    };
    let min_bend_radius = match p.get("min_bend_radius") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|r| r.is_finite() && *r > 0.0 && *r <= MAX_COORD)
                .ok_or_else(|| bad(c, "min_bend_radius must be a positive length in mm"))?,
        ),
    };
    if min_bend_radius.is_some() && surface.is_none() {
        return Err(bad(c, "min_bend_radius requires a free surface"));
    }
    let terminal = id_opt(c, p, "terminal")?
        .map(|g| match s.model.elements.get(g) {
            Some(Element::ElectricalTerminal(t)) if t.device == s.model.guid(device_id) => Ok(s.model.guid(g)),
            _ => Err(bad(c, "terminal must belong to the connected device")),
        })
        .transpose()?;
    let mut connection = crate::model::WireConnection {
        terminal,
        min_bend_radius,
        device: s.model.guid(device_id),
        end,
        point,
        in_carrier: flag(c, p, "in_carrier")?,
        carrier,
        landing: None,
        surface,
    };
    let before = s.model.clone();
    let Some(Element::Wire(wire)) = s.model_mut().elements.get_mut(wire_id) else {
        return Err(bad(c, "wire must name an existing main wire"));
    };
    if wire.feeder.is_some() {
        return Err(bad(c, "select the main wire, not an owned feeder"));
    }
    if let Some(existing) = wire.connections.iter_mut().find(|v| v.device == connection.device && v.end == connection.end) {
        if p.get("min_bend_radius").is_none() && connection.surface.is_some() {
            connection.min_bend_radius = existing.min_bend_radius;
        }
        if p.get("terminal").is_none() {
            connection.terminal = existing.terminal;
        }
        *existing = connection;
    } else {
        if wire.connections.len() >= MAX_POINTS {
            return Err(bad(c, "too many equipment connections"));
        }
        wire.connections.push(connection);
    }
    s.model_mut().refresh_wire_connections(&before)?;
    let parent = s.model.guid(wire_id);
    let device = s.model.guid(device_id);
    let mut ids = Vec::new();
    let mut carriers = Vec::new();
    for (id, e) in s.model.elements.iter() {
        if e.run().and_then(|r| r.feeder.as_ref()).is_some_and(|f| f.wire == parent && f.device == device && f.end == end) {
            if e.category() == "wire" {
                ids.push(*id);
            } else {
                carriers.push(*id);
            }
        }
    }
    Ok(json!({"ids":ids,"carriers":carriers,"wire":wire_id,"device":device_id,"end":end}))
}

fn cable_carrier(s: &mut Session, p: &Value) -> R {
    if let Some(scope) = p.get("from_wires") {
        let c = "cable_carrier";
        only(c, p, &["from_wires", "type"])?;
        only(c, scope, &["level"])?;
        let chosen = string(c, p, "type")?;
        if chosen.as_ref().is_some_and(|name| !s.model.types.runs.get("cablecarrier").is_some_and(|types| types.contains_key(name))) {
            return Err(bad(c, "select an existing cable carrier type"));
        }
        let level = scope.get("level").map(|_| level_param(c, &s.model, scope, "level")).transpose()?;
        let wires: Vec<_> = s
            .model
            .elements
            .iter()
            .filter_map(|(id, element)| {
                matches!(element, Element::Wire(w) if w.feeder.is_none() && w.carrier.is_none() && level.is_none_or(|l| w.level == l))
                    .then_some(*id)
            })
            .take(MAX_POINTS + 1)
            .collect();
        if wires.len() > MAX_POINTS {
            return Err(bad(c, "too many wires in one operation"));
        }
        let mut ids = Vec::with_capacity(wires.len());
        let mut wires = wires;
        // Parallel wires share one carrier over their common envelope before any single-wire carrier is laid.
        for group in parallel_wire_groups(&s.model, &wires) {
            if group.len() < 2 {
                continue;
            }
            let before = (*s.model).clone();
            match group_carrier(s, &group, chosen.as_deref()) {
                Ok(carrier) => {
                    ids.push(json!(carrier));
                    wires.retain(|wire| !group.contains(wire));
                }
                Err(_) => *s.model_mut() = before,
            }
        }
        for wire in wires {
            if share_carrier(s.model_mut(), wire, chosen.as_deref()) {
                continue;
            }
            let result =
                cable_carrier(s, &json!({"from_wire":wire,"type":chosen})).map_err(|error| bad(c, format!("wire {wire}: {error}")))?;
            let carrier = result["ids"][0].as_u64().ok_or_else(|| bad(c, "missing created carrier"))?;
            if !carrier_fill_fits(&s.model, s.model.guid(carrier)) {
                return Err(bad(c, format!("wire {wire}: the carrier cannot be verified to fit under the selected fill rules")));
            }
            ids.push(result["ids"][0].clone());
        }
        return Ok(json!({"ids":ids}));
    }
    if let Some(value) = p.get("from_wire") {
        let c = "cable_carrier";
        only(c, p, &["from_wire", "type", "placement", "local_offset"])?;
        let m = &s.model;
        let wire_id = element_id(c, m, value)?;
        let element = m.elements.get(wire_id).ok_or_else(|| bad(c, "from_wire must name an existing wire"))?;
        let Element::Wire(wire) = element else { return Err(bad(c, "from_wire must name a wire")) };
        if wire.feeder.is_some() {
            return Err(bad(c, "connect equipment in its main wire's carrier instead of editing an owned feeder"));
        }
        if wire.carrier.is_some() {
            return Err(bad(c, "wire already has a carrier; detach it before creating another"));
        }
        let geometry = m.run_geom(element).ok_or_else(|| bad(c, "wire has no valid physical geometry"))?;
        let type_name = if absent(p.get("type")) {
            m.carrier_settings
                .as_ref()
                .map(|v| v.default_type.clone())
                .filter(|v| !v.is_empty())
                .map(Ok)
                .unwrap_or_else(|| type_param(c, p, m, "cablecarrier"))?
        } else {
            type_param(c, p, m, "cablecarrier")?
        };
        let section = m
            .types
            .runs
            .get("cablecarrier")
            .and_then(|types| types.get(&type_name))
            .ok_or_else(|| bad(c, "select an existing cable carrier type"))?;
        let placement = string(c, p, "placement")?.unwrap_or_else(|| "centre".into());
        let local_offset = num(c, p, "local_offset")?.unwrap_or(0.0);
        if wire.mount.is_some() && (placement != "centre" || local_offset != 0.0) {
            return Err(bad(c, "a mounted wire requires centre placement and zero lateral offset to keep its support"));
        }
        let depth = if section.round { section.width } else { section.depth };
        if geometry.width > section.width + TOL
            || geometry.depth > depth + TOL
            || local_offset.abs() + geometry.width / 2.0 > section.width / 2.0 + TOL
        {
            return Err(bad(c, "the wire and its lateral offset must fit the carrier section"));
        }
        let clearance = if section.round {
            let (radius, lateral, half_depth) = if geometry.round {
                ((section.width - geometry.width) / 2.0, local_offset.abs(), 0.0)
            } else {
                (section.width / 2.0, local_offset.abs() + geometry.width / 2.0, geometry.depth / 2.0)
            };
            let clearance = (radius * radius - lateral * lateral).max(0.0).sqrt() - half_depth;
            if clearance < -TOL {
                return Err(bad(c, "the wire corners must fit the round carrier section"));
            }
            clearance.max(0.0)
        } else {
            (depth - geometry.depth) / 2.0
        };
        let vertical = match placement.as_str() {
            "centre" => 0.0,
            "top" => clearance,
            "bottom" => -clearance,
            _ => return Err(bad(c, "placement must be centre, top or bottom")),
        };
        let (_, across, up) = geometry.frame();
        let shift = view3d::add(view3d::mul(across, local_offset), view3d::mul(up, vertical));
        let a = view3d::sub(geometry.a, shift);
        let b = view3d::sub(geometry.b, shift);
        let mut carrier = wire.clone();
        if carrier.mount.is_none() {
            let base = m.elevation(wire.level);
            carrier.start = P2(a[0], a[1]);
            carrier.end = P2(b[0], b[1]);
            carrier.offset = a[2] - base;
            carrier.end_offset = Some(b[2] - base);
        }
        carrier.square_ends = geometry.square_ends;
        carrier.type_name = type_name;
        carrier.justify = crate::model::Justify::Centre;
        carrier.source = None;
        carrier.devices.clear();
        carrier.cable.clear();
        carrier.reserve = 0.0;
        carrier.along_walls = false;
        carrier.carrier = None;
        carrier.local_offset = 0.0;
        carrier.size_locked = false;
        carrier.compartments.clear();
        carrier.compartment.clear();
        carrier.params.clear();
        carrier.connections.clear();
        carrier.feeder = None;
        let base = m.elevation(wire.level);
        let mut wire = wire.clone();
        set_wire_axis(&mut wire, geometry, base);
        let m = s.model_mut();
        let id = m.insert(Element::CableCarrier(carrier));
        if m.elements.get(id).and_then(Element::run).is_some_and(|run| run.mount.is_some())
            && !m.carrier_wire_candidates(id)?.contains(&wire_id)
        {
            return Err(bad(c, "the supported carrier does not contain the wire's physical section"));
        }
        wire.carrier = Some(m.guid(id));
        wire.local_offset = local_offset;
        m.elements.insert(wire_id, Element::Wire(wire));
        return Ok(json!({"ids": [id], "wire": wire_id}));
    }
    chain(s, p, "cablecarrier")
}

fn set_wire_axis(wire: &mut crate::model::Run, geometry: crate::runs::Geom, base: f64) {
    wire.mount = None;
    wire.section_roll = geometry.roll;
    wire.square_ends = geometry.square_ends;
    wire.start = P2(geometry.a[0], geometry.a[1]);
    wire.end = P2(geometry.b[0], geometry.b[1]);
    wire.offset = geometry.a[2] - base;
    wire.end_offset = Some(geometry.b[2] - base);
    wire.justify = crate::model::Justify::Centre;
}

fn carrier_fill_fits(m: &Model, carrier: Guid) -> bool {
    let Some(Element::CableCarrier(run)) = m.elements.get(carrier) else { return false };
    let profile = m.carrier_settings.as_ref().map_or(crate::carrier_limits::Profile::default(), |p| p.profile);
    let percent = m.carrier_settings.as_ref().map_or(100.0, |p| p.fill_percent);
    let used = m.carrier_sections().get(&carrier).copied().unwrap_or_default();
    let envelope = m.carrier_envelopes().get(&carrier).copied().unwrap_or_default();
    m.carrier_capacity(&run.type_name, profile, percent).is_ok_and(|allowed| used <= allowed)
        && m.carrier_type_fits(&run.type_name, envelope, profile)
}

/// Uncarried wires that may share one carrier: same level, system and mount, parallel axes, overlapping spans.
fn parallel_wire_groups(m: &Model, wires: &[Id]) -> Vec<Vec<Id>> {
    let mut groups: Vec<Vec<Id>> = Vec::new();
    for &id in wires {
        let Some(Element::Wire(wire)) = m.elements.get(id) else { continue };
        let Some(geometry) = m.elements.get(id).and_then(|element| m.run_geom(element)) else { continue };
        let (axis, _, _) = geometry.frame();
        let mut placed = false;
        for group in &mut groups {
            let Some(Element::Wire(head)) = m.elements.get(group[0]) else { continue };
            let Some(head_geometry) = m.elements.get(group[0]).and_then(|element| m.run_geom(element)) else { continue };
            let (head_axis, _, _) = head_geometry.frame();
            if head.level != wire.level
                || head.system != wire.system
                || head.mount != wire.mount
                || view3d::dot(axis, head_axis).abs() < 1.0 - 1e-8
            {
                continue;
            }
            let span = |g: &crate::runs::Geom| {
                let (a, b) = (view3d::dot(g.a, head_axis), view3d::dot(g.b, head_axis));
                (a.min(b), a.max(b))
            };
            let (hlo, hhi) = span(&head_geometry);
            let (lo, hi) = span(&geometry);
            if lo.max(hlo) <= hi.min(hhi) + TOL {
                group.push(id);
                placed = true;
                break;
            }
        }
        if !placed {
            groups.push(vec![id]);
        }
    }
    groups
}

/// One carrier around a group of parallel wires: its axis on the envelope centre, its span over their common reach.
/// A mounted carrier keeps its contact on the mounting plane. The caller restores the model when the group does not fit.
fn group_carrier(s: &mut Session, group: &[Id], chosen: Option<&str>) -> Result<Id, String> {
    let result = cable_carrier(s, &json!({"from_wire":group[0],"type":chosen}))?;
    let carrier = result["ids"][0].as_u64().ok_or("missing created carrier")?;
    let m = s.model_mut();
    let guid = m.guid(carrier);
    let head = m.run_geom(m.elements.get(carrier).ok_or("carrier is missing")?).ok_or("carrier has no physical axis")?;
    let (axis, x, y) = head.frame();
    let (mut lo, mut hi, mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &id in group {
        let g = m.run_geom(m.elements.get(id).ok_or("wire is missing")?).ok_or("wire has no physical axis")?;
        for point in [g.a, g.b] {
            let delta = view3d::sub(point, head.a);
            let (span, px, py) = (view3d::dot(delta, axis), view3d::dot(delta, x), view3d::dot(delta, y));
            (lo, hi) = (lo.min(span), hi.max(span));
            (xmin, xmax) = (xmin.min(px), xmax.max(px));
            (ymin, ymax) = (ymin.min(py), ymax.max(py));
        }
    }
    let centre = view3d::add(view3d::mul(x, (xmin + xmax) / 2.0), view3d::mul(y, (ymin + ymax) / 2.0));
    let (mut a, mut b) =
        (view3d::add(view3d::add(head.a, view3d::mul(axis, lo)), centre), view3d::add(view3d::add(head.a, view3d::mul(axis, hi)), centre));
    let run = m.elements.get(carrier).and_then(Element::run).ok_or("carrier is missing")?;
    let (base, mount) = (m.elevation(run.level), run.mount.clone());
    if let Some(mount) = &mount {
        // The contact line must stay on the mounting plane: only the in-plane part of the envelope centre is reachable.
        let (origin, _, normal) = m.run_mount_frame(mount)?;
        let shift = view3d::mul(normal, head.depth / 2.0);
        let pin = |point: view3d::P3| {
            let contact = view3d::sub(point, shift);
            view3d::sub(contact, view3d::mul(normal, view3d::dot(view3d::sub(contact, origin), normal)))
        };
        (a, b) = (pin(a), pin(b));
    }
    let Some(Element::CableCarrier(run)) = m.elements.get_mut(carrier) else { return Err("carrier is missing".into()) };
    run.start = P2(a[0], a[1]);
    run.end = P2(b[0], b[1]);
    run.offset = a[2] - base;
    run.end_offset = Some(b[2] - base);
    // Unlink the head wire so the containment check sees the whole group at the moved axis.
    let Some(Element::Wire(head_wire)) = m.elements.get_mut(group[0]) else { return Err("wire is missing".into()) };
    head_wire.carrier = None;
    let candidates = m.carrier_wire_candidates(carrier)?;
    if !group.iter().all(|id| candidates.contains(id)) {
        return Err("the group does not fit the carrier section".into());
    }
    let host = m.run_geom(m.elements.get(carrier).ok_or("carrier is missing")?).ok_or("carrier has no physical axis")?;
    let (_, across, _) = host.frame();
    let host_mid = view3d::mul(view3d::add(host.a, host.b), 0.5);
    for &id in group {
        let Some(Element::Wire(original)) = m.elements.get(id).cloned() else { return Err("wire is missing".into()) };
        let geometry = m.run_geom(m.elements.get(id).ok_or("wire is missing")?).ok_or("wire has no physical axis")?;
        let mut wire = original.clone();
        if wire.mount.is_some() {
            let elevation = m.elevation(wire.level);
            set_wire_axis(&mut wire, geometry, elevation);
        }
        let mid = view3d::mul(view3d::add(geometry.a, geometry.b), 0.5);
        wire.local_offset = view3d::dot(view3d::sub(mid, host_mid), across);
        wire.carrier = Some(guid);
        m.elements.insert(id, Element::Wire(wire));
    }
    if !carrier_fill_fits(m, guid) {
        return Err("the group overfills the carrier under the selected fill rules".into());
    }
    Ok(carrier)
}

fn share_carrier(m: &mut Model, id: Id, chosen: Option<&str>) -> bool {
    let Some(Element::Wire(original)) = m.elements.get(id).cloned() else { return false };
    let candidates: Vec<_> = m
        .elements
        .iter()
        .filter_map(|(id, element)| {
            matches!(element, Element::CableCarrier(run)
            if run.level == original.level && run.system == original.system && run.compartments.is_empty()
                && (original.mount.is_none() || run.mount == original.mount)
                && chosen.is_none_or(|name| run.type_name == name))
            .then_some(*id)
        })
        .collect();
    for carrier in candidates {
        let guid = m.guid(carrier);
        let compatible = m.elements.values().all(|element| {
            !matches!(element, Element::Wire(run)
            if run.carrier == Some(guid) && run.system != original.system)
        });
        if !compatible || !m.carrier_wire_candidates(carrier).is_ok_and(|wires| wires.contains(&id)) {
            continue;
        }
        let mut wire = original.clone();
        if wire.mount.is_some()
            && let Some(geometry) = m.elements.get(id).and_then(|element| m.run_geom(element))
        {
            set_wire_axis(&mut wire, geometry, m.elevation(original.level));
        }
        wire.carrier = Some(guid);
        m.elements.insert(id, Element::Wire(wire));
        if carrier_fill_fits(m, guid) {
            return true;
        }
        m.elements.insert(id, Element::Wire(original.clone()));
    }
    false
}

fn cable_carrier_candidates(s: &mut Session, p: &Value) -> R {
    let c = "cable_carrier_candidates";
    only(c, p, &["carrier"])?;
    let carrier = element_id(c, &s.model, p.get("carrier").ok_or_else(|| bad(c, "carrier is required"))?)?;
    let wires = s.model.carrier_wire_candidates(carrier)?;
    Ok(json!({"carrier":carrier,"ambiguous":wires.len()>1,"wires":wires}))
}

fn carrier_section(s: &mut Session, p: &Value) -> R {
    let c = "carrier_section";
    only(c, p, &["type", "section"])?;
    let key = p.get("type").ok_or_else(|| bad(c, "type is required"))?;
    let library = &s.model.library;
    let kind = LibKind::Type("cablecarrier".into());
    let entry = match key {
        Value::String(name) => library
            .entries()
            .find_map(|(_, e)| (e.kind == kind && e.name == *name).then_some(e))
            .or_else(|| Guid::parse(name).and_then(|g| library.by_guid(g).map(|(_, e)| e))),
        _ => key.as_u64().and_then(|id| library.get(id)),
    }
    .ok_or_else(|| bad(c, "cable-carrier type was not found"))?
    .clone();
    if entry.kind != kind || entry.placeholder {
        return Err(bad(c, "type must be an existing non-placeholder cable-carrier type"));
    }
    let value = p.get("section").ok_or_else(|| bad(c, "section is required; use null to clear it"))?;
    let section = if value.is_null() {
        None
    } else {
        Some(serde_json::from_value::<crate::carrier_limits::Section>(value.clone()).map_err(|e| bad(c, e))?)
    };
    let mut product = entry.product.unwrap_or_default();
    product.carrier = section;
    let product = (product != crate::library::Product::default()).then_some(product);
    s.model_mut().library.set_product(&kind, &entry.name, product)?;
    Ok(json!({"type":entry.name,"section":value}))
}

pub(crate) fn cable_carrier_check(s: &mut Session, p: &Value) -> R {
    let c = "cable_carrier_check";
    only(c, p, &["mode", "fill_percent", "configure", "default_type", "profile"])?;
    let profile: crate::carrier_limits::Profile = match p.get("profile") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| bad(c, e))?,
        None => s.model.carrier_settings.as_ref().map_or(crate::carrier_limits::Profile::default(), |p| p.profile),
    };
    let mode = string(c, p, "mode")?.unwrap_or_else(|| "warn".into());
    if !matches!(mode.as_str(), "warn" | "auto") {
        return Err(bad(c, "mode must be warn or auto"));
    }
    let percent = num(c, p, "fill_percent")?.ok_or_else(|| bad(c, "fill_percent is required"))?;
    if !percent.is_finite() || percent <= 0.0 || percent > 100.0 {
        return Err(bad(c, "fill_percent must be greater than 0 and at most 100"));
    }
    let configure = flag(c, p, "configure")?;
    if p.get("default_type").is_some() && !configure {
        return Err(bad(c, "default_type requires configure:true"));
    }
    let default_type = if p.get("default_type").is_some() {
        string(c, p, "default_type")?.unwrap_or_default()
    } else {
        s.model.carrier_settings.as_ref().map(|p| p.default_type.clone()).unwrap_or_default()
    };
    let m = &s.model;
    let area = |t: &crate::model::RunType| if t.round { std::f64::consts::PI * t.width * t.width / 4.0 } else { t.width * t.depth };
    let maker = |name: &str| {
        m.library.entries().find_map(|(_, e)| {
            (e.kind == LibKind::Type("cablecarrier".into()) && e.name == name && !e.placeholder)
                .then(|| e.product.as_ref().map(|p| p.manufacturer.trim()))
                .flatten()
                .filter(|s| !s.is_empty())
        })
    };
    let mut changes = Vec::new();
    let mut warnings = Vec::new();
    let mut affected = Vec::new();
    for (guid, notes) in m.carrier_compartment_warnings(if profile == crate::carrier_limits::Profile::Project { percent } else { 100.0 }) {
        affected.push(guid);
        warnings.extend(notes);
    }
    let sections = m.carrier_sections();
    let envelopes = m.carrier_envelopes();
    let carriers: Vec<_> = m
        .elements
        .iter()
        .filter_map(|(id, e)| matches!(e, Element::CableCarrier(_)).then(|| m.run_geom(e).map(|g| (m.guid(*id), g))).flatten())
        .collect();
    let mut endpoints: std::collections::BTreeMap<[i64; 3], Vec<(usize, [f64; 3])>> = std::collections::BTreeMap::new();
    let mut adjacent = vec![Vec::new(); carriers.len()];
    for (i, (guid, g)) in carriers.iter().enumerate() {
        if matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if r.size_locked) {
            continue;
        }
        for point in [g.a, g.b] {
            let key = point.map(|v| (v / crate::model::TOL).floor() as i64);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(previous) = endpoints.get(&[key[0] + dx, key[1] + dy, key[2] + dz]) {
                            if previous.len() > 64 {
                                return Err(bad(c, "too many carrier endpoints at one junction; separate the junction before sizing"));
                            }
                            for &(j, other) in previous {
                                if i == j {
                                    continue;
                                }
                                let distance = crate::view3d::sub(point, other).iter().map(|v| v * v).sum::<f64>().sqrt();
                                let u = crate::view3d::sub(g.b, g.a);
                                let h = &carriers[j].1;
                                let v = crate::view3d::sub(h.b, h.a);
                                let cross = crate::view3d::cross(u, v).iter().map(|x| x * x).sum::<f64>().sqrt();
                                if distance <= crate::model::TOL && cross <= 1e-6 * g.length() * h.length() && g.round == h.round {
                                    adjacent[i].push(j);
                                    adjacent[j].push(i);
                                }
                            }
                        }
                    }
                }
            }
            endpoints.entry(key).or_default().push((i, point));
        }
    }
    let mut seen = vec![false; carriers.len()];
    for root in 0..carriers.len() {
        if seen[root] {
            continue;
        }
        let mut stack = vec![root];
        let mut group = Vec::new();
        while let Some(i) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            group.push(carriers[i].0);
            stack.extend(adjacent[i].iter().copied().filter(|j| !seen[*j]));
        }
        let used = group.iter().filter_map(|g| sections.get(g)).copied().fold(0.0_f64, f64::max);
        if used == 0.0 && group.iter().all(|g| matches!(m.elements.get(*g), Some(Element::CableCarrier(r)) if r.compartments.is_empty())) {
            continue;
        }
        let Some(host @ Element::CableCarrier(run)) = m.elements.get(group[0]) else { continue };
        let Some(current) = m.run_type(host) else { continue };
        let required = group.iter().filter_map(|g| envelopes.get(g)).fold([0.0_f64; 3], |mut a, b| {
            for i in 0..3 {
                a[i] = a[i].max(b[i]);
            }
            a
        });
        let geometry_conflict = group.iter().any(|g| {
            matches!(m.elements.get(*g), Some(Element::CableCarrier(r)) if !m.carrier_type_fits(&r.type_name, required, profile)
                || !m.carrier_compartments_fit(&r.type_name, r, profile))
        });
        let insufficient =
            group.iter().any(|guid| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if m.carrier_capacity(&r.type_name, profile, percent).map_or(true, |allowed| used > allowed)));
        let uniform =
            group.iter().all(|guid| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if r.type_name==run.type_name));
        if !insufficient && !geometry_conflict && (mode != "auto" || uniform) {
            continue;
        }
        let required_area = group
            .iter()
            .filter_map(|guid| m.elements.get(*guid).and_then(|e| m.run_type(e)))
            .map(area)
            .fold(if profile == crate::carrier_limits::Profile::Project { used * 100.0 / percent } else { 0.0 }, f64::max);
        if run.size_locked {
            affected.push(group[0]);
            warnings.push(format!("cable carrier {}: size is manually locked; capacity or cable envelope cannot be satisfied", group[0]));
            continue;
        }
        let selected = if mode == "auto" {
            maker(&run.type_name).and_then(|manufacturer| {
                if !group
                    .iter()
                    .all(|guid| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if maker(&r.type_name)==Some(manufacturer)))
                {
                    return None;
                }
                m.types
                    .runs
                    .get("cablecarrier")?
                    .iter()
                    .filter(|(name, t)| {
                        maker(name) == Some(manufacturer)
                            && t.round == current.round
                            && area(t) >= required_area
                            && m.carrier_capacity(name, profile, percent).is_ok_and(|allowed| used <= allowed)
                            && m.carrier_type_fits(name, required, profile)
                            && (profile == crate::carrier_limits::Profile::Project || m.carrier_section(&run.type_name).and_then(|s| s.enclosure).is_some_and(|kind|
                                group.iter().all(|guid| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if m.carrier_section(&r.type_name).and_then(|s| s.enclosure)==Some(kind)))
                                && m.carrier_section(name).and_then(|s| s.enclosure)==Some(kind)))
                            && group.iter().all(|guid| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if m.carrier_compartments_fit(name, r, profile)))
                    })
                    .min_by(|a, b| area(a.1).total_cmp(&area(b.1)).then(a.0.cmp(b.0)))
                    .map(|(name, _)| name.clone())
            })
        } else {
            None
        };
        if let Some(name) = selected {
            changes.extend(group.into_iter().map(|guid| (guid, name.clone())));
        } else {
            for guid in group {
                affected.push(guid);
                if let Some(Element::CableCarrier(r)) = m.elements.get(guid)
                    && let Err(error) = m.carrier_capacity(&r.type_name, profile, percent)
                {
                    warnings.push(format!("cable carrier {guid}: {error}; capacity cannot be verified"));
                }
                if geometry_conflict {
                    warnings.push(format!(
                        "cable carrier {guid}: geometric conflict; cable envelope or measured compartment does not fit the carrier"
                    ));
                }
                if let Some(Element::CableCarrier(r)) = m.elements.get(guid)
                    && let Ok(allowed) = m.carrier_capacity(&r.type_name, profile, percent)
                    && used > allowed
                {
                    warnings.push(format!("cable carrier {guid}: cable section {used:.1} mm2 exceeds allowed section {allowed:.1} mm2"));
                }
                warnings.push(format!("cable carrier {guid}: select a sufficient uniform type from the same manufacturer"));
            }
        }
    }
    changes.retain(|(guid, name)| matches!(m.elements.get(*guid), Some(Element::CableCarrier(r)) if r.type_name != *name));
    if configure {
        s.model_mut().carrier_settings =
            Some(crate::model::CarrierSettings { profile, fill_percent: percent, auto: mode == "auto", default_type });
    }
    for (guid, name) in &changes {
        // Use the same host-following geometry path as an explicit type edit.
        modify(s, &json!({"id":guid,"type":name}))?;
    }
    affected.sort();
    affected.dedup();
    Ok(json!({"changed":changes.iter().map(|(guid,_)| guid).collect::<Vec<_>>(),"warnings":warnings,"elements":affected}))
}

pub(crate) fn split_electrical_segment(m: &mut Model, first: Guid, second: Guid) -> Result<(), String> {
    for (_, element) in m.elements.iter_mut() {
        if let Element::ElectricalLine(line) = element
            && let Some(index) = line.segments.iter().position(|guid| *guid == first)
        {
            if line.segments.len() >= MAX_POINTS {
                return Err(bad("split", "electrical line has too many segments"));
            }
            line.segments.insert(index + 1, second);
        }
    }
    Ok(())
}

pub(crate) fn split_wire_connections(
    m: &Model,
    guid: Guid,
    first: &mut crate::model::Run,
    second: &mut crate::model::Run,
    geometry: crate::runs::Geom,
    fraction: f64,
) -> BTreeSet<(Guid, u8)> {
    use crate::view3d::{dot, sub};
    let direction = sub(geometry.b, geometry.a);
    let boundary = dot(direction, direction) * fraction;
    first.connections.retain(|connection| {
        let landing = wire_landing(m, guid, connection);
        let move_to_second = landing.is_some_and(|point| dot(sub(point, geometry.a), direction) > boundary);
        if move_to_second {
            second.connections.push(connection.clone());
        }
        !move_to_second
    });
    second.connections.iter().map(|connection| (connection.device, connection.end)).collect()
}

fn wire_landing(m: &Model, guid: Guid, connection: &crate::model::WireConnection) -> Option<[f64; 3]> {
    connection
        .landing
        .or_else(|| {
            m.elements
                .values()
                .filter_map(|element| {
                    let Element::Wire(wire) = element else { return None };
                    let owner = wire.feeder.as_ref()?;
                    (owner.wire == guid && owner.device == connection.device && owner.end == connection.end)
                        .then(|| m.run_geom(element).map(|g| (owner.leg, g.b)))?
                })
                .max_by_key(|(leg, _)| *leg)
                .map(|(_, point)| point)
        })
        .or_else(|| {
            // A fixed connection sitting on the axis produces no feeder legs.
            let Element::Component(component) = m.elements.get(connection.device)? else { return None };
            connection.surface.is_none().then(|| m.fixed_wire_contact(component, connection.point))
        })
}

pub(crate) fn transfer_feeders(m: &mut Model, from: Guid, to: Guid, connections: &BTreeSet<(Guid, u8)>) {
    if connections.is_empty() {
        return;
    }
    for (_, element) in m.elements.iter_mut() {
        if let Some(owner) = element.run_mut().and_then(|run| run.feeder.as_mut())
            && owner.wire == from
            && connections.contains(&(owner.device, owner.end))
        {
            owner.wire = to;
        }
    }
}

pub(crate) fn split_carried_wires(m: &mut Model, carriers: [Id; 2], old: crate::runs::Geom, fraction: f64) -> Result<(), String> {
    use crate::view3d::{P3, add, dot, mul, sub};
    let [first, second] = carriers.map(|id| m.guid(id));
    let [Some(a), Some(b)] = carriers.map(|id| m.elements.get(id).and_then(|e| m.run_geom(e))) else {
        return Err(bad("split", "carrier parts have no physical axis"));
    };
    let direction = sub(old.b, old.a);
    let length2 = dot(direction, direction);
    let project = |p| dot(sub(p, old.a), direction) / length2;
    let changes: Vec<_> = m
        .elements
        .iter()
        .filter_map(|(&id, e)| {
            let Element::Wire(wire) = e else { return None };
            if wire.carrier != Some(first) {
                return None;
            }
            m.run_geom(e).map(|g| (id, wire.clone(), g))
        })
        .collect();
    for (id, wire, geometry) in changes {
        if wire.feeder.is_some() || wire.source.is_some() || !wire.devices.is_empty() {
            return Err(bad("split", "disconnect the equipment from the cable inside this carrier before splitting"));
        }
        let start = project(geometry.a);
        let end = project(geometry.b);
        let map = |point: P3, t: f64, part| {
            let (g, local) = if part == first { (a, t / fraction) } else { (b, (t - fraction) / (1.0 - fraction)) };
            let lateral = sub(point, add(old.a, mul(direction, t)));
            add(add(g.a, mul(sub(g.b, g.a), local)), lateral)
        };
        let make = |part, from: P3, to: P3| {
            let mut wire = wire.clone();
            let base = m.elevation(wire.level);
            wire.start = P2(from[0], from[1]);
            wire.end = P2(to[0], to[1]);
            wire.offset = from[2] - base;
            wire.end_offset = Some(to[2] - base);
            wire.justify = crate::model::Justify::Centre;
            wire.mount = None;
            wire.carrier = Some(part);
            Element::Wire(wire)
        };
        let first_part = if start.min(end) < fraction { first } else { second };
        let (mut edited, extra) = if start.min(end) < fraction && fraction < start.max(end) {
            let gap = sub(a.b, b.a);
            if dot(gap, gap) > TOL * TOL {
                return Err(bad("split", "use centre justification so the carrier axes meet at the bend"));
            }
            let part = if start < end { first } else { second };
            let next = if part == first { second } else { first };
            let cable_fraction = (fraction - start) / (end - start);
            let at = add(geometry.a, mul(sub(geometry.b, geometry.a), cable_fraction));
            (
                make(part, map(geometry.a, start, part), map(at, fraction, part)),
                Some((make(next, map(at, fraction, next), map(geometry.b, end, next)), cable_fraction)),
            )
        } else {
            (make(first_part, map(geometry.a, start, first_part), map(geometry.b, end, first_part)), None)
        };
        if let Some((mut extra, cable_fraction)) = extra {
            check_split_bend(m, &edited, &extra)?;
            let (Element::Wire(first), Element::Wire(second)) = (&mut edited, &mut extra) else {
                return Err(bad("split", "expected two cable parts"));
            };
            let guid = m.guid(id);
            second.connections.clear();
            let transferred = split_wire_connections(m, guid, first, second, geometry, cable_fraction);
            let extra = m.insert(extra);
            let extra_guid = m.guid(extra);
            transfer_feeders(m, guid, extra_guid, &transferred);
            split_electrical_segment(m, guid, extra_guid)?;
        }
        m.elements.insert(id, edited);
    }
    Ok(())
}

/// Detail line `id` split where it passes nearest `at`, or a closed one opened there.
pub(crate) fn wire_loses_all_devices(w: &crate::model::Run, gone: &BTreeSet<Guid>) -> bool {
    (w.source.is_some() || !w.devices.is_empty()) && w.source.into_iter().chain(w.devices.iter().copied()).all(|g| gone.contains(&g))
}

pub(crate) fn electrical_after_delete(e: &Element, gone: &BTreeSet<Guid>) -> Option<Element> {
    if let Element::Link(link) = e
        && e.refs().iter().any(|id| gone.contains(id))
    {
        let mut link = link.clone();
        link.detector_groups.retain_mut(|group| {
            group.ids.retain(|id| !gone.contains(id));
            !group.ids.is_empty()
                && !gone.contains(&group.room)
                && !group.space.is_some_and(|bounds| bounds.iter().any(|bound| matches!(bound, IfcSlab::Local(id) if gone.contains(id))))
        });
        return Some(Element::Link(link));
    }
    if let Element::FireZone(zone) = e
        && zone.include_rooms.iter().chain(&zone.exclude_rooms).chain(&zone.notify).any(|g| gone.contains(g))
    {
        let mut zone = zone.clone();
        zone.include_rooms.retain(|g| !gone.contains(g));
        zone.exclude_rooms.retain(|g| !gone.contains(g));
        zone.notify.retain(|g| !gone.contains(g));
        return Some(Element::FireZone(zone));
    }
    if let Element::ElectricalLine(line) = e
        && line
            .members()
            .chain(line.segments.iter().copied())
            .chain(line.terminals.iter().copied())
            .chain(line.isolators.iter().copied())
            .any(|g| gone.contains(&g))
    {
        let mut line = line.clone();
        let mut predecessors = BTreeMap::new();
        for (start, devices) in
            std::iter::once((line.source, &line.devices)).chain(line.branches.iter().map(|branch| (branch.from, &branch.devices)))
        {
            predecessors.extend(
                std::iter::once(start).chain(devices.iter().copied()).zip(devices.iter().copied()).map(|(parent, child)| (child, parent)),
            );
        }
        line.devices.retain(|g| !gone.contains(g));
        for branch in &mut line.branches {
            while gone.contains(&branch.from) {
                branch.from = predecessors.get(&branch.from).copied().unwrap_or(line.source);
            }
            branch.devices.retain(|g| !gone.contains(g));
        }
        line.branches.retain(|branch| !branch.devices.is_empty());
        line.segments.retain(|g| !gone.contains(g));
        line.isolators.retain(|g| !gone.contains(g));
        line.terminals.retain(|g| !gone.contains(g));
        line.locked_marks.retain(|entry| !gone.contains(&entry.device));
        return Some(Element::ElectricalLine(line));
    }
    // A wired run loses the components it names that go; the run itself stays while it has a place.
    if let Element::Wire(w) = e
        && w.source
            .into_iter()
            .chain(w.devices.iter().copied())
            .chain(w.carrier)
            .chain(w.connections.iter().flat_map(|connection| connection.carrier.into_iter().chain(connection.terminal)))
            .any(|g| gone.contains(&g))
    {
        let mut w = w.clone();
        if w.source.into_iter().any(|g| gone.contains(&g)) {
            w.source = None;
        }
        w.devices.retain(|d| !gone.contains(d));
        if w.source.is_none() || w.devices.is_empty() {
            w.source = None;
            w.devices.clear();
        }
        if w.carrier.is_some_and(|g| gone.contains(&g)) {
            w.carrier = None;
            for connection in &mut w.connections {
                if connection.carrier.is_none() {
                    connection.in_carrier = false;
                }
            }
        }
        for connection in &mut w.connections {
            if connection.terminal.is_some_and(|guid| gone.contains(&guid)) {
                connection.terminal = None;
            }
            if connection.carrier.is_some_and(|guid| gone.contains(&guid)) {
                connection.carrier = None;
                connection.in_carrier = false;
            }
        }
        return Some(Element::Wire(w));
    }
    None
}

pub(crate) const CABLE_CARRIER_CANDIDATES: Command = other(
    "cable_carrier_candidates",
    "Cable Carrier Candidates",
    "{carrier: id|GUID} lists unlinked whole wire segments inside the carrier's actual 3D span and section; never attaches silently; ambiguous is true for multiple candidates. Confirm a selected wire through modify {id: wire, carrier}",
    cable_carrier_candidates,
);
pub(crate) const CARRIER_SECTION: Command = edit(
    "carrier_section",
    bimcraft_i18n::ui_msg!("Carrier Type Properties"),
    &[],
    "{type: name|GUID|id, section: null|{enclosure?: blind_box|lidded_box|tray|ladder|conduit, clear_width?: mm, clear_depth?: mm}} replace measured cable-carrier product section; null clears it. Other product metadata and type identity remain unchanged; missing measurements mean unknown",
    carrier_section,
);
pub(crate) const CABLE_CARRIER_CHECK: Command = edit(
    "cable_carrier_check",
    "Check Cable Carrier",
    &[],
    "{mode?: warn|auto, fill_percent: number, profile?: project|ru_sp76_2016_max_fill, configure?: bool, default_type?: name|GUID|null} omitted profile inherits saved policy or project; configure persists policy; normative box maximum uses measured clear section (not complete compliance); check peak simultaneous cable sections; auto selects the smallest sufficient type of the same known manufacturer; percentage is explicit, not an assumed regulation",
    cable_carrier_check,
);
pub(crate) const CABLE_CARRIER: Command = edit(
    "cable_carrier",
    ui_msg!("Cable Carrier"),
    &["CC"],
    "{points: [[x,y],…], closed?, type?, level?, offset?, end_offset?, justify?: top|centre|bottom, system?} a cable carrier \
         (КНС) for each segment of the points, as `pipe` lays them: round types write as an IFC conduit segment, rectangular \
         types as an IFC cable tray segment; `system` names the cable group it carries. Alternatively \
         {from_wire: id|GUID, type?, placement?: centre|top|bottom, local_offset?: mm} creates a carrier around the wire's \
         physical axis and links that wire in one undo step, preserving the wire's heights and slope. A mounted wire transfers \
         its support to the carrier and requires centre placement and zero lateral offset. Placement and lateral \
         offset must fit the section; already linked wires and owned equipment feeders are refused. Use mount: {host: id|GUID, face} with points to create a supported contact line. {from_wires: {level?: name|id|GUID}, type?: carrier type name} applies centre placement to all uncarried main wires on the given level, or throughout the project if level is omitted",
    cable_carrier,
);
pub(crate) const WIRE: Command = edit(
    "wire",
    ui_msg!("Wire"),
    &["EW"],
    "{points: [[x,y],…], closed?, type?, level?, offset?, end_offset?, justify?: top|centre|bottom, system?, source?, devices?: [id,…], cable?, reserve?, along_walls?, carrier?, local_offset?} a wire (the one \
         physical and logical cable) for each segment of the points, as `pipe` lays them: round, `width` its outside diameter; `system` \
         names its circuit or shield; `source` and `devices` give the panel-to-device loop it carries; `carrier` is the cable carrier it runs in. \
         Optional `start_connection: {device: id|GUID, point: [x,y,z]}` connects equipment at a local point to the first segment, `end_connection` likewise to the last segment's far end; mount: {host: id|GUID, face} defines a supported contact line as for cable_carrier",
    wire,
);
pub(crate) const ELECTRICAL_TERMINAL: Command = edit(
    "electrical_terminal",
    "Electrical Terminal",
    &[],
    "{device:id|GUID, name:string, purpose?:string, address?:string, load_ma?:number|null, limit_ma?:number|null} add a logical terminal; give id instead of device to edit supplied fields",
    electrical_terminal,
);
pub(crate) const ELECTRICAL_LINE: Command = edit(
    "electrical_line",
    "Electrical Line",
    &[],
    "{source:id|GUID, devices:[id|GUID,…], branches?:[{from:id|GUID,devices:[id|GUID,…]}], segments?:[wire id|GUID,…], terminals?:[terminal id|GUID,…], system?:string, profile?:sps|soue_light|soue_sound|soue_voice|sot|skud|ots|mgn, mark_prefix?:string} create a line from its source and ordered devices",
    electrical_line,
);
pub(crate) const ELECTRICAL_LINE_EDIT: Command = edit(
    "electrical_line_edit",
    "Edit Electrical Line",
    &[],
    "{id:id|GUID, devices?:[id|GUID,…], branches?:[{from:id|GUID,devices:[id|GUID,…]}], segments?:[wire id|GUID,…], terminals?:[terminal id|GUID,…], system?:string, profile?:sps|soue_light|soue_sound|soue_voice|sot|skud|ots|mgn, mark_prefix?:string} update the supplied properties and recalculate automatic marks",
    electrical_line_edit,
);
pub(crate) const ELECTRICAL_LINE_INSPECT: Command = other(
    "electrical_line_inspect",
    "Inspect Electrical Line",
    "{id:id|GUID} read the line, ordered devices and their marks",
    electrical_line_inspect,
);
pub(crate) const FIRE_LINE_FAULTS: Command = other(
    "fire_line_faults",
    "Fire Line Faults",
    "{id:line id|GUID, isolators?:[device id|GUID,…], source_isolates_ports?:bool, propose?:bool, allow_pair?:bool, max_isolators?:1..4, power?:{id?:separate power line id|GUID, lines?:[{id,source_isolates_ports?},…], independent?:[device id|GUID,…], source_isolates_ports?:bool}} simulate open and short faults under SP 484 Amendment 1, 6.3.5. Supply either one power id or lines. Lines can feed other lines through their sources. Use independent only for confirmed separately powered communication devices. With propose, test locations for one communication isolator; allow_pair also tests two distinct cable splits if one does not pass, up to 24 candidate cables. max_isolators enables sets of three or four distinct cable splits with up to 16 candidates. Set source_isolates_ports only for independently protected and feeding source terminals",
    fire_line_faults,
);
pub(crate) const FIRE_ISOLATOR: Command = edit(
    "fire_isolator",
    "Place Fire Line Isolator",
    &[],
    "{id:line id|GUID, wire:main wire id|GUID, family:string, type?:string, fraction?:number, apply?:bool} preview a standalone short-circuit isolator (default midway between connections). Apply only for a verified bidirectional isolator family and type",
    fire_isolator,
);
pub(crate) const CABLE_JOURNAL: Command = other(
    "cable_journal",
    "Cable Journal",
    "{id:line id|GUID, mode?:segments|line} measured cable lengths in mm including owned leads and reserve",
    cable_journal,
);
pub(crate) const ELECTRICAL_LINE_MARK: Command = edit(
    "electrical_line_mark",
    "Lock Electrical Line Mark",
    &[],
    "{id:id|GUID, device:id|GUID, mark:string|null} lock a unique mark for a device on the line; null restores automatic numbering",
    electrical_line_mark,
);
pub(crate) const ELECTRICAL_MARK_TEMPLATE: Command = edit(
    "electrical_mark_template",
    "Electrical Mark Template",
    &[],
    "{system:string, prefix:string|null} set or remove a system's default mark prefix",
    electrical_mark_template,
);
pub(crate) const ELECTRICAL_LINE_RENUMBER: Command = edit(
    "electrical_line_renumber",
    "Renumber Electrical Line",
    &[],
    "{id:id|GUID, mark_prefix?:string} compute marks in current traversal order; an optional new prefix changes automatic marks and keeps locked marks",
    electrical_line_renumber,
);
pub(crate) const WIRE_CONNECT: Command = edit(
    "wire_connect",
    ui_msg!("Connect Wire"),
    &[],
    "{wire: id|GUID, device: id|GUID, end?: 0|1, point?: [x,y,z], surface?: {host: id|GUID, face: interior|exterior}, carrier?: id|GUID|null, in_carrier?: bool, min_bend_radius?: mm|null, terminal?: id|GUID|null} choose exactly one of a fixed local point or a free wall-surface contact. Free contacts slide on the device connection_sides (bottom/top/right/left, empty means any), independent of cable diameter; the physical cable is tangent to the wall. end defaults to 0 and distinguishes returning cable ends. An explicit carrier selects this end's route independently; omitted/null uses the main wire route. Deleting an explicit carrier preserves the last landing and detaches its carrier branches; reconnect to choose a route again. Creates and updates physical feeder legs without duplicating them. A free surface lead follows the wall with a vertical transition when needed. min_bend_radius checks space for the transition and at the route entry. Reconnecting a free end keeps the previous radius if omitted; null clears it",
    wire_connect,
);

/// An electrical line's connectivity, independent of physical wire geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricalLine {
    pub source: Guid,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<super::systems::System>,
    pub devices: Vec<Guid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<ElectricalBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<Guid>,
    /// Confirmed standalone short-circuit isolators, separate from the addressed device order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolators: Vec<Guid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<Guid>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mark_prefix: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_marks: Vec<ElectricalMark>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricalBranch {
    pub from: Guid,
    pub devices: Vec<Guid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricalTerminal {
    pub device: Guid,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_ma: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_ma: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricalMark {
    pub device: Guid,
    pub mark: String,
}

impl ElectricalLine {
    pub fn system_profile(&self) -> Option<super::systems::System> {
        self.profile.or_else(|| super::systems::System::named(&self.system))
    }

    pub(crate) fn section(&self) -> &str {
        match self.system_profile() {
            Some(profile) => profile.section(),
            None => super::systems::section_for_name(&self.system),
        }
    }

    pub fn members(&self) -> impl Iterator<Item = Guid> + '_ {
        std::iter::once(self.source)
            .chain(self.devices.iter().copied())
            .chain(self.branches.iter().flat_map(|branch| branch.devices.iter().copied()))
    }

    pub fn order(&self, model: &Model) -> Vec<Guid> {
        let at = |guid| model.anchor(guid).unwrap_or(P2(0.0, 0.0));
        let mut continuation = BTreeMap::new();
        let mut branches = BTreeMap::<_, Vec<_>>::new();
        continuation.extend(std::iter::once(self.source).chain(self.devices.iter().copied()).zip(self.devices.iter().copied()));
        for ElectricalBranch { from, devices } in &self.branches {
            continuation.extend(devices.windows(2).map(|pair| (pair[0], pair[1])));
            if let Some(first) = devices.first() {
                let length = at(*from).dist(at(*first)) + devices.windows(2).map(|pair| at(pair[0]).dist(at(pair[1]))).sum::<f64>();
                let delta = at(*first) - at(*from);
                let angle = (-delta.1.atan2(delta.0)).rem_euclid(std::f64::consts::TAU);
                branches.entry(*from).or_default().push((length, angle, *first));
            }
        }
        let mut order = Vec::new();
        let mut stack = vec![self.source];
        let mut seen = BTreeSet::new();
        while let Some(device) = stack.pop() {
            if !seen.insert(device) {
                continue;
            }
            order.push(device);
            stack.extend(continuation.get(&device).copied());
            if let Some(mut children) = branches.remove(&device) {
                children.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
                stack.extend(children.into_iter().rev().map(|(_, _, guid)| guid));
            }
        }
        order
    }

    pub fn marks(&self, model: &Model) -> Vec<String> {
        let locked: BTreeMap<_, _> = self.locked_marks.iter().map(|entry| (entry.device, entry.mark.as_str())).collect();
        let reserved: BTreeSet<_> = locked.values().copied().collect();
        let prefix = if self.mark_prefix.is_empty() {
            model
                .electrical_mark_templates
                .get(&self.system)
                .map_or_else(|| self.profile.map_or("", super::systems::System::mark_prefix), String::as_str)
        } else {
            &self.mark_prefix
        };
        let mut automatic = (1..).map(|number| format!("{prefix}{number}")).filter(|mark| !reserved.contains(mark.as_str()));
        self.order(model)
            .into_iter()
            .map(|device| locked.get(&device).map_or_else(|| automatic.next().unwrap_or_default(), |mark| (*mark).to_owned()))
            .collect()
    }
}

pub(super) fn check(m: &Model) -> Result<(), String> {
    m.check_wire_connections()?;
    if m.electrical_mark_templates.len() > MAX_POINTS {
        return Err("Too many electrical mark templates".into());
    }
    for (system, prefix) in &m.electrical_mark_templates {
        text("Electrical mark template", "system", system)?;
        text("Electrical mark template", "prefix", prefix)?;
        if system.trim().is_empty() {
            return Err("Electrical mark template needs a system name".into());
        }
    }
    for element in m.elements.values() {
        let Element::ElectricalLine(line) = element else { continue };
        let mut devices = BTreeSet::new();
        if line.branches.len() > MAX_POINTS
            || line.branches.iter().any(|branch| branch.devices.is_empty())
            || line.members().skip(1).take(MAX_POINTS + 1).count() > MAX_POINTS
            || line.members().any(|g| !devices.insert(g) || !matches!(m.elements.get(g), Some(Element::Component(c)) if c.is_placed()))
        {
            return Err("Electrical line needs distinct placed components as source and devices".into());
        }
        if line.order(m).len() != devices.len() {
            return Err("Electrical line branches must connect to the source without cycles".into());
        }
        if line.isolators.len() > MAX_POINTS
            || line
                .isolators
                .iter()
                .any(|&g| !devices.insert(g) || !matches!(m.elements.get(g), Some(Element::Component(c)) if c.is_placed()))
        {
            return Err("Electrical line standalone isolators must be distinct placed components outside the addressed device order".into());
        }
        let mut segments = BTreeSet::new();
        if line.segments.len() > MAX_POINTS
            || line
                .segments
                .iter()
                .any(|g| !segments.insert(*g) || !matches!(m.elements.get(*g), Some(Element::Wire(w)) if w.feeder.is_none()))
        {
            return Err("Electrical line segments must be distinct main wires".into());
        }
        let mut terminals = BTreeSet::new();
        if line.terminals.len() > MAX_POINTS
            || line.terminals.iter().any(|g| {
                !terminals.insert(*g) || !matches!(m.elements.get(*g), Some(Element::ElectricalTerminal(t)) if devices.contains(&t.device))
            })
        {
            return Err("Electrical line terminals must be distinct and belong to devices on the line".into());
        }
        text("Electrical line", "mark_prefix", &line.mark_prefix)?;
        text("Electrical line", "system", &line.system)?;
        let mut marked_devices = BTreeSet::new();
        let mut marks = BTreeSet::new();
        for entry in &line.locked_marks {
            text("Electrical line", "locked mark", &entry.mark)?;
            if entry.mark.trim().is_empty()
                || !devices.contains(&entry.device)
                || line.isolators.contains(&entry.device)
                || !marked_devices.insert(entry.device)
                || !marks.insert(&entry.mark)
            {
                return Err("Electrical line locked marks must be nonempty and unique, with one per device on the line".into());
            }
        }
    }
    Ok(())
}

pub(super) fn check_terminal(model: &Model, who: &str, t: &ElectricalTerminal) -> Result<(), String> {
    text(who, "name", &t.name)?;
    text(who, "purpose", &t.purpose)?;
    text(who, "address", &t.address)?;
    for current in [t.load_ma, t.limit_ma].into_iter().flatten() {
        if !current.is_finite() || !(0.0..=MAX_COORD).contains(&current) {
            return Err(format!("{who}: terminal currents must be finite nonnegative milliamps"));
        }
    }
    if t.name.trim().is_empty() || !matches!(model.elements.get(t.device), Some(Element::Component(c)) if c.is_placed()) {
        return Err(format!("{who}: a terminal needs a name and a placed component"));
    }
    Ok(())
}

macro_rules! visit_line {
    ($line:expr, $f:expr, $($m:tt)?) => {{
        let line = $line;
        let f = $f;
                for terminal in & $($m)? line.terminals { f(terminal); }
                f(& $($m)? line.source);
                for device in & $($m)? line.devices { f(device); }
                for branch in & $($m)? line.branches {
                    f(& $($m)? branch.from);
                    for device in & $($m)? branch.devices { f(device); }
                }
                for segment in & $($m)? line.segments { f(segment); }
                for isolator in & $($m)? line.isolators { f(isolator); }
                for entry in & $($m)? line.locked_marks { f(& $($m)? entry.device); }
    }};
}
pub(crate) use visit_line;
