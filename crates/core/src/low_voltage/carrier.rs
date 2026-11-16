use crate::geom::P2;
use crate::model::{Element, Guid, Justify, MAX_POINTS, Model, Run, RunType, TOL, coord, size, text};
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    /// Peak simultaneous cable section along each carrier, compared with its gross section.
    /// Regulatory limits and internal usable dimensions require an explicit profile.
    pub fn carrier_sections(&self) -> BTreeMap<Guid, f64> {
        let area = |t: &RunType| if t.round { std::f64::consts::PI * t.width * t.width / 4.0 } else { t.width * t.depth };
        let mut events: BTreeMap<Guid, Vec<(f64, f64)>> = BTreeMap::new();
        for e in self.elements.values() {
            let Element::Wire(wire) = e else { continue };
            let Some(carrier) = wire.carrier else { continue };
            let Some(host @ Element::CableCarrier(_)) = self.elements.get(carrier) else { continue };
            let (Some(h), Some(w), Some(t)) = (self.run_geom(host), self.run_geom(e), self.run_type(e)) else { continue };
            let axis = [h.b[0] - h.a[0], h.b[1] - h.a[1], h.b[2] - h.a[2]];
            let length2: f64 = axis.iter().map(|v| v * v).sum();
            if length2 <= TOL * TOL {
                continue;
            }
            let project = |p: [f64; 3]| (0..3).map(|i| (p[i] - h.a[i]) * axis[i]).sum::<f64>() / length2;
            let (a, b) = (project(w.a), project(w.b));
            let (lo, hi) = (a.min(b).max(0.0), a.max(b).min(1.0));
            if hi <= lo {
                continue;
            }
            let section = area(t);
            let list = events.entry(carrier).or_default();
            list.push((lo, section));
            list.push((hi, -section));
        }
        let mut peaks = BTreeMap::new();
        for (carrier, mut list) in events {
            list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let (mut current, mut peak) = (0.0_f64, 0.0_f64);
            for (_, delta) in list {
                current += delta;
                peak = peak.max(current);
            }
            peaks.insert(carrier, peak);
        }
        peaks
    }

    /// Follow a mounted carrier's physical frame, or a legacy carrier's type-height shift.
    /// Explicit cable edits already moved themselves and take precedence.
    pub(crate) fn follow_carrier_type_changes(&mut self, before: &Self) {
        let types_changed = self.types.runs.get("cablecarrier") != before.types.runs.get("cablecarrier");
        let mut moved = Vec::new();
        for (&id, element) in &self.elements {
            let Element::Wire(wire) = element else { continue };
            if before.elements.get(id) != Some(element) {
                continue;
            }
            let Some(host) = wire.carrier.and_then(|id| self.elements.get(id)) else { continue };
            let Some(old_host) = wire.carrier.and_then(|id| before.elements.get(id)) else { continue };
            if host.run().is_some_and(|r| r.mount.is_some()) {
                let (Some(old), Some(new), Some(old_wire)) = (before.run_geom(old_host), self.run_geom(host), before.run_geom(element))
                else {
                    continue;
                };
                if old.a == new.a && old.b == new.b && old.frame() == new.frame() {
                    continue;
                }
                let (oa, ox, oy) = old.frame();
                let (na, nx, ny) = new.frame();
                let vector = |p| {
                    use crate::view3d::{add, dot, mul};
                    add(add(mul(na, dot(p, oa)), mul(nx, dot(p, ox))), mul(ny, dot(p, oy)))
                };
                let map = |p| crate::view3d::add(new.a, vector(crate::view3d::sub(p, old.a)));
                let (a, b) = (map(old_wire.a), map(old_wire.b));
                let mut updated = wire.clone();
                updated.start = P2(a[0], a[1]);
                updated.end = P2(b[0], b[1]);
                updated.offset = a[2] - self.elevation(wire.level);
                updated.end_offset = Some(b[2] - self.elevation(wire.level));
                updated.justify = Justify::Centre;
                let depth = vector(old_wire.frame().2);
                let (_, x, y) = crate::runs::Geom { a, b, roll: 0.0, ..old_wire }.frame();
                updated.section_roll = (-crate::view3d::dot(depth, x)).atan2(crate::view3d::dot(depth, y)).to_degrees();
                moved.push((id, Element::Wire(updated)));
                continue;
            }
            if !types_changed {
                continue;
            }
            if host != old_host {
                continue;
            }
            let (Some(old), Some(new)) = (before.run_geom(old_host), self.run_geom(host)) else { continue };
            let dz = new.a[2] - old.a[2];
            if dz == 0.0 {
                continue;
            }
            let (Some(old_wire), Some(new_wire)) = (before.run_geom(element), self.run_geom(element)) else { continue };
            let shift = dz - (new_wire.a[2] - old_wire.a[2]);
            let mut wire = wire.clone();
            wire.offset += shift;
            if let Some(end) = &mut wire.end_offset {
                *end += shift;
            }
            moved.push((id, Element::Wire(wire)));
        }
        for (id, wire) in moved {
            self.elements.insert(id, wire);
        }
    }

    /// Cable envelope around each carrier axis: width, depth and enclosing diameter.
    /// Includes endpoint offsets and the cable's oriented section, not just its area.
    pub fn carrier_envelopes(&self) -> BTreeMap<Guid, [f64; 3]> {
        use crate::view3d::sub;
        let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
        let frame = |g: crate::runs::Geom| g.frame();
        let mut result: BTreeMap<Guid, [f64; 3]> = BTreeMap::new();
        for (_, _, e) in self.elements.with_guids() {
            let Element::Wire(wire) = e else { continue };
            let Some(guid) = wire.carrier else { continue };
            let Some(host) = self.elements.get(guid).and_then(|e| self.run_geom(e)) else { continue };
            let Some(cable) = self.run_geom(e) else { continue };
            let (_, x, y) = frame(host);
            let (axis, cx, cy) = frame(cable);
            let required = result.entry(guid).or_default();
            for end in [cable.a, cable.b] {
                let delta = sub(end, host.a);
                let (px, py) = (dot(delta, x), dot(delta, y));
                if cable.round {
                    let r = cable.width / 2.0;
                    required[0] = required[0].max(2.0 * (px.abs() + r * (1.0 - dot(x, axis).powi(2)).max(0.0).sqrt()));
                    required[1] = required[1].max(2.0 * (py.abs() + r * (1.0 - dot(y, axis).powi(2)).max(0.0).sqrt()));
                    required[2] = required[2].max(2.0 * (px.hypot(py) + r));
                } else {
                    for a in [-0.5, 0.5] {
                        for b in [-0.5, 0.5] {
                            let vx = px + a * cable.width * dot(cx, x) + b * cable.depth * dot(cy, x);
                            let vy = py + a * cable.width * dot(cx, y) + b * cable.depth * dot(cy, y);
                            required[0] = required[0].max(2.0 * vx.abs());
                            required[1] = required[1].max(2.0 * vy.abs());
                            required[2] = required[2].max(2.0 * vx.hypot(vy));
                        }
                    }
                }
            }
        }
        result
    }

    pub fn carrier_fits(t: &RunType, required: [f64; 3]) -> bool {
        if t.round { t.width + TOL >= required[2] } else { t.width + TOL >= required[0] && t.depth + TOL >= required[1] }
    }

    /// Advisory per-compartment occupancy and explicitly configured system separation.
    pub fn carrier_compartment_warnings(&self, percent: f64) -> BTreeMap<Guid, Vec<String>> {
        use crate::view3d::sub;
        let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
        let mut warnings: BTreeMap<Guid, Vec<String>> = BTreeMap::new();
        let mut events: BTreeMap<(Guid, String), Vec<(f64, f64)>> = BTreeMap::new();
        let mut profiles = BTreeMap::<Guid, BTreeSet<_>>::new();
        for element in self.elements.values() {
            if let Element::ElectricalLine(line) = element
                && let Some(profile) = line.system_profile()
            {
                for &segment in &line.segments {
                    profiles.entry(segment).or_default().insert(profile);
                }
            }
        }
        for (_, wire_guid, e) in self.elements.with_guids() {
            let Element::Wire(wire) = e else { continue };
            let Some(guid) = wire.carrier else { continue };
            let Some(host @ Element::CableCarrier(run)) = self.elements.get(guid) else { continue };
            if run.compartments.is_empty() && wire.compartment.is_empty() {
                continue;
            }
            let Some(slot) = run.compartments.get(&wire.compartment) else {
                warnings.entry(guid).or_default().push(format!("cable carrier {guid}: assign the wire to an existing compartment"));
                continue;
            };
            let notes = warnings.entry(guid).or_default();
            let segment = wire.feeder.as_ref().map_or(wire_guid, |feeder| feeder.wire);
            let accepts = |profile| super::systems::allows_compartment(&slot.systems, &wire.system, profile);
            if !profiles.get(&segment).map_or_else(|| accepts(None), |profiles| profiles.iter().all(|&profile| accepts(Some(profile)))) {
                notes
                    .push(format!("cable carrier {guid}: compartment {} system separation conflict for {}", wire.compartment, wire.system));
            }
            let (Some(h), Some(w)) = (self.run_geom(host), self.run_geom(e)) else { continue };
            let (axis, x, y) = h.frame();
            let (wa, wx, wy) = w.frame();
            let support = |n| {
                if w.round {
                    w.width / 2.0 * (1.0 - dot(n, wa).powi(2)).max(0.0).sqrt()
                } else {
                    (w.width * dot(n, wx).abs() + w.depth * dot(n, wy).abs()) / 2.0
                }
            };
            if [w.a, w.b].iter().any(|p| {
                let d = sub(*p, h.a);
                (dot(d, x) - slot.offset).abs() + support(x) > slot.width / 2.0 + TOL
                    || dot(d, y).abs() + support(y) > slot.depth / 2.0 + TOL
            }) {
                notes.push(format!(
                    "cable carrier {guid}: compartment {} geometric conflict; cable envelope does not fit its usable section",
                    wire.compartment
                ));
            }
            let a = dot(sub(w.a, h.a), axis) / h.length();
            let b = dot(sub(w.b, h.a), axis) / h.length();
            let lo = a.min(b).max(0.0);
            let hi = a.max(b).min(1.0);
            if hi > lo {
                let area = if w.round { std::f64::consts::PI * w.width * w.width / 4.0 } else { w.width * w.depth };
                events.entry((guid, wire.compartment.clone())).or_default().extend([(lo, area), (hi, -area)]);
            }
        }
        for ((guid, name), mut list) in events {
            list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let (mut current, mut peak) = (0.0_f64, 0.0_f64);
            for (_, delta) in list {
                current += delta;
                peak = peak.max(current);
            }
            let Some(Element::CableCarrier(run)) = self.elements.get(guid) else { continue };
            let Some(slot) = run.compartments.get(&name) else { continue };
            if peak > slot.width * slot.depth * percent / 100.0 {
                warnings.entry(guid).or_default().push(format!(
                    "cable carrier {guid}: compartment {name} simultaneous cable section exceeds {percent}% of its usable section"
                ));
            }
        }
        warnings.retain(|_, notes| !notes.is_empty());
        warnings
    }

    /// Measured product section, kept with the catalogue type rather than a placed run.
    pub fn carrier_section(&self, name: &str) -> Option<&crate::carrier_limits::Section> {
        let id = self.library.id(&crate::library::Kind::Type("cablecarrier".into()), name)?;
        self.library.get(id)?.product.as_ref()?.carrier.as_ref()
    }

    pub fn carrier_capacity(&self, name: &str, profile: crate::carrier_limits::Profile, percent: f64) -> Result<f64, String> {
        let t = self.types.runs.get("cablecarrier").and_then(|types| types.get(name)).ok_or("carrier type is missing")?;
        let gross = if t.round { std::f64::consts::PI * t.width * t.width / 4.0 } else { t.width * t.depth };
        crate::carrier_limits::capacity(profile, percent, gross, self.carrier_section(name))
    }

    pub fn carrier_type_fits(&self, name: &str, required: [f64; 3], profile: crate::carrier_limits::Profile) -> bool {
        let Some(t) = self.types.runs.get("cablecarrier").and_then(|types| types.get(name)) else { return false };
        if !Self::carrier_fits(t, required) {
            return false;
        }
        if profile == crate::carrier_limits::Profile::Project {
            return true;
        }
        self.carrier_section(name).is_some_and(|section| {
            section.clear_width.is_some_and(|w| w + TOL >= required[0]) && section.clear_depth.is_some_and(|d| d + TOL >= required[1])
        })
    }

    pub(crate) fn carrier_compartments_fit(&self, name: &str, run: &Run, profile: crate::carrier_limits::Profile) -> bool {
        run.compartments.is_empty()
            || self.carrier_type_fits(
                name,
                run.compartments
                    .values()
                    .fold([0.0_f64; 3], |a, slot| [a[0].max(slot.width + 2.0 * slot.offset.abs()), a[1].max(slot.depth), 0.0]),
                profile,
            )
    }

    pub fn carrier_warnings(&self) -> Vec<String> {
        let profile = self.carrier_settings.as_ref().map_or(crate::carrier_limits::Profile::default(), |p| p.profile);
        let percent = self.carrier_settings.as_ref().map_or(100.0, |p| p.fill_percent);
        let mut warnings: Vec<_> = self.carrier_sections().into_iter().filter_map(|(carrier, peak)| {
            let Element::CableCarrier(run) = self.elements.get(carrier)? else { return None };
            match self.carrier_capacity(&run.type_name, profile, percent) {
                Err(error) => Some(format!("cable carrier {carrier}: {error}; capacity cannot be verified")),
                Ok(allowed) if peak > allowed => Some(format!("cable carrier {carrier}: simultaneous cable section {peak:.1} mm2 exceeds allowed section {allowed:.1} mm2; choose a larger carrier or smaller cables")),
                _ => None,
            }
        }).collect();
        for (guid, required) in self.carrier_envelopes() {
            let Some(Element::CableCarrier(run)) = self.elements.get(guid) else { continue };
            if !self.carrier_type_fits(&run.type_name, required, profile) {
                warnings.push(format!(
                    "cable carrier {guid}: geometric conflict; cable envelope needs {:.1} x {:.1} mm, diameter {:.1} mm",
                    required[0], required[1], required[2]
                ));
            }
        }
        for (id, element) in self.elements.iter() {
            if let Element::CableCarrier(run) = element
                && !self.carrier_compartments_fit(&run.type_name, run, profile)
            {
                warnings.push(format!(
                    "cable carrier {}: geometric conflict; measured compartment does not fit the carrier section",
                    self.guid(*id)
                ));
            }
        }
        warnings.extend(
            self.carrier_compartment_warnings(if profile == crate::carrier_limits::Profile::Project { percent } else { 100.0 })
                .into_values()
                .flatten(),
        );
        warnings
    }
}

