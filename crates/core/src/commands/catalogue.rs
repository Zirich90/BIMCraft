use super::{Model, Path, PathBuf, R, Session, Value, bad, flag, json, only, string};

pub(super) fn catalogue_export(s: &mut Session, p: &Value) -> R {
    const C: &str = "catalogue_export";
    only(C, p, &["path"])?;
    if let Some(how) = s.read_only {
        return Err(bad(C, format!("cannot write a catalogue from a read-only session: {how}")));
    }
    let path = string(C, p, "path")?.ok_or_else(|| bad(C, "path is required"))?;
    let path = crate::ods::workbook_path(Path::new(&path)).map_err(|e| bad(C, e))?;
    crate::ods::workbook_exists(&path).map_err(|e| bad(C, e))?;
    if s.path.as_ref().is_some_and(|project| crate::engine::same_file(project, &path)) {
        return Err(bad(C, "path must differ from the project file"));
    }
    let bytes = crate::ods::write(&crate::ods::catalogue_rows(&s.model)?).map_err(|e| bad(C, e))?;
    crate::autosave::atomic_write_bytes(&path, &bytes).map_err(|e| bad(C, e))?;
    Ok(json!({"path":path}))
}

pub(super) fn catalogue_import(s: &mut Session, p: &Value) -> R {
    const C: &str = "catalogue_import";
    only(C, p, &["path"])?;
    if let Some(how) = s.read_only {
        return Err(bad(C, format!("cannot import a catalogue into a read-only project: {how}")));
    }
    let path = string(C, p, "path")?.ok_or_else(|| bad(C, "path is required"))?;
    let path = crate::ods::workbook_path(Path::new(&path)).map_err(|e| bad(C, e))?;
    let bytes = crate::ods::read_workbook(&path).map_err(|e| bad(C, e))?;
    let rows = crate::ods::read(&bytes).map_err(|e| bad(C, e))?;
    let count = crate::ods::apply_catalogue(s.model_mut(), &rows).map_err(|e| bad(C, e))?;
    Ok(json!({"rows":count}))
}

pub(super) fn read_catalogue(model: &mut Model, project: &Path, workbook: &Path) -> Result<usize, String> {
    let workbook = crate::ods::workbook_path(workbook)?;
    if catalogue_target(&workbook)? != workbook {
        return Err("Catalogue directory has changed; approve the path again".into());
    }
    if crate::engine::same_file(project, &workbook) {
        return Err("Catalogue path must differ from the project file".into());
    }
    let bytes = crate::ods::read_workbook(&workbook)?;
    let rows = crate::ods::read(&bytes)?;
    if rows.iter().skip(1).any(|row| row.first().is_some_and(String::is_empty) && row.iter().any(|cell| !cell.is_empty())) {
        return Err("Linked catalogue rows need a GUID. Use catalogue_import to create new types, then export their GUIDs".into());
    }
    crate::ods::apply_catalogue(model, &rows)
}

fn catalogue_target(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("Catalogue path must name a file")?;
    let parent = path.parent().ok_or("Catalogue path needs a directory")?;
    Ok(std::fs::canonicalize(parent).map_err(|e| e.to_string())?.join(name))
}

pub(super) fn catalogue_link(s: &mut Session, p: &Value) -> R {
    const C: &str = "catalogue_link";
    only(C, p, &["path"])?;
    if s.read_only.is_some() {
        return Err(bad(C, "cannot change a catalogue binding in a read-only session"));
    }
    let project = s.path.as_deref().ok_or_else(|| bad(C, "save the project before linking a catalogue"))?;
    let key = std::fs::canonicalize(project).map_err(|e| bad(C, e.to_string()))?;
    if p.get("path") == Some(&Value::Null) {
        s.catalogue_bindings.remove(&key);
        return Ok(json!({"path":null}));
    }
    let path = string(C, p, "path")?.ok_or_else(|| bad(C, "path is required"))?;
    let path = crate::ods::workbook_path(Path::new(&path)).map_err(|e| bad(C, e))?;
    let path = catalogue_target(&path).map_err(|e| bad(C, e))?;
    let mut model = (*s.model).clone();
    read_catalogue(&mut model, project, &path).map_err(|e| bad(C, e))?;
    model.sync_library();
    model.check().map_err(|e| bad(C, e))?;
    s.catalogue_bindings.insert(key, path.clone());
    Ok(json!({"path":path}))
}

pub(super) fn catalogue_sync(s: &mut Session, p: &Value) -> R {
    const C: &str = "catalogue_sync";
    only(C, p, &["read_only"])?;
    if s.read_only.is_some() {
        return Err(bad(C, "cannot apply a catalogue in a read-only session"));
    }
    if p.get("read_only").is_some() && !flag(C, p, "read_only")? {
        return Err(bad(C, "use catalogue_export to write a workbook explicitly"));
    }
    let project = s.path.clone().ok_or_else(|| bad(C, "no catalogue is linked to this project"))?;
    let key = std::fs::canonicalize(&project).map_err(|e| bad(C, e.to_string()))?;
    let path = s.catalogue_bindings.get(&key).cloned().ok_or_else(|| bad(C, "no catalogue is linked to this project"))?;
    let count = read_catalogue(s.model_mut(), &project, &path).map_err(|e| bad(C, e))?;
    Ok(json!({"path":path,"rows":count}))
}
