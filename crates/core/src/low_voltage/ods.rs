use crate::commands::*;
use crate::engine::{Command, Files, Session};
use crate::font;
use crate::geom::P2;
use crate::library::Kind as LibKind;
use crate::model::{Element, Guid, Id, MAX_POINTS, Model};
use crate::pen::Pen;
use crate::render::{Drawing, Prim, Rgb};
use crate::sheet::{DENSITY, page};
use crate::{pdf, schedule};
use bimcraft_i18n::msg;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
const CELL_TEXT: f32 = 12.5;
const TITLE_TEXT: f32 = 17.5;
const PAD: f32 = 10.0;
const INK: Rgb = Rgb(25, 25, 25);

fn electrical_line_table(s: &mut Session, p: &Value) -> R {
    use crate::schedule::{Column, Kind, Table};
    let c = "electrical_line_table";
    if !p.is_object() {
        return Err(bad(c, "parameters must be an object"));
    }
    only(c, p, &["id", "system", "level", "path"])?;
    let m = &s.model;
    let selected = id_opt(c, p, "id")?.map(|guid| element_id(c, m, &json!(guid))).transpose()?;
    if selected.is_some_and(|id| !matches!(m.elements.get(id), Some(Element::ElectricalLine(_)))) {
        return Err(bad(c, "select an electrical line"));
    }
    let system = string(c, p, "system")?;
    let level = p.get("level").filter(|v| !v.is_null()).map(|_| level_param(c, m, p, "level")).transpose()?;
    let path = string(c, p, "path")?;
    if let Some(path) = &path
        && (path.trim().is_empty() || !path.to_lowercase().ends_with(".csv") && !path.to_lowercase().ends_with(".xlsx"))
    {
        return Err(bad(c, "path must end in .csv or .xlsx"));
    }
    let columns = [
        ("Line", Kind::Number),
        ("Line GUID", Kind::Text),
        ("System", Kind::Text),
        ("Level", Kind::Text),
        ("Role", Kind::Text),
        ("Order", Kind::Number),
        ("Id", Kind::Number),
        ("GUID", Kind::Text),
        ("Mark", Kind::Text),
        ("Family", Kind::Text),
        ("Type", Kind::Text),
        ("Terminals", Kind::Text),
        ("Placeholder", Kind::YesNo),
    ];
    let mut table = Table {
        columns: columns.into_iter().map(|(heading, kind)| Column { heading: heading.into(), kind }).collect(),
        rows: Vec::new(),
        ids: Vec::new(),
        totals: None,
    };
    let mut groups = BTreeMap::<(String, Option<Guid>, Id), Vec<usize>>::new();
    let mut devices = BTreeSet::new();
    let (mut line_count, mut terminal_count, mut bytes) = (0, 0, 0);
    for (&id, element) in m.elements.iter() {
        let Element::ElectricalLine(line) = element else { continue };
        if selected.is_some_and(|chosen| chosen != id) || system.as_ref().is_some_and(|system| *system != line.system) {
            continue;
        }
        line_count += 1;
        if line_count > MAX_POINTS {
            return Err(bad(c, format!("at most {MAX_POINTS} lines per table")));
        }
        let mut terminals = BTreeMap::<Guid, Vec<Value>>::new();
        for guid in &line.terminals {
            let Some(Element::ElectricalTerminal(t)) = m.elements.get(*guid) else { continue };
            terminals.entry(t.device).or_default().push(json!({
                "id":m.elements.id(*guid),"guid":guid,"name":t.name,"purpose":t.purpose,
                "address":t.address,"load_ma":t.load_ma,"limit_ma":t.limit_ma,
            }));
        }
        let ordered = line.order(m);
        let marks = line.marks(m);
        let addressed = ordered.iter().copied().enumerate().map(|(i, guid)| (guid, Some(i)));
        let isolators = line.isolators.iter().copied().map(|guid| (guid, None));
        for (guid, order) in addressed.chain(isolators) {
            let Some(Element::Component(device)) = m.elements.get(guid) else { continue };
            if level.is_some_and(|level| device.level != level) {
                continue;
            }
            let attached = terminals.remove(&guid).unwrap_or_default();
            terminal_count += attached.len();
            if table.rows.len() >= MAX_POINTS || terminal_count > MAX_POINTS {
                return Err(bad(c, format!("at most {MAX_POINTS} rows and copied terminals per table")));
            }
            let device_id = m.elements.id(guid).unwrap_or_default();
            let row = vec![
                json!(id),
                json!(m.guid(id)),
                json!(line.system),
                json!(m.level(device.level).map(|l| &l.name)),
                json!(if guid == line.source {
                    "source"
                } else if order.is_none() {
                    "isolator"
                } else {
                    "device"
                }),
                json!(order.map(|i| i + 1)),
                json!(device_id),
                json!(guid),
                json!(order.and_then(|i| marks.get(i))),
                json!(device.family),
                json!(device.type_name),
                json!(attached),
                json!(m.library.placeholder(&LibKind::FamilyType, &format!("{}: {}", device.family, device.type_name))),
            ];
            bytes += serde_json::to_vec(&row).map_err(|e| bad(c, e))?.len();
            if bytes as u64 > MAX_FILE / 16 {
                return Err(bad(c, "table cells exceed 16 MiB; filter by line, floor or system"));
            }
            groups.entry((line.system.clone(), Some(device.level), id)).or_default().push(table.rows.len());
            table.rows.push(row);
            table.ids.push(vec![device_id]);
            if guid != line.source {
                devices.insert(guid);
            }
        }
    }
    if let Some(path) = &path {
        export_report_table(c, &table, path, "Line Devices")?;
    }
    let groups: Vec<_> = groups
        .into_iter()
        .map(|((system, level, id), rows)| {
            json!({"line":id,"line_guid":m.guid(id),"system":system,"level":level.and_then(|g| m.elements.id(g)),
            "level_guid":level,"name":level.and_then(|g| m.level(g)).map(|l| &l.name),"rows":rows})
        })
        .collect();
    Ok(json!({"columns":table.columns.iter().map(|col| &col.heading).collect::<Vec<_>>(),
        "rows":table.rows,"elements":table.ids,"groups":groups,"device_count":devices.len()}))
}

