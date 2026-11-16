//! Cable fault simulation for SP 484 Amendment 1, 6.3.5.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::family::Category;
use crate::model::{ElectricalLine, Element, Guid, Id, Model, WireConnection};

const MAX_CABLES: usize = 512;
pub(crate) const MAX_PLACEMENT_CABLES: usize = 128;
const MAX_PAIR_CANDIDATES: usize = 24;

/// Voltage at each load on a radial DC line, from round-trip resistance and local current.
pub(crate) fn radial_voltage(source_v: f64, segments: &[(f64, f64)]) -> Vec<f64> {
    let mut current = 0.0;
    let mut voltages: Vec<_> = segments
        .iter()
        .rev()
        .map(|&(resistance, load)| {
            current += load;
            resistance * current
        })
        .collect();
    voltages.reverse();
    let mut voltage = source_v;
    for drop in &mut voltages {
        voltage -= *drop;
        *drop = voltage;
    }
    voltages
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Node {
    device: Guid,
    port: Option<Guid>,
}

#[derive(Clone)]
struct Cable {
    id: Id,
    ends: [Node; 2],
}

#[derive(Clone)]
struct Network {
    cables: Vec<Cable>,
    incident: BTreeMap<Node, Vec<usize>>,
    sources: BTreeSet<Node>,
}

type SupplyNetworks = (Vec<(Guid, Network)>, BTreeSet<Guid>);

impl Network {
    fn read(model: &Model, line: &ElectricalLine, independent_ports: bool) -> Result<Self, &'static str> {
        if line.segments.is_empty() || line.segments.len() > MAX_CABLES {
            return Err("the line needs between 1 and 512 main cable segments");
        }
        let members: BTreeSet<_> = line.members().chain(line.isolators.iter().copied()).collect();
        let terminals: BTreeSet<_> = line.terminals.iter().copied().collect();
        let source_port = |end: &WireConnection| {
            if independent_ports && end.device == line.source { end.terminal } else { None }
        };
        let mut net = Self { cables: Vec::new(), incident: BTreeMap::new(), sources: BTreeSet::new() };
        for &guid in &line.segments {
            let Some((id, Element::Wire(wire))) = model.elements.id(guid).zip(model.elements.get(guid)) else {
                return Err("a cable segment could not be resolved");
            };
            let [a, b] = wire.connections.as_slice() else {
                return Err("each cable needs two explicit equipment end connections");
            };
            if a.device == b.device && a.end == b.end {
                return Err("a cable returning to the same device needs two distinct ends");
            }
            let node = |end: &WireConnection| {
                if !members.contains(&end.device) || end.terminal.is_some_and(|port| !terminals.contains(&port)) {
                    return Err("a connected device or terminal is missing from the line");
                }
                let port = source_port(end);
                if independent_ports && end.device == line.source && port.is_none() {
                    return Err("independent source ports require a named terminal on every source cable end");
                }
                Ok(Node { device: end.device, port })
            };
            let ends = [node(a)?, node(b)?];
            if ends[0] == ends[1] {
                return Err("a cable has both ends on the same electrical node");
            }
            for end in ends {
                net.incident.entry(end).or_default().push(net.cables.len());
                if end.device == line.source {
                    net.sources.insert(end);
                }
            }
            net.cables.push(Cable { id, ends });
        }
        let connected: BTreeSet<_> = net.energized(&[], &BTreeSet::new(), |_| true).into_iter().map(|node| node.device).collect();
        if connected != members {
            return Err("the physical cables do not connect every member of the line to the source");
        }
        Ok(net)
    }

    fn shorted(&self, cable: usize, isolators: &BTreeSet<Guid>) -> (Vec<bool>, BTreeSet<Node>) {
        let mut blocked = vec![false; self.cables.len()];
        let mut contaminated = BTreeSet::new();
        let mut stack = vec![cable];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut blocked[i], true) {
                continue;
            }
            for end in self.cables[i].ends {
                if !isolators.contains(&end.device) && contaminated.insert(end) {
                    stack.extend(self.incident.get(&end).into_iter().flatten());
                }
            }
        }
        (blocked, contaminated)
    }

    fn energized(&self, blocked: &[bool], contaminated: &BTreeSet<Node>, available: impl Fn(Guid) -> bool) -> BTreeSet<Node> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<_> = self.sources.iter().copied().filter(|root| !contaminated.contains(root)).collect();
        while let Some(node) = stack.pop() {
            if !available(node.device) || !seen.insert(node) {
                continue;
            }
            for &i in self.incident.get(&node).into_iter().flatten() {
                if !blocked.get(i).copied().unwrap_or(false) {
                    stack.extend(self.cables[i].ends);
                }
            }
        }
        seen
    }

    fn with_isolator(&self, index: usize, device: Guid) -> Self {
        let mut net = self.clone();
        let end = net.cables[index].ends[1];
        let node = Node { device, port: None };
        let second = net.cables.len();
        net.cables[index].ends[1] = node;
        net.cables.push(Cable { id: net.cables[index].id, ends: [node, end] });
        net.incident.insert(node, vec![index, second]);
        if let Some(cables) = net.incident.get_mut(&end) {
            cables.retain(|&i| i != index);
            cables.push(second);
        }
        net
    }
}

