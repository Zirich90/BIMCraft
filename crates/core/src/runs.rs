//! Runs: the geometry of a straight element between two points, whatever it carries (a beam, a strip footing, a pipe, a
//! conduit, a cable tray, a wire, a conductor, a cable): the ends of its centre line, its footprint in plan, the prism or the
//! tube it makes in 3D. It knows no category but for the table that gives its IFC class and the defaults it takes (may it stand,
//! where its heights are); the plan, the section, the 3D view, the snaps and IFC ask it.

use crate::geom::{self, P2};
use crate::model::{Component, Element, Id, Justify, MAX_SLOPE, Model, Run, RunType, TOL, Wall, heading};
use crate::view3d::{P3, add, cross, dot, mul, sub, unit};
use crate::walls::{self, Axis, Plane};

mod connections;
mod device_exit;

pub(crate) use connections::bend_fits;

impl Model {
    /// Local height of the point used when fitting a component to a named face.
    pub fn component_contact_height(&self, component: &Component, face: &str) -> f64 {
        match face {
            "interior" | "exterior" => self
                .families
                .get(&component.family)
                .and_then(|family| family.values(&component.type_name, &component.params).ok())
                .and_then(|values| match values.get(self.language.word("Mounting Height")) {
                    Some(crate::family::Val::Number(height)) => Some(*height),
                    _ => None,
                })
                .unwrap_or_default(),
            "top" => self.component_local_pieces(component).iter().map(|p| p.bottom).min_by(f64::total_cmp).unwrap_or_default(),
            _ => self.component_local_pieces(component).iter().map(|p| p.top).max_by(f64::total_cmp).unwrap_or_default(),
        }
    }

    fn component_contact(&self, c: &Component, face: &str) -> P3 {
        [c.at.0, c.at.1, self.elevation(c.level) + c.offset + self.component_contact_height(c, face)]
    }

    pub(crate) fn follow_component_mounts(&mut self, before: &Self) -> Result<(), String> {
        let mut updates = Vec::new();
        for (id, element) in self.elements.iter() {
            let Element::Component(c) = element else { continue };
            let Some(mount) = &c.mount else { continue };
            let Some(Element::Component(old)) = before.elements.get(*id) else { continue };
            if old.mount != c.mount {
                continue;
            }
            let mut updated = c.clone();
            if self.elements.get(mount.host).is_none() && before.elements.get(mount.host).is_some() {
                updated.mount = None;
            } else {
                if c.at != old.at || c.offset != old.offset || c.level != old.level || c.angle != old.angle {
                    continue;
                }
                let previous = before.run_mount_frame(mount)?;
                let current = self.run_mount_frame(mount)?;
                let contact = before.component_contact(old, &mount.face);
                if previous == current && contact == self.component_contact(c, &mount.face) {
                    continue;
                }
                let delta = sub(contact, previous.0);
                let mapped = add(
                    add(current.0, mul(current.1, dot(delta, previous.1))),
                    mul(cross(current.2, current.1), dot(delta, cross(previous.2, previous.1))),
                );
                updated.at = P2(mapped[0], mapped[1]);
                updated.offset = mapped[2] - self.elevation(c.level) - self.component_contact_height(c, &mount.face);
                let angle = |axis: P3| axis[1].atan2(axis[0]).to_degrees();
                if previous.1 != current.1 {
                    updated.angle = (c.angle + angle(current.1) - angle(previous.1)).rem_euclid(360.0);
                }
            }
            if &updated != c {
                updates.push((*id, updated));
            }
        }
        for (id, updated) in updates {
            self.elements.insert(id, Element::Component(updated));
        }
        Ok(())
    }

    pub(crate) fn check_component_mounts(&self) -> Result<(), String> {
        let mut scene = None;
        for (id, element) in self.elements.iter() {
            let Element::Component(c) = element else { continue };
            let Some(mount) = &c.mount else { continue };
            let (origin, tangent, normal) = self.run_mount_frame(mount)?;
            let point = self.component_contact(c, &mount.face);
            let projected = sub(point, mul(normal, dot(sub(point, origin), normal)));
            let probe = mul(tangent, 0.001);
            if !c.is_placed()
                || dot(sub(point, origin), normal).abs() > 0.001
                || !scene.get_or_insert_with(|| crate::view3d::Scene::new(self)).supports_segment(
                    self.elements.id(mount.host).ok_or("mount host is missing")?,
                    &mount.face,
                    sub(projected, probe),
                    add(projected, probe),
                    normal,
                )?
            {
                return Err(format!("{id}: equipment contact must stay on its mounting face"));
            }
        }
        Ok(())
    }

    /// Origin on the named host face, unit tangent, outward normal. Curved
    /// walls need a different contact representation and cannot use a single plane.
    pub(crate) fn run_mount_frame(&self, mount: &crate::model::RunMount) -> Result<(P3, P3, P3), String> {
        let host = self.elements.get(mount.host).ok_or("mount host is missing")?;
        let slab = match host {
            crate::model::Element::Floor(f) => {
                let top = self.floor_top(f);
                Some((top, self.floor_depth(f), true, &f.boundary))
            }
            crate::model::Element::Ceiling(c) => {
                let under = Plane::flat(self.elevation(c.level) + c.offset);
                let thickness = self.types.ceiling.get(&c.type_name).map_or(0.0, |t| t.thickness);
                Some((under, thickness, false, &c.boundary))
            }
            _ => None,
        };
        if let Some((mut plane, thickness, is_top, boundary)) = slab {
            let sign = match mount.face.as_str() {
                "top" => 1.0,
                "bottom" => -1.0,
                _ => return Err("slab mount face must be top or bottom".into()),
            };
            if is_top && sign < 0.0 {
                plane.z -= thickness;
            }
            if !is_top && sign > 0.0 {
                plane.z += thickness;
            }
            let normal = mul(unit([-plane.g.0, -plane.g.1, 1.0]), sign);
            let at = *boundary.first().ok_or("mount host boundary is empty")?;
            let edge = boundary.iter().map(|p| *p - at).find(|d| d.dot(*d) > 1e-12).ok_or("mount host boundary has no direction")?;
            return Ok(([at.0, at.1, plane.at(at)], unit([edge.0, edge.1, plane.g.dot(edge)]), normal));
        }
        let crate::model::Element::Wall(wall) = host else {
            return Err("mount host must be a straight wall, floor or ceiling".into());
        };
        if wall.arc.abs() > 1e-9 {
            return Err("curved wall mounting is not supported by a planar contact line".into());
        }
        let along = (wall.end - wall.start).unit();
        let left = along.perp();
        let (exterior, interior) = crate::walls::faces(self, wall);
        let (offset, sign) = match mount.face.as_str() {
            "exterior" => (exterior, 1.0),
            "interior" => (interior, -1.0),
            _ => return Err("wall mount face must be exterior or interior".into()),
        };
        let at = wall.start + left * offset;
        Ok(([at.0, at.1, self.base_z(wall)], [along.0, along.1, 0.0], [left.0 * sign, left.1 * sign, 0.0]))
    }