fn electrical_specification(s: &mut Session, p: &Value) -> R {
    let c = "electrical_specification";
    if !p.is_object() {
        return Err(bad(c, "parameters must be an object"));
    }
    only(c, p, &["system", "path"])?;
    let system = string(c, p, "system")?;
    let path = string(c, p, "path")?;
    if path.as_ref().is_some_and(|path| {
        let path = path.to_lowercase();
        path.trim().is_empty() || !path.ends_with(".csv") && !path.ends_with(".xlsx") && !path.ends_with(".pdf") && !path.ends_with(".ods")
    }) {
        return Err(bad(c, "path must end in .csv, .xlsx, .ods or .pdf"));
    }
    let m = &s.model;
    let by_section = path.as_ref().is_some_and(|path| path.to_ascii_lowercase().ends_with(".ods"));
    let spec = specification(m, system.as_deref(), by_section)?;
    let Specification { table, items, excluded, diagnostics, sections } = spec;
    let mut pages = 0;
    if let Some(path) = path {
        if path.to_lowercase().ends_with(".ods") {
            write_specification_ods(&path, &table, &sections)?;
        } else if path.to_lowercase().ends_with(".pdf") {
            let drawings = specification_pages(m, &table).map_err(|e| bad(c, e))?;
            pages = drawings.len();
            let refs: Vec<_> = drawings.iter().map(|page| (page, SPECIFICATION_PAPER)).collect();
            let bytes = pdf::pdf(&refs);
            if bytes.len() as u64 > MAX_FILE {
                return Err(bad(c, "PDF exceeds the file size limit"));
            }
            crate::autosave::atomic_write_bytes(Path::new(&path), &bytes).map_err(|e| bad(c, e))?;
        } else {
            export_report_table(c, &table, &path, "Specification")?;
        }
    }
    Ok(json!({"columns":table.columns.iter().map(|column| &column.heading).collect::<Vec<_>>(),
        "rows":table.rows,"elements":table.ids,"items":items,"excluded":excluded,"diagnostics":diagnostics,"pages":pages,"sections":sections,
        "ready":!table.rows.is_empty() && diagnostics.is_empty()}))
}

