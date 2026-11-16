//! Grouped floor inventory, independently laid out from explicit electrical-line membership.
//! Risers express membership only: this is not device order, branch topology or ring evidence.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::family::{Align, Mark};
use crate::geom::P2;
use crate::ids::{Guid, Id};
use crate::library::Kind;
use crate::model::{Component, Element, MAX_COORD, MAX_TEXT, Model};
use crate::pen::Pen;
use crate::render::{Drawing, Prim, Rgb, View};

pub(crate) const PAPER: P2 = P2(420.0, 297.0);
const MAX_MEMBERS: usize = 10_000;
const MAX_PAGES: usize = 64;
const LANES: usize = 6;
const STRIP: usize = 8;
const MAX_SYMBOL_POINTS: usize = 4096;
const MAX_SYMBOL_WORK: usize = 200_000;
const MAX_PRIMITIVES: usize = 200_000;
const MAX_VERTICES: usize = 2_000_000;
const TOP: f32 = 44.0;
const BOTTOM: f32 = 254.0;
const BLOCK_X: f32 = 194.0;
const BLOCK_W: f32 = 212.0;
const TEXT: f32 = 2.5;
const INK: Rgb = Rgb(25, 25, 25);
const MUTED: Rgb = Rgb(100, 100, 100);
const INVENTORY: &str = "Blocks are floor inventories, not device order, true branches or proof of a ring.";