fn best_isolator_set(
    net: &Network,
    isolators: &BTreeSet<Guid>,
    splits: &[(Id, usize)],
    count: usize,
    baseline: Score,
    audit: impl Fn(&Network, &BTreeSet<Guid>) -> Score,
) -> Option<(Score, Vec<Id>)> {
    if splits.len() < count {
        return None;
    }
    // Pending IDs cannot occur in a validated network. Splits append cable halves,
    // so later splits can still use their original cable index.
    let devices: Vec<_> = (0..count as Id).map(Guid::of_id).collect();
    let protected = isolators.iter().copied().chain(devices.iter().copied()).collect();
    let mut choice: Vec<_> = (0..count).collect();
    let mut best = None;
    loop {
        let mut network = net.clone();
        let mut wires = Vec::with_capacity(count);
        for (&slot, &device) in choice.iter().zip(&devices) {
            let (wire, index) = splits[slot];
            network = network.with_isolator(index, device);
            wires.push(wire);
        }
        let score = audit(&network, &protected);
        if score < baseline && best.as_ref().is_none_or(|(previous, _)| score < *previous) {
            best = Some((score, wires));
        }
        let Some(pivot) = (0..count).rev().find(|&i| choice[i] < splits.len() - count + i) else { break };
        choice[pivot] += 1;
        for i in pivot + 1..count {
            choice[i] = choice[i - 1] + 1;
        }
    }
    best
}

fn insufficient(reason: &str) -> Value {
    json!({"clause":"6.3.5", "scope":"communication", "status":"insufficient_data", "reason":reason})
}