pub(super) struct Specification {
    pub(super) table: schedule::Table,
    items: Vec<Value>,
    excluded: Vec<Id>,
    diagnostics: Vec<String>,
    sections: BTreeMap<String, Vec<usize>>,
}

pub(super) fn specification(m: &Model, system: Option<&str>, by_section: bool) -> Result<Specification, String> {
    use crate::schedule::{Column, Kind, Table};
    let c = "electrical_specification";
    let mut memberships = BTreeMap::<Guid, BTreeSet<String>>::new();
    let mut included = BTreeMap::<Guid, BTreeSet<String>>::new();
    let mut occurrences = 0;
    for element in m.elements.values() {
        if let Element::ElectricalLine(line) = element {
            if system.is_some_and(|system| system != line.system) {
                continue;
            }
            for guid in line.members().chain(line.isolators.iter().copied()).chain(line.segments.iter().copied()) {
                occurrences += 1;
                if occurrences > MAX_POINTS {
                    return Err(bad(c, "at most 10000 line member occurrences per specification; filter by system"));
                }
                included.entry(guid).or_default().insert(line.system.clone());
                memberships.entry(guid).or_default().insert(line.section().into());
            }
        }
    }
    for (_, guid, element) in m.elements.with_guids() {
        if let Element::Component(component) = element
            && let Some(provision) = &component.provision
            && system.is_none_or(|system| system == provision.system)
        {
            occurrences += 1;
            if occurrences > MAX_POINTS {
                return Err(bad(c, "at most 10000 system memberships per specification; filter by system"));
            }
            included.entry(guid).or_default().insert(provision.system.clone());
            memberships.entry(guid).or_default().insert(super::systems::section_for_name(&provision.system).into());
        }
        if let Element::Wire(wire) = element
            && let Some(feeder) = &wire.feeder
            && let Some(systems) = included.get(&feeder.wire)
        {
            occurrences += systems.len();
            if occurrences > MAX_POINTS {
                return Err(bad(c, "at most 10000 copied system memberships per specification; filter by system"));
            }
            let systems = systems.clone();
            if let Some(sections) = memberships.get(&feeder.wire).cloned() {
                memberships.entry(guid).or_default().extend(sections);
            }
            included.entry(guid).or_default().extend(systems);
        }
        if included.len() > MAX_POINTS {
            return Err(bad(c, "at most 10000 included elements per specification; filter by system"));
        }
    }
    let mut carriers = Vec::new();
    for (guid, systems) in &included {
        if let Some(Element::Wire(wire)) = m.elements.get(*guid)
            && let Some(carrier) = wire.carrier
        {
            occurrences += systems.len();
            if occurrences > MAX_POINTS {
                return Err(bad(c, "at most 10000 copied system memberships per specification; filter by system"));
            }
            carriers.push((carrier, systems.clone(), memberships.get(guid).cloned().unwrap_or_default()));
        }
    }
    for (guid, systems, sections) in carriers {
        memberships.entry(guid).or_default().extend(sections);
        included.entry(guid).or_default().extend(systems);
        if included.len() > MAX_POINTS {
            return Err(bad(c, "at most 10000 included elements per specification; filter by system"));
        }
    }
    struct Item {
        elements: Vec<Id>,
        guids: Vec<Guid>,
        systems: BTreeSet<String>,
        quantity: f64,
        length_mm: Option<f64>,
        unit: &'static str,
        name: String,
    }
    let mut groups = BTreeMap::<(Guid, String, String), Item>::new();
    let mut excluded = Vec::new();
    let mut diagnostics = Vec::new();
    for (guid, systems) in included {
        let element = m.elements.get(guid).ok_or_else(|| bad(c, "included element is missing"))?;
        let id = m.elements.id(guid).ok_or_else(|| bad(c, "included element identity is missing"))?;
        if crate::ods::placeholder_element(m, element) {
            excluded.push(id);
            diagnostics.push(format!("Element {id}: placeholder type excluded from specification"));
            continue;
        }
        let (kind, name, designation, quantity, length_mm, unit) = match element {
            Element::Component(component) => (
                LibKind::FamilyType,
                format!("{}: {}", component.family, component.type_name),
                String::new(),
                f64::from(component.provision.as_ref().map_or(1, |p| p.quantity)),
                None,
                "pcs",
            ),
            _ => {
                let run = element.run().ok_or_else(|| bad(c, "included material is not a physical run"))?;
                let geometry = m.run_geom(element).ok_or_else(|| bad(c, "included material has no physical geometry"))?;
                let reserve = if let Element::Wire(wire) = element {
                    if let Some(feeder) = &wire.feeder {
                        let Some(Element::Wire(parent)) = m.elements.get(feeder.wire) else {
                            return Err(bad(c, "wire feeder parent is missing"));
                        };
                        parent.reserve
                    } else {
                        wire.reserve
                    }
                } else {
                    0.0
                };
                let length = geometry.length() * (1.0 + reserve / 100.0);
                (LibKind::Type(element.category().into()), run.type_name.clone(), run.cable.clone(), length / 1000.0, Some(length), "m")
            }
        };
        let type_guid = m.library.guid(&kind, &name).ok_or_else(|| bad(c, "specification type has no library identity"))?;
        let section = match memberships.get(&guid).filter(|_| by_section) {
            Some(sections) if sections.len() > 1 => "shared",
            Some(sections) => sections.first().map_or("", String::as_str),
            None => "",
        };
        let item = groups.entry((type_guid, designation, section.into())).or_insert_with(|| Item {
            elements: Vec::new(),
            guids: Vec::new(),
            systems: BTreeSet::new(),
            quantity: 0.0,
            length_mm: length_mm.map(|_| 0.0),
            unit,
            name,
        });
        item.elements.push(id);
        item.guids.push(guid);
        item.systems.extend(systems);
        item.quantity += quantity;
        if let (Some(total), Some(length)) = (&mut item.length_mm, length_mm) {
            *total += length;
        }
        if !item.quantity.is_finite() || item.length_mm.is_some_and(|length| !length.is_finite()) {
            return Err(bad(c, "specification quantity exceeds its numeric range"));
        }
    }
    let mut table = Table {
        columns: [
            (msg!("Position"), Kind::Number),
            (msg!("Description"), Kind::Text),
            (msg!("Brand"), Kind::Text),
            (msg!("Article"), Kind::Text),
            (msg!("Manufacturer"), Kind::Text),
            (msg!("Unit"), Kind::Text),
            (msg!("Quantity"), Kind::Number),
            (msg!("Mass kg"), Kind::Number),
            (msg!("Note"), Kind::Text),
        ]
        .into_iter()
        .map(|(heading, kind)| Column { heading: heading.into(), kind })
        .collect(),
        rows: Vec::new(),
        ids: Vec::new(),
        totals: None,
    };
    let mut items = Vec::new();
    let mut sections = BTreeMap::<String, Vec<usize>>::new();
    let mut bytes = 0;
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|((a_guid, a_designation, a_section), a), ((b_guid, b_designation, b_section), b)| {
        crate::low_voltage::diagram_layout::natural_label(&a.name, &b.name)
            .then(a_designation.cmp(b_designation))
            .then(a_guid.cmp(b_guid))
            .then(a_section.cmp(b_section))
    });
    for ((type_guid, designation, section), item) in sorted {
        let (_, entry) = m.library.by_guid(type_guid).ok_or_else(|| bad(c, "specification type is missing"))?;
        let product = entry.product.clone().unwrap_or_default();
        let missing: Vec<_> = [
            ("description", product.description.as_str()),
            ("brand", product.brand.as_str()),
            ("article", product.article.as_str()),
            ("manufacturer", product.manufacturer.as_str()),
        ]
        .into_iter()
        .filter_map(|(field, value)| value.trim().is_empty().then_some(field))
        .collect();
        let position = table.rows.len() + 1;
        if !missing.is_empty() {
            diagnostics.push(format!("Position {position} ({}): missing product fields {}", item.name, missing.join(", ")));
        }
        let note = if missing.is_empty() { String::new() } else { format!("Missing: {}", missing.join(", ")) };
        let row = vec![
            json!(position),
            json!(if product.description.trim().is_empty() { &item.name } else { &product.description }),
            json!(if designation.is_empty() { &product.brand } else { &designation }),
            json!(product.article),
            json!(product.manufacturer),
            json!(item.unit),
            json!(item.quantity),
            json!(product.mass_kg),
            json!(note),
        ];
        bytes += serde_json::to_vec(&row).map_err(|e| bad(c, e))?.len();
        if bytes as u64 > MAX_FILE / 16 {
            return Err(bad(c, "specification cells exceed 16 MiB; filter by system"));
        }
        items.push(json!({"position":position,"type_guid":type_guid,"name":item.name,"systems":item.systems,
            "elements":item.elements,"guids":item.guids,"quantity":item.quantity,"unit":item.unit,"length_mm":item.length_mm,
            "missing_product_fields":missing}));
        let section = if section.is_empty() { "unclassified".into() } else { section };
        sections.entry(section).or_default().push(table.rows.len());
        table.ids.push(item.elements);
        table.rows.push(row);
    }
    Ok(Specification { table, items, excluded, diagnostics, sections })
}