    /// Follow a moving host unless the same command explicitly moved the contact line.
    /// Removing a host detaches at the last physical axis, preserving cable geometry.
    pub(crate) fn follow_run_mounts(&mut self, before: &Self) -> Result<(), String> {
        let mut updates = Vec::new();
        for (id, element) in self.elements.iter() {
            let Some(run) = element.run() else { continue };
            let Some(mount) = &run.mount else { continue };
            let old_element = before.elements.get(*id);
            let old = old_element.and_then(crate::model::Element::run);
            if self.elements.get(mount.host).is_none() {
                if let Some(old_element) = old_element.filter(|_| old.is_some_and(|old| old.mount == run.mount))
                    && before.elements.get(mount.host).is_some()
                    && let Some(geometry) = before.run_geom(old_element)
                {
                    let mut updated = run.clone();
                    updated.mount = None;
                    updated.section_roll = geometry.roll;
                    updated.square_ends = geometry.square_ends;
                    updated.start = P2(geometry.a[0], geometry.a[1]);
                    updated.end = P2(geometry.b[0], geometry.b[1]);
                    updated.offset = geometry.a[2] - self.elevation(run.level);
                    updated.end_offset = Some(geometry.b[2] - self.elevation(run.level));
                    updated.justify = Justify::Centre;
                    updates.push((*id, updated));
                    continue;
                }
                return Err(format!("{id}: mount host is missing"));
            }
            let current = self.run_mount_frame(mount)?;
            if let Some(old) = old.filter(|old| {
                old.mount == run.mount
                    && old.start == run.start
                    && old.end == run.end
                    && old.level == run.level
                    && old.offset == run.offset
                    && old.end_offset == run.end_offset
            }) {
                let previous = before.run_mount_frame(mount)?;
                if previous != current {
                    let mut updated = run.clone();
                    let map = |point: P2, z| {
                        let p = [point.0, point.1, z];
                        let delta = sub(p, previous.0);
                        let old_across = cross(previous.2, previous.1);
                        let new_across = cross(current.2, current.1);
                        add(
                            add(current.0, mul(current.1, crate::view3d::dot(delta, previous.1))),
                            mul(new_across, crate::view3d::dot(delta, old_across)),
                        )
                    };
                    let base = self.elevation(run.level);
                    let old_base = before.elevation(old.level);
                    let a = map(old.start, old_base + old.offset);
                    let b = map(old.end, old_base + old.end_offset.unwrap_or(old.offset));
                    updated.start = P2(a[0], a[1]);
                    updated.end = P2(b[0], b[1]);
                    updated.offset = a[2] - base;
                    updated.end_offset = Some(b[2] - base);
                    updates.push((*id, updated));
                }
            } else {
                // Rigid commands round XY to 0.001 mm independently of the host.
                // Restore only that rounding error to the semantic contact plane.
                let base = self.elevation(run.level);
                let points =
                    [[run.start.0, run.start.1, base + run.offset], [run.end.0, run.end.1, base + run.end_offset.unwrap_or(run.offset)]];
                let distances = points.map(|p| crate::view3d::dot(sub(p, current.0), current.2));
                if distances.iter().all(|d| d.abs() <= 0.002) && distances.iter().any(|d| d.abs() > 1e-8) {
                    let [a, b] = std::array::from_fn::<_, 2, _>(|i| sub(points[i], mul(current.2, distances[i])));
                    let mut updated = run.clone();
                    updated.start = P2(a[0], a[1]);
                    updated.end = P2(b[0], b[1]);
                    updated.offset = a[2] - base;
                    updated.end_offset = Some(b[2] - base);
                    updates.push((*id, updated));
                }
            }
        }
        for (id, updated) in updates {
            if let Some(run) = self.elements.get_mut(id).and_then(crate::model::Element::run_mut) {
                *run = updated;
            }
        }
        Ok(())
    }

    pub(crate) fn check_run_mounts(&self) -> Result<(), String> {
        let mounted: Vec<_> =
            self.elements.iter().filter_map(|(id, e)| e.run().filter(|r| r.mount.is_some()).map(|r| (*id, e, r))).collect();
        if mounted.is_empty() {
            return Ok(());
        }
        for (id, element, run) in &mounted {
            if !self.run_type(element).is_some_and(|t| t.round || element.category() == "cablecarrier")
                || run.carrier.is_some()
                || run.feeder.is_some()
                || run.justify != Justify::Centre
                || !matches!(element.category(), "wire" | "pipe" | "conduit" | "cablecarrier" | "cable")
            {
                return Err(format!("{id}: mount requires an independent round MEP run or cable carrier with centre justification"));
            }
        }
        let scene = crate::view3d::Scene::new(self);
        for (id, element, run) in mounted {
            let Some(mount) = &run.mount else { continue };
            let (origin, _, normal) = self.run_mount_frame(mount)?;
            let base = self.elevation(run.level);
            let a = [run.start.0, run.start.1, base + run.offset];
            let b = [run.end.0, run.end.1, base + run.end_offset.unwrap_or(run.offset)];
            if [a, b].iter().any(|p| crate::view3d::dot(sub(*p, origin), normal).abs() > 1e-5)
                || !scene.supports_segment(self.elements.id(mount.host).ok_or("missing mount host")?, &mount.face, a, b, normal)?
            {
                return Err(format!(
                    "{id}: contact line leaves its host face or crosses an opening; detach or move it onto supported surface"
                ));
            }
            if let Some(g) = self.run_geom(element).filter(|g| !g.round) {
                let (_, across, _) = g.frame();
                let half = mul(across, g.width / 2.0);
                let corners = [sub(a, half), sub(b, half), add(b, half), add(a, half)];
                let hit = crate::view3d::SurfaceHit {
                    host: self.elements.id(mount.host).ok_or("missing mount host")?,
                    face: match mount.face.as_str() {
                        "exterior" => "exterior",
                        "interior" => "interior",
                        "top" => "top",
                        "bottom" => "bottom",
                        _ => return Err("unsupported mount face".into()),
                    },
                    point: a,
                    normal,
                    distance: 0.0,
                };
                if !scene.supports_patch(&hit, corners)? {
                    return Err(format!("{id}: mounting back leaves its host face or overlaps an opening"));
                }
            }
        }
        Ok(())
    }

    /// Unlinked whole wire segments contained in this carrier's actual 3D section and span.
    /// This query never confirms a link, even when there is just one candidate.
    pub fn carrier_wire_candidates(&self, carrier: crate::model::Id) -> Result<Vec<crate::model::Id>, String> {
        use crate::model::Element;
        use crate::view3d::dot;
        let Some(element @ Element::CableCarrier(_)) = self.elements.get(carrier) else {
            return Err("select an existing cable carrier".into());
        };
        let host = self.run_geom(element).ok_or("carrier has no valid physical axis")?;
        let frame = |g: &Geom| g.frame();
        let (axis, x, y) = frame(&host);
        let mut candidates = Vec::new();
        for (id, element) in self.elements.iter() {
            let Element::Wire(run) = element else { continue };
            if run.carrier.is_some() || run.feeder.is_some() {
                continue;
            }
            let Some(wire) = self.run_geom(element) else { continue };
            let (along, cx, cy) = frame(&wire);
            if dot(axis, along).abs() < 1.0 - 1e-8 {
                continue;
            }
            let support = |normal| {
                if wire.round { wire.width / 2.0 } else { (wire.width * dot(cx, normal).abs() + wire.depth * dot(cy, normal).abs()) / 2.0 }
            };
            if [wire.a, wire.b].into_iter().all(|point| {
                let delta = sub(point, host.a);
                let span = dot(delta, axis);
                let (px, py) = (dot(delta, x).abs(), dot(delta, y).abs());
                span >= -TOL
                    && span <= host.length() + TOL
                    && if host.round {
                        // Bounding the corners is conservative for rectangular cables in a round carrier.
                        let radius = if wire.round { px.hypot(py) + wire.width / 2.0 } else { (px + support(x)).hypot(py + support(y)) };
                        radius <= host.width / 2.0 + TOL
                    } else {
                        px + support(x) <= host.width / 2.0 + TOL && py + support(y) <= host.depth / 2.0 + TOL
                    }
            }) {
                candidates.push(*id);
            }
        }
        Ok(candidates)
    }

