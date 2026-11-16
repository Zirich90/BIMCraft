//! Bounded, structure-preserving copies of ODT templates.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use quick_xml::{
    NsReader,
    events::{BytesStart, Event},
    name::ResolveResult,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_ARCHIVE: u64 = 256 << 20;
const MAX_XML: u64 = 32 << 20;
const MAX_ENTRIES: usize = 256;
const MAX_FIELDS: usize = 256;
const MAX_FIELD_NAME: usize = 256;
const MAX_FIELD_VALUE: usize = 16_384;
const MAX_FIELD_BYTES: usize = 16 << 20;
pub(crate) const MAX_SET_FILES: usize = 32;
const TEXT_NS: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const XLINK_NS: &[u8] = b"http://www.w3.org/1999/xlink";

pub(crate) fn template_path(path: &Path) -> Result<PathBuf, String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    if !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("odt")) || path.as_os_str().len() > 4096 {
        return Err("ODT path must name an .odt file and be at most 4096 bytes".into());
    }
    Ok(path)
}

pub(crate) fn document_path(path: &Path) -> Result<PathBuf, String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let ok =
        path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("odt") || ext.eq_ignore_ascii_case("odm"));
    if !ok || path.as_os_str().len() > 4096 {
        return Err("ODT set path must name an .odt or .odm file and be at most 4096 bytes".into());
    }
    Ok(path)
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub(crate) fn read_template(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("ODT template must be a regular file, not a link or special file".into());
    }
    if meta.len() > MAX_ARCHIVE {
        return Err("ODT template exceeds the 256 MiB size limit".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(MAX_ARCHIVE + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("ODT template exceeds the 256 MiB size limit".into());
    }
    Ok(bytes)
}

pub(crate) fn fill(bytes: &[u8], fields: &BTreeMap<String, String>) -> Result<Vec<u8>, String> {
    let content = content_xml(bytes)?;
    let content = apply(&content, fields, true)?;
    write_content(bytes, &content)
}

/// Copy a linked ODT/ODM set into one folder, remapping internal links and filling fields where present.
pub(crate) fn fill_set(sources: &[(PathBuf, Vec<u8>)], fields: &BTreeMap<String, String>) -> Result<Vec<(String, Vec<u8>)>, String> {
    validate_fields(fields, false)?;
    if sources.is_empty() || sources.len() > MAX_SET_FILES {
        return Err(format!("ODT set must contain 1 to {MAX_SET_FILES} documents"));
    }
    let mut names = Vec::new();
    for (path, _) in sources {
        let name = path.file_name().and_then(|n| n.to_str()).ok_or("ODT set document has no usable file name")?.to_owned();
        if names.iter().any(|n: &String| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("ODT set has duplicate document name `{name}`"));
        }
        names.push(name);
    }
    let mut contents = Vec::new();
    for (_, bytes) in sources {
        contents.push(content_xml(bytes)?);
    }
    let sections: Vec<BTreeSet<String>> = contents.iter().map(|c| section_names(c)).collect::<Result<_, _>>()?;
    let canon: Vec<String> =
        sources.iter().map(|(p, _)| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()).to_string_lossy().to_lowercase()).collect();
    for (index, (path, _)) in sources.iter().enumerate() {
        let (content, _) = remap_links(&contents[index], path, &canon, &names, &sections)?;
        contents[index] = content;
    }
    let mut output = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let content = apply(&contents[index], fields, false)?;
        output.push((name.clone(), write_content(&sources[index].1, &content)?));
    }
    Ok(output)
}

fn content_xml(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("ODT template exceeds the 256 MiB size limit".into());
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("ODT archive has too many entries".into());
    }
    let mut content_entries = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        if entry.name().map_err(|e| e.to_string())? == "content.xml" {
            content_entries += 1;
        }
    }
    if content_entries != 1 {
        return Err("ODT archive must contain one content.xml entry".into());
    }
    let mut content = String::new();
    archive.by_name("content.xml").map_err(|e| e.to_string())?.take(MAX_XML + 1).read_to_string(&mut content).map_err(|e| e.to_string())?;
    if content.len() as u64 > MAX_XML {
        return Err("ODT content.xml exceeds the 32 MiB size limit".into());
    }
    Ok(content)
}

fn write_content(bytes: &[u8], content: &str) -> Result<Vec<u8>, String> {
    if content.len() as u64 > MAX_XML {
        return Err("ODT content.xml exceeds the 32 MiB size limit".into());
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        if entry.name().map_err(|e| e.to_string())? == "content.xml" {
            let options = SimpleFileOptions::default().compression_method(entry.compression());
            drop(entry);
            output.start_file("content.xml", options).map_err(|e| e.to_string())?;
            output.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        } else {
            output.raw_copy_file(entry).map_err(|e| e.to_string())?;
        }
    }
    let output = output.finish().map_err(|e| e.to_string())?.into_inner();
    if output.len() as u64 > MAX_ARCHIVE {
        return Err("ODT output exceeds the 256 MiB size limit".into());
    }
    Ok(output)
}