fn write_specification_ods(path: &str, table: &schedule::Table, sections: &BTreeMap<String, Vec<usize>>) -> Result<(), String> {
    let headings: Vec<_> = table.columns.iter().map(|column| column.heading.clone()).collect();
    let rows = |indices: &[usize]| {
        std::iter::once(headings.clone())
            .chain(indices.iter().map(|&index| {
                table.rows[index]
                    .iter()
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    })
                    .collect()
            }))
            .collect::<Vec<_>>()
    };
    let sheets: Vec<_> = std::iter::once(("Specification".to_owned(), rows(&(0..table.rows.len()).collect::<Vec<_>>())))
        .chain(sections.iter().enumerate().map(|(i, (name, indices))| {
            let label: String = name.chars().filter(|c| !c.is_control() && !"[]:*?/\\'".contains(*c)).take(24).collect();
            (format!("{} {}", i + 1, label), rows(indices))
        }))
        .collect();
    let bytes = crate::ods::write_sheets(sheets.iter().map(|(name, rows)| (name.as_str(), rows.as_slice())))?;
    crate::autosave::atomic_write_bytes(Path::new(path), &bytes)
}

fn export_report_table(c: &str, table: &crate::schedule::Table, path: &str, sheet: &str) -> Result<(), String> {
    let xlsx = path.to_lowercase().ends_with(".xlsx");
    if xlsx {
        for value in table.rows.iter().flatten() {
            let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
            if text.encode_utf16().count() > 32767 {
                return Err(bad(c, "an XLSX cell exceeds 32767 characters; export CSV or filter the table"));
            }
            if text.chars().any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') || matches!(c, '\u{FFFE}' | '\u{FFFF}')) {
                return Err(bad(c, "an XLSX cell contains characters XML cannot preserve; export CSV"));
            }
        }
    }
    let bytes = if xlsx { schedule::to_xlsx(table, sheet) } else { schedule::to_csv(table).into_bytes() };
    if bytes.len() as u64 > MAX_FILE {
        return Err(bad(c, "export exceeds the file size limit"));
    }
    crate::autosave::atomic_write_bytes(std::path::Path::new(path), &bytes).map_err(|e| bad(c, e))
}