    pub(crate) fn check_wire_connections(&self) -> Result<(), String> {
        use crate::model::{Element, MAX_COORD, MAX_POINTS};
        let mut owned = Vec::new();
        for (id, element) in self.elements.iter() {
            let Some(run) = element.run() else { continue };
            if run.connections.len() > MAX_POINTS || (!run.connections.is_empty() && (element.category() != "wire" || run.feeder.is_some()))
            {
                return Err(format!("{id}: only a main wire may have at most {MAX_POINTS} connections"));
            }
            let mut ends = Vec::new();
            for connection in &run.connections {
                let key = (connection.device, connection.end);
                if !connection.point.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORD) || connection.end > 1 || ends.contains(&key) {
                    return Err(format!("{id}: connection points must be finite and bounded; connect each device/end (0 or 1) once"));
                }
                ends.push(key);
                if connection.landing.is_some_and(|point| !point.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORD)) {
                    return Err(format!("{id}: the last route landing must be finite and within coordinate limits"));
                }
                if connection
                    .carrier
                    .is_some_and(|guid| !matches!(self.elements.get(guid), Some(Element::CableCarrier(r)) if r.feeder.is_none()))
                {
                    return Err(format!("{id}: a connection route must name an existing main cable carrier"));
                }
                if !matches!(self.elements.get(connection.device), Some(Element::Component(c)) if c.is_placed()) {
                    return Err(format!("{id}: a connection must name placed equipment"));
                }
                if connection.terminal.is_some_and(
                    |guid| !matches!(self.elements.get(guid), Some(Element::ElectricalTerminal(t)) if t.device == connection.device),
                ) {
                    return Err(format!("{id}: a connection terminal must belong to its equipment"));
                }
                if connection.surface.is_some() {
                    let Some(Element::Component(component)) = self.elements.get(connection.device) else {
                        return Err("connection equipment is missing".into());
                    };
                    let main = self.run_geom(element).ok_or("wire has no physical geometry")?;
                    let route = connection.carrier.and_then(|g| self.elements.get(g)).and_then(|e| self.run_geom(e)).unwrap_or(main);
                    self.free_connection(component, connection, &route, &main)?;
                }
                if connection.in_carrier && connection.carrier.or(run.carrier).is_none() {
                    return Err(format!("{id}: select a carrier before connecting equipment in a carrier"));
                }
            }
            if let Some(owner) = &run.feeder {
                let Some(Element::Wire(parent)) = self.elements.get(owner.wire) else {
                    return Err(format!("{id}: its feeder owner must name a main wire"));
                };
                let key = (owner.wire, owner.device, owner.end, owner.leg, element.category());
                if parent.feeder.is_some()
                    || owner.leg > 1
                    || !parent.connections.iter().any(|c| c.device == owner.device && c.end == owner.end)
                    || !matches!(element.category(), "wire" | "cablecarrier")
                    || owned.contains(&key)
                {
                    return Err(format!("{id}: invalid or duplicate physical feeder ownership"));
                }
                owned.push(key);
            }
        }
        Ok(())
    }

    /// Regenerate owned legs in place; surviving roles keep their element ids and GUIDs.
    pub(crate) fn refresh_wire_connections(&mut self, before: &Self) -> Result<(), String> {
        use crate::model::{Element, Feeder};
        // Deleting equipment disconnects its owned legs. An arbitrary missing reference is still an error.
        let removed: Vec<_> =
            before.elements.iter().filter(|(id, _)| self.elements.get(**id).is_none()).map(|(id, _)| before.guid(*id)).collect();
        for (_, element) in self.elements.iter_mut() {
            if let Element::Wire(run) = element {
                run.connections.retain(|c| !removed.contains(&c.device));
            }
        }
        let stale: Vec<_> = self
            .elements
            .iter()
            .filter_map(|(id, element)| {
                let owner = element.run()?.feeder.as_ref()?;
                (removed.contains(&owner.wire) || removed.contains(&owner.device)).then_some(*id)
            })
            .collect();
        for id in stale {
            self.elements.remove(id);
        }
        self.follow_connection_surfaces(before)?;
        self.check_wire_connections()?;
        let parents: Vec<_> = self
            .elements
            .iter()
            .filter_map(|(id, element)| match element {
                Element::Wire(run) if !run.connections.is_empty() => Some((*id, run.clone())),
                _ => None,
            })
            .collect();
        let mut keep = Vec::new();
        for (id, parent) in parents {
            let parent_guid = self.guid(id);
            let element = Element::Wire(parent.clone());
            let main_geometry = self.run_geom(&element).ok_or_else(|| format!("{id}: wire has no physical axis"))?;
            for (index, connection) in parent.connections.iter().enumerate() {
                let route_geometry = connection.carrier.and_then(|guid| self.elements.get(guid)).and_then(|e| self.run_geom(e));
                let geometry = route_geometry.as_ref().unwrap_or(&main_geometry);
                let Some(Element::Component(component)) = self.elements.get(connection.device) else {
                    return Err(format!("{id}: connection equipment is missing"));
                };
                let (device, target, elbow, free_normal) = if connection.surface.is_some() {
                    let solved = self.free_connection(component, connection, geometry, &main_geometry)?;
                    if let Some(Element::Wire(run)) = self.elements.get_mut(id)
                        && let Some(updated) = run.connections.get_mut(index)
                    {
                        updated.point = solved.point;
                        updated.surface = Some(solved.surface);
                        if connection.carrier.is_some() {
                            updated.landing = Some(solved.target);
                        }
                    }
                    (solved.device, solved.target, solved.elbow, Some(solved.normal))
                } else {
                    let [x, mut y, z] = connection.point;
                    if component.mirrored {
                        y = -y;
                    }
                    let (sin, cos) = component.angle.to_radians().sin_cos();
                    let device = [
                        component.at.0 + x * cos - y * sin,
                        component.at.1 + x * sin + y * cos,
                        self.elevation(component.level) + component.offset + z,
                    ];
                    let dx = geometry.b[0] - geometry.a[0];
                    let dy = geometry.b[1] - geometry.a[1];
                    let flat = dx * dx + dy * dy;
                    let t = if flat > TOL * TOL {
                        (((device[0] - geometry.a[0]) * dx + (device[1] - geometry.a[1]) * dy) / flat).clamp(0.0, 1.0)
                    } else {
                        let dz = geometry.b[2] - geometry.a[2];
                        if dz.abs() > TOL { ((device[2] - geometry.a[2]) / dz).clamp(0.0, 1.0) } else { 0.0 }
                    };
                    let projected = add(geometry.a, mul(sub(geometry.b, geometry.a), t));
                    let target = if connection.carrier.is_some() { projected } else { connection.landing.unwrap_or(projected) };
                    if connection.carrier.is_some()
                        && let Some(Element::Wire(run)) = self.elements.get_mut(id)
                        && let Some(updated) = run.connections.get_mut(index)
                    {
                        updated.landing = Some(target);
                    }
                    (device, target, [device[0], device[1], target[2]], None)
                };
                for (leg, a, b) in [(0, device, elbow), (1, elbow, target)] {
                    if sub(b, a).iter().map(|v| v * v).sum::<f64>().sqrt() <= TOL {
                        continue;
                    }
                    let owner = Feeder { wire: parent_guid, device: connection.device, end: connection.end, leg };
                    let mut wire = parent.clone();
                    // Generated legs use the solved physical axis, not the parent's contact line.
                    wire.mount = None;
                    wire.connections.clear();
                    wire.feeder = Some(owner.clone());
                    wire.start = P2(a[0], a[1]);
                    wire.end = P2(b[0], b[1]);
                    wire.offset = a[2] - self.elevation(parent.level);
                    wire.end_offset = Some(b[2] - self.elevation(parent.level));
                    wire.justify = Justify::Centre;
                    wire.source = None;
                    wire.devices.clear();
                    wire.carrier = None;
                    wire.compartment.clear();
                    wire.local_offset = 0.0;
                    if connection.in_carrier {
                        let Some(Element::CableCarrier(carrier)) = connection.carrier.or(parent.carrier).and_then(|g| self.elements.get(g))
                        else {
                            return Err(format!("{id}: connection requires a cable carrier"));
                        };
                        let mut branch = wire.clone();
                        branch.type_name = carrier.type_name.clone();
                        branch.reserve = 0.0;
                        branch.cable.clear();
                        branch.compartments.clear();
                        branch.size_locked = false;
                        if let Some(normal) = free_normal {
                            let element = Element::CableCarrier(branch.clone());
                            let geom = self.run_geom(&element).ok_or("connection branch type has no geometry")?;
                            let depth = if geom.round { geom.width } else { geom.depth };
                            let shift = mul(normal, depth / 2.0 - main_geometry.width / 2.0);
                            branch.start = P2(a[0] + shift[0], a[1] + shift[1]);
                            branch.end = P2(b[0] + shift[0], b[1] + shift[1]);
                            branch.offset += shift[2];
                            branch.end_offset = Some(b[2] + shift[2] - self.elevation(parent.level));
                            let (_, x, y) = Geom { roll: 0.0, ..geom }.frame();
                            branch.section_roll = (-crate::view3d::dot(normal, x)).atan2(crate::view3d::dot(normal, y)).to_degrees();
                            branch.square_ends = !geom.round;
                            if main_geometry.width > depth + TOL || main_geometry.width > geom.width + TOL {
                                return Err("connection branch is too small for the cable diameter".into());
                            }
                        }
                        let branch_id = self.upsert_feeder(Element::CableCarrier(branch), &owner);
                        keep.push(branch_id);
                        wire.carrier = Some(self.guid(branch_id));
                    }
                    keep.push(self.upsert_feeder(Element::Wire(wire), &owner));
                }
            }
        }
        let stale: Vec<_> = self
            .elements
            .iter()
            .filter_map(|(id, e)| (e.run().is_some_and(|r| r.feeder.is_some()) && !keep.contains(id)).then_some(*id))
            .collect();
        for id in stale {
            self.elements.remove(id);
        }
        Ok(())
    }

    fn upsert_feeder(&mut self, element: crate::model::Element, owner: &crate::model::Feeder) -> crate::model::Id {
        let existing = self.elements.iter().find_map(|(id, e)| {
            (e.category() == element.category() && e.run().and_then(|r| r.feeder.as_ref()) == Some(owner)).then_some(*id)
        });
        if let Some(id) = existing {
            self.elements.insert(id, element);
            id
        } else {
            self.insert(element)
        }
    }
}

