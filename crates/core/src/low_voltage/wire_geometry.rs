use crate::geom::P2;
use crate::model::{Component, Justify, Model, TOL};
use crate::runs::Geom;
use crate::view3d::{P3, add, mul, sub};

mod connections;
mod device_exit;
pub(crate) use connections::bend_fits;

impl Model {
    pub(crate) fn fixed_wire_contact(&self, component: &Component, [x, y, z]: P3) -> P3 {
        let y = if component.mirrored { -y } else { y };
        let (sin, cos) = component.angle.to_radians().sin_cos();
        [component.at.0 + x * cos - y * sin, component.at.1 + x * sin + y * cos, self.elevation(component.level) + component.offset + z]
    }

    /// Unlinked wire segments that fit the carrier’s physical section and span.
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
        let disconnected: Vec<_> = self
            .elements
            .iter()
            .filter_map(|(&id, element)| match element {
                Element::Wire(run) if run.connections.iter().any(|c| removed.contains(&c.device)) => Some(id),
                _ => None,
            })
            .collect();
        for id in disconnected {
            if let Some(Element::Wire(run)) = self.elements.get_mut(id) {
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
                    let device = self.fixed_wire_contact(component, connection.point);
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