fn catalogue_link(s: &mut Session, p: &Value) -> R {
    only("catalogue_link", p, &["path"])?;
    if s.read_only.is_some() {
        return Err("Cannot change a catalogue binding in a read-only session".into());
    }
    if p.get("path") == Some(&Value::Null) {
        let model = s.model_mut();
        model.catalogue_retired.clear();
        model.catalogue_unexported.clear();
        model.catalogue_pending_edits.clear();
        s.set_catalogue_path(None);
        s.catalogue_owned.clear();
        s.catalogue_baseline = Default::default();
        return Ok(json!({"path":null}));
    }
    require_catalogue_write(s)?;
    let path = string("catalogue_link", p, "path")?.ok_or("catalogue_link: path is required")?;
    let path = crate::ods::workbook_path(Path::new(&path))?;
    if s.path.as_ref().and_then(|p| std::path::absolute(p).ok()).as_ref() == Some(&path) {
        return Err("Catalogue path must differ from the project file".into());
    }
    let mut model = (*s.model).clone();
    let current = s.catalogue_path();
    let already_linked = current == Some(path.as_path());
    let exists = crate::ods::workbook_exists(&path)?;
    // A project can retain pending edits from a legacy or unavailable approval.
    let resume_pending = current.is_none()
        && exists
        && (!model.catalogue_retired.is_empty() || !model.catalogue_unexported.is_empty() || !model.catalogue_pending_edits.is_empty());
    if !already_linked && !resume_pending {
        model.catalogue_retired.clear();
        model.catalogue_unexported.clear();
        model.catalogue_pending_edits.clear();
    }
    model.check()?;
    let unowned = BTreeSet::new();
    let reply = if exists {
        let owned = if already_linked { &s.catalogue_owned } else { &unowned };
        crate::ods::sync_bound(&mut model, &path, already_linked || resume_pending, !resume_pending, owned)?
    } else {
        use std::io::Write;
        let mut staged =
            tempfile::NamedTempFile::new_in(path.parent().ok_or("Catalogue path needs a parent directory")?).map_err(|e| e.to_string())?;
        staged.write_all(&crate::ods::write(&crate::ods::catalogue_rows(&model))?).map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        staged.persist_noclobber(&path).map_err(|e| e.to_string())?;
        json!({"path":path,"updated":0})
    };
    if model != *s.model {
        *s.model_mut() = model;
    }
    s.set_catalogue_path(Some(path));
    s.remember_catalogue();
    Ok(reply)
}