/// Sides a round run's tube is drawn with.
pub const SIDES: usize = 24;

/// Whether a run of `category` may stand upright (a riser, a down conductor) rather than run along: not a beam or a
/// strip footing, which are columns when they stand.
pub fn may_stand(category: &str) -> bool {
    !matches!(category, "beam" | "stripfooting")
}

/// How each run category is written in IFC: its class and predefined type (all of them take a product's nine attributes). Import
/// reads the same table back, a class and a type to the first category that has them.
pub const IFC: &[(&str, &str, &str)] = &[
    ("beam", "IFCBEAM", ".BEAM."),
    ("stripfooting", "IFCFOOTING", ".STRIP_FOOTING."),
    ("pipe", "IFCPIPESEGMENT", ".RIGIDSEGMENT."),
    ("duct", "IFCDUCTSEGMENT", ".RIGIDSEGMENT."),
    ("conduit", "IFCCABLECARRIERSEGMENT", ".CONDUITSEGMENT."),
    ("cabletray", "IFCCABLECARRIERSEGMENT", ".CABLETRAYSEGMENT."),
    ("cablecarrier", "IFCCABLECARRIERSEGMENT", ".CONDUITSEGMENT."),
    ("cablecarrier", "IFCCABLECARRIERSEGMENT", ".CABLETRAYSEGMENT."),
    // A ladder is a tray to BIMCraft, written as a tray: only a file of another tool's says it.
    ("cablecarrier", "IFCCABLECARRIERSEGMENT", ".CABLELADDERSEGMENT."),
    ("wire", "IFCCABLESEGMENT", ".CABLESEGMENT."),
    // IFC has no lightning protection: a down conductor is a conductor segment, as is an earthing conductor (BIMCraft's own
    // `Category` tells them apart).
    ("downconductor", "IFCCABLESEGMENT", ".CONDUCTORSEGMENT."),
    ("groundconductor", "IFCCABLESEGMENT", ".CONDUCTORSEGMENT."),
    // A data cable is a cable segment, as a wire is (BIMCraft's own `Category` tells them apart).
    ("cable", "IFCCABLESEGMENT", ".CABLESEGMENT."),
];

/// The IFC class and predefined type a run of `category` is written as.
pub fn ifc(category: &str) -> Option<(&'static str, &'static str)> {
    IFC.iter().find(|(c, ..)| *c == category).map(|(_, class, kind)| (*class, *kind))
}

/// The IFC class and predefined type a run of `category` is written as for its shape: a cable carrier's
/// round section writes as a conduit segment, a rectangular one as a cable tray segment.
pub fn ifc_shape(category: &str, round: bool) -> Option<(&'static str, &'static str)> {
    if category != "cablecarrier" {
        return ifc(category);
    }
    let kind = if round { ".CONDUITSEGMENT." } else { ".CABLETRAYSEGMENT." };
    IFC.iter().find(|(c, _, t)| *c == category && *t == kind).map(|(_, class, kind)| (*class, *kind))
}

/// The category of a run an IFC product is, from its `class`, its `kind` (the predefined type) and, when a file of BIMCraft's says
/// so, the `declared` category: that one if the class has it, else the one whose type it is, else the class's first.
pub fn from_ifc(class: &str, kind: Option<&str>, declared: Option<&str>) -> Option<&'static str> {
    let class = if class == "IFCBEAMSTANDARDCASE" { "IFCBEAM" } else { class };
    let rows = || IFC.iter().filter(move |(_, c, _)| *c == class);
    let by_kind = |k: &str| rows().find(|(_, _, t)| t.trim_matches('.') == k);
    let by_name = |d: &str| rows().find(|(c, ..)| *c == d);
    declared.and_then(by_name).or_else(|| kind.and_then(by_kind)).or_else(|| rows().next()).map(|(c, ..)| *c)
}

/// The line of its section a run of `category` has its heights at unless told: the top of a beam or a footing, as engineers
/// think of them (top of beam at the top of slab), the middle of anything that carries something (a pipe, a cable).
pub fn default_justify(category: &str) -> Justify {
    if may_stand(category) { Justify::Centre } else { Justify::Top }
}

/// Why `g`, a run of `category`, isn't one the model takes, if it isn't.
pub fn check(who: &str, category: &str, g: &Geom) -> Result<(), String> {
    if g.length() <= TOL {
        return Err(format!("{who}: too short ({} mm)", g.length()));
    }
    if !may_stand(category) {
        if g.vertical() {
            return Err(format!("{who}: its ends are one over the other; a {category} runs along, and what stands is a column"));
        }
        let tilt = (g.b[2] - g.a[2]).abs().atan2(g.plan_length()).to_degrees();
        if tilt > MAX_SLOPE {
            return Err(format!("{who}: it slopes {tilt:.1}°, more than the {MAX_SLOPE}° a {category} may; what stands is a column"));
        }
    }
    Ok(())
}

/// A run worked out: its centre line from `a` to `b` (mm: x east, y north, z up), and its section, `width` across by `depth`
/// deep, square to that line; round, `width` is the diameter and `depth` is as much.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub square_ends: bool,
    pub roll: f64,
    pub a: P3,
    pub b: P3,
    pub width: f64,
    pub depth: f64,
    pub round: bool,
}

impl Geom {
    /// `r` of type `t`: its heights are those of its `justify` line, so its centre line is half the section's depth (square
    /// to the line) under it for a top, over it for a bottom.
    pub fn of(m: &Model, r: &Run, t: &RunType) -> Geom {
        let base = m.elevation(r.level);
        let (z0, z1) = (base + r.offset, base + r.end_offset.unwrap_or(r.offset));
        let depth = if t.round { t.width } else { t.depth };
        let g = Geom {
            square_ends: r.square_ends,
            roll: r.section_roll,
            a: [r.start.0, r.start.1, z0],
            b: [r.end.0, r.end.1, z1],
            width: t.width,
            depth,
            round: t.round,
        };
        if let Some(mount) = &r.mount
            && let Ok((_, _, normal)) = m.run_mount_frame(mount)
        {
            let (_, x, y) = Geom { roll: 0.0, ..g }.frame();
            let roll =
                if t.round { r.section_roll } else { (-crate::view3d::dot(normal, x)).atan2(crate::view3d::dot(normal, y)).to_degrees() };
            let shift = mul(normal, depth / 2.0);
            return Geom { a: add(g.a, shift), b: add(g.b, shift), roll, square_ends: !t.round || g.square_ends, ..g };
        }
        // Upright, it has no top or bottom to be the line.
        let shift = match r.justify {
            Justify::Top => -g.half(),
            Justify::Centre => 0.0,
            Justify::Bottom => g.half(),
        };
        Geom { a: [g.a[0], g.a[1], z0 + shift], b: [g.b[0], g.b[1], z1 + shift], ..g }
    }

    /// Along the centre line: its true length.
    pub fn length(&self) -> f64 {
        self.plan_length().hypot(self.b[2] - self.a[2])
    }

    /// Physical longitudinal, width and depth axes shared by drawing, export and containment.
    pub fn frame(&self) -> (P3, P3, P3) {
        let axis = unit(sub(self.b, self.a));
        let x = if self.vertical() { [1.0, 0.0, 0.0] } else { unit([-axis[1], axis[0], 0.0]) };
        let y = cross(axis, x);
        let (sin, cos) = self.roll.to_radians().sin_cos();
        (axis, add(mul(x, cos), mul(y, sin)), sub(mul(y, cos), mul(x, sin)))
    }

    fn box_rings(&self) -> [[P3; 4]; 2] {
        let (_, x, y) = self.frame();
        [self.a, self.b].map(|at| {
            [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(w, d)| add(at, add(mul(x, w * self.width), mul(y, d * self.depth))))
        })
    }

    pub fn box_triangles(&self) -> Vec<[P3; 3]> {
        let [a, b] = self.box_rings();
        let mut tris = vec![[a[0], a[2], a[1]], [a[0], a[3], a[2]], [b[0], b[1], b[2]], [b[0], b[2], b[3]]];
        for i in 0..4 {
            let j = (i + 1) % 4;
            tris.extend([[a[i], a[j], b[j]], [a[i], b[j], b[i]]]);
        }
        tris
    }

    /// Its length in plan.
    pub fn plan_length(&self) -> f64 {
        P2(self.b[0] - self.a[0], self.b[1] - self.a[1]).dist(P2(0.0, 0.0))
    }

