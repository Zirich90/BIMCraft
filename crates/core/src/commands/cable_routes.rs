use super::{
    BTreeSet, Element, Guid, Id, MAX_POINTS, Model, P2, R, Session, Value, bad, element_id, ids_param, json, level_param, only,
    refuse_pinned, string, type_param,
};

pub(super) fn cable_ports(s: &mut Session, p: &Value) -> R {
    const C: &str = "cable_ports";
    only(C, p, &["id", "ends"])?;
    let m = &s.model;
    let id = element_id(C, m, &p["id"])?;
    let Some(e @ Element::Cable(r)) = m.elements.get(id) else {
        return Err(bad(C, format!("{id} is not a data cable")));
    };
    if let Some(ends) = p.get("ends") {
        refuse_pinned(C, m, &[id], "attach")?;
        let mut ports: [Option<crate::runs::CablePort>; 2] =
            serde_json::from_value(ends.clone()).map_err(|e| bad(C, format!("`ends`: {e}")))?;
        for end in ports.iter_mut().flatten() {
            end.component = m.elements.guid(end.component).ok_or_else(|| bad(C, format!("no component {}", end.component)))?;
        }
        let mut r = r.clone();
        if !crate::runs::no_ports(&r.ports) {
            let g = m.run_geom(e).ok_or_else(|| bad(C, format!("{id} has no cable geometry")))?;
            r.keep_centreline(g, m.elevation(r.level));
        }
        r.ports = ports;
        let reply = cable_ports_json(m, id, &r);
        s.model_mut().elements.insert(id, Element::Cable(r));
        return Ok(reply);
    }
    Ok(cable_ports_json(m, id, r))
}

fn cable_ports_json(m: &Model, id: Id, r: &crate::model::Run) -> Value {
    let ends = r.ports.map(|end| {
        end.map(|end| {
            json!({
                "component": end.component, "port": end.port, "at": end.position(m)
            })
        })
    });
    json!({"id": id, "ends": ends})
}

pub(super) fn cable_carrier(s: &mut Session, p: &Value) -> R {
    const C: &str = "cable_carrier";
    only(C, p, &["id", "span"])?;
    let m = &s.model;
    let id = element_id(C, m, &p["id"])?;
    let Some(e @ Element::Cable(r)) = m.elements.get(id) else {
        return Err(bad(C, format!("{id} is not a data cable")));
    };
    let ty = m.run_type(e).ok_or_else(|| bad(C, format!("{id} has no cable type")))?;
    let report = |r: &crate::model::Run| {
        let g = crate::runs::Geom::of(m, r, ty);
        json!({"id":id,"span":r.carrier_span,"axis":[g.a,g.b],"length":g.length()})
    };
    let Some(span) = p.get("span") else { return Ok(report(r)) };
    refuse_pinned(C, m, &[id], "change its carrier")?;
    let mut span: Option<crate::runs::CarrierSpan> = serde_json::from_value(span.clone()).map_err(|e| bad(C, format!("`span`: {e}")))?;
    if let Some(span) = &mut span {
        span.carrier = m.elements.guid(span.carrier).ok_or_else(|| bad(C, format!("no carrier {}", span.carrier)))?;
    }
    let mut r = r.clone();
    if r.carrier_span.is_some() {
        r.keep_centreline(crate::runs::Geom::of(m, &r, ty), m.elevation(r.level));
    }
    r.carrier_span = span;
    let reply = report(&r);
    s.model_mut().elements.insert(id, Element::Cable(r));
    Ok(reply)
}

fn cable_gap(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2])
}

pub(super) fn cable_route_check(s: &mut Session, p: &Value) -> R {
    const C: &str = "cable_route_check";
    only(C, p, &["ids"])?;
    let m = &s.model;
    let ids = ids_param(C, m, p)?;
    if ids.is_empty() || p["ids"].as_array().is_some_and(|values| values.len() != ids.len()) {
        return Err(bad(C, "`ids` must list at least one segment and each segment only once"));
    }
    let mut previous = None;
    let mut gaps = Vec::new();
    let mut length = 0.0;
    for id in ids {
        let Some(e @ Element::Cable(r)) = m.elements.get(id) else {
            return Err(bad(C, format!("{id} is not a data cable")));
        };
        let ty = m.run_type(e).ok_or_else(|| bad(C, format!("{id} has no cable type")))?;
        let axis = crate::runs::Geom::of(m, r, ty);
        if let Some((from, end)) = previous {
            let distance = cable_gap(end, axis.a);
            if distance > crate::model::TOL {
                gaps.push(json!({"from":from, "to":id, "distance":distance}));
            }
        }
        previous = Some((id, axis.b));
        length += axis.length();
    }
    Ok(json!({"connected":gaps.is_empty(), "gaps":gaps, "length":length}))
}

