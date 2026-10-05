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
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.sym().or(x.str()).map(str::to_owned))
            .collect(),
        None => vec![
            "org-attach-id-uuid-folder-format".into(),
            "org-attach-id-ts-folder-format".into(),
            "org-attach-id-fallback-folder-format".into(),
        ],
    };
    names
        .iter()
        .filter_map(|n| match n.as_str() {
            "org-attach-id-uuid-folder-format" => {
                Some(uuid_folder_format as fn(&str) -> Option<String>)
            }
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
        && let Some(target) = std::fs::canonicalize(&p)
            .ok()
            .and_then(|t| t.parent().map(Path::to_path_buf))
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
        let fb: Vec<PathBuf> = fallback
            .iter()
            .map(|d| d.join(&name))
            .filter(|d| d.is_dir())
            .collect();
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
    if dir.is_some_and(|d| super::options::expand(d).is_absolute())
        || super::options::expand(&id_dir_option()).is_absolute()
        || ed.path.is_some()
    {
        Ok(())
    } else {
        Err("Need absolute `org-attach-id-dir' to attach in buffers without filename".into())
    }
}

/// A symbol-valued option: `t`, `nil` or the symbol's name (None: unset).
pub(crate) fn opt_sym(name: &str) -> Option<String> {
    use super::sexp::Sexp;
    super::sexp::option(name).map(|v| match v {
        Sexp::T => "t".into(),
        v if v.is_nil() => "nil".into(),
        v => v.str().map_or_else(|| "t".into(), str::to_owned),
    })
}

/// org-attach-use-inheritance.
fn inheritance() -> Inherit {
    match opt_sym("org-attach-use-inheritance").as_deref() {
        None | Some("selective") => Inherit::Selective,
        Some("nil") => Inherit::No,
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
    let found = if let Some(d) =
        props::get(ed, h, "DIR", inh).or_else(|| props::get(ed, h, "ATTACH_DIR", inh))
    {
        check_absolute_path(ed, Some(&d))?;
        Some(expand_in(&d, &default_dir(ed)))
    } else if let Some(id) = props::get(ed, h, "ID", inh) {
        check_absolute_path(ed, None)?;
        dir_from_id(ed, &id, true)
    } else {
        None
    };
    Ok(if no_fs_check {
        found
    } else {
        found.filter(|d| d.is_dir())
    })
}

/// org-attach-dir-get-create: the directory (created) is passed to `then`;
/// org-attach-preferred-new-method `ask` and `dir` prompt first.
pub fn with_dir(
    ed: &mut Editor,
    then: impl FnOnce(&mut Editor, PathBuf) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    if let Some(d) = dir(ed, true)? {
        return finish_dir(ed, d, then);
    }
    let method = opt_sym("org-attach-preferred-new-method").unwrap_or_else(|| "id".into());
    match method.as_str() {
        "id" => {
            let d = create_id_dir(ed)?;
            finish_dir(ed, d, then)
        }
        "dir" => set_directory(ed, Box::new(move |ed, d| finish_dir(ed, d, then))),
        "ask" => {
            let entries = vec![
                ("1".into(), "ID property".into()),
                ("2".into(), "DIR property".into()),
            ];
            super::menu(
                ed,
                "Create new ID [1] property or DIR [2] property for attachments?",
                entries,
                move |ed, k| {
                    let r = if k == "1" {
                        create_id_dir(ed).and_then(|d| finish_dir(ed, d, then))
                    } else {
                        set_directory(ed, Box::new(move |ed, d| finish_dir(ed, d, then)))
                    };
                    if let Err(e) = r {
                        ed.set_err(e);
                    }
                },
            );
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
    dir_from_id(ed, &id, false).ok_or_else(|| {
        format!("Failed to get folder for id {id}, adjust `org-attach-id-to-path-function-list'")
    })
}

fn finish_dir(
    ed: &mut Editor,
    d: PathBuf,
    then: impl FnOnce(&mut Editor, PathBuf) -> Result<(), String>,
) -> Result<(), String> {
    if !d.is_dir() {
        std::fs::create_dir_all(&d)
            .map_err(|e| format!("Creating directory: {e}, {}", d.display()))?;
    }
    then(ed, d)
}

type DirThen = Box<dyn FnOnce(&mut Editor, PathBuf) -> Result<(), String> + Send>;

/// org-attach-set-directory: read DIR, ask to copy and delete the old one.
fn set_directory(ed: &mut Editor, then: DirThen) -> Result<(), String> {
    let old = dir(ed, false)?;
    let here = default_dir(ed);
    let initial = props::get(ed, entry(ed), "DIR", Inherit::No)
        .unwrap_or_else(|| format!("{}/", here.display()));
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

/// y-or-n-p / yes-or-no-p whose continuation sees the answer.
fn ask(ed: &mut Editor, q: &str, then: impl FnOnce(&mut Editor, bool) + Send + 'static) {
    super::menu(
        ed,
        q,
        vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
        move |ed, k| then(ed, k == "y"),
    );
}

/// The "Copy over attachments from old directory?" and "Delete OLD"
/// questions of org-attach-set-directory and org-attach-unset-directory.
fn move_old(
    ed: &mut Editor,
    old: Option<PathBuf>,
    new: Option<PathBuf>,
    done: impl FnOnce(&mut Editor) + Send + 'static,
) {
    let Some(old) = old.filter(|o| Some(o) != new.as_ref()) else {
        return done(ed);
    };
    let ask_delete = {
        let old = old.clone();
        move |ed: &mut Editor| {
            let q = format!("Delete {}", old.display());
            ask(ed, &q, move |ed, yes| {
                if yes && let Err(e) = std::fs::remove_dir_all(&old) {
                    ed.set_err(format!("Removing directory: {e}, {}", old.display()));
                }
                done(ed);
            });
        }
    };
    match new {
        Some(new) => ask(
            ed,
            "Copy over attachments from old directory? ",
            move |ed, yes| {
                if yes && let Err(e) = copy_dir(&old, &new) {
                    ed.set_err(e);
                }
                ask_delete(ed);
            },
        ),
        None => ask_delete(ed),
    }
}

/// copy-directory OLD NEW, merging its contents into NEW.
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for e in std::fs::read_dir(from)
        .map_err(|e| format!("{}: {e}", from.display()))?
        .flatten()
    {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("Copying {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// file-relative-name.
pub fn relative(path: &Path, base: &Path) -> PathBuf {
    let (p, b): (Vec<_>, Vec<_>) = (path.components().collect(), base.components().collect());
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// org-attach-unset-directory.
fn unset_directory(ed: &mut Editor) -> Result<(), String> {
    let old = dir(ed, false)?;
    let h = entry(ed);
    props::delete(ed, h, "DIR");
    props::delete(ed, h, "ATTACH_DIR");
    let new = dir(ed, false)?;
    move_old(ed, old, new, |_| {});
    Ok(())
}

/// org-attach-file-list: names in DIRECTORY, sorted, without `.`, `..` and
/// backups ending in `~`.
pub fn file_list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.ends_with('~'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// org-attach-expand: FILE in the entry's attachment directory.
pub fn expand(ed: &Editor, file: &str) -> PathBuf {
    match dir(ed, false).ok().flatten() {
        Some(d) => expand_in(file, &d),
        None => expand_in(file, &default_dir(ed)),
    }
}

// ---- tag and hooks ----

/// org-attach-tag: the auto tag on (or off) on the entry's heading.
pub fn tag(ed: &mut Editor, on: bool) {
    let t = match opt_sym("org-attach-auto-tag").as_deref() {
        None => "ATTACH".to_owned(),
        Some("nil") => return,
        Some(s) => s.to_owned(),
    };
    if let Some(h) = fold::back_to_heading(ed, ed.cur.line) {
        let (line, byte) = (ed.cur.line, ed.cur.byte);
        tags::toggle_tag(ed, h, &t, Some(on));
        ed.set_cursor(line, byte);
        ed.clamp_cursor();
    }
}

fn hook(name: &str) -> Vec<String> {
    super::sexp::option(name)
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.sym().or(x.str()).map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// org-attach-after-change-hook.
fn after_change(ed: &mut Editor, dir: &Path) {
    for f in hook("org-attach-after-change-hook") {
        if f == "org-attach-git-commit"
            && let Err(e) = git_commit(ed, dir)
        {
            ed.set_err(e);
        }
    }
}

/// org-attach-open-hook, then `then`.
fn open_hook(
    ed: &mut Editor,
    path: PathBuf,
    then: impl FnOnce(&mut Editor) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    if hook("org-attach-open-hook")
        .iter()
        .any(|f| f == "org-attach-git-annex-get-maybe")
    {
        return annex_get_maybe(ed, &path, then);
    }
    then(ed)
}

// ---- org-attach-git ----

/// org-attach-git-dir: the directory whose repository records attachments.
fn git_base(ed: &Editor) -> Option<PathBuf> {
    match opt_sym("org-attach-git-dir").as_deref() {
        Some("individual-repository") => dir(ed, false).ok().flatten(),
        _ => Some(id_dir(ed)),
    }
}

/// vc-git-root.
fn git_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// org-attach-git-annex-cutoff.
fn annex_cutoff() -> Option<u64> {
    match super::sexp::option("org-attach-git-annex-cutoff") {
        None => Some(32 * 1024),
        Some(v) => v.int().map(|n| n.max(0) as u64),
    }
}

/// org-attach-git-use-annex.
pub fn git_use_annex(ed: &Editor) -> bool {
    let Some(root) = git_base(ed).as_deref().and_then(git_root) else {
        return false;
    };
    annex_cutoff().is_some() && (root.join("annex").exists() || root.join(".git/annex").exists())
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// org-attach-git-commit: add new and modified files (annexed above the
/// cutoff), remove deleted ones, and commit "Synchronized attachments".
pub fn git_commit(ed: &mut Editor, _changed: &Path) -> Result<(), String> {
    let Some(dir) = git_base(ed) else {
        return Ok(());
    };
    if git_root(&dir).is_none() || !dir.is_dir() {
        return Ok(());
    }
    let annex = git_use_annex(ed);
    let cutoff = annex_cutoff().unwrap_or(u64::MAX);
    let mut changes = 0;
    for f in run_git(&dir, &["ls-files", "-zmo", "--exclude-standard"])?
        .split('\0')
        .filter(|f| !f.is_empty())
    {
        let big = annex && std::fs::metadata(dir.join(f)).is_ok_and(|m| m.len() >= cutoff);
        if big {
            run_git(&dir, &["annex", "add", f])?;
        } else {
            run_git(&dir, &["add", f])?;
        }
        changes += 1;
    }
    for f in run_git(&dir, &["ls-files", "-z", "--deleted"])?
        .split('\0')
        .filter(|f| !f.is_empty())
    {
        run_git(&dir, &["rm", f])?;
        changes += 1;
    }
    if changes > 0 {
        run_git(&dir, &["commit", "-m", "Synchronized attachments"])?;
    }
    Ok(())
}

/// org-attach-git-annex-get-maybe: fetch PATH's content with `git annex
/// get` when it is not present (org-attach-git-annex-auto-get).
fn annex_get_maybe(
    ed: &mut Editor,
    path: &Path,
    then: impl FnOnce(&mut Editor) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let Some(dir) = git_base(ed) else {
        return then(ed);
    };
    if !git_use_annex(ed) {
        return then(ed);
    }
    let rel = relative(path, &dir).display().to_string();
    if run_git(
        &dir,
        &["annex", "find", "--format=found", "--in=here", &rel],
    )? == "found"
    {
        return then(ed);
    }
    let q = format!("Run git annex get {rel}? ");
    let get = move |ed: &mut Editor| -> Result<(), String> {
        ed.set_msg(format!("Running git annex get \"{rel}\"."));
        run_git(&dir, &["annex", "get", &rel])?;
        then(ed)
    };
    let err = format!(
        "File {} stored in git annex but unavailable",
        path.display()
    );
    match opt_sym("org-attach-git-annex-auto-get").as_deref() {
        Some("nil") => Err(err),
        Some(v) if v != "ask" => get(ed),
        _ => {
            ask(ed, &q, move |ed, yes| {
                let r = if yes { get(ed) } else { Err(err) };
                if let Err(e) = r {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
    }
}

// ---- attaching ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Cp,
    Mv,
    Ln,
    Lns,
    Url,
}

/// org-attach-method.
fn method_option() -> Method {
    match super::options::string("org-attach-method", "cp").as_str() {
        "mv" => Method::Mv,
        "ln" => Method::Ln,
        "lns" => Method::Lns,
        _ => Method::Cp,
    }
}

/// file-name-nondirectory (of a path or URL).
fn basename(file: &str) -> String {
    let f = file.trim_end_matches('/');
    let f = f.split(['?', '#']).next().unwrap_or(f);
    f.rsplit('/').next().unwrap_or(f).to_owned()
}

fn transfer(method: Method, file: &str, to: &Path) -> Result<(), String> {
    let src = super::options::expand(file);
    let e = |e: std::io::Error| format!("{file}: {e}");
    match method {
        Method::Mv => std::fs::rename(&src, to).or_else(|_| {
            // Across file systems: copy, then delete.
            transfer(Method::Cp, file, to)?;
            if src.is_dir() {
                std::fs::remove_dir_all(&src)
            } else {
                std::fs::remove_file(&src)
            }
            .map_err(e)
        }),
        Method::Cp if src.is_dir() => copy_dir(&src, to),
        Method::Cp => std::fs::copy(&src, to).map(|_| ()).map_err(e),
        Method::Ln => std::fs::hard_link(&src, to).map_err(e),
        Method::Lns => std::os::unix::fs::symlink(&src, to).map_err(e),
        Method::Url => {
            let o = Command::new("curl")
                .args(["-fsSL", "-o"])
                .arg(to)
                .arg(file)
                .output()
                .map_err(|e| format!("curl: {e}"))?;
            if o.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Downloading {file}: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ))
            }
        }
    }
}

/// org-attach-attach: FILE into the entry's directory (dired on it when
/// VISIT-DIR); stores a link per org-attach-store-link-p.
pub fn attach_file(
    ed: &mut Editor,
    file: String,
    visit_dir: bool,
    method: Method,
) -> Result<(), String> {
    let file = if super::options::expand(&file).is_dir() {
        file.trim_end_matches('/').to_owned()
    } else {
        file
    };
    with_dir(ed, move |ed, dir| {
        let base = basename(&file);
        let target = dir.join(&base);
        let exists = target.exists() || target.is_symlink();
        let t2 = target.clone();
        let go = move |ed: &mut Editor| -> Result<(), String> {
            transfer(method, &file, &target)?;
            after_change(ed, &dir);
            tag(ed, true);
            let mode = opt_sym("org-attach-store-link-p");
            let stored = match mode.as_deref() {
                None | Some("attached") => Some(format!("attachment:{base}")),
                Some("file") => Some(format!("file:{}", target.display())),
                Some("nil") => None,
                Some(_) => Some(format!("file:{file}")),
            };
            if let Some(link) = stored {
                links::add_stored(ed, link, Some(base.clone()));
            }
            if visit_dir {
                open_dired(ed, dir);
            } else {
                ed.set_msg(format!("File \"{base}\" is now an attachment"));
            }
            Ok(())
        };
        if exists {
            let q = format!("Attachment {} already exists.  Overwrite?", t2.display());
            ask(ed, &q, move |ed, yes| {
                let r = if yes {
                    let rm = if t2.is_dir() && !t2.is_symlink() {
                        std::fs::remove_dir_all(&t2)
                    } else {
                        std::fs::remove_file(&t2)
                    };
                    rm.map_err(|e| e.to_string()).and_then(|()| go(ed))
                } else {
                    Err(format!("Attachment already exists: {}", t2.display()))
                };
                if let Err(e) = r {
                    ed.set_err(e);
                }
            });
            return Ok(());
        }
        go(ed)
    })
}

fn open_dired(ed: &mut Editor, dir: PathBuf) {
    ed.pending_effect = Some(crate::ex::ExEffect::Open {
        path: dir,
        line: 0,
        col: 0,
        pattern: None,
    });
}

/// read-file-name for "File to keep as an attachment: ".
fn read_file(
    ed: &mut Editor,
    prompt: &str,
    then: impl FnOnce(&mut Editor, String) + Send + 'static,
) {
    let here = default_dir(ed);
    super::read(ed, prompt, &format!("{}/", here.display()), move |ed, f| {
        let f = f.trim();
        if f.is_empty() {
            return;
        }
        then(ed, expand_in(f, &here).display().to_string());
    });
}

fn interactive_attach(ed: &mut Editor, arg: Prefix, method: Method) -> Result<(), String> {
    if let Some(f) = super::take_call_arg() {
        return attach_file(ed, f, !arg.is_none(), method);
    }
    read_file(ed, "File to keep as an attachment: ", move |ed, f| {
        if let Err(e) = attach_file(ed, f, !arg.is_none(), method) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// Run `f` now with a string argument from [`super::call_with`], or after
/// reading one.
fn with_string(
    ed: &mut Editor,
    prompt: &str,
    f: impl FnOnce(&mut Editor, String) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    if let Some(s) = super::take_call_arg() {
        return f(ed, s);
    }
    super::read(ed, prompt, "", move |ed, s| {
        if let Err(e) = f(ed, s) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// org-attach-buffer: a buffer's text as a new attachment.
fn attach_buffer(ed: &mut Editor) -> Result<(), String> {
    super::effect(ed, |s| {
        let me = s.org_current();
        let mut bufs: Vec<(String, String)> = vec![];
        for i in 0..s.org_buffer_count() {
            let (name, text) = s.org_with_buffer(i, |ed| {
                let name = ed
                    .org_buffer_name
                    .clone()
                    .or_else(|| {
                        ed.path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| "[No Name]".into());
                (name, ed.buf.text())
            });
            // Other buffers first, as other-buffer would offer them.
            if i != me {
                bufs.insert(0, (name, text));
            } else {
                bufs.push((name, text));
            }
        }
        let names = bufs.iter().map(|(n, _)| n.clone()).collect();
        super::complete(
            &mut s.ed,
            "Buffer whose contents should be attached: ",
            names,
            true,
            move |ed, name| {
                let text = bufs
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, t)| t)
                    .unwrap_or_default();
                let r = with_dir(ed, move |ed, dir| {
                    let out = dir.join(&name);
                    if out.exists() {
                        return Err(format!("File exists: {}", out.display()));
                    }
                    std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
                    after_change(ed, &dir);
                    tag(ed, true);
                    Ok(())
                });
                if let Err(e) = r {
                    ed.set_err(e);
                }
            },
        );
    });
    Ok(())
}

/// org-attach-new: visit a new file in the attachment directory.
fn attach_new(ed: &mut Editor, file: String) -> Result<(), String> {
    with_dir(ed, move |ed, dir| {
        tag(ed, true);
        let path = expand_in(&file, &dir);
        ed.pending_effect = Some(crate::ex::ExEffect::Open {
            path,
            line: 0,
            col: 0,
            pattern: None,
        });
        ed.set_msg(format!("New attachment {file}"));
        Ok(())
    })
}

/// org-attach-delete-one.
fn delete_one(ed: &mut Editor) -> Result<(), String> {
    let d = dir(ed, false)?.ok_or("No attachment directory exist")?;
    let files = file_list(&d);
    let del = move |ed: &mut Editor, name: String| -> Result<(), String> {
        let p = expand_in(&name, &d);
        if !p.exists() && !p.is_symlink() {
            return Err(format!("No such attachment: {}", p.display()));
        }
        std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        after_change(ed, &d);
        Ok(())
    };
    if let Some(f) = super::take_call_arg() {
        return del(ed, f);
    }
    super::complete(ed, "Delete attachment: ", files, false, move |ed, f| {
        if let Err(e) = del(ed, f) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// org-attach-delete-all (FORCE: recursive, no questions).
fn delete_all(ed: &mut Editor, force: bool) -> Result<(), String> {
    let Some(d) = dir(ed, false)? else {
        return Ok(());
    };
    delete_dir(ed, d, force, true);
    Ok(())
}

fn delete_dir(ed: &mut Editor, d: PathBuf, force: bool, untag: bool) {
    let remove = move |ed: &mut Editor, recursive: bool| {
        let r = if recursive {
            std::fs::remove_dir_all(&d)
        } else {
            std::fs::remove_dir(&d)
        };
        if let Err(e) = r {
            ed.set_err(format!("Removing directory: {e}, {}", d.display()));
            return;
        }
        ed.set_msg("Attachment directory removed");
        after_change(ed, &d);
        if untag {
            tag(ed, false);
        }
    };
    if force {
        return remove(ed, true);
    }
    ask(
        ed,
        "Really remove all attachments of this entry? ",
        move |ed, yes| {
            if yes {
                ask(ed, "Recursive?", remove);
            }
        },
    );
}

/// org-attach-sync.
fn sync(ed: &mut Editor) -> Result<(), String> {
    let Some(d) = dir(ed, false)? else {
        tag(ed, false);
        return Ok(());
    };
    after_change(ed, &d);
    let files = file_list(&d);
    tag(ed, !files.is_empty());
    let opt = opt_sym("org-attach-sync-delete-empty-dir");
    if opt.as_deref() == Some("nil") || !files.is_empty() {
        return Ok(());
    }
    if opt.is_none() || opt.as_deref() == Some("query") {
        ask(
            ed,
            "Attachment directory is empty.  Delete?",
            move |ed, yes| {
                if yes && let Err(e) = std::fs::remove_dir(&d) {
                    ed.set_err(format!("Removing directory: {e}"));
                }
            },
        );
    } else {
        std::fs::remove_dir(&d).map_err(|e| format!("Removing directory: {e}"))?;
    }
    Ok(())
}

/// org-attach-open (IN-EMACS: visit in Fred).
fn open(ed: &mut Editor, in_emacs: bool) -> Result<(), String> {
    let d = dir(ed, false)?.ok_or("No attachment directory exist")?;
    let files = file_list(&d);
    let arg = if in_emacs { Prefix::U(1) } else { Prefix::None };
    let go = move |ed: &mut Editor, f: String| -> Result<(), String> {
        let path = d.join(f);
        let p = path.display().to_string();
        open_hook(ed, path, move |ed| {
            links::open_file(ed, &p, None, arg, None)
        })
    };
    if let [f] = files.as_slice() {
        return go(ed, f.clone());
    }
    super::complete(ed, "Open attachment: ", files, true, move |ed, f| {
        if let Err(e) = go(ed, f) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// org-attach-follow: an `attachment:` link (FILE, maybe `FILE::search`).
fn follow(ed: &mut Editor, link: &str, arg: Prefix) -> Result<(), String> {
    let (file, search) = match link.split_once("::") {
        Some((f, s)) => (f, Some(s.to_owned())),
        None => (link, None),
    };
    let p = expand(ed, file).display().to_string();
    links::open_file(ed, &p, search, arg, None)
}

/// org-attach-complete-link: an `attachment:` link to a file of the entry,
/// inserted at point.
fn complete_link(ed: &mut Editor) -> Result<(), String> {
    let d = dir(ed, false)?.ok_or("No attachment directory exist")?;
    let files = file_list(&d);
    super::complete(ed, "File: ", files, false, move |ed, f| {
        let full = expand_in(&f, &d);
        let rel = full
            .strip_prefix(&d)
            .map_or(f.clone(), |r| r.display().to_string());
        if let Err(e) =
            links::insert_link(ed, Prefix::None, Some(format!("attachment:{rel}")), None)
        {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// org-attach-expand-links: `attachment:` links become `file:` links to
/// the expanded path (export runs it on a copy of the buffer).
pub fn expand_links(ed: &mut Editor) {
    let st = super::settings(ed);
    let (cl, cb) = (ed.cur.line, ed.cur.byte);
    for l in 0..ed.line_count() {
        let line = ed.buf.line(l);
        if !line.contains("attachment:") {
            continue;
        }
        let found: Vec<links::Link> = links::links_in(&line, &st)
            .into_iter()
            .filter(|lk| lk.kind == "attachment")
            .collect();
        if found.is_empty() {
            continue;
        }
        ed.set_cursor(l, 0);
        let mut out = line.clone();
        for lk in found.iter().rev() {
            let target = format!("file:{}", expand(ed, &lk.path).display());
            if let Ok(s) = links::make_string(&target, lk.desc.as_deref()) {
                out.replace_range(lk.range.clone(), &s);
            }
        }
        super::set_line(ed, l, &out);
    }
    ed.set_cursor(cl, cb);
    ed.clamp_cursor();
}

/// org-attach-archive-delete-maybe (org-archive-hook).
fn archive_delete_maybe(ed: &mut Editor) -> Result<(), String> {
    let Some(v) = opt_sym("org-attach-archive-delete").filter(|v| v != "nil") else {
        return Ok(());
    };
    let Some(d) = dir(ed, false)? else {
        return Ok(());
    };
    delete_dir(ed, d, v != "query", false);
    Ok(())
}

/// org-attach-dired-to-subtree: dired's marked files (or the file at
/// point) attached to the entry at point in the most recent Org buffer.
fn dired_to_subtree(ed: &mut Editor) -> Result<(), String> {
    if ed.dired.is_none() {
        return Err("This command must be triggered in a `dired' buffer".into());
    }
    let files = crate::dired::selection(ed);
    super::effect(ed, move |s| {
        let dired = s.org_current();
        let Some(i) = s.org_recent_org_buffer() else {
            s.ed.set_err("Can't attach to subtree.  No window displaying an Org buffer");
            return;
        };
        s.org_show(i);
        let method = method_option();
        for f in files {
            if let Err(e) = attach_file(&mut s.ed, f.display().to_string(), false, method) {
                s.ed.set_err(e);
                return;
            }
            if s.ed.mode != crate::editor::Mode::Normal {
                // A question is pending in the Org buffer: stay there.
                return;
            }
        }
        s.org_show(dired);
        if method == Method::Mv {
            crate::dired::refresh(&mut s.ed);
        }
    });
    Ok(())
}

/// The dispatcher's commands (org-attach-commands): key, command, doc.
const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "a",
        "org-attach-attach",
        "Select a file and attach it to the task, using `org-attach-method'.",
    ),
    (
        "c",
        "org-attach-attach-cp",
        "Attach a file using copy method.",
    ),
    (
        "m",
        "org-attach-attach-mv",
        "Attach a file using move method.",
    ),
    (
        "l",
        "org-attach-attach-ln",
        "Attach a file using link method.",
    ),
    (
        "y",
        "org-attach-attach-lns",
        "Attach a file using symbolic-link method.",
    ),
    (
        "u",
        "org-attach-url",
        "Attach a file from URL (downloading it).",
    ),
    (
        "b",
        "org-attach-buffer",
        "Select a buffer and attach its contents to the task.",
    ),
    (
        "n",
        "org-attach-new",
        "Create a new attachment, as an Emacs buffer.",
    ),
    (
        "z",
        "org-attach-sync",
        "Synchronize the current node with its attachment directory, in case you added attachments yourself.",
    ),
    ("o", "org-attach-open", "Open current node's attachments."),
    (
        "O",
        "org-attach-open-in-emacs",
        "Like \"o\", but force opening in Emacs.",
    ),
    (
        "f",
        "org-attach-reveal",
        "Open current node's attachment directory.  Create if missing.",
    ),
    (
        "F",
        "org-attach-reveal-in-emacs",
        "Like \"f\", but force using Dired in Emacs.",
    ),
    (
        "d",
        "org-attach-delete-one",
        "Delete one attachment, you will be prompted for a file name.",
    ),
    (
        "D",
        "org-attach-delete-all",
        "Delete all of a node's attachments.  A safer way is to open the directory in dired and delete from there.",
    ),
    (
        "s",
        "org-attach-set-directory",
        "Set a specific attachment directory for this entry. Sets DIR property.",
    ),
    (
        "S",
        "org-attach-unset-directory",
        "Unset the attachment directory for this entry.  Removes DIR property.",
    ),
    ("q", "", "Abort."),
];

/// Keys with a control-key alias in the dispatcher (`C-a` runs `a`...).
const CTRL: &str = "acmlyunzofds";

/// org-attach: the dispatcher.
fn dispatcher(ed: &mut Editor) -> Result<(), String> {
    if ed.org_view.is_some() {
        // ponytail: agenda items need the agenda port's markers.
        return Err("No item in current line".into());
    }
    let d = dir(ed, true)?;
    let mut entries: Vec<(String, String)> = vec![];
    if !super::options::bool("org-attach-expert", false) {
        let folder = match &d {
            Some(d) if d.is_dir() => d.display().to_string(),
            Some(d) => format!("{}  (Not yet created)", d.display()),
            None => "Can't find an existing attachment-folder".into(),
        };
        entries.push((String::new(), format!("#Attachment folder: {folder}")));
        entries.push((String::new(), "#Select an Attachment Command:".into()));
    }
    for (k, _, doc) in COMMANDS {
        entries.push(((*k).into(), (*doc).into()));
    }
    for c in CTRL.chars() {
        if let Some((_, cmd, _)) = COMMANDS.iter().find(|(k, _, _)| k.starts_with(c)) {
            entries.push((format!("C-{c}"), format!("{cmd} (as {c})")));
        }
    }
    let keys: String = COMMANDS.iter().map(|(k, _, _)| *k).collect();
    super::menu(
        ed,
        &format!("Select command: [{keys}]"),
        entries,
        |ed, k| {
            let k = k.strip_prefix("C-").map_or(k.clone(), str::to_owned);
            match COMMANDS.iter().find(|(key, _, _)| *key == k) {
                Some((_, "", _)) => ed.set_msg("Abort"),
                Some((_, cmd, _)) => super::run(ed, cmd, Prefix::None),
                None => ed.set_err(format!("No such attachment command: {k}")),
            }
        },
    );
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-attach" => dispatcher(ed),
        "org-attach-attach" => interactive_attach(ed, arg, method_option()),
        "org-attach-attach-cp" => interactive_attach(ed, arg, Method::Cp),
        "org-attach-attach-mv" => interactive_attach(ed, arg, Method::Mv),
        "org-attach-attach-ln" => interactive_attach(ed, arg, Method::Ln),
        "org-attach-attach-lns" => interactive_attach(ed, arg, Method::Lns),
        "org-attach-url" => with_string(ed, "URL of the file to attach: ", |ed, url| {
            attach_file(ed, url, false, Method::Url)
        }),
        "org-attach-buffer" => attach_buffer(ed),
        "org-attach-new" => with_string(ed, "Create attachment named: ", attach_new),
        "org-attach-sync" => sync(ed),
        "org-attach-open" => open(ed, !arg.is_none()),
        "org-attach-open-in-emacs" => open(ed, true),
        "org-attach-reveal" => with_dir(ed, |ed, d| {
            links::open_file(ed, &d.display().to_string(), None, Prefix::None, None)
        }),
        "org-attach-reveal-in-emacs" => with_dir(ed, |ed, d| {
            open_dired(ed, d);
            Ok(())
        }),
        "org-attach-delete-one" => delete_one(ed),
        "org-attach-delete-all" => delete_all(ed, !arg.is_none()),
        "org-attach-set-directory" => set_directory(ed, Box::new(|_, _| Ok(()))),
        "org-attach-unset-directory" => unset_directory(ed),
        "org-attach-dir-get-create" => with_dir(ed, |ed, d| {
            ed.set_msg(d.display().to_string());
            Ok(())
        }),
        "org-attach-dired-to-subtree" => dired_to_subtree(ed),
        // Functions other modules reach by name.
        "org-attach-follow" | "org-attach-open-link" => {
            let link = super::take_call_arg().unwrap_or_default();
            follow(ed, &link, arg)
        }
        "org-attach-complete-link" => complete_link(ed),
        "org-attach-expand-links" => {
            expand_links(ed);
            Ok(())
        }
        "org-attach-archive-delete-maybe" => archive_delete_maybe(ed),
        "org-attach-tag" => {
            tag(ed, arg.is_none());
            Ok(())
        }
        "org-attach-untag" => {
            tag(ed, false);
            Ok(())
        }
        "org-attach-git-commit" => {
            let d = dir(ed, false).ok().flatten().unwrap_or_default();
            git_commit(ed, &d)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