#[derive(Serialize)]
pub(crate) struct Diagram {
    pub(crate) heads: Vec<Head>,
    pub(crate) bands: Vec<Band>,
    pub(crate) diagnostics: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct Head {
    pub(crate) id: Id,
    pub(crate) guid: Guid,
    pub(crate) level: Guid,
    pub(crate) mark: String,
    pub(crate) line_marks: BTreeMap<Id, String>,
    pub(crate) family: String,
    pub(crate) type_name: String,
    /// Only these explicit lines belong to this source; names never imply a connection.
    pub(crate) lines: Vec<Id>,
}

#[derive(Serialize)]
pub(crate) struct Band {
    pub(crate) level: Guid,
    pub(crate) name: String,
    pub(crate) elevation: f64,
    pub(crate) rows: Vec<Row>,
}

#[derive(Serialize)]
pub(crate) struct Row {
    pub(crate) line: Id,
    pub(crate) line_guid: Guid,
    pub(crate) system: String,
    pub(crate) label: String,
    pub(crate) groups: Vec<Group>,
}

#[derive(Serialize)]
pub(crate) struct Group {
    pub(crate) family: String,
    pub(crate) type_name: String,
    pub(crate) role: String,
    pub(crate) ids: Vec<Id>,
    pub(crate) guids: Vec<Guid>,
    pub(crate) marks: Vec<Option<String>>,
    pub(crate) count: usize,
    pub(crate) placeholder: bool,
}

type GroupKey = (String, String, String, bool);

pub(crate) fn natural_label(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (lower_a, lower_b) = (a.to_lowercase(), b.to_lowercase());
    let mut aa = lower_a.as_bytes().chunk_by(|x, y| x.is_ascii_digit() == y.is_ascii_digit());
    let mut bb = lower_b.as_bytes().chunk_by(|x, y| x.is_ascii_digit() == y.is_ascii_digit());
    loop {
        let order = match (aa.next(), bb.next()) {
            (Some(x), Some(y)) if x[0].is_ascii_digit() && y[0].is_ascii_digit() => {
                let x = &x[x.iter().take_while(|&&c| c == b'0').count()..];
                let y = &y[y.iter().take_while(|&&c| c == b'0').count()..];
                x.len().cmp(&y.len()).then(x.cmp(y))
            }
            (Some(x), Some(y)) => x.cmp(y),
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

fn checked_text(s: &str) -> Result<(), String> {
    if s.chars().count() > MAX_TEXT * 4 || s.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err("Electrical diagram text is too long or contains control characters".into());
    }
    Ok(())
}

fn placed(m: &Model, guid: Guid) -> Result<(Id, &Component), String> {
    let Some(Element::Component(c)) = m.elements.get(guid) else {
        return Err(format!("Electrical diagram member {guid} must be a placed component"));
    };
    if !c.is_placed() {
        return Err(format!("Electrical diagram member {guid} is not placed"));
    }
    let id = m.elements.id(guid).ok_or_else(|| format!("Electrical diagram member {guid} has no ID"))?;
    let Some(Element::Level(level)) = m.elements.get(c.level) else {
        return Err(format!("Electrical diagram member {id} needs an actual floor"));
    };
    if !level.elevation.is_finite() || level.elevation.abs() > MAX_COORD {
        return Err(format!("Electrical diagram floor {} has an invalid elevation", c.level));
    }
    for text in [&c.family, &c.type_name, &level.name] {
        checked_text(text)?;
    }
    if !m.families.get(&c.family).is_some_and(|f| f.types.contains_key(&c.type_name)) {
        return Err(format!("Electrical diagram member {id} needs an existing family and type"));
    }
    Ok((id, c))
}

pub(crate) fn build(m: &Model, selected: Option<Id>, system: Option<&str>, section: Option<&str>) -> Result<Diagram, String> {
    if let Some(id) = selected
        && !matches!(m.elements.get(id), Some(Element::ElectricalLine(_)))
    {
        return Err(format!("Electrical diagram selection {id} must be an explicit electrical line"));
    }
    if let Some(system) = system {
        checked_text(system)?;
    }
    let mut lines = Vec::new();
    let mut occurrences = 0;
    for (&id, element) in m.elements.iter() {
        let Element::ElectricalLine(line) = element else { continue };
        if selected.is_some_and(|selected| selected != id)
            || system.is_some_and(|system| system != line.system)
            || section.is_some_and(|section| section != line.section())
        {
            continue;
        }
        for text in [&line.mark_prefix, &line.system] {
            checked_text(text)?;
        }
        if line.branches.len() > MAX_MEMBERS || line.locked_marks.len() > MAX_MEMBERS {
            return Err("Electrical diagram exceeds 10000 aggregate member occurrences".into());
        }
        let mut seen = BTreeSet::new();
        for guid in line.members().chain(line.isolators.iter().copied()) {
            occurrences += 1;
            if occurrences > MAX_MEMBERS {
                return Err("Electrical diagram exceeds 10000 aggregate member occurrences".into());
            }
            if !seen.insert(guid) {
                return Err(format!("Electrical diagram line {id} repeats member {guid}"));
            }
            placed(m, guid)?;
        }
        lines.push((id, line));
    }
    if lines.is_empty() {
        return Ok(Diagram { heads: Vec::new(), bands: Vec::new(), diagnostics: Vec::new() });
    }
    lines.sort_by(|(a_id, a), (b_id, b)| {
        natural_label(&a.system, &b.system).then_with(|| natural_label(&a.mark_prefix, &b.mark_prefix)).then(a_id.cmp(b_id))
    });
    let mut heads = BTreeMap::<Guid, Head>::new();
    let mut bands = BTreeMap::<Guid, Band>::new();
    for (id, line) in lines {
        let (source_id, source) = placed(m, line.source)?;
        let order = line.order(m);
        let marks = line.marks(m);
        if order.len() != line.members().count() || marks.len() != order.len() {
            return Err(format!("Electrical diagram line {id} has unreachable or inconsistent members"));
        }
        let source_mark = marks.first().ok_or("Electrical diagram source mark is missing")?.clone();
        checked_text(&source_mark)?;
        let head = heads.entry(line.source).or_insert_with(|| Head {
            id: source_id,
            guid: line.source,
            level: source.level,
            // A shared source can have conflicting per-line traversal marks. Its ID is unambiguous.
            mark: source_id.to_string(),
            line_marks: BTreeMap::new(),
            family: source.family.clone(),
            type_name: source.type_name.clone(),
            lines: Vec::new(),
        });
        head.lines.push(id);
        head.line_marks.insert(id, source_mark);
        let mut floors = BTreeMap::<Guid, BTreeMap<GroupKey, Group>>::new();
        floors.entry(source.level).or_default();
        let members = order.into_iter().zip(marks).filter(|(guid, _)| *guid != line.source).map(|(g, mark)| (g, Some(mark), "device"));
        for (guid, mark, role) in members.chain(line.isolators.iter().map(|&g| (g, None, "isolator"))) {
            if let Some(mark) = &mark {
                checked_text(mark)?;
            }
            let (member_id, c) = placed(m, guid)?;
            let placeholder = m.library.placeholder(&Kind::FamilyType, &format!("{}: {}", c.family, c.type_name));
            let key = (c.family.clone(), c.type_name.clone(), role.to_owned(), placeholder);
            let group = floors.entry(c.level).or_default().entry(key).or_insert_with(|| Group {
                family: c.family.clone(),
                type_name: c.type_name.clone(),
                role: role.to_owned(),
                ids: Vec::new(),
                guids: Vec::new(),
                marks: Vec::new(),
                count: 0,
                placeholder,
            });
            group.ids.push(member_id);
            group.guids.push(guid);
            group.marks.push(mark);
            group.count = group.ids.len();
        }
        for (level_guid, groups) in floors {
            let Some(Element::Level(level)) = m.elements.get(level_guid) else {
                return Err(format!("Electrical diagram floor {level_guid} is missing"));
            };
            let mut groups: Vec<_> = groups.into_values().collect();
            groups.sort_by(|a, b| {
                b.count
                    .cmp(&a.count)
                    .then_with(|| natural_label(&a.type_name, &b.type_name))
                    .then(a.family.cmp(&b.family))
                    .then(a.type_name.cmp(&b.type_name))
                    .then(a.role.cmp(&b.role))
                    .then(a.placeholder.cmp(&b.placeholder))
            });
            bands
                .entry(level_guid)
                .or_insert_with(|| Band { level: level_guid, name: level.name.clone(), elevation: level.elevation, rows: Vec::new() })
                .rows
                .push(Row {
                    line: id,
                    line_guid: m.elements.guid(id).ok_or_else(|| format!("Electrical diagram line {id} has no GUID"))?,
                    system: line.system.clone(),
                    label: if line.mark_prefix.is_empty() { id.to_string() } else { line.mark_prefix.clone() },
                    groups,
                });
        }
    }
    let mut heads: Vec<_> = heads.into_values().collect();
    for head in &mut heads {
        if let Some(mark) = head.line_marks.values().next()
            && head.line_marks.values().all(|value| value == mark)
        {
            head.mark = mark.clone();
        }
    }
    heads.sort_by_key(|head| head.id);
    let mut bands: Vec<_> = bands.into_values().collect();
    bands.sort_by(|a, b| b.elevation.total_cmp(&a.elevation).then(a.level.cmp(&b.level)));
    for band in &mut bands {
        band.rows
            .sort_by(|a, b| natural_label(&a.system, &b.system).then_with(|| natural_label(&a.label, &b.label)).then(a.line.cmp(&b.line)));
    }
    let diagram = Diagram {
        heads,
        bands,
        diagnostics: vec![
            INVENTORY.into(),
            "Risers and source links show explicit line membership only; no logical, power or ring ties are inferred.".into(),
            "Family symbols are not certified standard UGO. Each type uses its first member's instance symbol.".into(),
        ],
    };
    pages(&diagram)?;
    Ok(diagram)
}

/// Wrap without discarding characters or reducing the paper text height.
fn wrap(text: &str, width: f32) -> Result<Vec<String>, String> {
    checked_text(text)?;
    let mut lines = Vec::new();
    let mut line = String::new();
    for c in text.chars() {
        if c == '\n' || c == '\r' {
            lines.push(std::mem::take(&mut line));
            continue;
        }
        line.push(if c.is_whitespace() { ' ' } else { c });
        if crate::font::width(&line, TEXT * 5.0) > width * 5.0 {
            line.pop();
            if line.is_empty() {
                return Err("Electrical diagram contains a character too wide to print".into());
            }
            lines.push(std::mem::take(&mut line));
            line.push(c);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

fn row_label(row: &Row) -> String {
    format!("Line {} [ID {}] / {}", row.label, row.line, row.system)
}

fn group_label(group: &Group, number: usize) -> String {
    format!(
        "G{number}: {} / {} / {}{} : {}",
        group.type_name,
        group.family,
        group.role,
        if group.placeholder { " / PLACEHOLDER" } else { "" },
        group.count,
    )
}

struct Slot<'a> {
    row: &'a Row,
    first: usize,
    labels: Vec<String>,
    texts: Vec<Vec<String>>,
    top: f32,
    height: f32,
}

struct Page<'a> {
    band: &'a Band,
    lanes: Vec<&'a Row>,
    slots: Vec<Slot<'a>>,
}

fn push_page<'a>(out: &mut Vec<Page<'a>>, page: Page<'a>) -> Result<(), String> {
    if out.len() >= MAX_PAGES {
        return Err("Electrical diagram needs more than 64 A3 pages; select fewer explicit lines".into());
    }
    out.push(page);
    Ok(())
}

fn pages(diagram: &Diagram) -> Result<Vec<Page<'_>>, String> {
    if diagram.bands.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines = BTreeMap::new();
    let mut occurrences = diagram.heads.len();
    for band in &diagram.bands {
        if !band.elevation.is_finite() || band.rows.len() > MAX_MEMBERS {
            return Err("Electrical diagram has an invalid floor band".into());
        }
        for row in &band.rows {
            lines.entry(row.line).or_insert(row);
            for group in &row.groups {
                if group.count == 0
                    || group.count != group.ids.len()
                    || group.count != group.guids.len()
                    || group.count != group.marks.len()
                {
                    return Err("Electrical diagram group IDs, GUIDs, marks and count must correspond".into());
                }
                occurrences = occurrences.checked_add(group.count).ok_or("Electrical diagram member count overflow")?;
                if occurrences > MAX_MEMBERS {
                    return Err("Electrical diagram exceeds 10000 aggregate member occurrences".into());
                }
            }
        }
    }
    if lines.is_empty() || lines.len() > MAX_MEMBERS || diagram.bands.len() > MAX_PAGES {
        return Err("Electrical diagram needs explicit lines and at most 64 floor bands".into());
    }
    let mut lines: Vec<_> = lines.into_values().collect();
    lines.sort_by(|a, b| natural_label(&a.system, &b.system).then_with(|| natural_label(&a.label, &b.label)).then(a.line.cmp(&b.line)));
    if lines.iter().any(|row| crate::font::width(&row.line.to_string(), 10.0) > 55.0) {
        return Err("Electrical diagram line ID is too wide for a readable riser label".into());
    }
    let mut out = Vec::new();
    // shortcut: six risers per horizontal section; extra sections continue on separate A3 pages.
    for lanes in lines.chunks(LANES) {
        for band in &diagram.bands {
            let rows: Vec<_> = band.rows.iter().filter(|row| lanes.iter().any(|lane| lane.line == row.line)).collect();
            if rows.is_empty() {
                continue;
            }
            let mut page = Page { band, lanes: lanes.to_vec(), slots: Vec::new() };
            let mut top = TOP;
            for row in rows {
                let labels = wrap(&row_label(row), BLOCK_W - 8.0)?;
                let mut first = 0;
                loop {
                    let mut texts = Vec::new();
                    let mut height = labels.len() as f32 * 4.0 + 24.0;
                    for (i, group) in row.groups.iter().enumerate().skip(first).take(STRIP) {
                        let text = wrap(&group_label(group, i + 1), BLOCK_W - 8.0)?;
                        let extra = text.len() as f32 * 4.0 + 2.0;
                        if height + extra > BOTTOM - TOP {
                            if texts.is_empty() {
                                return Err("Electrical diagram type or line label is too long for a readable A3 block".into());
                            }
                            break;
                        }
                        height += extra;
                        texts.push(text);
                    }
                    if height > BOTTOM - TOP {
                        return Err("Electrical diagram line label is too long for a readable A3 block".into());
                    }
                    if top + height > BOTTOM {
                        push_page(&mut out, page)?;
                        page = Page { band, lanes: lanes.to_vec(), slots: Vec::new() };
                        top = TOP;
                    }
                    let count = texts.len();
                    page.slots.push(Slot { row, first, labels: labels.clone(), texts, top, height });
                    top += height + 5.0;
                    first += count;
                    if first >= row.groups.len() {
                        break;
                    }
                }
            }
            push_page(&mut out, page)?;
        }
    }
    Ok(out)
}

fn px(x: f32, y: f32) -> (f32, f32) {
    (x * 5.0, y * 5.0)
}

fn stroke(d: &mut Drawing, points: &[(f32, f32)], dashed: bool) {
    d.stroke(points.iter().map(|&(x, y)| px(x, y)).collect(), Pen::PROJECTION, INK, dashed.then(|| vec![10.0, 7.0]), false);
}

struct Connection {
    line: Id,
    parent: Id,
    points: Vec<(f32, f32)>,
    dashed: bool,
}

fn connections(d: &mut Drawing, routes: &[Connection]) {
    for (route_index, route) in routes.iter().enumerate() {
        d.own(Some(route.line));
        for pair in route.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.1 != b.1 {
                stroke(d, &[a, b], route.dashed);
                continue;
            }
            let (left, right) = (a.0.min(b.0), a.0.max(b.0));
            let mut gaps = Vec::new();
            for (other_index, other) in routes.iter().enumerate() {
                if route_index == other_index {
                    continue;
                }
                for vertical in other.points.windows(2) {
                    let (v, w) = (vertical[0], vertical[1]);
                    let shared_stem = (route.line, route.parent) == (other.line, other.parent) && a.1 == v.1;
                    if !shared_stem && v.0 == w.0 && v.0 > left && v.0 < right && a.1 >= v.1.min(w.1) && a.1 <= v.1.max(w.1) {
                        gaps.push(((v.0 - 0.7).max(left), (v.0 + 0.7).min(right)));
                    }
                }
            }
            gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut start = left;
            for (lo, hi) in gaps {
                if lo > start {
                    stroke(d, &[(start, a.1), (lo, a.1)], route.dashed);
                }
                start = start.max(hi);
            }
            if start < right {
                stroke(d, &[(start, a.1), (right, a.1)], route.dashed);
            }
        }
    }
}

fn rectangle(d: &mut Drawing, x: f32, y: f32, w: f32, h: f32) {
    d.stroke(vec![px(x, y), px(x + w, y), px(x + w, y + h), px(x, y + h)], Pen::PROJECTION, INK, None, true);
}

fn text(d: &mut Drawing, s: &str, x: f32, y: f32, height: f32, color: Rgb) {
    let width = crate::font::width(s, height * 5.0);
    d.text(s, (x * 5.0 + width / 2.0, y * 5.0), height * 5.0, color);
}

fn generic(d: &mut Drawing, id: Id, x: f32, y: f32, reason: &str) {
    rectangle(d, x - 5.0, y - 5.0, 10.0, 10.0);
    d.text("GEN", px(x, y), 10.0, MUTED);
    d.notes.push(format!("Component {id}: generic rectangle, not standard UGO: {reason}"));
}

fn symbol(d: &mut Drawing, m: &Model, id: Id, x: f32, y: f32, work: &mut usize) -> Result<(), String> {
    let Some(Element::Component(c)) = m.elements.get(id) else {
        return Err(format!("Electrical diagram symbol component {id} is missing"));
    };
    let Some(family) = m.families.get(&c.family) else {
        generic(d, id, x, y, "family missing");
        return Ok(());
    };
    *work += family.symbols.len();
    if *work > MAX_SYMBOL_WORK {
        return Err("Electrical diagram exceeds its symbol evaluation budget".into());
    }
    if family.symbols.is_empty() || family.symbols.len() > crate::family::MAX_SYMBOLS {
        generic(d, id, x, y, "missing or oversized family symbol");
        return Ok(());
    }
    let marks = match family.values(&c.type_name, &c.params).and_then(|values| family.marks(&values)) {
        Ok(marks) => marks,
        Err(_) => {
            generic(d, id, x, y, "family symbol cannot be evaluated");
            return Ok(());
        }
    };
    let points = marks
        .iter()
        .map(|mark| match mark {
            Mark::Line(points) => points.len(),
            Mark::Dot { .. } => 24,
            Mark::Label { text, .. } => text.len(),
        })
        .sum::<usize>();
    *work += points;
    if *work > MAX_SYMBOL_WORK {
        return Err("Electrical diagram exceeds its symbol geometry budget".into());
    }
    if marks.is_empty() || points > MAX_SYMBOL_POINTS {
        generic(d, id, x, y, "empty or oversized evaluated symbol");
        return Ok(());
    }
    let (mut lo, mut hi) = (P2(f64::INFINITY, f64::INFINITY), P2(f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut extend = |p: P2| -> Result<(), String> {
        if !p.0.is_finite() || !p.1.is_finite() || p.0.abs() > MAX_COORD || p.1.abs() > MAX_COORD {
            return Err("invalid symbol coordinates".into());
        }
        lo.0 = lo.0.min(p.0);
        lo.1 = lo.1.min(p.1);
        hi.0 = hi.0.max(p.0);
        hi.1 = hi.1.max(p.1);
        Ok(())
    };
    let bounds = (|| -> Result<(), String> {
        for mark in &marks {
            match mark {
                Mark::Line(points) => {
                    if points.len() < 2 {
                        return Err("unsupported degenerate line symbol".into());
                    }
                    for &p in points {
                        extend(p)?;
                    }
                }
                Mark::Dot { centre, radius } => {
                    if !radius.is_finite() || *radius <= 0.0 || *radius > MAX_COORD {
                        return Err("invalid symbol dot".into());
                    }
                    extend(P2(centre.0 - radius, centre.1 - radius))?;
                    extend(P2(centre.0 + radius, centre.1 + radius))?;
                }
                Mark::Label { at, height, text, align } => {
                    checked_text(text)?;
                    if !crate::font::supports(text) {
                        return Err("unsupported symbol label glyph".into());
                    }
                    if !height.is_finite() || *height <= 0.0 || *height > MAX_COORD || text.contains(['\n', '\r']) {
                        return Err("unsupported symbol label".into());
                    }
                    let width = f64::from(crate::font::width(text, *height as f32));
                    let left = at.0
                        - match align {
                            Align::Left => 0.0,
                            Align::Center => width / 2.0,
                            Align::Right => width,
                        };
                    extend(P2(left, at.1 - height / 2.0))?;
                    extend(P2(left + width, at.1 + height / 2.0))?;
                }
            }
        }
        Ok(())
    })();
    if let Err(reason) = bounds {
        generic(d, id, x, y, &reason);
        return Ok(());
    }
    let span = (hi.0 - lo.0).max(hi.1 - lo.1);
    if !span.is_finite() || span <= 1e-9 {
        generic(d, id, x, y, "degenerate symbol bounds");
        return Ok(());
    }
    let scale = 10.0 / span;
    if marks.iter().any(|mark| matches!(mark, Mark::Label { height, .. } if height * scale < 2.0)) {
        generic(d, id, x, y, "symbol lettering would be unreadable in 10 mm");
        return Ok(());
    }
    let centre = P2((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
    let map = |p: P2| (x + ((p.0 - centre.0) * scale) as f32, y - ((p.1 - centre.1) * scale) as f32);
    for mark in marks {
        match mark {
            Mark::Line(points) => stroke(d, &points.into_iter().map(map).collect::<Vec<_>>(), false),
            Mark::Dot { centre, radius } => {
                let (cx, cy) = map(centre);
                let radius = (radius * scale) as f32;
                let ring = (0..24)
                    .map(|i| {
                        let (s, c) = (std::f32::consts::TAU * i as f32 / 24.0).sin_cos();
                        px(cx + radius * c, cy + radius * s)
                    })
                    .collect();
                d.push(Prim::Fill { rings: vec![ring], color: INK });
            }
            Mark::Label { at, height, text, align } => {
                let (mut tx, ty) = map(at);
                let height = (height * scale) as f32;
                let width = crate::font::width(&text, height * 5.0) / 5.0;
                tx += match align {
                    Align::Left => width / 2.0,
                    Align::Center => 0.0,
                    Align::Right => -width / 2.0,
                };
                d.text(&text, px(tx, ty), height * 5.0, INK);
            }
        }
    }
    Ok(())
}

fn lane_x(i: usize) -> f32 {
    118.0 + i as f32 * 12.0
}

fn check_effort(d: &Drawing, primitives: &mut usize, vertices: &mut usize) -> Result<(), String> {
    *primitives += d.prims.len();
    *vertices += d
        .prims
        .iter()
        .map(|prim| match prim {
            Prim::Line { pts, .. } => pts.len(),
            Prim::Fill { rings, .. } => rings.iter().map(Vec::len).sum(),
            Prim::Image { .. } => 4,
        })
        .sum::<usize>();
    if *primitives > MAX_PRIMITIVES || *vertices > MAX_VERTICES {
        return Err("Electrical diagram exceeds its primitive or vertex budget; select fewer lines".into());
    }
    Ok(())
}

struct DeviceSlot<'a> {
    segment: &'a crate::low_voltage::diagram_devices::Segment,
    range: std::ops::Range<usize>,
    labels: Vec<String>,
    marks: Vec<Vec<String>>,
    top: f32,
    height: f32,
}

struct DevicePage<'a> {
    band: &'a Band,
    lines: Vec<Id>,
    slots: Vec<DeviceSlot<'a>>,
}

pub(crate) fn draw_devices(
    m: &Model,
    diagram: &Diagram,
    segments: &[crate::low_voltage::diagram_devices::Segment],
    simplified: bool,
) -> Result<Vec<Drawing>, String> {
    let mut all_lines: Vec<_> = diagram.bands.iter().flat_map(|b| &b.rows).collect();
    all_lines.sort_by(|a, b| natural_label(&a.system, &b.system).then_with(|| natural_label(&a.label, &b.label)).then(a.line.cmp(&b.line)));
    all_lines.dedup_by_key(|row| row.line);
    let mut pages = Vec::new();
    for lanes in all_lines.chunks(LANES) {
        let lines: Vec<_> = lanes.iter().map(|row| row.line).collect();
        for band in &diagram.bands {
            if !band.rows.iter().any(|row| lines.contains(&row.line)) {
                continue;
            }
            let mut page = DevicePage { band, lines: lines.clone(), slots: Vec::new() };
            let mut top = TOP;
            for segment in segments.iter().filter(|s| s.level == band.level && lines.contains(&s.line)) {
                for start in (0..segment.items.len()).step_by(7) {
                    let end = (start + 7).min(segment.items.len());
                    let root = if start == 0 { segment.from } else { segment.items[start - 1].ids.last().copied() };
                    let root_mark = if start == 0 {
                        segment.from_mark.as_deref()
                    } else {
                        segment.items[start - 1].marks.last().and_then(|m| m.as_deref())
                    };
                    let source = diagram.heads.iter().any(|h| Some(h.id) == root && h.lines.contains(&segment.line));
                    let mut label = format!("Линия {} / ID {}", segment.label, segment.line);
                    if !source {
                        label.push_str(
                            &root.map_or_else(
                                || " / изолятор без адреса".into(),
                                |id| format!(" / от {} (ID {id})", root_mark.unwrap_or("")),
                            ),
                        );
                    }
                    if start > 0 {
                        label.push_str(" / продолжение");
                    }
                    let labels = wrap(&label, 60.0)?;
                    let marks: Vec<_> = segment.items[start..end]
                        .iter()
                        .map(|item| {
                            let label = if item.kind == "gap" {
                                format!("{} шт.", item.omitted)
                            } else {
                                item.marks
                                    .first()
                                    .and_then(|m| m.clone())
                                    .unwrap_or_else(|| format!("ИЗ {}", item.ids.first().copied().unwrap_or_default()))
                            };
                            wrap(&label, 17.0)
                        })
                        .collect::<Result<_, _>>()?;
                    let height = (labels.len() as f32 * 4.0 + 22.0)
                        .max(marks.iter().map(|v| v.len()).max().unwrap_or(0) as f32 * 4.0 + 22.0)
                        .max(32.0);
                    if height > BOTTOM - TOP {
                        return Err("Electrical diagram mark is too long for readable A3 lettering".into());
                    }
                    if top + height > BOTTOM {
                        pages.push(page);
                        page = DevicePage { band, lines: lines.clone(), slots: Vec::new() };
                        top = TOP;
                    }
                    page.slots.push(DeviceSlot { segment, range: start..end, labels, marks, top, height });
                    top += height + 6.0;
                }
            }
            pages.push(page);
            if pages.len() > MAX_PAGES {
                return Err("Electrical diagram needs more than 64 A3 pages; select fewer lines".into());
            }
        }
    }
    let mut positions = BTreeMap::new();
    for (page, plan) in pages.iter().enumerate() {
        for slot in &plan.slots {
            for (i, item) in slot.segment.items[slot.range.clone()].iter().enumerate() {
                for id in &item.ids {
                    positions.insert((slot.segment.line, *id), (page, 276.0 + i as f32 * 19.0, slot.top + slot.height - 10.0));
                }
            }
        }
    }
    let (mut work, mut primitives, mut vertices) = (0, 0, 0);
    let mut drawings = Vec::new();
    let mut original_heads = BTreeSet::new();
    for (index, page) in pages.iter().enumerate() {
        let mut d = Drawing::new(View { scale: 5.0, cx: 210.0, cy: 148.5, width: 2100, height: 1485, angle: 0.0 });
        let mut routes = Vec::new();
        rectangle(&mut d, 10.0, 10.0, 400.0, 277.0);
        text(&mut d, "Структурная схема слаботочных систем", 14.0, 18.0, 3.5, INK);
        text(
            &mut d,
            &format!("{} / лист {} из {}", if simplified { "Упрощённый вид" } else { "Полный вид" }, index + 1, pages.len()),
            14.0,
            26.0,
            TEXT,
            INK,
        );
        let floor = wrap(&format!("{} / отм. {:.0} мм", page.band.name, page.band.elevation), 194.0)?;
        if floor.len() > 2 {
            return Err("Electrical diagram floor heading is too long".into());
        }
        for (i, line) in floor.iter().enumerate() {
            text(&mut d, line, 210.0, 26.0 + i as f32 * 4.0, TEXT, INK);
        }
        stroke(&mut d, &[(12.0, 38.0), (408.0, 38.0)], false);
        for (lane, id) in page.lines.iter().enumerate() {
            d.own(Some(*id));
            let x = lane_x(lane);
            let parent = diagram.heads.iter().find(|head| head.lines.contains(id)).ok_or("Electrical diagram line has no source")?.id;
            routes.push(Connection { line: *id, parent, points: vec![(x, 52.0), (x, 244.0)], dashed: false });
            d.text(&id.to_string(), px(x, 41.0), 10.0, INK);
            let previous = pages[..index].iter().rposition(|p| p.lines.contains(id));
            let next = pages[index + 1..].iter().position(|p| p.lines.contains(id)).map(|n| index + n + 1);
            if let Some(p) = previous {
                d.text(&format!("л.{}", p + 1), px(x, 47.0), 10.0, INK);
            }
            if let Some(p) = next {
                d.text(&format!("л.{}", p + 1), px(x, 251.0), 10.0, INK);
            }
        }
        let mut head_y = 54.0;
        for head in &diagram.heads {
            let connected: Vec<_> = page.lines.iter().enumerate().filter(|(_, id)| head.lines.contains(id)).collect();
            if connected.is_empty() {
                continue;
            }
            let reference = head.level != page.band.level || !original_heads.insert(head.guid);
            let mut labels =
                wrap(&format!("{}{} / {} / {}", if reference { "Ссылка на " } else { "" }, head.mark, head.family, head.type_name), 86.0)?;
            if head.line_marks.values().any(|mark| mark != &head.mark) {
                for (_, id) in &connected {
                    let mark = head.line_marks.get(id).ok_or("Electrical diagram source line mark is missing")?;
                    labels.extend(wrap(&format!("ID {id}: {mark}"), 86.0)?);
                }
            }
            if head_y + 12.0 + labels.len() as f32 * 4.0 > BOTTOM {
                return Err("Electrical diagram source column exceeds A3".into());
            }
            d.own(Some(head.id));
            symbol(&mut d, m, head.id, 23.0, head_y, &mut work)?;
            for (i, label) in labels.iter().enumerate() {
                text(&mut d, label, 14.0, head_y + 9.0 + i as f32 * 4.0, TEXT, INK);
            }
            for (lane, id) in connected {
                d.own(Some(*id));
                routes.push(Connection {
                    line: *id,
                    parent: head.id,
                    points: vec![(30.0, head_y), (lane_x(lane), head_y)],
                    dashed: reference,
                });
            }
            head_y += 18.0 + labels.len() as f32 * 4.0;
        }
        for (slot_index, slot) in page.slots.iter().enumerate() {
            let segment = slot.segment;
            let y = slot.top + slot.height - 10.0;
            d.own(Some(segment.line));
            for (i, label) in slot.labels.iter().enumerate() {
                text(&mut d, label, 194.0, slot.top + 4.0 + i as f32 * 4.0, TEXT, INK);
            }
            let root = if slot.range.start == 0 { segment.from } else { segment.items[slot.range.start - 1].ids.last().copied() };
            let source = diagram.heads.iter().any(|head| Some(head.id) == root && head.lines.contains(&segment.line));
            if source {
                let lane = page.lines.iter().position(|id| *id == segment.line).ok_or("Electrical diagram line has no riser")?;
                routes.push(Connection {
                    line: segment.line,
                    parent: root.ok_or("Electrical diagram chain source is missing")?,
                    points: vec![(lane_x(lane), y), (271.0, y)],
                    dashed: false,
                });
            } else if let Some(id) = root {
                if let Some(&(page_index, x, parent_y)) = positions.get(&(segment.line, id)) {
                    if page_index == index {
                        let gutter = 256.0 + slot_index as f32 * 2.0;
                        routes.push(Connection {
                            line: segment.line,
                            parent: id,
                            points: vec![(x, parent_y + 5.0), (x, parent_y + 8.0), (gutter, parent_y + 8.0), (gutter, y), (271.0, y)],
                            dashed: false,
                        });
                    } else {
                        text(&mut d, &format!("от ID {id}, лист {}", page_index + 1), 194.0, y + 6.0, TEXT, MUTED);
                        stroke(&mut d, &[(266.0, y), (271.0, y)], true);
                    }
                } else {
                    text(&mut d, &format!("от ID {id}, другой этаж"), 194.0, y + 6.0, TEXT, MUTED);
                    stroke(&mut d, &[(266.0, y), (271.0, y)], true);
                }
            }
            for (i, item) in segment.items[slot.range.clone()].iter().enumerate() {
                let x = 276.0 + i as f32 * 19.0;
                if i > 0 {
                    stroke(&mut d, &[(x - 14.0, y), (x - 5.0, y)], false);
                }
                for (j, mark) in slot.marks[i].iter().enumerate() {
                    text(&mut d, mark, x - 8.0, slot.top + 4.0 + j as f32 * 4.0, TEXT, INK);
                }
                if item.kind == "gap" {
                    stroke(&mut d, &[(x - 5.0, y), (x + 5.0, y)], true);
                } else {
                    let id = *item.ids.first().ok_or("Electrical diagram device has no ID")?;
                    d.own(Some(id));
                    symbol(&mut d, m, id, x, y, &mut work)?;
                    d.own(Some(segment.line));
                }
            }
        }
        connections(&mut d, &routes);
        d.own(None);
        text(
            &mut d,
            "Линии показаны по явной логической связности. Возврат кольца и питание не выводятся из марок.",
            14.0,
            266.0,
            TEXT,
            MUTED,
        );
        text(&mut d, "Изоляторы вне адресного порядка показаны отдельно, без выдуманных логических подключений.", 14.0, 273.0, TEXT, MUTED);
        text(&mut d, "GEN — условный прямоугольник: у семейства нет пригодного символа. Это не нормативное УГО.", 14.0, 280.0, TEXT, MUTED);
        check_effort(&d, &mut primitives, &mut vertices)?;
        drawings.push(d);
    }
    Ok(drawings)
}

pub(crate) fn draw(m: &Model, diagram: &Diagram) -> Result<Vec<Drawing>, String> {
    let pages = pages(diagram)?;
    for head in &diagram.heads {
        if m.elements.guid(head.id) != Some(head.guid) {
            return Err(format!("Electrical diagram source {} changed identity; rebuild the diagram", head.id));
        }
    }
    let mut drawings = Vec::new();
    let mut original_heads = BTreeSet::new();
    let (mut work, mut primitives, mut vertices) = (0, 0, 0);
    for (index, page) in pages.iter().enumerate() {
        let mut d = Drawing::new(View { scale: 5.0, cx: 210.0, cy: 148.5, width: 2100, height: 1485, angle: 0.0 });
        d.notes.extend(diagram.diagnostics.iter().cloned());
        let mut routes = Vec::new();
        rectangle(&mut d, 10.0, 10.0, PAPER.0 as f32 - 20.0, PAPER.1 as f32 - 20.0);
        text(&mut d, "Grouped electrical structure / floor inventory", 14.0, 18.0, 3.5, INK);
        text(&mut d, &format!("A3 / page {} of {}", index + 1, pages.len()), 322.0, 18.0, TEXT, INK);
        let floor = wrap(&format!("{} / elevation {:.0} mm", page.band.name, page.band.elevation), 392.0)?;
        if floor.len() > 2 {
            return Err("Electrical diagram floor label is too long for its A3 heading".into());
        }
        for (i, line) in floor.iter().enumerate() {
            text(&mut d, line, 14.0, 29.0 + i as f32 * 4.0, TEXT, INK);
        }
        stroke(&mut d, &[(12.0, 38.0), (408.0, 38.0)], false);
        stroke(&mut d, &[(12.0, 257.0), (408.0, 257.0)], false);
        for (i, row) in page.lanes.iter().enumerate() {
            if m.elements.guid(row.line) != Some(row.line_guid) {
                return Err(format!("Electrical diagram line {} changed identity; rebuild the diagram", row.line));
            }
            let x = lane_x(i);
            d.own(Some(row.line));
            let parent =
                diagram.heads.iter().find(|head| head.lines.contains(&row.line)).ok_or("Electrical diagram line has no source")?.id;
            routes.push(Connection { line: row.line, parent, points: vec![(x, 52.0), (x, 244.0)], dashed: false });
            d.text(&row.line.to_string(), px(x, 41.0), 10.0, INK);
            d.text(&row.line.to_string(), px(x, 253.0), 10.0, INK);
            let labels = wrap(&row_label(row), 184.0)?;
            if labels.len() > 2 {
                return Err("Electrical diagram line label is too long for its readable riser caption".into());
            }
            for (j, label) in labels.iter().enumerate() {
                d.text_turned(label, px(x + 3.0 + j as f32 * 4.0, 148.0), TEXT * 5.0, std::f32::consts::FRAC_PI_2, MUTED);
            }
            let previous = pages[..index].iter().rposition(|p| p.lanes.iter().any(|r| r.line == row.line)).map(|i| i + 1);
            let next = pages[index + 1..].iter().position(|p| p.lanes.iter().any(|r| r.line == row.line)).map(|i| index + i + 2);
            if let Some(previous) = previous {
                d.text(&format!("p{previous}"), px(x, 47.0), 10.0, INK);
                stroke(&mut d, &[(x - 1.0, 54.0), (x, 52.0), (x + 1.0, 54.0)], false);
            }
            if let Some(next) = next {
                d.text(&format!("p{next}"), px(x, 249.0), 10.0, INK);
                stroke(&mut d, &[(x - 1.0, 242.0), (x, 244.0), (x + 1.0, 242.0)], false);
            }
            d.notes.push(format!(
                "Line {} ({}) / system {}: continued from page {previous:?}, to page {next:?}",
                row.line, row.label, row.system
            ));
        }
        d.own(None);
        text(&mut d, "Source / source reference", 14.0, 41.0, TEXT, MUTED);
        let mut head_y = 52.0;
        for head in &diagram.heads {
            let connected: Vec<_> = page.lanes.iter().enumerate().filter(|(_, row)| head.lines.contains(&row.line)).collect();
            if connected.is_empty() {
                continue;
            }
            let primary = head.level == page.band.level && original_heads.insert(head.guid);
            let source_floor =
                diagram.bands.iter().find(|band| band.level == head.level).ok_or("Electrical diagram source floor is missing")?;
            let heading =
                if primary { format!("Source {}", head.mark) } else { format!("REF source {} / {}", head.mark, source_floor.name) };
            let mut labels = wrap(&heading, 86.0)?;
            if head.line_marks.values().any(|mark| mark != &head.mark) {
                for (_, row) in &connected {
                    let mark = head.line_marks.get(&row.line).ok_or("Electrical diagram source line mark is missing")?;
                    labels.extend(wrap(&format!("{}: {mark}", row.label), 86.0)?);
                }
            }
            let names = wrap(&format!("{} / {}", head.type_name, head.family), 86.0)?;
            let height = 16.0 + (labels.len() + names.len()) as f32 * 4.0;
            if head_y + height > BOTTOM {
                return Err("Electrical diagram source labels exceed the A3 source column; select fewer lines or shorten names".into());
            }
            d.own(Some(head.id));
            symbol(&mut d, m, head.id, 23.0, head_y, &mut work)?;
            for (i, label) in labels.iter().enumerate() {
                text(&mut d, label, 14.0, head_y + 9.0 + i as f32 * 4.0, TEXT, INK);
            }
            for (i, name) in names.iter().enumerate() {
                text(&mut d, name, 14.0, head_y + 9.0 + (labels.len() + i) as f32 * 4.0, TEXT, INK);
            }
            for (lane, row) in connected {
                d.own(Some(row.line));
                routes.push(Connection {
                    line: row.line,
                    parent: head.id,
                    points: vec![(30.0, head_y), (lane_x(lane), head_y)],
                    dashed: !primary,
                });
                rectangle(&mut d, lane_x(lane) - 0.6, head_y - 0.6, 1.2, 1.2);
            }
            head_y += height;
        }
        for slot in &page.slots {
            let lane = page.lanes.iter().position(|row| row.line == slot.row.line).ok_or("Electrical diagram row has no explicit riser")?;
            d.own(Some(slot.row.line));
            for (i, label) in slot.labels.iter().enumerate() {
                text(&mut d, label, BLOCK_X + 4.0, slot.top + i as f32 * 4.0, TEXT, INK);
            }
            let strip_y = slot.top + slot.labels.len() as f32 * 4.0 + 6.0;
            let block_y = strip_y + 10.0;
            rectangle(&mut d, BLOCK_X, strip_y - 7.0, BLOCK_W, slot.top + slot.height - strip_y + 7.0);
            stroke(&mut d, &[(BLOCK_X, block_y), (BLOCK_X + BLOCK_W, block_y)], false);
            if slot.texts.is_empty() {
                text(&mut d, "No grouped devices on source floor", BLOCK_X + 4.0, block_y + 4.0, TEXT, MUTED);
                continue;
            }
            let parent =
                diagram.heads.iter().find(|head| head.lines.contains(&slot.row.line)).ok_or("Electrical diagram line has no source")?.id;
            routes.push(Connection {
                line: slot.row.line,
                parent,
                points: vec![(lane_x(lane), strip_y), (BLOCK_X, strip_y)],
                dashed: false,
            });
            rectangle(&mut d, lane_x(lane) - 0.6, strip_y - 0.6, 1.2, 1.2);
            let mut y = block_y + 4.0;
            for (i, lines) in slot.texts.iter().enumerate() {
                let number = slot.first + i;
                let group = slot.row.groups.get(number).ok_or("Electrical diagram group is missing")?;
                let id = *group.ids.first().ok_or("Electrical diagram group has no representative")?;
                let guid = *group.guids.first().ok_or("Electrical diagram group has no representative GUID")?;
                if m.elements.guid(id) != Some(guid) {
                    return Err(format!("Electrical diagram component {id} changed identity; rebuild the diagram"));
                }
                d.own(Some(id));
                let x = BLOCK_X + BLOCK_W * (i as f32 + 0.5) / slot.texts.len() as f32;
                symbol(&mut d, m, id, x, strip_y, &mut work)?;
                d.text(&format!("G{}", number + 1), px(x, strip_y + 7.0), 10.0, INK);
                d.own(Some(slot.row.line));
                for line in lines {
                    text(&mut d, line, BLOCK_X + 4.0, y, TEXT, INK);
                    y += 4.0;
                }
                y += 2.0;
            }
        }
        connections(&mut d, &routes);
        d.own(None);
        text(&mut d, INVENTORY, 14.0, 264.0, TEXT, MUTED);
        text(
            &mut d,
            "Riser ID repeats on continuation pages. REF is a source on another page/floor, not another device.",
            14.0,
            270.0,
            TEXT,
            MUTED,
        );
        text(
            &mut d,
            "GEN = generic rectangle, missing/unsupported family symbol; see diagnostics. Symbols are not certified UGO.",
            14.0,
            276.0,
            TEXT,
            MUTED,
        );
        text(
            &mut d,
            "Only explicit memberships are shown. No inferred logical, power, branch or ring connections.",
            14.0,
            282.0,
            TEXT,
            MUTED,
        );
        check_effort(&d, &mut primitives, &mut vertices)?;
        drawings.push(d);
    }
    Ok(drawings)
}
