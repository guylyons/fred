//! Attachments (org-attach.el, org-attach-git.el).
//!
//! Every entry may own a directory: its DIR property (or the obsolete
//! ATTACH_DIR), else a path derived from its ID under org-attach-id-dir by
//! org-attach-id-to-path-function-list. Files are attached by cp/mv/ln/lns
//! or downloaded with `curl`; the entry gets org-attach-auto-tag. Hooks:
//! org-attach-after-change-hook and org-attach-open-hook name
//! org-attach-git-commit and org-attach-git-annex-get-maybe (git CLI).

use super::props::{self, Inherit};
use super::{Prefix, fold, links, tags};
use crate::editor::Editor;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---- directories ----

/// org-attach-id-uuid-folder-format.
pub fn uuid_folder_format(id: &str) -> Option<String> {
    (id.chars().count() > 2).then(|| {
        let (a, b) = split_chars(id, 2);
        format!("{a}/{b}")
    })
}

/// org-attach-id-ts-folder-format.
pub fn ts_folder_format(id: &str) -> Option<String> {
    (id.chars().count() > 6).then(|| {
        let (a, b) = split_chars(id, 6);
        format!("{a}/{b}")
    })
}

/// org-attach-id-fallback-folder-format.
pub fn fallback_folder_format(id: &str) -> Option<String> {
    let (a, _) = split_chars(id, 1);
    Some(format!("__/{a}/{id}"))
}

fn split_chars(s: &str, n: usize) -> (&str, &str) {
    let i = s.char_indices().nth(n).map_or(s.len(), |(i, _)| i);
    s.split_at(i)
}

/// org-attach-id-to-path-function-list (functions not ported are skipped).
fn id_functions() -> Vec<fn(&str) -> Option<String>> {
    let names: Vec<String> = match super::sexp::option("org-attach-id-to-path-function-list") {
        Some(v) => v.list().unwrap_or(&[]).iter().filter_map(|x| x.sym().or(x.str()).map(str::to_owned)).collect(),
        None => vec![
            "org-attach-id-uuid-folder-format".into(),
            "org-attach-id-ts-folder-format".into(),
            "org-attach-id-fallback-folder-format".into(),
        ],
    };
    names
        .iter()
        .filter_map(|n| match n.as_str() {
            "org-attach-id-uuid-folder-format" => Some(uuid_folder_format as fn(&str) -> Option<String>),
            "org-attach-id-ts-folder-format" => Some(ts_folder_format),
            "org-attach-id-fallback-folder-format" => Some(fallback_folder_format),
            _ => None,
        })
        .collect()
}

/// The buffer's directory (Emacs default-directory).
fn default_dir(ed: &Editor) -> PathBuf {
    ed.path
        .as_deref()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// expand-file-name of `name` in `dir`.
fn expand_in(name: &str, dir: &Path) -> PathBuf {
    let p = super::options::expand(name);
    if p.is_relative() { dir.join(p) } else { p }
}

/// org-attach-id-dir, expanded.
fn id_dir(ed: &Editor) -> PathBuf {
    expand_in(&id_dir_option(), &default_dir(ed))
}

fn id_dir_option() -> String {
    super::options::string("org-attach-id-dir", "data/")
}

/// org-attach-dir-from-id.
pub fn dir_from_id(ed: &Editor, id: &str, existing: bool) -> Option<PathBuf> {
    let here = default_dir(ed);
    let base = id_dir(ed);
    let mut fallback = vec![here.join("data/")];
    // A symlinked file also looks next to its target.
    if let Some(p) = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok())
        && p.is_symlink()
        && let Some(target) = std::fs::canonicalize(&p).ok().and_then(|t| t.parent().map(Path::to_path_buf))
    {
        for d in [target.join("data/"), expand_in(&id_dir_option(), &target)] {
            if !fallback.contains(&d) {
                fallback.insert(0, d);
            }
        }
    }
    fallback.retain(|d| *d != base && d.is_dir());
    let (mut preferred, mut first) = (None, None);
    for f in id_functions() {
        let Some(name) = f(id) else { continue };
        let candidate = base.join(&name);
        let fb: Vec<PathBuf> = fallback.iter().map(|d| d.join(&name)).filter(|d| d.is_dir()).collect();
        if !existing || candidate.is_dir() {
            preferred = Some(candidate);
            break;
        }
        first.get_or_insert(candidate);
        if let Some(d) = fb.into_iter().next() {
            preferred = Some(d);
            break;
        }
    }
    preferred.or(first)
}

/// org-attach-check-absolute-path.
fn check_absolute_path(ed: &Editor, dir: Option<&str>) -> Result<(), String> {
    if dir.is_some_and(|d| super::options::expand(d).is_absolute()) || super::options::expand(&id_dir_option()).is_absolute() || ed.path.is_some() {
        Ok(())
    } else {
        Err("Need absolute `org-attach-id-dir' to attach in buffers without filename".into())
    }
}