fn apply(content: &str, fields: &BTreeMap<String, String>, required: bool) -> Result<String, String> {
    validate_fields(fields, required)?;
    let mut content = content.to_owned();
    let replacements = replacements(&content, fields, required)?;
    for (range, replacement) in replacements.into_iter().rev() {
        content.replace_range(range, &replacement);
    }
    if content.len() as u64 > MAX_XML {
        return Err("ODT content.xml exceeds the 32 MiB size limit".into());
    }
    Ok(content)
}

fn validate_fields(fields: &BTreeMap<String, String>, required: bool) -> Result<(), String> {
    if (required && fields.is_empty()) || fields.len() > MAX_FIELDS {
        return Err(format!("ODT fields must contain {} to {MAX_FIELDS} entries", if required { 1 } else { 0 }));
    }
    let mut bytes = 0usize;
    for (name, value) in fields {
        if name.is_empty() || name.len() > MAX_FIELD_NAME {
            return Err(format!("ODT field names must contain 1 to {MAX_FIELD_NAME} bytes"));
        }
        if value.len() > MAX_FIELD_VALUE {
            return Err(format!("ODT field `{name}` exceeds {MAX_FIELD_VALUE} bytes"));
        }
        if name
            .chars()
            .chain(value.chars())
            .any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') || matches!(c, '\u{FFFE}' | '\u{FFFF}'))
        {
            return Err(format!("ODT field `{name}` contains characters XML cannot preserve"));
        }
        bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
        if bytes > MAX_FIELD_BYTES {
            return Err("ODT fields exceed the 16 MiB size limit".into());
        }
    }
    Ok(())
}

fn escape_value(value: &str) -> String {
    // XML parsers normalise a literal CR to LF; only a character reference keeps it.
    quick_xml::escape::escape(value).replace('\r', "&#13;")
}

fn is_text_ns(resolved: &ResolveResult) -> bool {
    matches!(resolved, ResolveResult::Bound(ns) if ns.0 == TEXT_NS)
}

