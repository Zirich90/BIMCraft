use crate::commands::*;
use crate::engine::{Command, Files, Session};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

fn odt_template(_s: &mut Session, p: &Value) -> R {
    let c = "odt_template";
    if !p.is_object() {
        return Err(bad(c, "parameters must be an object"));
    }
    only(c, p, &["template", "path", "fields"])?;
    let template = string(c, p, "template")?.ok_or_else(|| bad(c, "template is required"))?;
    let path = string(c, p, "path")?.ok_or_else(|| bad(c, "path is required"))?;
    let values = p.get("fields").and_then(Value::as_object).ok_or_else(|| bad(c, "fields must be an object of text values"))?;
    let mut fields = BTreeMap::new();
    for (name, value) in values {
        let value = value.as_str().ok_or_else(|| bad(c, format!("field `{name}` must be text")))?;
        fields.insert(name.clone(), value.to_owned());
    }
    let template = crate::low_voltage::odt::template_path(Path::new(&template)).map_err(|e| bad(c, e))?;
    let path = crate::low_voltage::odt::template_path(Path::new(&path)).map_err(|e| bad(c, e))?;
    if crate::low_voltage::odt::same_file(&template, &path) {
        return Err(bad(c, "path must differ from the template"));
    }
    let input = crate::low_voltage::odt::read_template(&template).map_err(|e| bad(c, e))?;
    let output = crate::low_voltage::odt::fill(&input, &fields).map_err(|e| bad(c, e))?;
    crate::autosave::atomic_write_bytes(&path, &output).map_err(|e| bad(c, e))?;
    Ok(json!({"template":template,"path":path,"fields":fields.len()}))
}

fn odt_set(_s: &mut Session, p: &Value) -> R {
    let c = "odt_set";
    if !p.is_object() {
        return Err(bad(c, "parameters must be an object"));
    }
    only(c, p, &["templates", "path", "fields"])?;
    let list = p.get("templates").and_then(Value::as_array).ok_or_else(|| bad(c, "templates must be an array of ODT/ODM paths"))?;
    if list.is_empty() || list.len() > crate::low_voltage::odt::MAX_SET_FILES {
        return Err(bad(c, format!("templates must contain 1 to {} paths", crate::low_voltage::odt::MAX_SET_FILES)));
    }
    let dir = string(c, p, "path")?.ok_or_else(|| bad(c, "path is required"))?;
    let dir = std::path::absolute(Path::new(&dir)).map_err(|e| bad(c, e.to_string()))?;
    let values = p.get("fields").and_then(Value::as_object).ok_or_else(|| bad(c, "fields must be an object of text values"))?;
    let mut fields = BTreeMap::new();
    for (name, value) in values {
        let value = value.as_str().ok_or_else(|| bad(c, format!("field `{name}` must be text")))?;
        fields.insert(name.clone(), value.to_owned());
    }
    let mut sources = Vec::new();
    for item in list {
        let raw = item.as_str().ok_or_else(|| bad(c, "each template must be a path string"))?;
        let path = crate::low_voltage::odt::document_path(Path::new(raw)).map_err(|e| bad(c, e))?;
        if path.starts_with(&dir) {
            return Err(bad(c, "a template must not live inside the output directory"));
        }
        let bytes = crate::low_voltage::odt::read_template(&path).map_err(|e| bad(c, e))?;
        sources.push((path, bytes));
    }
    let output = crate::low_voltage::odt::fill_set(&sources, &fields).map_err(|e| bad(c, e))?;
    std::fs::create_dir_all(&dir).map_err(|e| bad(c, e.to_string()))?;
    let mut written = Vec::new();
    for (name, bytes) in &output {
        let target = dir.join(name);
        crate::autosave::atomic_write_bytes(&target, bytes).map_err(|e| bad(c, e))?;
        written.push(target);
    }
    Ok(json!({"path":dir,"files":written,"fields":fields.len()}))
}

pub(crate) const ODT_TEMPLATE: Command = other_files(
    "odt_template",
    "Fill ODT Template",
    "{template:ODT, path:ODT, fields:{name:string,…}} copy one ODT and replace only existing text:variable-set values. Every named variable must occur exactly once; computed text:formula fields are left untouched. Other XML, styles, images, links and manual text are preserved; the template and project are unchanged. Writes atomically; at most 256 fields, 256 ZIP entries, 32 MiB content.xml and 256 MiB input/output",
    odt_template,
    Files::Write,
);
pub(crate) const ODT_SET: Command = other_files(
    "odt_set",
    "Copy ODT Document Set",
    "{templates:[ODT/ODM,…], path:DIR, fields:{name:string,…}} copy a linked ODT/ODM set into one folder. Internal section-source links are remapped to the copied files and their target sections must exist; links leaving the set are refused. Fields fill each document where the variable occurs once (missing names are skipped, formula fields untouched). Originals and project are unchanged; each file writes atomically. At most 32 documents, 256 fields, 256 ZIP entries, 32 MiB content.xml and 256 MiB per file",
    odt_set,
    Files::Write,
);
use crate::geom::P2;
use crate::model::{Element, MAX_TEXT};
use crate::pen::Pen;
use crate::render::{Drawing, Rgb};

pub(crate) const ELECTRICAL_DESIGN_PDF: Command = other_files(
    "electrical_design_pdf",
    "Export Low-Voltage Design PDF",
    "{path:PDF, code:string, title:string, sheets?:[sheet id|GUID,…], system?:string} export an A4 cover and contents, the selected project sheets in order, A3 floor/riser diagrams by low-voltage system, and the equipment and materials specification. Put manually edited text and general notes on the selected sheets. Writes atomically, at most 64 pages, without changing the project",
    electrical_design_pdf,
    Files::Write,
);