fn catalogue_sync(s: &mut Session, p: &Value) -> R {
    only("catalogue_sync", p, &["read_only"])?;
    let write = !flag("catalogue_sync", p, "read_only")?;
    if write {
        require_catalogue_write(s)?;
    }
    let path = s.catalogue_path().ok_or("No ODS catalogue is linked")?;
    let mut model = (*s.model).clone();
    let reply = crate::ods::sync_bound(&mut model, path, true, write, &s.catalogue_owned)?;
    if model != *s.model {
        *s.model_mut() = model;
    }
    if write {
        s.remember_catalogue();
    }
    Ok(reply)
}

fn require_catalogue_write(s: &Session) -> Result<(), String> {
    if s.catalogue_writable() { Ok(()) } else { Err("Cannot write an ODS catalogue in a read-only session".into()) }
}

fn catalogue_pending(s: &mut Session, p: &Value) -> R {
    only("catalogue_pending", p, &[])?;
    let types: Vec<_> = crate::ods::pending_products(&s.model)
        .into_iter()
        .map(|(id, entry)| json!({"id":id,"guid":entry.guid,"name":entry.name,"kind":entry.kind}))
        .collect();
    Ok(json!({"types":types}))
}

fn catalogue_export(s: &mut Session, p: &Value) -> R {
    only("catalogue_export", p, &["path"])?;
    require_catalogue_write(s)?;
    let path = string("catalogue_export", p, "path")?.ok_or("catalogue_export: path is required")?;
    let path = crate::ods::workbook_path(Path::new(&path))?;
    crate::ods::workbook_exists(&path)?;
    if s.path.as_ref().and_then(|p| std::path::absolute(p).ok()).as_ref() == Some(&path) {
        return Err("Catalogue path must differ from the project file".into());
    }
    let bytes = crate::ods::write(&crate::ods::catalogue_rows(&s.model))?;
    crate::autosave::atomic_write_bytes(&path, &bytes)?;
    Ok(json!({"path":path}))
}

pub(crate) const SPECIFICATION_PAPER: P2 = P2(420.0, 297.0);

pub(crate) fn specification_cell(units: crate::units::Units, kind: schedule::Kind, value: &serde_json::Value) -> String {
    let shown = schedule::shown(units, kind, value);
    if shown == "0" && value.as_f64().is_some_and(|number| number != 0.0) { value.to_string() } else { shown }
}