fn replacements(content: &str, fields: &BTreeMap<String, String>, required: bool) -> Result<Vec<(Range<usize>, String)>, String> {
    let mut reader = NsReader::from_str(content);
    let mut buf = Vec::new();
    let mut occurrences: HashMap<String, usize> = HashMap::new();
    let mut replacements = Vec::new();
    let mut active: Option<(String, usize, usize)> = None;
    loop {
        let before = reader.buffer_position() as usize;
        let (ns, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| e.to_string())?;
        let text_ns = is_text_ns(&ns);
        let after = reader.buffer_position() as usize;
        let variable = text_ns && matches!(&event, Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"variable-set");
        match event {
            Event::Start(ref element) if variable => {
                if active.is_some() {
                    return Err("ODT variable-set elements cannot be nested".into());
                }
                if has_formula(element, &reader)? {
                    continue;
                }
                let name = variable_name(element, &reader)?;
                *occurrences.entry(name.clone()).or_default() += 1;
                if fields.contains_key(&name) {
                    active = Some((name, after, 1));
                }
            }
            Event::Empty(ref element) if variable => {
                if has_formula(element, &reader)? {
                    continue;
                }
                let name = variable_name(element, &reader)?;
                *occurrences.entry(name.clone()).or_default() += 1;
                if let Some(value) = fields.get(&name) {
                    let raw = &content[before..after];
                    let close = raw.rfind("/>").ok_or("ODT empty variable-set is malformed")?;
                    let qualified = std::str::from_utf8(element.name().as_ref()).map_err(|e| e.to_string())?.to_owned();
                    replacements.push((before..after, format!("{}>{}</{qualified}>", &raw[..close], escape_value(value))));
                }
            }
            Event::Start(_) if let Some((_, _, depth)) = active.as_mut() => *depth += 1,
            Event::End(_) if let Some((name, start, depth)) = active.as_mut() => {
                *depth = depth.saturating_sub(1);
                if *depth == 0 {
                    let name = name.clone();
                    let start = *start;
                    let value = fields.get(&name).ok_or("ODT field replacement is inconsistent")?;
                    replacements.push((start..before, escape_value(value)));
                    active = None;
                }
            }
            Event::DocType(_) => return Err("ODT document types are not supported".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if active.is_some() {
        return Err("ODT variable-set is incomplete".into());
    }
    for name in fields.keys() {
        match occurrences.get(name).copied().unwrap_or(0) {
            0 if required => return Err(format!("ODT variable `{name}` does not exist")),
            0 | 1 => {}
            _ => return Err(format!("ODT variable `{name}` occurs more than once")),
        }
    }
    Ok(replacements)
}

fn attr_value(element: &BytesStart<'_>, reader: &NsReader<&[u8]>, ns: &[u8], local: &[u8]) -> Result<Option<String>, String> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| e.to_string())?;
        let (resolved, name) = reader.resolver().resolve_attribute(attribute.key);
        let bound = matches!(&resolved, ResolveResult::Bound(b) if b.0 == ns);
        if bound && name.as_ref() == local {
            return attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                .map(|value| Some(value.into_owned()))
                .map_err(|e| e.to_string());
        }
    }
    Ok(None)
}

fn variable_name(element: &BytesStart<'_>, reader: &NsReader<&[u8]>) -> Result<String, String> {
    attr_value(element, reader, TEXT_NS, b"name")?.ok_or_else(|| "ODT variable-set has no text:name".into())
}

fn has_formula(element: &BytesStart<'_>, reader: &NsReader<&[u8]>) -> Result<bool, String> {
    Ok(attr_value(element, reader, TEXT_NS, b"formula")?.is_some())
}

fn section_names(content: &str) -> Result<BTreeSet<String>, String> {
    let mut reader = NsReader::from_str(content);
    let mut buf = Vec::new();
    let mut names = BTreeSet::new();
    loop {
        let (ns, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| e.to_string())?;
        let text_ns = is_text_ns(&ns);
        match event {
            Event::Start(ref e) | Event::Empty(ref e) if text_ns && e.local_name().as_ref() == b"section" => {
                if let Some(name) = attr_value(e, &reader, TEXT_NS, b"name")? {
                    names.insert(name);
                }
            }
            Event::DocType(_) => return Err("ODT document types are not supported".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(names)
}

fn remap_links(
    content: &str,
    source: &Path,
    canon: &[String],
    names: &[String],
    sections: &[BTreeSet<String>],
) -> Result<(String, usize), String> {
    let mut reader = NsReader::from_str(content);
    let mut buf = Vec::new();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let parent = source.parent().unwrap_or(Path::new("."));
    loop {
        let before = reader.buffer_position() as usize;
        let (ns, event) = reader.read_resolved_event_into(&mut buf).map_err(|e| e.to_string())?;
        let text_ns = is_text_ns(&ns);
        let after = reader.buffer_position() as usize;
        let is_source = text_ns && matches!(&event, Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"section-source");
        if !is_source {
            if matches!(event, Event::Eof) {
                break;
            }
            continue;
        }
        let element = match &event {
            Event::Start(e) | Event::Empty(e) => e,
            _ => continue,
        };
        let href = attr_value(element, &reader, XLINK_NS, b"href")?.ok_or("ODT section-source has no xlink:href")?;
        let target = percent_decode(&href);
        // Original template links may point outside the folder and not exist; match a set member by
        // resolved path when possible, otherwise by file name.
        let index = std::fs::canonicalize(parent.join(&target))
            .ok()
            .and_then(|p| canon.iter().position(|c| c == &p.to_string_lossy().to_lowercase()))
            .or_else(|| {
                let base = Path::new(&target).file_name()?.to_str()?.to_lowercase();
                names.iter().position(|n| n.to_lowercase() == base)
            })
            .ok_or_else(|| format!("ODT link `{href}` leaves the document set"))?;
        if let Some(section) = attr_value(element, &reader, TEXT_NS, b"section-name")?
            && !sections[index].contains(&section)
        {
            return Err(format!("ODT link `{href}` names missing section `{section}`"));
        }
        let new_href = percent_encode(&names[index]);
        let empty = matches!(event, Event::Empty(_));
        let tag = rebuild_tag(element, &reader, &new_href, empty)?;
        edits.push((before..after, tag));
    }
    let mut content = content.to_owned();
    let count = edits.len();
    for (range, tag) in edits.into_iter().rev() {
        content.replace_range(range, &tag);
    }
    Ok((content, count))
}

fn rebuild_tag(element: &BytesStart<'_>, reader: &NsReader<&[u8]>, new_href: &str, empty: bool) -> Result<String, String> {
    let mut tag = String::from("<");
    tag.push_str(std::str::from_utf8(element.name().as_ref()).map_err(|e| e.to_string())?);
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| e.to_string())?;
        let (resolved, name) = reader.resolver().resolve_attribute(attribute.key);
        let is_href = matches!(&resolved, ResolveResult::Bound(b) if b.0 == XLINK_NS) && name.as_ref() == b"href";
        let value = if is_href {
            escape_value(new_href)
        } else {
            let decoded =
                attribute.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder()).map_err(|e| e.to_string())?;
            escape_value(&decoded)
        };
        tag.push(' ');
        tag.push_str(std::str::from_utf8(attribute.key.as_ref()).map_err(|e| e.to_string())?);
        tag.push_str("=\"");
        tag.push_str(&value);
        tag.push('"');
    }
    tag.push_str(if empty { "/>" } else { ">" });
    Ok(tag)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
