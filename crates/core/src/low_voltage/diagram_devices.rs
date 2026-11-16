//! Full and simplified device chains from explicit line topology, never traversal adjacency.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ids::{Guid, Id};
use crate::model::{Component, Element, Model};

const MAX_OCCURRENCES: usize = 10_000;

#[derive(Serialize)]
pub(crate) struct Segment {
    pub(crate) line: Id,
    pub(crate) line_guid: Guid,
    pub(crate) system: String,
    pub(crate) label: String,
    pub(crate) level: Guid,
    pub(crate) from: Option<Id>,
    pub(crate) from_guid: Option<Guid>,
    pub(crate) from_mark: Option<String>,
    pub(crate) items: Vec<Item>,
}

#[derive(Serialize)]
pub(crate) struct Item {
    pub(crate) kind: String,
    pub(crate) role: String,
    pub(crate) family: String,
    pub(crate) type_name: String,
    pub(crate) ids: Vec<Id>,
    pub(crate) guids: Vec<Guid>,
    pub(crate) marks: Vec<Option<String>>,
    pub(crate) omitted: usize,
}

struct Device<'a> {
    id: Id,
    component: &'a Component,
    mark: Option<String>,
    elevation: f64,
}

impl Device<'_> {
    fn item(&self, guid: Guid, role: &str) -> Item {
        Item {
            kind: "device".into(),
            role: role.into(),
            family: self.component.family.clone(),
            type_name: self.component.type_name.clone(),
            ids: vec![self.id],
            guids: vec![guid],
            marks: vec![self.mark.clone()],
            omitted: 0,
        }
    }
}

fn device(m: &Model, guid: Guid, mark: Option<String>) -> Result<Device<'_>, String> {
    let Some(Element::Component(component)) = m.elements.get(guid) else {
        return Err(format!("Electrical diagram member {guid} must be a placed component"));
    };
    if !component.is_placed() {
        return Err(format!("Electrical diagram member {guid} is not placed"));
    }
    let id = m.elements.id(guid).ok_or_else(|| format!("Electrical diagram member {guid} has no ID"))?;
    let Some(Element::Level(level)) = m.elements.get(component.level) else {
        return Err(format!("Electrical diagram member {id} needs an actual floor"));
    };
    if !level.elevation.is_finite() {
        return Err(format!("Electrical diagram floor {} has an invalid elevation", component.level));
    }
    Ok(Device { id, component, mark, elevation: level.elevation })
}