pub(crate) fn specification_pages(m: &Model, t: &schedule::Table) -> Result<Vec<Drawing>, String> {
    const WIDTHS: [f32; 9] = [12.0, 146.0, 44.0, 36.0, 44.0, 14.0, 22.0, 22.0, 50.0];
    const LEFT: f32 = 100.0;
    const TOP: f32 = 100.0;
    const BOTTOM: f32 = 1385.0;
    if t.rows.is_empty() {
        return Err("no items match the PDF selection".into());
    }
    if t.columns.len() != WIDTHS.len() || t.rows.len() > crate::model::MAX_POINTS {
        return Err("specification needs nine columns and at most 10000 rows".into());
    }
    fn wrap(text: &str, width: f32) -> Result<Vec<String>, String> {
        if text.chars().count() > 1024 {
            return Err("specification PDF cell exceeds 1024 characters; export CSV or XLSX".into());
        }
        if text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n')) {
            return Err("specification PDF cell contains control characters".into());
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', " ");
        if !font::supports(&text.replace('\n', " ")) {
            return Err("specification PDF cell contains characters the lettering font cannot print; export CSV or XLSX".into());
        }
        let mut out = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for c in paragraph.chars() {
                line.push(c);
                if font::width(&line, CELL_TEXT) > width {
                    line.pop();
                    if line.is_empty() {
                        return Err("specification PDF character does not fit its column".into());
                    }
                    if let Some(at) = line.rfind(' ').filter(|&at| at > 0) {
                        let rest = line.split_off(at + 1);
                        line.pop();
                        out.push(std::mem::replace(&mut line, rest));
                    } else {
                        out.push(std::mem::take(&mut line));
                    }
                    line.push(c);
                }
            }
            out.push(line);
        }
        Ok(out)
    }
    fn height(cells: &[Vec<String>]) -> f32 {
        cells.iter().map(Vec::len).max().unwrap_or(1) as f32 * 20.0 + 20.0
    }
    fn row(d: &mut Drawing, cells: &[Vec<String>], y: f32, height: f32) {
        let mut x = LEFT;
        for (cell, width) in cells.iter().zip(WIDTHS) {
            let width = width * DENSITY as f32;
            d.stroke(vec![(x, y), (x + width, y), (x + width, y + height), (x, y + height)], Pen::FINE, INK, None, true);
            for (i, text) in cell.iter().enumerate() {
                d.text(text, (x + PAD + font::width(text, CELL_TEXT) / 2.0, y + 20.0 + 20.0 * i as f32), CELL_TEXT, INK);
            }
            x += width;
        }
    }
    fn effort(d: &Drawing, from: usize, primitives: &mut usize, vertices: &mut usize) -> Result<(), String> {
        *primitives += d.prims.len() - from;
        *vertices += d
            .prims
            .iter()
            .skip(from)
            .map(|p| match p {
                Prim::Line { pts, .. } => pts.len(),
                Prim::Fill { rings, .. } => rings.iter().map(Vec::len).sum(),
                Prim::Image { .. } => 4,
            })
            .sum::<usize>();
        if *primitives > 200_000 || *vertices > 2_000_000 {
            return Err("specification PDF exceeds its geometry budget; filter by system".into());
        }
        Ok(())
    }
    let headings: Vec<_> = t
        .columns
        .iter()
        .zip(WIDTHS)
        .map(|(c, width)| wrap(m.language.word(&c.heading), width * DENSITY as f32 - 2.0 * PAD))
        .collect::<Result<_, _>>()?;
    let header_height = height(&headings);
    let start = || {
        let mut d = Drawing::new(page(SPECIFICATION_PAPER));
        d.weights = m.line_weights().at(m.units.default_scale());
        d.text(m.language.word(msg!("Electrical Equipment and Material Specification")), (1050.0, 50.0), TITLE_TEXT, INK);
        row(&mut d, &headings, TOP, header_height);
        d
    };
    let (mut primitives, mut vertices) = (0, 0);
    let mut pages = Vec::new();
    let mut d = start();
    effort(&d, 0, &mut primitives, &mut vertices)?;
    let mut y = TOP + header_height;
    for cells in &t.rows {
        if cells.len() != WIDTHS.len() {
            return Err("specification PDF row must have nine cells".into());
        }
        let cells: Vec<_> = cells
            .iter()
            .zip(&t.columns)
            .zip(WIDTHS)
            .map(|((value, column), width)| wrap(&specification_cell(m.units, column.kind, value), width * DENSITY as f32 - 2.0 * PAD))
            .collect::<Result<_, _>>()?;
        let h = height(&cells);
        if TOP + header_height + h > BOTTOM {
            return Err("specification PDF row does not fit a readable A3 page; shorten the cell or export CSV or XLSX".into());
        }
        if y + h > BOTTOM {
            if pages.len() >= 63 {
                return Err("specification PDF needs more than 64 A3 pages; filter by system".into());
            }
            d.weigh(|_| true);
            pages.push(d);
            d = start();
            effort(&d, 0, &mut primitives, &mut vertices)?;
            y = TOP + header_height;
        }
        let from = d.prims.len();
        row(&mut d, &cells, y, h);
        effort(&d, from, &mut primitives, &mut vertices)?;
        y += h;
    }
    pages.push(d);
    let count = pages.len();
    for (i, d) in pages.iter_mut().enumerate() {
        let from = d.prims.len();
        d.text(&format!("{}/{}", i + 1, count), (2000.0, 1425.0), CELL_TEXT, INK);
        effort(d, from, &mut primitives, &mut vertices)?;
        d.weigh(|_| true);
    }
    Ok(pages)
}