    /// Whether it stands upright: its ends one over the other in plan.
    pub fn vertical(&self) -> bool {
        self.plan_length() <= TOL
    }

    /// Half the section's depth, taken straight up: more than half of it, square to the line, where the line slopes.
    pub fn half(&self) -> f64 {
        if !self.round && (self.square_ends || self.roll.abs() > 1e-9) {
            let (_, x, y) = self.frame();
            return (x[2].abs() * self.width + y[2].abs() * self.depth) / 2.0;
        }
        let plan = self.plan_length();
        if plan > TOL { self.depth / 2.0 * self.length() / plan } else { 0.0 }
    }

    /// Along it in plan: east when upright.
    fn along(&self) -> P2 {
        if self.vertical() { P2(1.0, 0.0) } else { P2(self.b[0] - self.a[0], self.b[1] - self.a[1]).unit() }
    }

    /// Its footprint in plan, counter-clockwise: a strip `width` wide from end to end, or, upright, its section, `width`
    /// along east and `depth` along north, or round.
    pub fn footprint(&self) -> Vec<P2> {
        if !self.round && (self.square_ends || self.roll.abs() > 1e-9) {
            return crate::geom::hull(self.box_rings().into_iter().flatten().map(|p| P2(p[0], p[1])).collect());
        }
        let (a, b) = (P2(self.a[0], self.a[1]), P2(self.b[0], self.b[1]));
        if !self.vertical() {
            let n = self.along().perp() * (self.width / 2.0);
            return vec![a - n, b - n, b + n, a + n];
        }
        if self.round {
            return (0..SIDES).map(|i| a + heading(360.0 * i as f64 / SIDES as f64) * (self.width / 2.0)).collect();
        }
        let (w, d) = (self.width / 2.0, self.depth / 2.0);
        [(-w, -d), (w, -d), (w, d), (-w, d)].into_iter().map(|(x, y)| a + P2(x, y)).collect()
    }

    /// The lowest and the highest it reaches.
    pub fn z_span(&self) -> (f64, f64) {
        if !self.round && (self.square_ends || self.roll.abs() > 1e-9) {
            let (_, x, y) = self.frame();
            let h = (x[2].abs() * self.width + y[2].abs() * self.depth) / 2.0;
            return (self.a[2].min(self.b[2]) - h, self.a[2].max(self.b[2]) + h);
        }
        let h = self.half();
        (self.a[2].min(self.b[2]) - h, self.a[2].max(self.b[2]) + h)
    }

    /// As a prism: its footprint between a bottom plane and a top one, a section a rectangle (round, the same square to it).
    pub fn prism(&self) -> (Vec<P2>, Plane, Plane) {
        let ring = self.footprint();
        if self.vertical() {
            let (lo, hi) = self.z_span();
            return (ring, Plane::flat(lo), Plane::flat(hi));
        }
        let a = P2(self.a[0], self.a[1]);
        let g = self.along() * ((self.b[2] - self.a[2]) / self.plan_length());
        let z = self.a[2] - g.dot(a);
        (ring, Plane { z: z - self.half(), g }, Plane { z: z + self.half(), g })
    }

    /// A round run as a tube of triangles turned out: its two caps' first, and how many they are. It is [`unit_tube`]
    /// placed by [`Geom::tube_place`], to the last bit, so the 3D view's card can draw every round run from one tube.
    pub fn tube(&self) -> (Vec<[P3; 3]>, usize) {
        let (tris, caps) = unit_tube();
        let place = self.tube_place(crate::model::Id::default(), 0);
        (tris.iter().map(|t| t.map(|q| place.place(q))).collect(), caps)
    }

    /// Where [`unit_tube`] goes to be this run's, as `owner`'s copy of the `shape`th shape: its line along the tube's x, the
    /// radius along y and z, round its line from east (from the north, upright).
    pub(crate) fn tube_place(&self, owner: crate::model::Id, shape: usize) -> crate::component_geometry::ComponentInstance {
        let (d, r) = (unit(sub(self.b, self.a)), self.width / 2.0);
        let e1 = if self.vertical() { [1.0, 0.0, 0.0] } else { unit([-d[1], d[0], 0.0]) };
        let e2 = cross(d, e1);
        crate::component_geometry::ComponentInstance { owner, shape, at: self.a, axes: [sub(self.b, self.a), mul(e1, r), mul(e2, r)] }
    }
}