pub(crate) fn report(
    model: &Model,
    line: &ElectricalLine,
    isolators: &BTreeSet<Guid>,
    independent_ports: bool,
    candidates: Option<&BTreeMap<Id, (f64, [f64; 3])>>,
    max_isolators: usize,
    power: Option<PowerScope<'_>>,
) -> Value {
    let category = |device: Guid| {
        let Element::Component(component) = model.elements.get(device)? else { return None };
        model.families.get(&component.family).map(|family| (family.category, component.level))
    };
    if !matches!(category(line.source), Some((Category::AlarmPanel, _))) {
        return insufficient("select a line with a fire alarm panel as its source");
    }
    let net = match Network::read(model, line, independent_ports) {
        Ok(net) => net,
        Err(reason) => return insufficient(reason),
    };
    if isolators
        .iter()
        .any(|&device| device == line.source || net.incident.get(&Node { device, port: None }).is_none_or(|cables| cables.len() != 2))
    {
        return insufficient("each declared bidirectional isolator needs exactly two connected cables on the line");
    }
    let detectors: Vec<_> = line
        .members()
        .filter_map(|guid| {
            let (kind, level) = category(guid)?;
            let automatic = match kind {
                Category::SmokeDetector | Category::HeatDetector => true,
                Category::CallPoint => false,
                _ => return None,
            };
            Some((guid, model.elements.id(guid)?, automatic, level))
        })
        .collect();
    let levels = detectors.iter().filter_map(|(_, _, automatic, level)| automatic.then_some(*level)).collect();
    let automatic_ids: Vec<_> = detectors.iter().filter_map(|(_, id, automatic, _)| automatic.then_some(*id)).collect();
    let memberships = crate::low_voltage::zone_geometry::detection_memberships(model, &levels, &automatic_ids);
    let audit = |net: &Network, isolators: &BTreeSet<Guid>, details| {
        assess(net, isolators, &detectors, memberships.as_ref(), details, |blocked, contaminated, short| {
            let mut live = net.energized(blocked, contaminated, |_| true);
            if short {
                // The sensing head of a built-in isolator may lose power on its shorted side.
                live.retain(|node| {
                    !isolators.contains(&node.device) || !net.incident.get(node).is_some_and(|cables| cables.iter().any(|&i| blocked[i]))
                });
            }
            live
        })
    };
    let (before, faults) = audit(&net, isolators, true);
    let mut report = json!({"clause":"6.3.5", "scope":"communication", "status":before.status(), "faults":faults});
    if let Some(candidates) = candidates {
        if before.status() == "pass" {
            report["placement_status"] = json!("already_passes");
        } else if net.cables.len() > MAX_PLACEMENT_CABLES {
            report["placement_status"] = json!("too_many_cables");
        } else {
            // The virtual device is absent from any valid model. Audit both halves of its split.
            let virtual_device = Guid::default();
            let mut protected = isolators.clone();
            protected.insert(virtual_device);
            let best = net
                .cables
                .iter()
                .enumerate()
                .filter_map(|(index, cable)| {
                    let &(fraction, at) = candidates.get(&cable.id)?;
                    let (after, _) = audit(&net.with_isolator(index, virtual_device), &protected, false);
                    (after < before).then_some((after, cable.id, fraction, at))
                })
                .min_by_key(|&(score, id, _, _)| (score, id));
            report["placement_status"] = json!(if best.is_some() { "proposed" } else { "no_improving_placement" });
            if let Some((after, wire, fraction, at)) = best {
                report["placement"] = json!({"wire":wire,"fraction":fraction,"at":at,"scope":"communication","status_after":after.status(),
                    "failing_faults_after":after.failed,"unknown_faults_after":after.unknown});
            }
            let mut best_set = None;
            let mut baseline = best.map_or(before, |(score, ..)| score);
            for count in 2..=max_isolators {
                if baseline.status() == "pass" {
                    break;
                }
                let limit = if count == 2 { MAX_PAIR_CANDIDATES } else { 16 };
                if candidates.len() > limit {
                    let field = if count == 2 { "pair_search" } else { "set_search" };
                    report[field] = json!("too_many_candidates");
                    break;
                }
                let mut splits: Vec<_> = net
                    .cables
                    .iter()
                    .enumerate()
                    .filter(|(_, cable)| candidates.contains_key(&cable.id))
                    .map(|(index, cable)| (cable.id, index))
                    .collect();
                splits.sort_unstable();
                if let Some((score, wires)) =
                    best_isolator_set(&net, isolators, &splits, count, baseline, |network, devices| audit(network, devices, false).0)
                {
                    baseline = score;
                    best_set = Some((score, wires));
                }
            }
            if let Some((score, wires)) = best_set {
                let pair = wires.len() == 2;
                report["placement_status"] = json!(if pair { "proposed_pair" } else { "proposed_set" });
                let prefix = if pair { "pair" } else { "set" };
                report[&format!("{prefix}_status_after")] = json!(score.status());
                report[&format!("{prefix}_failing_faults_after")] = json!(score.failed);
                report[&format!("{prefix}_unknown_faults_after")] = json!(score.unknown);
                report["placements"] = json!(
                    wires
                        .into_iter()
                        .filter_map(|wire| {
                            let (fraction, at) = candidates.get(&wire)?;
                            Some(json!({"wire":wire,"fraction":fraction,"at":at,"scope":"communication"}))
                        })
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    if let Some(power) = power {
        let supply = power.networks(model, line).map_or_else(
            |reason| json!({"status":"insufficient_data","reason":reason}),
            |(supplies, roots)| {
                let devices = |network: &Network, blocked: &[bool], contaminated: &BTreeSet<Node>| {
                    network.energized(blocked, contaminated, |_| true).into_iter().map(|node| node.device).collect::<BTreeSet<_>>()
                };
                let empty = BTreeSet::new();
                let healthy: Vec<_> = supplies.iter().map(|(_, supply)| devices(supply, &[], &empty)).collect();
                let nominal = powered(&supplies, &healthy, &roots, None);
                if supplies.iter().any(|(source, _)| !nominal.contains(source)) {
                    return json!({"status":"insufficient_data","reason":"a power source has no path from an independent supply"});
                }
                let mut faults = Vec::new();
                let mut status = "pass";
                for (index, (_, supply)) in supplies.iter().enumerate() {
                    let (score, cases) =
                        assess(supply, &BTreeSet::new(), &detectors, memberships.as_ref(), true, |blocked, contaminated, _| {
                            let affected = devices(supply, blocked, contaminated);
                            let supplied = powered(&supplies, &healthy, &roots, Some((index, &affected)));
                            net.energized(&[], &empty, |device| power.independent.contains(&device) || supplied.contains(&device))
                        });
                    if score.failed > 0 || (score.unknown > 0 && status == "pass") {
                        status = score.status();
                    }
                    faults.extend(cases);
                }
                json!({"status":status,"faults":faults})
            },
        );
        report["scope"] = json!("communication_and_power");
        report["communication_status"] = json!(before.status());
        report["status"] = if before.status() == "fail" || supply["status"] == "fail" {
            json!("fail")
        } else if before.status() == "pass" && supply["status"] == "pass" {
            json!("pass")
        } else {
            json!("insufficient_data")
        };
        report["power"] = supply;
    }
    report
}

pub(crate) struct PowerScope<'a> {
    pub(crate) lines: Vec<(&'a ElectricalLine, bool)>,
    pub(crate) independent: BTreeSet<Guid>,
}

impl PowerScope<'_> {
    fn networks(&self, model: &Model, communication: &ElectricalLine) -> Result<SupplyNetworks, &'static str> {
        let mut segments: BTreeSet<_> = communication.segments.iter().copied().collect();
        let communication_cables = segments.len();
        let mut loads = BTreeSet::new();
        let mut supplied = BTreeSet::new();
        let mut networks = Vec::new();
        for &(line, independent_ports) in &self.lines {
            if line.segments.iter().any(|&id| !segments.insert(id)) {
                return Err("the power lines must use separate cables from each other and from the communication line");
            }
            loads.extend(line.members().chain(line.isolators.iter().copied()).filter(|&device| device != line.source));
            supplied.insert(line.source);
            networks.push((line.source, Network::read(model, line, independent_ports)?));
        }
        if segments.len() - communication_cables > MAX_CABLES {
            return Err("the power assessment supports up to 512 cable segments across its lines");
        }
        supplied.extend(loads.iter().copied());
        let communication_devices: BTreeSet<_> = communication.members().chain(communication.isolators.iter().copied()).collect();
        if !self.independent.is_subset(&communication_devices) || !self.independent.is_disjoint(&supplied) {
            return Err("independent devices must belong to the communication line and not use the selected power line");
        }
        if communication_devices.iter().any(|device| !supplied.contains(device) && !self.independent.contains(device))
            || self.independent.len() == communication_devices.len()
        {
            return Err("account for power to every communication device and use a power line that feeds at least one");
        }
        let roots = networks.iter().map(|(source, _)| *source).filter(|source| !loads.contains(source)).collect();
        Ok((networks, roots))
    }
}

fn powered(
    supplies: &[(Guid, Network)],
    healthy: &[BTreeSet<Guid>],
    roots: &BTreeSet<Guid>,
    fault: Option<(usize, &BTreeSet<Guid>)>,
) -> BTreeSet<Guid> {
    let mut live = BTreeSet::new();
    for _ in supplies {
        let before = live.len();
        for (index, ((source, _), devices)) in supplies.iter().zip(healthy).enumerate() {
            if roots.contains(source) || live.contains(source) {
                let available = fault.filter(|(failed, _)| *failed == index).map_or(devices, |(_, devices)| devices);
                live.extend(available);
            }
        }
        if live.len() == before {
            break;
        }
    }
    live
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    failed: usize,
    unknown: usize,
    worst_loss: usize,
    total_loss: usize,
}

impl Score {
    fn status(self) -> &'static str {
        if self.failed > 0 {
            "fail"
        } else if self.unknown > 0 {
            "insufficient_data"
        } else {
            "pass"
        }
    }
}

fn assess(
    net: &Network,
    isolators: &BTreeSet<Guid>,
    detectors: &[(Guid, Id, bool, Guid)],
    memberships: Option<&BTreeMap<Id, Vec<Id>>>,
    details: bool,
    reachable: impl Fn(&[bool], &BTreeSet<Node>, bool) -> BTreeSet<Node>,
) -> (Score, Vec<Value>) {
    let mut score = Score::default();
    let mut faults = Vec::new();
    for (i, cable) in net.cables.iter().enumerate() {
        let mut open = vec![false; net.cables.len()];
        open[i] = true;
        let (short, contaminated) = net.shorted(i, isolators);
        for (kind, blocked, contaminated) in [("open", open, BTreeSet::new()), ("short", short, contaminated)] {
            let live = reachable(&blocked, &contaminated, kind == "short");
            let mut automatic_lost = 0;
            let mut manual_lost = 0;
            let mut unassigned_automatic = 0;
            let mut zones = BTreeSet::new();
            let mut lost = Vec::new();
            for &(device, id, automatic, _) in detectors {
                let node = Node { device, port: None };
                if live.contains(&node) {
                    continue;
                }
                if details && lost.len() < 32 {
                    lost.push(id);
                }
                if automatic {
                    automatic_lost += 1;
                    match memberships.and_then(|members| members.get(&id)).map(Vec::as_slice) {
                        Some([zone]) => {
                            zones.insert(*zone);
                        }
                        _ => unassigned_automatic += 1,
                    }
                } else {
                    manual_lost += 1;
                }
            }
            let exceeds = automatic_lost > 24 || manual_lost > 1 || (automatic_lost > 0 && manual_lost > 0) || zones.len() > 1;
            score.failed += usize::from(exceeds);
            score.unknown += usize::from(!exceeds && unassigned_automatic > 0);
            score.worst_loss = score.worst_loss.max(automatic_lost + manual_lost);
            score.total_loss += automatic_lost + manual_lost;
            let status = if exceeds {
                "fail"
            } else if unassigned_automatic > 0 {
                "insufficient_data"
            } else {
                "pass"
            };
            if details {
                faults.push(json!({"wire":cable.id,"kind":kind,"status":status,"lost_sample":lost,"automatic_lost":automatic_lost,
                    "manual_lost":manual_lost,"zones_lost":zones,"unassigned_automatic":unassigned_automatic}));
            }
        }
    }
    (score, faults)
}