pub(super) fn cable_route(s: &mut Session, p: &Value) -> R {
    const C: &str = "cable_route";
    only(C, p, &["spans", "ends", "type", "level", "system"])?;
    let spans = p["spans"]
        .as_array()
        .filter(|spans| (1..MAX_POINTS).contains(&spans.len()))
        .ok_or_else(|| bad(C, format!("`spans` needs 1 to {} carrier spans", MAX_POINTS - 1)))?;
    let m = &s.model;
    let level = level_param(C, m, p, "level")?;
    let base = m.elevation(level);
    let type_name = type_param(C, p, m, "cable")?;
    let system = string(C, p, "system")?.unwrap_or_default();
    let mut ports: [Option<crate::runs::CablePort>; 2] = match p.get("ends") {
        Some(ends) => serde_json::from_value(ends.clone()).map_err(|e| bad(C, format!("`ends`: {e}")))?,
        None => [None, None],
    };
    if spans.len() + ports.iter().flatten().count() > MAX_POINTS {
        return Err(bad(C, format!("a route takes at most {MAX_POINTS} segments including leads")));
    }
    for port in ports.iter_mut().flatten() {
        port.component = m.elements.guid(port.component).ok_or_else(|| bad(C, format!("no component {}", port.component)))?;
    }
    let mut segments = Vec::with_capacity(spans.len() + 2);
    let mut previous: Option<[f64; 3]> = None;
    let mut length = 0.0;
    let make_run = |a: [f64; 3], b: [f64; 3], carrier_span, ports| {
        Element::Cable(crate::model::Run {
            type_name: type_name.clone(),
            level,
            start: P2(a[0], a[1]),
            end: P2(b[0], b[1]),
            offset: a[2] - base,
            end_offset: Some(b[2] - base).filter(|z| *z != a[2] - base),
            ports,
            carrier_span,
            justify: crate::model::Justify::Centre,
            join: true,
            system: system.clone(),
            params: Default::default(),
        })
    };
    let lead = |index: usize, joint: [f64; 3]| -> Result<Option<(Element, f64)>, String> {
        let Some(port) = ports[index] else { return Ok(None) };
        let at = port.position(m).ok_or_else(|| bad(C, format!("end {} needs a placed data port", index + 1)))?;
        let distance = cable_gap(at, joint);
        if distance <= crate::model::TOL {
            return Err(bad(C, format!("end {} needs more than {} mm for a separate lead", index + 1, crate::model::TOL)));
        }
        let (a, b) = if index == 0 { (at, joint) } else { (joint, at) };
        let mut ends = [None, None];
        ends[index] = Some(port);
        Ok(Some((make_run(a, b, None, ends), distance)))
    };
    for (index, value) in spans.iter().enumerate() {
        let label = format!("span {}", index + 1);
        let mut span: crate::runs::CarrierSpan = serde_json::from_value(value.clone()).map_err(|e| bad(C, format!("{label}: {e}")))?;
        span.carrier = m.elements.guid(span.carrier).ok_or_else(|| bad(C, format!("{label}: unknown carrier {}", span.carrier)))?;
        if !span.fractions_in_range() {
            return Err(bad(C, format!("{label}: carrier span fractions must be between 0 and 1")));
        }
        let [a, b] = span.axis(m).ok_or_else(|| bad(C, format!("{label}: select an existing conduit or cable tray")))?;
        if previous.is_some_and(|end| cable_gap(end, a) > crate::model::TOL) {
            return Err(bad(C, format!("{label} must meet the previous span in 3D")));
        }
        if index == 0
            && let Some((segment, distance)) = lead(0, a)?
        {
            segments.push(segment);
            length += distance;
        }
        previous = Some(b);
        length += cable_gap(a, b);
        segments.push(make_run(a, b, Some(span), [None, None]));
    }
    if let Some(joint) = previous
        && let Some((segment, distance)) = lead(1, joint)?
    {
        segments.push(segment);
        length += distance;
    }
    let m = s.model_mut();
    let ids: Vec<Id> = segments.into_iter().map(|e| m.insert(e)).collect();
    Ok(json!({"ids":ids,"length":length}))
}

pub(super) fn detached_cables(m: &Model, candidates: BTreeSet<Id>, gone: &BTreeSet<Guid>) -> Vec<(Id, crate::model::Run)> {
    candidates
        .into_iter()
        .filter(|id| !gone.contains(&m.guid(*id)))
        .filter_map(|id| {
            let e @ Element::Cable(r) = m.elements.get(id)? else { return None };
            let carrier_gone = r.carrier_span.is_some_and(|span| gone.contains(&span.carrier));
            if !carrier_gone && !r.ports.iter().flatten().any(|p| gone.contains(&p.component)) {
                return None;
            }
            let mut r = r.clone();
            r.keep_centreline(m.run_geom(e)?, m.elevation(r.level));
            if carrier_gone {
                r.carrier_span = None;
            }
            for p in &mut r.ports {
                *p = p.filter(|p| !gone.contains(&p.component));
            }
            Some((id, r))
        })
        .collect()
}
