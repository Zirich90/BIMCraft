use crate::commands::*;
use crate::engine::{Command, Files, Session};
use crate::model::{Element, Model};
use crate::pdf;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn electrical_line_diagram(s: &mut Session, p: &Value) -> R {
    let c = "electrical_line_diagram";
    if !p.is_object() {
        return Err(bad(c, "parameters must be an object"));
    }
    only(c, p, &["id", "system", "mode", "path"])?;
    let m = &s.model;
    let selected = id_opt(c, p, "id")?.map(|guid| element_id(c, m, &json!(guid))).transpose()?;
    let system = string(c, p, "system")?;
    let mode = string(c, p, "mode")?.unwrap_or_else(|| "full".into());
    if !matches!(mode.as_str(), "full" | "simplified" | "grouped") {
        return Err(bad(c, "mode must be full, simplified or grouped"));
    }
    let path = string(c, p, "path")?;
    if path.as_ref().is_some_and(|p| !p.to_lowercase().ends_with(".pdf")) {
        return Err(bad(c, "path must end in .pdf"));
    }
    let sections = if selected.is_some() {
        vec![None]
    } else {
        m.elements
            .values()
            .filter_map(|element| match element {
                Element::ElectricalLine(line) if system.as_ref().is_none_or(|name| *name == line.system) => Some(Some(line.section())),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let diagram = super::diagram_layout::build(m, selected, system.as_deref(), None).map_err(|e| bad(c, e))?;
    let mut pages = Vec::new();
    let mut segments = Vec::new();
    for section in sections {
        let (drawings, members) = section_pages(m, selected, system.as_deref(), section, &mode).map_err(|e| bad(c, e))?;
        if pages.len() + drawings.len() > 64 {
            return Err(bad(c, "diagram needs more than 64 pages; filter by system"));
        }
        pages.extend(drawings);
        segments.extend(members);
    }
    if let Some(path) = path {
        if pages.is_empty() {
            return Err(bad(c, "no lines match the PDF selection"));
        }
        let refs: Vec<_> = pages.iter().map(|page| (page, crate::low_voltage::diagram_layout::PAPER)).collect();
        let bytes = pdf::pdf(&refs);
        if bytes.len() as u64 > MAX_FILE {
            return Err(bad(c, "PDF exceeds the file size limit"));
        }
        crate::autosave::atomic_write_bytes(std::path::Path::new(&path), &bytes).map_err(|e| bad(c, e))?;
    }
    let notes: BTreeSet<_> = pages.iter().flat_map(|page| page.notes.iter().cloned()).collect();
    Ok(json!({"mode":mode,"diagram":diagram,"segments":segments,"pages":pages.len(),
        "paper_mm":[420,297],"diagnostics":notes}))
}

pub(crate) const ELECTRICAL_LINE_DIAGRAM: Command = other_files(
    "electrical_line_diagram",
    "Electrical Line Structural Diagram",
    "{id?:line id|GUID, system?:string, mode?:full|simplified|grouped, path?:PDF} structural diagram by floor and explicit line, source heads at left and separate risers. Full is default; simplified keeps first/last of identical consecutive devices and counts omitted devices, without crossing floors, forks or locked marks. Grouped lists types and quantities per floor/line. Own family symbols, not certified UGO; no inferred ring or power ties. Returns inventory, segment identities, page count and diagnostics; optional atomic A3 PDF. At most 10000 member occurrences and 64 pages; project unchanged",
    electrical_line_diagram,
    Files::Conditional,
);

pub(super) fn section_pages(
    m: &Model,
    selected: Option<crate::ids::Id>,
    system: Option<&str>,
    section: Option<&str>,
    mode: &str,
) -> Result<(Vec<crate::render::Drawing>, Vec<super::diagram_devices::Segment>), String> {
    let diagram = super::diagram_layout::build(m, selected, system, section)?;
    if mode == "grouped" {
        return Ok((super::diagram_layout::draw(m, &diagram)?, Vec::new()));
    }
    let simplified = mode == "simplified";
    let segments = super::diagram_devices::segments(m, selected, system, section, simplified)?;
    let pages = super::diagram_layout::draw_devices(m, &diagram, &segments, simplified)?;
    Ok((pages, segments))
}