/// Whether two runs' heights overlap the way joined runs' must, strictly, as [`crate::walls::stacked`] holds two
/// walls': a footing whose top sits exactly on a beam's bottom meets it nowhere to mitre, while the touch of a
/// footing's top with a wall's base is the joint a glued footing makes ([`arms`] allows it of a wall).
fn overlapped(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// Whether a run lies flat along the plan — from the run itself, what [`joinable`] says of its worked-out
/// geometry: upright it would be a column's stance and `end_offset` would make it slope.
pub(crate) fn flat(r: &Run) -> bool {
    r.start.dist(r.end) > TOL && (r.end_offset.unwrap_or(r.offset) - r.offset).abs() <= TOL
}

/// Whether `g` lies flat along the plan: upright it is a column's section, and with an `end_offset` its heights
/// change along it, so neither can mitre against anything (they draw as they did). A round run joins as its plan
/// outline shows it — its body is cut by the same planes as the outline (see [`tube_at`]).
pub(crate) fn joinable(g: &Geom) -> bool {
    !g.vertical() && (g.b[2] - g.a[2]).abs() <= TOL
}

/// Run `g`'s arm leaving the node at its start (or with `at_end`, at its end): [`run_arm_at`] from that end.
fn run_arm(g: &Geom, at_end: bool, mine: bool) -> walls::Arm {
    run_arm_at(g, if at_end { g.plan_length() } else { 0.0 }, !at_end, mine)
}

/// Run `g`'s arm leaving the node `s` mm along its line (from its `a`), going on with `forward` (else back along
/// itself): its way, where its two faces are there — the one left of the way first, as a wall's [`walls::Arm`] has
/// them — and how wide it is across.
fn run_arm_at(g: &Geom, s: f64, forward: bool, mine: bool) -> walls::Arm {
    let (a, b) = (P2(g.a[0], g.a[1]), P2(g.b[0], g.b[1]));
    let t = (b - a).unit();
    let n = t.perp() * (g.width / 2.0);
    let p = a + t * s;
    let (way, ccw, cw) = if forward { (t, p + n, p - n) } else { (t * -1.0, p - n, p + n) };
    walls::Arm { mine, way, ccw, cw, centre: p, thick: g.width, ..Default::default() }
}

/// A wall's arm at `s` mm along its line, as [`walls::arm`] makes one at an end: left of its way first.
fn wall_arm_at(m: &Model, w: &Wall, s: f64, forward: bool, mine: bool) -> walls::Arm {
    let axis = Axis::of(w);
    let (ext, int) = walls::faces(m, w);
    let t = axis.tangent(s);
    let (way, ccw, cw) = if forward { (t, ext, int) } else { (t * -1.0, int, ext) };
    walls::Arm {
        mine,
        way,
        ccw: axis.at(s, ccw),
        cw: axis.at(s, cw),
        centre: axis.at(s, (ext + int) / 2.0),
        thick: ext - int,
        ..Default::default()
    }
}

/// How far along `g` the node lies where its body takes it: over [`TOL`] from either end and within its half
/// width across; none, the node is beside it or past its ends.
fn lying_on(g: &Geom, node: P2) -> Option<f64> {
    let (a, b) = (P2(g.a[0], g.a[1]), P2(g.b[0], g.b[1]));
    let t = (b - a).unit();
    let s = (node - a).dot(t);
    let across = (node - a).dot(t.perp());
    (s > TOL && s < g.plan_length() - TOL && across.abs() <= g.width / 2.0 + TOL).then_some(s)
}

/// Run `id`'s arms round the node at its start (or with `at_end`, at its end), its own first, then those of the
/// runs of its own category that end there or that it ends on (or that its node lies on), and — of a strip footing
/// — of the walls it ends on, round the node counter-clockwise; and whether it ends ON one, which fills the node
/// past its own end. Runs meet only where their heights overlap (a footing meets a wall by its top touching the
/// wall's base), only when both join (`join`), and only runs lying flat. Candidates come through `index` when there
/// is one, else from every element of the model — the neighbours a plan of thousands of runs would cost O(n²).
fn arms(m: &Model, id: Id, at_end: bool, g: &Geom, index: Option<&walls::Index>) -> (Vec<walls::Arm>, bool) {
    let Some(e) = m.elements.get(id) else { return (Vec::new(), false) };
    let mut arms = vec![run_arm(g, at_end, true)];
    let mut through = false;
    // An element that refuses to join, or doesn't lie flat, meets nothing: its own square end is its whole cap.
    if !e.run().is_some_and(|r| r.join) || !joinable(g) {
        return (arms, false);
    }
    let category = e.category();
    let node = if at_end { P2(g.b[0], g.b[1]) } else { P2(g.a[0], g.a[1]) };
    let (lo, hi) = g.z_span();
    // The runs near the node: through the tile index when there is one, else every run, kept only those whose
    // bodies reach within half a width of the node.
    let near: Vec<(Id, &Element)> = match index {
        Some(index) => index.runs_near(node, lo, hi).into_iter().filter_map(|oid| Some((oid, m.elements.get(oid)?))).collect(),
        None => m
            .elements
            .iter()
            .filter(|(oid, oe)| **oid != id && oe.run().is_some())
            .filter_map(|(oid, oe)| {
                let og = m.run_geom(oe)?;
                let (a, b) = (P2(og.a[0], og.a[1]), P2(og.b[0], og.b[1]));
                (geom::dist_to_segment(node, a, b) <= og.width / 2.0 + TOL).then_some((*oid, oe))
            })
            .collect(),
    };
    for &(other, o) in &near {
        if other == id {
            continue;
        }
        let (Some(oe), Some(og)) = (o.run(), m.run_geom(o)) else { continue };
        if !oe.join || o.category() != category || !joinable(&og) {
            continue;
        }
        if !overlapped((lo, hi), og.z_span()) {
            continue;
        }
        let (oa, ob) = (P2(og.a[0], og.a[1]), P2(og.b[0], og.b[1]));
        if oa.dist(node) <= TOL {
            arms.push(run_arm(&og, false, false));
        } else if ob.dist(node) <= TOL {
            arms.push(run_arm(&og, true, false));
        } else if let Some(s) = lying_on(&og, node) {
            // Ending on it: it runs on both ways from where the node meets it, and fills the node.
            arms.push(run_arm_at(&og, s, true, false));
            arms.push(run_arm_at(&og, s, false, false));
            through = true;
        }
    }
    // A strip footing ends on walls: they stop it at their faces and are never changed by it — the walls' own
    // outlines know nothing of runs. Its top touching the wall's base is enough: that is how a footing meets the
    // wall it carries.
    if category == "stripfooting" {
        // The query asks the bands a touch falls in: a footing's top exactly on the wall's base shares no
        // band with it, and the tile bands would leave the wall out before the exact check below could
        // allow it. That check is still the one that decides, with the touch allowed.
        let at = ([node.0, node.1, node.0, node.1], lo - TOL, hi + TOL);
        let thickest = m.types.wall.values().map(|t| t.thickness).filter(|t| t.is_finite()).fold(0.0, f64::max);
        let lookup = m.lookup();
        for oid in lookup.walls_near_at(&[at], thickest + 1.0) {
            if oid == id {
                continue;
            }
            let Some(Element::Wall(w)) = m.elements.get(oid) else { continue };
            if !w.join || !(lo - TOL <= m.top_z(w) && m.base_z(w) - TOL <= hi) {
                continue;
            }
            let axis = Axis::of(w);
            // A straight wall directly over a footing at this joint is already represented by that run. Counting
            // the wall as another arm made the adjoining footing see a through-joint too: the duplicate collinear
            // arm set `through` and put the node back into the cap, breaking the mitre.
            if w.arc == 0.0 {
                let (s, across) = axis.project(node);
                let tangent = axis.tangent(s);
                let lies_under_wall = |rg: &Geom| {
                    let (rlo, rhi) = rg.z_span();
                    if !(rlo - TOL <= m.top_z(w) && m.base_z(w) - TOL <= rhi) {
                        return false;
                    }
                    let a = P2(rg.a[0], rg.a[1]);
                    let b = P2(rg.b[0], rg.b[1]);
                    let (_, a_off) = axis.project(a);
                    let (_, b_off) = axis.project(b);
                    let parallel = tangent.cross(rg.along()).abs() <= TOL / axis.length().max(rg.plan_length()).max(TOL);
                    a_off.abs() <= TOL && b_off.abs() <= TOL && parallel && geom::dist_to_segment(node, a, b) <= rg.width / 2.0 + TOL
                };
                let footing_under_wall = lies_under_wall(g)
                    || near.iter().any(|(_, re)| {
                        if re.category() != category || !re.run().is_some_and(|r| r.join) {
                            return false;
                        }
                        m.run_geom(re).filter(joinable).is_some_and(|rg| lies_under_wall(&rg))
                    });
                if across.abs() <= TOL && footing_under_wall {
                    continue;
                }
            }
            let (ext, int) = walls::faces(m, w);
            if w.start.dist(node) <= TOL {
                arms.push(wall_arm_at(m, w, 0.0, true, false));
            } else if w.end.dist(node) <= TOL {
                arms.push(wall_arm_at(m, w, axis.length(), false, false));
            } else {
                let (s, across) = axis.project(node);
                if s > TOL && s < axis.length() - TOL && (int - TOL..=ext + TOL).contains(&across) && axis.distance(node, across) <= TOL {
                    arms.push(wall_arm_at(m, w, s, true, false));
                    arms.push(wall_arm_at(m, w, s, false, false));
                    through = true;
                }
            }
        }
    }
    arms.sort_by(|a, b| a.way.1.atan2(a.way.0).total_cmp(&b.way.1.atan2(b.way.0)));
    (arms, through)
}

/// Run `id`'s cap at its start (or with `at_end`, at its end): the corners closing it from its face on the
/// clockwise side of the way it leaves the node round to the other, exactly as a wall's [`walls::cap`] closes
/// its own — faces crossing near the node (no farther than the widest arm is wide: past that the end goes
/// square), the node itself between them where several meet there, and just the two faces where it ends on one.
fn cap(m: &Model, id: Id, at_end: bool, g: &Geom, index: Option<&walls::Index>) -> Vec<P2> {
    let (arms, through) = arms(m, id, at_end, g, index);
    let n = arms.len();
    let Some(i) = arms.iter().position(|a| a.mine) else { return Vec::new() };
    let (prev, me, next) = (&arms[(i + n - 1) % n], &arms[i], &arms[(i + 1) % n]);
    if n < 2 {
        return vec![me.cw, me.ccw];
    }
    let node = if at_end { P2(g.b[0], g.b[1]) } else { P2(g.a[0], g.a[1]) };
    let limit = arms.iter().map(|a| a.thick).fold(0.0, f64::max);
    let meet = |face: P2, other: P2, other_way: P2| {
        geom::line_intersection(face, face + me.way, other, other + other_way)
            .filter(|c| (-limit..=limit).contains(&(*c - node).dot(me.way)))
            .unwrap_or(face)
    };
    let (cw, ccw) = (meet(me.cw, prev.ccw, prev.way), meet(me.ccw, next.cw, next.way));
    if through { vec![cw, ccw] } else { vec![cw, node, ccw] }
}

/// Run `id`'s outline in plan: its caps and its faces, mitred against the runs and (of a strip footing) the walls
/// they meet, as [`crate::walls::outline`] is of walls; its plain rectangle where it joins nothing there, its
/// `join` is off, or it doesn't lie flat. The line from `start` to `end` never changes: the join only shapes what
/// the plan fills, 3D cuts and IFC sweeps.
pub fn outline(m: &Model, id: Id) -> Vec<P2> {
    outline_in(m, id, None)
}

/// [`outline`], its joins found through `index` when there is one.
pub(crate) fn outline_in(m: &Model, id: Id, index: Option<&walls::Index>) -> Vec<P2> {
    let Some(e) = m.elements.get(id) else { return Vec::new() };
    let Some(g) = m.run_geom(e) else { return Vec::new() };
    if !joinable(&g) {
        return g.footprint();
    }
    // Its faces are straight, so the caps alone carry the corners, as a straight wall's caps do: each cap's last
    // point lies on the face the next cap's first lies on, and the ring closes along the other face.
    let (start, end) = (cap(m, id, false, &g, index), cap(m, id, true, &g, index));
    // Nothing joins it at either end: the plain rectangle, exactly as it was drawn before it could join.
    if plain(&g, false, &start) && plain(&g, true, &end) {
        return g.footprint();
    }
    let mut ring = start;
    ring.extend(end);
    ring.dedup_by(|a, b| a.dist(*b) < 1e-6);
    // Counter-clockwise, as [`Geom::footprint`] gives it, for the fills and prisms built from it.
    if ring.len() >= 3 && geom::area(&ring) < 0.0 {
        ring.reverse();
    }
    ring
}

/// Whether `cap` is just this end's own two corners: nothing joined there, so the rectangle (or the tube)
/// already ends where the outline does and nothing needs cutting.
fn plain(g: &Geom, at_end: bool, cap: &[P2]) -> bool {
    let square = run_arm(g, at_end, true);
    cap.len() == 2 && cap[0].dist(square.cw) <= 1e-6 && cap[1].dist(square.ccw) <= 1e-6
}

/// One end's cap of run `id` as the vertical planes a round run's body is cut by: each segment of the cap,
/// and the run's point at the other end, which side of the plane is its inside. None where the run doesn't lie
/// flat, or where the cap is its plain square end — the tube already ends there, and stays the one shape a view
/// shares between round runs of a material. The same plane twice (a mitre of equal widths, whose cap is one
/// straight line through the node) is taken once.
pub(crate) fn end_cuts(m: &Model, id: Id, at_end: bool, g: &Geom, index: Option<&walls::Index>) -> Option<Vec<(P2, P2, P2)>> {
    if !joinable(g) {
        return None;
    }
    let cap = cap(m, id, at_end, g, index);
    if cap.len() < 2 || plain(g, at_end, &cap) {
        return None;
    }
    let inside = if at_end { P2(g.a[0], g.a[1]) } else { P2(g.b[0], g.b[1]) };
    let mut cuts: Vec<(P2, P2, P2)> = Vec::new();
    for seg in cap.windows(2) {
        let (p0, p1) = (seg[0], seg[1]);
        if p0.dist(p1) <= 1e-9 {
            continue;
        }
        // A segment of one straight cap line is the same plane again: one cut is enough.
        let same_line = cuts.iter().any(|(q0, q1, _)| {
            let (q0, q1) = (*q0, *q1);
            let (d, e) = (p1 - p0, q1 - q0);
            d.cross(e).abs() <= 1e-9 * d.length() * e.length() && (q0 - p0).cross(d).abs() <= 1e-6 * d.length()
        });
        if !same_line {
            cuts.push((p0, p1, inside));
        }
    }
    (!cuts.is_empty()).then_some(cuts)
}

/// `tris` (a round run's tube, its first `caps` its flat end faces) cut by the planes of `cuts` — (p0, p1,
/// inside) — the side `inside` lies on kept, each cut open filled back flat: the body ends where the outline
/// says, so plan and 3D never show different joints. The flat faces stay first, for `caps` to count them.
pub(crate) fn clip_tube(tris: Vec<[P3; 3]>, caps: usize, cuts: &[(P2, P2, P2)]) -> (Vec<[P3; 3]>, usize) {
    /// On the plane, or near enough for a point of it: coordinates run to 1e9 mm, where the rounding of a
    /// crossing is still far below anything the model holds.
    const EPS: f64 = 1e-6;
    type Vertex = (u64, u64);
    type Edge = (Vertex, Vertex);
    let plan = |q: P3| P2(q[0], q[1]);
    let key = |q: P3| -> Vertex {
        let n = std::array::from_fn::<_, 2, _>(|i| if q[i] == 0.0 { 0.0 } else { q[i] }.to_bits());
        (n[0], n[1])
    };
    let edge = |a: P3, b: P3| -> Edge {
        let (ka, kb) = (key(a), key(b));
        if ka <= kb { (ka, kb) } else { (kb, ka) }
    };
    // Where a plane cuts an edge: cached by the edge's own vertices, so the two triangles that share an edge
    // fill it in with the very same point and the mesh stays welded (each is clipped on its own).
    let mut cut_at: std::collections::HashMap<Edge, P3> = std::collections::HashMap::new();
    let mut flat: Vec<[P3; 3]> = tris[..caps].to_vec();
    let mut sides: Vec<[P3; 3]> = tris[caps..].to_vec();
    for &(p0, p1, inside) in cuts {
        let n = (p1 - p0).perp();
        let nd = if (inside - p0).dot(n) < 0.0 { n * -1.0 } else { n };
        let d = |q: P3| (plan(q) - p0).dot(nd);
        // The rim of the cut face: the points of the clipped polygons that lie in the plane and are
        // joined there to another of them. The face a convex body takes through a plane is convex, so
        // ordering the rim about its middle closes the loop whatever the triangles under it were — no
        // chain to guess at, and the same order every time a scene is built.
        let mut rim: std::collections::BTreeMap<(i64, i64, i64), P3> = std::collections::BTreeMap::new();
        let grid = |q: P3| -> (i64, i64, i64) {
            let k = std::array::from_fn::<_, 3, _>(|i| (q[i] * 1e6).round() as i64);
            (k[0], k[1], k[2])
        };
        // One triangle kept where it lies, cut into a fan where the plane crosses it.
        let mut clip = |t: [P3; 3]| -> Vec<[P3; 3]> {
            let mut poly = Vec::with_capacity(4);
            for i in 0..3 {
                let (cur, next) = (t[i], t[(i + 1) % 3]);
                let (dc, dn) = (d(cur), d(next));
                if dc >= -EPS {
                    poly.push(cur);
                }
                if (dc > EPS && dn < -EPS) || (dc < -EPS && dn > EPS) {
                    let at = *cut_at.entry(edge(cur, next)).or_insert_with(|| {
                        let f = dc / (dc - dn);
                        std::array::from_fn(|k| cur[k] + (next[k] - cur[k]) * f)
                    });
                    poly.push(at);
                }
            }
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if d(a).abs() <= EPS && d(b).abs() <= EPS {
                    rim.entry(grid(a)).or_insert(a);
                    rim.entry(grid(b)).or_insert(b);
                }
            }
            (1..poly.len().saturating_sub(1)).map(|i| [poly[0], poly[i], poly[i + 1]]).collect()
        };
        for mesh in [&mut flat, &mut sides] {
            let was = std::mem::take(mesh);
            for t in was {
                mesh.extend(clip(t));
            }
        }
        // The cut face shows outward: where the kept side of the plane ends.
        let wanted = [-nd.0, -nd.1, 0.0];
        if rim.len() < 3 {
            continue;
        }
        let middle = rim.values().fold([0.0; 3], |s, p| std::array::from_fn(|i| s[i] + p[i] / rim.len() as f64));
        let along = (p1 - p0).unit();
        let mut ring: Vec<P3> = rim.into_values().collect();
        ring.sort_by(|a, b| {
            let turn = |q: &P3| {
                let s = P2(q[0] - middle[0], q[1] - middle[1]).dot(along);
                (q[2] - middle[2]).atan2(s)
            };
            turn(a).total_cmp(&turn(b))
        });
        // Newell's normal of the loop, turned to the side the cut face shows.
        let n = ring.iter().zip(ring.iter().cycle().skip(1)).fold([0.0; 3], |n, (a, b)| {
            [n[0] + (a[1] - b[1]) * (a[2] + b[2]), n[1] + (a[2] - b[2]) * (a[0] + b[0]), n[2] + (a[0] - b[0]) * (a[1] + b[1])]
        });
        if n[0] * wanted[0] + n[1] * wanted[1] < 0.0 {
            ring.reverse();
        }
        for i in 1..ring.len() - 1 {
            let (b, c) = (ring[i], ring[i + 1]);
            let cross = [
                (b[1] - ring[0][1]) * (c[2] - ring[0][2]) - (b[2] - ring[0][2]) * (c[1] - ring[0][1]),
                (b[2] - ring[0][2]) * (c[0] - ring[0][0]) - (b[0] - ring[0][0]) * (c[2] - ring[0][2]),
                (b[0] - ring[0][0]) * (c[1] - ring[0][1]) - (b[1] - ring[0][1]) * (c[0] - ring[0][0]),
            ];
            if cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2] > 1e-18 {
                flat.push([ring[0], b, c]);
            }
        }
    }
    let caps = flat.len();
    flat.extend(sides);
    (flat, caps)
}