fn chain_items(chain: &[(Guid, &Device<'_>)], locked: &BTreeSet<Guid>, simplified: bool) -> Vec<Item> {
    let mut items = Vec::new();
    for run in chain.chunk_by(|(a_guid, a), (b_guid, b)| {
        simplified
            && !locked.contains(a_guid)
            && !locked.contains(b_guid)
            && a.component.family == b.component.family
            && a.component.type_name == b.component.type_name
            && a.component.params == b.component.params
    }) {
        if run.len() < 3 {
            items.extend(run.iter().map(|(guid, device)| device.item(*guid, "device")));
            continue;
        }
        let mut middle = run.iter();
        if let (Some((first_guid, first)), Some((last_guid, last))) = (middle.next(), middle.next_back()) {
            items.push(first.item(*first_guid, "device"));
            let mut gap = Item {
                kind: "gap".into(),
                role: "device".into(),
                family: first.component.family.clone(),
                type_name: first.component.type_name.clone(),
                ids: Vec::new(),
                guids: Vec::new(),
                marks: Vec::new(),
                omitted: 0,
            };
            for (guid, device) in middle {
                gap.ids.push(device.id);
                gap.guids.push(*guid);
                gap.marks.push(device.mark.clone());
            }
            gap.omitted = gap.ids.len();
            items.push(gap);
            items.push(last.item(*last_guid, "device"));
        }
    }
    items
}

pub(crate) fn segments(
    m: &Model,
    selected: Option<Id>,
    system: Option<&str>,
    section: Option<&str>,
    simplified: bool,
) -> Result<Vec<Segment>, String> {
    if let Some(id) = selected
        && !matches!(m.elements.get(id), Some(Element::ElectricalLine(_)))
    {
        return Err(format!("Electrical diagram selection {id} must be an explicit electrical line"));
    }
    let mut lines = Vec::new();
    let mut occurrences = 0;
    // Count before order(), marks(), graph construction or output allocation; sources count too.
    for (&id, element) in m.elements.iter() {
        let Element::ElectricalLine(line) = element else { continue };
        if selected.is_some_and(|selected| selected != id)
            || system.is_some_and(|system| system != line.system)
            || section.is_some_and(|section| section != line.section())
        {
            continue;
        }
        if lines.len() == MAX_OCCURRENCES {
            return Err("Electrical diagram exceeds 10000 lines".into());
        }
        if line.branches.len() > MAX_OCCURRENCES || line.locked_marks.len() > MAX_OCCURRENCES {
            return Err("Electrical diagram exceeds 10000 aggregate device occurrences".into());
        }
        for _ in line.members().chain(line.isolators.iter().copied()) {
            if occurrences == MAX_OCCURRENCES {
                return Err("Electrical diagram exceeds 10000 aggregate device occurrences".into());
            }
            occurrences += 1;
        }
        lines.push((id, line));
    }

    let mut segments = Vec::new();
    for (id, line) in lines {
        let line_guid = m.elements.guid(id).ok_or_else(|| format!("Electrical diagram line {id} has no GUID"))?;
        let label = if line.mark_prefix.is_empty() { id.to_string() } else { line.mark_prefix.clone() };
        let order = line.order(m);
        let marks = line.marks(m);
        if order.len() != line.members().count() || marks.len() != order.len() {
            return Err(format!("Electrical diagram line {id} has unreachable or inconsistent members"));
        }
        let mut devices = BTreeMap::new();
        for (&guid, mark) in order.iter().zip(marks) {
            devices.insert(guid, device(m, guid, Some(mark))?);
        }
        let locked: BTreeSet<_> = line.locked_marks.iter().map(|mark| mark.device).collect();
        let branch_starts: BTreeSet<_> = line.branches.iter().filter_map(|branch| branch.devices.first().copied()).collect();
        let mut parents = BTreeMap::new();
        let mut children = BTreeMap::<Guid, Vec<Guid>>::new();
        let trunk = std::iter::once(line.source).chain(line.devices.iter().copied()).zip(line.devices.iter().copied());
        let branches = line
            .branches
            .iter()
            .flat_map(|branch| std::iter::once(branch.from).chain(branch.devices.iter().copied()).zip(branch.devices.iter().copied()));
        for (parent, child) in trunk.chain(branches) {
            if parents.insert(child, parent).is_some() || child == line.source {
                return Err(format!("Electrical diagram line {id} has inconsistent parent connections"));
            }
            children.entry(parent).or_default().push(child);
        }
        let mut seen = BTreeSet::new();
        for (index, &guid) in order.iter().enumerate().skip(1) {
            let first = devices.get(&guid).ok_or_else(|| format!("Electrical diagram member {guid} is missing"))?;
            let parent_guid = *parents.get(&guid).ok_or_else(|| format!("Electrical diagram member {guid} has no parent"))?;
            let parent = devices.get(&parent_guid).ok_or_else(|| format!("Electrical diagram parent {parent_guid} is missing"))?;
            if parent_guid != line.source
                && !branch_starts.contains(&guid)
                && children.get(&parent_guid).is_some_and(|children| children.len() == 1)
                && parent.component.level == first.component.level
            {
                continue;
            }
            let mut chain = Vec::new();
            let mut current = guid;
            loop {
                if !seen.insert(current) {
                    return Err(format!("Electrical diagram line {id} repeats member {current}"));
                }
                let member = devices.get(&current).ok_or_else(|| format!("Electrical diagram member {current} is missing"))?;
                chain.push((current, member));
                let Some(next) = children.get(&current).filter(|children| children.len() == 1).and_then(|children| children.first()) else {
                    break;
                };
                let next_member = devices.get(next).ok_or_else(|| format!("Electrical diagram member {next} is missing"))?;
                if branch_starts.contains(next) || next_member.component.level != first.component.level {
                    break;
                }
                current = *next;
            }
            segments.push((
                first.elevation,
                index,
                Segment {
                    line: id,
                    line_guid,
                    system: line.system.clone(),
                    label: label.clone(),
                    level: first.component.level,
                    from: Some(parent.id),
                    from_guid: Some(parent_guid),
                    from_mark: parent.mark.clone(),
                    items: chain_items(&chain, &locked, simplified),
                },
            ));
        }
        if seen.len() + 1 != order.len() {
            return Err(format!("Electrical diagram line {id} has members outside its chains"));
        }
        for (index, &guid) in line.isolators.iter().enumerate() {
            if devices.contains_key(&guid) || !seen.insert(guid) {
                return Err(format!("Electrical diagram line {id} repeats standalone isolator {guid}"));
            }
            let isolator = device(m, guid, None)?;
            segments.push((
                isolator.elevation,
                order.len() + index,
                Segment {
                    line: id,
                    line_guid,
                    system: line.system.clone(),
                    label: label.clone(),
                    level: isolator.component.level,
                    from: None,
                    from_guid: None,
                    from_mark: None,
                    items: vec![isolator.item(guid, "isolator")],
                },
            ));
        }
    }
    segments.sort_by(|(a_elevation, a_index, a), (b_elevation, b_index, b)| {
        b_elevation
            .total_cmp(a_elevation)
            .then(a.system.cmp(&b.system))
            .then_with(|| crate::low_voltage::diagram_layout::natural_label(&a.label, &b.label))
            .then(a.line.cmp(&b.line))
            .then(a_index.cmp(b_index))
    });
    Ok(segments.into_iter().map(|(_, _, segment)| segment).collect())
}