/// org-attach-use-inheritance.
fn inheritance() -> Inherit {
    match super::sexp::option("org-attach-use-inheritance") {
        None => Inherit::Selective,
        Some(v) if v.is_nil() => Inherit::No,
        Some(v) if v.sym() == Some("selective") => Inherit::Selective,
        Some(_) => Inherit::Yes,
    }
}

fn entry(ed: &Editor) -> Option<usize> {
    props::entry(ed, ed.cur.line)
}

/// org-attach-dir (without CREATE-IF-NOT-EXISTS-P; see [`with_dir`]).
pub fn dir(ed: &Editor, no_fs_check: bool) -> Result<Option<PathBuf>, String> {
    let h = entry(ed);
    let inh = inheritance();
    let found = if let Some(d) = props::get(ed, h, "DIR", inh).or_else(|| props::get(ed, h, "ATTACH_DIR", inh)) {
        check_absolute_path(ed, Some(&d))?;
        Some(expand_in(&d, &default_dir(ed)))
    } else if let Some(id) = props::get(ed, h, "ID", inh) {
        check_absolute_path(ed, None)?;
        dir_from_id(ed, &id, true)
    } else {
        None
    };
    Ok(if no_fs_check { found } else { found.filter(|d| d.is_dir()) })
}

/// org-attach-dir-get-create: the directory (created) is passed to `then`;
/// org-attach-preferred-new-method `ask` and `dir` prompt first.
pub fn with_dir(ed: &mut Editor, then: impl FnOnce(&mut Editor, PathBuf) -> Result<(), String> + Send + 'static) -> Result<(), String> {
    if let Some(d) = dir(ed, true)? {
        return finish_dir(ed, d, then);
    }
    let method = super::sexp::option("org-attach-preferred-new-method").map_or("id".to_owned(), |v| v.sym().unwrap_or(if v.is_nil() { "nil" } else { "id" }).to_owned());
    match method.as_str() {
        "id" => {
            let d = create_id_dir(ed)?;
            finish_dir(ed, d, then)
        }
        "dir" => set_directory(ed, Box::new(move |ed, d| finish_dir(ed, d, then))),
        "ask" => {
            let entries = vec![("1".into(), "ID property".into()), ("2".into(), "DIR property".into())];
            super::menu(ed, "Create new ID [1] property or DIR [2] property for attachments?", entries, move |ed, k| {
                let r = if k == "1" { create_id_dir(ed).and_then(|d| finish_dir(ed, d, then)) } else { set_directory(ed, Box::new(move |ed, d| finish_dir(ed, d, then))) };
                if let Err(e) = r {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        _ => Err("No existing directory.  DIR or ID property has to be explicitly created".into()),
    }
}

fn create_id_dir(ed: &mut Editor) -> Result<PathBuf, String> {
    let h = entry(ed);
    let id = match props::get(ed, h, "ID", Inherit::No) {
        Some(id) => id,
        None => {
            let id = links::new_id();
            props::put(ed, h, "ID", &id)?;
            id
        }
    };
    check_absolute_path(ed, None)?;
    dir_from_id(ed, &id, false).ok_or_else(|| format!("Failed to get folder for id {id}, adjust `org-attach-id-to-path-function-list'"))
}

fn finish_dir(ed: &mut Editor, d: PathBuf, then: impl FnOnce(&mut Editor, PathBuf) -> Result<(), String>) -> Result<(), String> {
    if !d.is_dir() {
        std::fs::create_dir_all(&d).map_err(|e| format!("Creating directory: {e}, {}", d.display()))?;
    }
    then(ed, d)
}

type DirThen = Box<dyn FnOnce(&mut Editor, PathBuf) -> Result<(), String> + Send>;

/// org-attach-set-directory: read DIR, ask to copy and delete the old one.
fn set_directory(ed: &mut Editor, then: DirThen) -> Result<(), String> {
    let old = dir(ed, false)?;
    let here = default_dir(ed);
    let initial = props::get(ed, entry(ed), "DIR", Inherit::No).unwrap_or_else(|| format!("{}/", here.display()));
    super::read(ed, "Attachment directory: ", &initial, move |ed, answer| {
        let new = expand_in(answer.trim(), &here);
        let value = if super::options::bool("org-attach-dir-relative", false) {
            relative(&new, &here).display().to_string()
        } else {
            new.display().to_string()
        };
        let h = entry(ed);
        if let Err(e) = props::put(ed, h, "DIR", &value) {
            ed.set_err(e);
            return;
        }
        let ret = new.clone();
        let done = move |ed: &mut Editor| {
            if let Err(e) = then(ed, ret) {
                ed.set_err(e);
            }
        };
        move_old(ed, old, Some(new), done);
    });
    Ok(())
}