pub(crate) fn check_run(m: &Model, who: &str, category: &str, r: &Run, t: &RunType) -> Result<(), String> {
    if r.compartments.len() > 32 || (!r.compartments.is_empty() && (category != "cablecarrier" || t.round)) {
        return Err(format!("{who}: at most 32 compartments belong to a rectangular cable carrier"));
    }
    text(who, "compartment", &r.compartment)?;
    if !r.compartment.is_empty() && category != "wire" {
        return Err(format!("{who}: only a wire names its compartment"));
    }
    for (name, slot) in &r.compartments {
        text(who, "compartment name", name)?;
        size(who, slot.width)?;
        size(who, slot.depth)?;
        coord(who, slot.offset)?;
        if name.trim().is_empty()
            || slot.systems.len() > 32
            || 2.0 * slot.offset.abs() + slot.width > t.width + TOL
            || slot.depth > t.depth + TOL
        {
            return Err(format!("{who}: named compartments must fit the carrier section and list at most 32 systems"));
        }
        for system in &slot.systems {
            text(who, "compartment system", system)?;
        }
    }
    let slots: Vec<_> = r.compartments.values().collect();
    for (i, a) in slots.iter().enumerate() {
        for b in &slots[i + 1..] {
            if (a.offset - b.offset).abs() + TOL < (a.width + b.width) / 2.0 {
                return Err(format!("{who}: compartments overlap; separate their usable sections"));
            }
        }
    }
    if r.size_locked && category != "cablecarrier" {
        return Err(format!("{who}: size_locked is only for cable carriers"));
    }
    let wire = category == "wire";
    if !wire && (r.source.is_some() || !r.devices.is_empty() || r.carrier.is_some()) {
        return Err(format!("{who}: only a wire carries its source, devices and carrier"));
    }
    coord(&format!("{who} local_offset"), r.local_offset)?;
    if let Some(g) = r.carrier {
        match m.elements.get(g) {
            Some(Element::CableCarrier(_)) => {}
            Some(other) => {
                return Err(format!("{who}: its carrier is a cable carrier, not {} {g}", other.category()));
            }
            None => {
                return Err(format!("{who}: element {g} doesn't exist"));
            }
        }
    }
    if !wire {
        return Ok(());
    }
    if r.devices.len() > MAX_POINTS {
        return Err(format!("{who}: a wire names at most {MAX_POINTS} devices"));
    }
    if !(r.reserve.is_finite() && (0.0..=100.0).contains(&r.reserve)) {
        return Err(format!("{who}: its reserve is a per cent from 0 to 100, not {}", r.reserve));
    }
    match (r.source, !r.devices.is_empty()) {
        (Some(source), true) => {
            match m.elements.get(source) {
                Some(Element::Component(c)) if c.is_placed() => {}
                Some(Element::Component(_)) => {
                    return Err(format!("{who}: source equipment must be placed before connecting"));
                }
                Some(other) => {
                    return Err(format!("{who}: its source is a device, a component, not {} {}", other.category(), source));
                }
                None => {
                    return Err(format!("{who}: element {source} doesn't exist"));
                }
            }
            let mut seen = BTreeSet::from([source]);
            for d in r.devices.iter().copied() {
                match m.elements.get(d) {
                    Some(Element::Component(c)) if c.is_placed() => {}
                    Some(Element::Component(_)) => {
                        return Err(format!("{who}: equipment {d} must be placed before connecting"));
                    }
                    Some(other) => {
                        return Err(format!("{who}: its device is a component, not {} {}", other.category(), d));
                    }
                    None => {
                        return Err(format!("{who}: element {d} doesn't exist"));
                    }
                }
                if !seen.insert(d) {
                    return Err(format!("{who}: element {d} is in it twice, or is its source"));
                }
            }
        }
        (Some(_), false) => {
            return Err(format!("{who}: a wire with a source reaches one device at least"));
        }
        (None, true) => {
            return Err(format!("{who}: a wire that reaches devices has a source"));
        }
        (None, false) => {}
    }
    Ok(())
}