pub(crate) const CATALOGUE_PENDING: Command = other(
    "catalogue_pending",
    "Unassigned Catalogue Products",
    "{} list ODS-added types without model instances; built-in unused types and placeholders are excluded",
    catalogue_pending,
);
pub(crate) const CATALOGUE_LINK: Command = edit_files(
    "catalogue_link",
    "Link Catalogue",
    &[],
    "{path:string|null} bind an ODS catalogue (import existing or create new); null unlinks without deleting the file",
    catalogue_link,
    Files::Conditional,
);
pub(crate) const CATALOGUE_SYNC: Command = edit_files(
    "catalogue_sync",
    "Synchronize Catalogue",
    &[],
    "{read_only?:bool} accept new ODS rows and protect existing model rows; read_only skips workbook writes",
    catalogue_sync,
    Files::Conditional,
);
pub(crate) const CATALOGUE_EXPORT: Command = other_files(
    "catalogue_export",
    "Export Catalogue",
    "{path} export cable, carrier and family type GUIDs, dimensions and product data to the first ODS sheet",
    catalogue_export,
    Files::Write,
);
pub(crate) const ELECTRICAL_LINE_TABLE: Command = other_files(
    "electrical_line_table",
    "Electrical Line Device Table",
    "{id?:line id|GUID, system?:string, level?:name|id|GUID, path?:string} read device tables for matching lines, with source and standalone isolators, GUIDs, global traversal marks and explicit terminal addresses. Groups reference row indices by system, floor and line; device_count excludes sources and counts each device once. Optional CSV/XLSX export; at most 10000 rows and copied terminals, 16 MiB of JSON cells. Does not change the project",
    electrical_line_table,
    Files::Conditional,
);
pub(crate) const ELECTRICAL_SPECIFICATION: Command = other_files(
    "electrical_specification",
    "Electrical Equipment and Material Specification",
    "{system?:string, path?:CSV|XLSX|ODS|PDF} read a bill of unique source/device/isolator GUIDs and physical wires/carriers of explicit lines, plus equipment provision quantities. Groups by catalogue type identity and cable designation, not by line; shared members count once. Wire lengths include physical feeder legs and parent reserve, never an inferred circuit route. Quantities use the stated unit (pcs or m); items retain length_mm and element identities. Placeholder types are excluded and reported; missing product metadata is explicit. Optional atomic CSV/XLSX or paginated A3 PDF, at most 10000 included elements, 64 PDF pages and 16 MiB of cells. Does not change the project or certify procurement readiness",
    electrical_specification,
    Files::Conditional,
);

fn catalogue_import(s: &mut Session, p: &Value) -> R {
    only("catalogue_import", p, &["path", "conflicts"])?;
    let path = string("catalogue_import", p, "path")?.ok_or("catalogue_import: path is required")?;
    let bytes = crate::ods::read_workbook(Path::new(&path))?;
    let rows = crate::ods::read(&bytes)?;
    let rows = crate::ods::transfer_rows(&s.model, &rows, string("catalogue_import", p, "conflicts")?.as_deref())?;
    let count = crate::ods::apply_catalogue(s.model_mut(), &rows)?;
    Ok(json!({"updated":count}))
}

pub(crate) const CATALOGUE_IMPORT: Command = edit_files(
    "catalogue_import",
    "Import Catalogue",
    &[],
    "{path, conflicts?: error|update|copy} import first ODS sheet; copied products receive local GUIDs; resolve conflicting products by update or separate copy",
    catalogue_import,
    Files::Read,
);