/// A round run of length 1 and radius 1, from the origin along x: its caps' triangles first, and how many they are.
pub fn unit_tube() -> (Vec<[P3; 3]>, usize) {
    let ring = |x: f64| -> Vec<P3> {
        (0..SIDES)
            .map(|k| {
                let t = std::f64::consts::TAU * k as f64 / SIDES as f64;
                [x, t.cos(), t.sin()]
            })
            .collect()
    };
    let (a, b) = ([0.0; 3], [1.0, 0.0, 0.0]);
    let (ra, rb) = (ring(0.0), ring(1.0));
    let next = |k: usize| (k + 1) % SIDES;
    let mut tris = Vec::with_capacity(4 * SIDES);
    for k in 0..SIDES {
        tris.push([a, ra[next(k)], ra[k]]);
        tris.push([b, rb[k], rb[next(k)]]);
    }
    for k in 0..SIDES {
        tris.push([ra[k], ra[next(k)], rb[k]]);
        tris.push([ra[next(k)], rb[next(k)], rb[k]]);
    }
    (tris, 2 * SIDES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> (Model, crate::model::Guid) {
        let m = Model::new("t");
        let level = m.guid(m.levels()[0].0);
        (m, level)
    }

    fn run(level: crate::model::Guid, a: P2, b: P2, offset: f64, end_offset: Option<f64>, justify: Justify) -> Run {
        Run {
            square_ends: false,
            section_roll: 0.0,
            mount: None,
            connections: Vec::new(),
            feeder: None,
            type_name: "t".into(),
            level,
            start: a,
            end: b,
            offset,
            end_offset,
            justify,
            join: true,
            system: String::new(),
            source: None,
            devices: Vec::new(),
            cable: String::new(),
            reserve: 0.0,
            along_walls: false,
            carrier: None,
            local_offset: 0.0,
            size_locked: false,
            compartments: Default::default(),
            compartment: String::new(),
            params: Default::default(),
        }
    }

    fn rect(width: f64, depth: f64) -> RunType {
        RunType { width, depth, round: false, material: String::new() }
    }

    fn volume(tris: &[[P3; 3]]) -> f64 {
        // The divergence theorem: the sum of the signed volumes of the tetrahedra to the origin; positive if turned out.
        tris.iter().map(|[a, b, c]| crate::view3d::dot(*a, crate::view3d::cross(*b, *c)) / 6.0).sum()
    }

    #[test]
    fn a_runs_heights_are_those_of_its_justify_line() {
        let (m, level) = model();
        let t = rect(200.0, 500.0);
        let at = |j| Geom::of(&m, &run(level, P2(0.0, 0.0), P2(6000.0, 0.0), 2800.0, None, j), &t);
        // The top of the section is at 2800 for a top, the bottom for a bottom, the middle for a centre.
        assert_eq!(at(Justify::Top).z_span(), (2300.0, 2800.0));
        assert_eq!(at(Justify::Bottom).z_span(), (2800.0, 3300.0));
        assert_eq!(at(Justify::Centre).z_span(), (2550.0, 3050.0));
        assert_eq!((at(Justify::Top).a, at(Justify::Top).b), ([0.0, 0.0, 2550.0], [6000.0, 0.0, 2550.0]));
        assert_eq!(at(Justify::Top).length(), 6000.0);
    }

    #[test]
    fn a_sloped_run_keeps_its_section_square_to_its_axis() {
        let (m, level) = model();
        // 3000 along, 4000 up the ends' difference ... a 3-4-5: 3000 in plan, 4000 up, 5000 along.
        let g = Geom::of(&m, &run(level, P2(0.0, 0.0), P2(3000.0, 0.0), 0.0, Some(4000.0), Justify::Centre), &rect(100.0, 200.0));
        assert_eq!(g.length(), 5000.0);
        // Vertically its section is 200 / cos θ = 200 / 0.6 deep, so the prism's thickness is that, on each side of the line.
        let (ring, bottom, top) = g.prism();
        let q = ring[0];
        assert!((top.at(q) - bottom.at(q) - 200.0 / 0.6).abs() < 1e-9, "{} {}", top.at(q), bottom.at(q));
        // Its volume is the section's area times its length, whatever its slope (its ends are cut upright).
        let plan = crate::geom::area(&ring);
        let v = plan * (top.at(P2(1500.0, 0.0)) - bottom.at(P2(1500.0, 0.0)));
        assert!((v - 100.0 * 200.0 * 5000.0).abs() < 1.0, "{v}");
        assert!(plan > 0.0, "counter-clockwise");
        // The planes are over the line wherever it starts: 500 up at its start, 4500 at its end, 200 / 0.6 / 2 each way.
        let g =
            Geom::of(&m, &run(level, P2(1000.0, 2000.0), P2(4000.0, 2000.0), 500.0, Some(4500.0), Justify::Centre), &rect(100.0, 200.0));
        let (_, bottom, top) = g.prism();
        let half = 100.0 / 0.6;
        assert!(
            (bottom.at(P2(1000.0, 2000.0)) - (500.0 - half)).abs() < 1e-9 && (top.at(P2(4000.0, 2000.0)) - (4500.0 + half)).abs() < 1e-9
        );
    }

    #[test]
    fn a_run_a_millimetre_long_in_plan_stands_and_a_shorter_one_is_too_short() {
        let (m, level) = model();
        let at = |to: P2, end_offset| Geom::of(&m, &run(level, P2(0.0, 0.0), to, 0.0, end_offset, Justify::Centre), &rect(100.0, 200.0));
        assert!(at(P2(TOL, 0.0), Some(500.0)).vertical() && !at(P2(TOL + 0.01, 0.0), Some(500.0)).vertical());
        assert!(check("a beam 1", "beam", &at(P2(TOL, 0.0), None)).unwrap_err().contains("too short"), "a millimetre is too short");
        assert!(check("a beam 2", "beam", &at(P2(0.5, 0.0), None)).unwrap_err().contains("too short"));
        assert!(check("a beam 3", "beam", &at(P2(2.0, 0.0), None)).is_ok());
    }

    #[test]
    fn a_footprint_is_a_strip_or_when_upright_its_section() {
        let (m, level) = model();
        let strip = Geom::of(&m, &run(level, P2(0.0, 0.0), P2(4000.0, 0.0), 0.0, None, Justify::Top), &rect(300.0, 500.0)).footprint();
        assert_eq!(strip, vec![P2(0.0, -150.0), P2(4000.0, -150.0), P2(4000.0, 150.0), P2(0.0, 150.0)]);
        // Upright: its section, width along east and depth along north, round a point; its heights from end to end.
        let up = Geom::of(&m, &run(level, P2(1000.0, 1000.0), P2(1000.0, 1000.0), 0.0, Some(3000.0), Justify::Centre), &rect(300.0, 500.0));
        assert!(up.vertical());
        assert_eq!(up.length(), 3000.0);
        assert_eq!(up.footprint(), vec![P2(850.0, 750.0), P2(1150.0, 750.0), P2(1150.0, 1250.0), P2(850.0, 1250.0)]);
        assert_eq!(up.z_span(), (0.0, 3000.0));
        let (ring, bottom, top) = up.prism();
        assert_eq!((ring.len(), bottom.at(ring[0]), top.at(ring[0])), (4, 0.0, 3000.0));
    }

    #[test]
    fn a_round_run_is_a_closed_tube_turned_out() {
        let (m, level) = model();
        let round = RunType { width: 200.0, depth: 200.0, round: true, material: String::new() };
        // From a point off the origin, where a cap turned the wrong way doesn't cancel in the volume.
        let from = P2(1000.0, -2000.0);
        for (by, end_offset) in
            [(P2(5000.0, 0.0), None), (P2(3000.0, 4000.0), Some(1000.0)), (P2(0.0, 0.0), Some(2000.0)), (P2(-3000.0, 1000.0), Some(-700.0))]
        {
            let g = Geom::of(&m, &run(level, from, from + by, 500.0, end_offset, Justify::Centre), &round);
            let (tris, caps) = g.tube();
            assert_eq!((tris.len(), caps), (4 * SIDES, 2 * SIDES));
            // The area of a regular polygon of SIDES sides in a circle of radius 100.
            let area = SIDES as f64 / 2.0 * 100.0 * 100.0 * (std::f64::consts::TAU / SIDES as f64).sin();
            assert!((volume(&tris) - area * g.length()).abs() < 1e-3 * area * g.length(), "{} {}", volume(&tris), area * g.length());
        }
    }

    #[test]
    fn a_runs_reach_covers_its_section_square_to_its_axis() {
        let (m, level) = model();
        let g = Geom::of(&m, &run(level, P2(0.0, 0.0), P2(3000.0, 0.0), 0.0, Some(4000.0), Justify::Centre), &rect(100.0, 200.0));
        let (lo, hi) = g.z_span();
        assert!((lo - -100.0 / 0.6).abs() < 1e-9 && (hi - (4000.0 + 100.0 / 0.6)).abs() < 1e-9, "{lo} {hi}");
    }
}