fn electrical_design_pdf(s: &mut Session, p: &Value) -> R {
    const C: &str = "electrical_design_pdf";
    const COVER: P2 = P2(210.0, 297.0);
    only(C, p, &["path", "code", "title", "sheets", "system"])?;
    let path = string(C, p, "path")?
        .filter(|value| value.to_ascii_lowercase().ends_with(".pdf"))
        .ok_or("electrical_design_pdf: path must end in .pdf")?;
    let label = |key| -> Result<String, String> {
        let value = string(C, p, key)?.filter(|value| !value.trim().is_empty()).ok_or_else(|| bad(C, format!("{key} is required")))?;
        if value.len() > MAX_TEXT
            || value.chars().any(char::is_control)
            || !crate::font::supports(&value)
            || crate::font::width(&value, 17.5) > 870.0
        {
            return Err(bad(C, format!("{key} must fit a printable line")));
        }
        Ok(value)
    };
    let code = label("code")?;
    let title = label("title")?;
    let system = string(C, p, "system")?;
    let m = &s.model;
    let sheets = p.get("sheets").map(|value| guids_param(C, m, value, "sheets")).transpose()?.unwrap_or_default();
    let sections: std::collections::BTreeSet<_> = m
        .elements
        .values()
        .filter_map(|element| match element {
            Element::ElectricalLine(line) if system.as_ref().is_none_or(|system| *system == line.system) => Some(line.section()),
            _ => None,
        })
        .collect();
    if sections.is_empty() {
        return Err(bad(C, "no electrical lines match the selection"));
    }
    if sheets.len() + sections.len() > 61 {
        return Err(bad(C, "design PDF exceeds its 64-page limit"));
    }
    let mut contents = Vec::new();
    let mut pages = Vec::new();
    for guid in sheets {
        let Some(Element::Sheet(sheet)) = m.elements.get(guid) else {
            return Err(bad(C, format!("{guid} is not a project sheet")));
        };
        let paper = crate::model::paper(&sheet.size).ok_or_else(|| bad(C, "invalid sheet size"))?;
        let drawing = crate::sheet::draw(m, guid).ok_or_else(|| bad(C, "cannot draw project sheet"))?;
        contents.push((3 + pages.len(), format!("{} {}", sheet.number, sheet.name)));
        pages.push((drawing, paper));
    }
    for section in sections {
        let (drawings, _) =
            super::diagrams::section_pages(m, None, system.as_deref(), Some(section), "full").map_err(|error| bad(C, error))?;
        if pages.len() + drawings.len() > 61 {
            return Err(bad(C, "design PDF exceeds its 64-page limit"));
        }
        let name = m.language.word(bimcraft_i18n::msg!("Electrical Line Diagram"));
        contents.push((3 + pages.len(), format!("{name} / {section}")));
        pages.extend(drawings.into_iter().map(|drawing| (drawing, super::diagram_layout::PAPER)));
    }
    let spec = super::ods::specification(m, system.as_deref(), true)?;
    if !spec.table.rows.is_empty() {
        let drawings = super::ods::specification_pages(m, &spec.table).map_err(|error| bad(C, error))?;
        if pages.len() + drawings.len() > 62 {
            return Err(bad(C, "design PDF exceeds its 64-page limit"));
        }
        contents
            .push((3 + pages.len(), m.language.word(bimcraft_i18n::msg!("Electrical Equipment and Material Specification")).to_owned()));
        pages.extend(drawings.into_iter().map(|drawing| (drawing, super::ods::SPECIFICATION_PAPER)));
    }
    let ink = Rgb(25, 25, 25);
    let framed_page = || {
        let mut page = Drawing::new(crate::sheet::page(COVER));
        page.line(&[P2(20.0, 5.0), P2(205.0, 5.0), P2(205.0, 292.0), P2(20.0, 292.0)], Pen::BORDER, ink, true);
        page
    };
    let mut cover = framed_page();
    cover.text(&title, (560.0, 550.0), 17.5, ink);
    cover.text(&code, (560.0, 600.0), 17.5, ink);
    let mut index = framed_page();
    index.text(m.language.word(bimcraft_i18n::msg!("Contents")), (560.0, 90.0), 17.5, ink);
    for (row, (page, name)) in contents.iter().enumerate() {
        if !crate::font::supports(name) || crate::font::width(name, 12.5) > 800.0 {
            return Err(bad(C, format!("contents entry on page {page} does not fit")));
        }
        let y = 125.0 + 21.0 * row as f32;
        index.text(&page.to_string(), (130.0, y), 12.5, ink);
        index.text(name, (160.0 + crate::font::width(name, 12.5) / 2.0, y), 12.5, ink);
    }
    let mut refs = vec![(&cover, COVER), (&index, COVER)];
    refs.extend(pages.iter().map(|(drawing, paper)| (drawing, *paper)));
    let bytes = crate::pdf::pdf(&refs);
    if bytes.len() as u64 > MAX_FILE {
        return Err(bad(C, "PDF exceeds the file size limit"));
    }
    crate::autosave::atomic_write_bytes(Path::new(&path), &bytes).map_err(|error| bad(C, error))?;
    Ok(json!({"pages":refs.len(),"contents":contents}))
}
