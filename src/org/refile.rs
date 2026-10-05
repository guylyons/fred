//! Refiling (org-refile.el) and archiving (org-archive.el).

use super::props::{Doc, Inherit};
use super::sexp::Sexp;
use super::syntax::{self, Lines, Settings};
use super::{Prefix, ctx, fold, structure, tags};
use crate::editor::Editor;
use crate::session::Session;
use std::path::{Path, PathBuf};

thread_local! {
    /// org-refile-history and the last refile location (file, line).
    static HISTORY: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(vec![]) };
    static LAST: std::cell::RefCell<Option<(PathBuf, usize)>> = const { std::cell::RefCell::new(None) };
}

/// A refile target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub file: PathBuf,
    /// The heading line (None: the file itself, top level).
    pub line: Option<usize>,
    pub heading: String,
}

/// org-get-outline-path for heading `h` (titles of ancestors, and itself).
pub fn outline_path<L: Lines + ?Sized>(
    b: &L,
    st: &Settings,
    h: usize,
    with_self: bool,
) -> Vec<String> {
    let mut out = vec![];
    let mut cur = if with_self {
        Some(h)
    } else {
        syntax::parent(b, h)
    };
    while let Some(x) = cur {
        let line = b.line_text(x);
        out.push(super::links::display_format(
            &syntax::headline(&line, st)
                .map(|hl| hl.title(&line).to_owned())
                .unwrap_or_default(),
        ));
        cur = syntax::parent(b, x);
    }
    out.reverse();
    out
}

/// One `(FILES . SPEC)` of org-refile-targets as (files, predicate on a headline).
fn target_specs(current: Option<&Path>) -> Vec<(Vec<PathBuf>, Sexp)> {
    let entries = super::sexp::option("org-refile-targets")
        .and_then(|v| v.list().map(<[Sexp]>::to_vec))
        .filter(|l| !l.is_empty());
    let entries = entries.unwrap_or_else(|| {
        vec![Sexp::Dotted(
            vec![Sexp::Nil],
            Box::new(Sexp::Dotted(
                vec![Sexp::Sym(":level".into())],
                Box::new(Sexp::Int(1)),
            )),
        )]
    });
    entries
        .into_iter()
        .map(|e| {
            let files = e.car().cloned().unwrap_or(Sexp::Nil);
            let spec = e.cdr();
            let files: Vec<PathBuf> = match &files {
                Sexp::Nil => current.map(Path::to_path_buf).into_iter().collect(),
                Sexp::Sym(s) if s == "org-agenda-files" => super::agenda_files(),
                Sexp::Str(f) => vec![super::options::expand(f)],
                Sexp::List(l) => l
                    .iter()
                    .filter_map(|x| x.str().map(super::options::expand))
                    .collect(),
                _ => vec![],
            };
            // (FILE :maxlevel N) written as a list.
            let spec = match &spec {
                Sexp::List(v) if v.len() == 2 => {
                    Sexp::Dotted(vec![v[0].clone()], Box::new(v[1].clone()))
                }
                s => s.clone(),
            };
            (files, spec)
        })
        .collect()
}

fn spec_matches(spec: &Sexp, line: &str, st: &Settings) -> bool {
    let odd = st.opt_bool("org-odd-levels-only", false);
    let Some(lv) = syntax::level(line) else {
        return false;
    };
    let key = spec.car().and_then(Sexp::sym).unwrap_or("");
    let val = spec.cdr();
    if spec.truthy() && spec.sym() == Some("t") {
        return true;
    }
    match key {
        ":level" => {
            let n = val.int().unwrap_or(1) as usize;
            lv == if odd { 2 * n - 1 } else { n }
        }
        ":maxlevel" => {
            let n = val.int().unwrap_or(1) as usize;
            lv <= if odd { 2 * n - 1 } else { n }
        }
        ":tag" => syntax::headline(line, st)
            .is_some_and(|h| h.tags.iter().any(|t| Some(t.as_str()) == val.str())),
        ":todo" => syntax::headline(line, st).is_some_and(|h| h.todo.as_deref() == val.str()),
        ":regexp" => val
            .str()
            .and_then(|r| super::re::compile(r, false).ok())
            .is_some_and(|r| r.is_match(line)),
        _ => false,
    }
}

/// org-refile-get-targets.
pub fn targets(s: &Session, current: Option<&Path>) -> Vec<Target> {
    let use_path = super::sexp::option("org-refile-use-outline-path")
        .map_or("nil".to_owned(), |v| {
            v.sym().or(v.str()).unwrap_or("t").to_owned()
        });
    let mut out: Vec<Target> = vec![];
    for (files, spec) in target_specs(current) {
        for f in files {
            let Ok(text) = s.org_text(&f) else { continue };
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            let st = super::settings_of(&text, f.parent());
            let fname = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let base: Vec<String> = match use_path.as_str() {
                "file" => vec![fname.clone()],
                "full-file-path" => vec![f.display().to_string()],
                "buffer-name" => vec![fname.clone()],
                "title" => vec![
                    st.value("TITLE")
                        .map(str::to_owned)
                        .unwrap_or(fname.clone()),
                ],
                _ => vec![],
            };
            if !base.is_empty() {
                out.push(Target {
                    name: base[0].clone(),
                    file: f.clone(),
                    line: None,
                    heading: String::new(),
                });
            }
            for (i, l) in lines.iter().enumerate() {
                if !spec_matches(&spec, l, &st) {
                    continue;
                }
                let Some(h) = syntax::headline(l, &st) else {
                    continue;
                };
                let title = h.title(l).to_owned();
                if title.is_empty() {
                    continue;
                }
                let name = if use_path == "nil" {
                    super::links::display_format(&title)
                } else {
                    base.iter()
                        .cloned()
                        .chain(
                            outline_path(&lines, &st, i, true)
                                .into_iter()
                                .map(|p| p.replace('/', "\\/")),
                        )
                        .collect::<Vec<_>>()
                        .join("/")
                };
                let t = Target {
                    name,
                    file: f.clone(),
                    line: Some(i),
                    heading: l.clone(),
                };
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Display names for completion (file shown when not the current one).
fn display(t: &Target, current: Option<&Path>, use_path: bool) -> String {
    let extra = if use_path { "/" } else { "" };
    let same =
        current.and_then(|c| std::path::absolute(c).ok()) == std::path::absolute(&t.file).ok();
    let path_style =
        super::sexp::option("org-refile-use-outline-path").and_then(|v| v.sym().map(str::to_owned));
    if !same
        && !matches!(
            path_style.as_deref(),
            Some("file" | "full-file-path" | "title")
        )
    {
        format!(
            "{}{extra} ({})",
            t.name,
            t.file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
    } else {
        format!("{}{extra}", t.name)
    }
}

/// The text of the subtree at `h` (with trailing newline), and its end.
fn subtree_text(ed: &Editor, h: usize) -> (String, usize) {
    let end = syntax::subtree_end(&ed.buf, h);
    (
        super::lines(ed, h..end)
            .iter()
            .map(|l| format!("{l}\n"))
            .collect(),
        end,
    )
}

/// Insert `tree` at line `at` with its top level set to `level`.
pub fn insert_tree(ed: &mut Editor, at: usize, level: usize, tree: &str) -> usize {
    let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
    let first = tree.lines().find_map(syntax::level).unwrap_or(level);
    let shift = level as i64 - first as i64;
    let lines: Vec<String> = tree
        .strip_suffix('\n')
        .unwrap_or(tree)
        .split('\n')
        .map(|l| match syntax::level(l) {
            Some(n) if shift != 0 => {
                let mut lv = n;
                for _ in 0..shift.abs() {
                    lv = structure::valid_level(odd, lv, shift.signum());
                }
                format!("{}{}", "*".repeat(lv), &l[n..])
            }
            _ => l.to_owned(),
        })
        .collect();
    let at = at.min(ed.line_count());
    if ed.line_count() == 1 && ed.buf.len_bytes() == 0 {
        super::splice(ed, 0, 1, &lines);
    } else {
        super::insert_lines(ed, at, &lines);
    }
    at
}

/// Where a refiled tree goes under target heading `t` (None: top level).
fn insertion(ed: &Editor, t: Option<usize>, reversed: bool) -> (usize, usize) {
    let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
    match t {
        Some(h) => {
            let lv = structure::valid_level(odd, syntax::level(&ed.buf.line(h)).unwrap_or(1), 1);
            let at = if reversed {
                syntax::next_heading(&ed.buf, h, usize::MAX).unwrap_or(ed.line_count())
            } else {
                syntax::subtree_end(&ed.buf, h)
            };
            (at, lv)
        }
        None => {
            let at = if reversed {
                (0..ed.line_count())
                    .find(|&l| ctx::at_heading(ed, l))
                    .unwrap_or(ed.line_count())
            } else {
                ed.line_count()
            };
            (at, 1)
        }
    }
}

/// org-notes-order-reversed-p.
fn reversed_order() -> bool {
    super::sexp::option("org-reverse-note-order").is_some_and(|v| v.truthy())
}

/// Find heading text `heading` near `line` (headings move during edits).
fn refind(ed: &Editor, line: usize, heading: &str) -> Option<usize> {
    if line < ed.line_count() && ed.buf.line(line) == heading {
        return Some(line);
    }
    (0..ed.line_count()).find(|&l| ed.buf.line(l) == heading)
}

/// Move (or copy) the subtree at `src` of the current buffer under `t`.
fn refile_to(
    s: &mut Session,
    src: (usize, usize),
    t: Target,
    keep: bool,
    msg: &str,
) -> Result<(), String> {
    let (h, end) = src;
    let tree: String = super::lines(&s.ed, h..end)
        .iter()
        .map(|l| format!("{l}\n"))
        .collect();
    let me = s.ed.path.clone().and_then(|p| std::path::absolute(p).ok());
    let same = me.as_deref() == std::path::absolute(&t.file).ok().as_deref();
    let reversed = reversed_order();
    let log = super::sexp::option("org-log-refile")
        .filter(Sexp::truthy)
        .map(|v| v.sym().map_or("time".to_owned(), str::to_owned));
    if same {
        let ed = &mut s.ed;
        let tl = match t.line {
            Some(l) => Some(refind(ed, l, &t.heading).ok_or("Refile target moved")?),
            None => None,
        };
        if tl.is_some_and(|l| l >= h && l < end) {
            return Err("Cannot refile to position inside the tree or region".into());
        }
        ed.undo.begin(ed.cur.pos());
        let (mut at, level) = insertion(ed, tl, reversed);
        if !keep {
            super::delete_lines(ed, h, end - h);
            if at > h {
                at -= end - h;
            }
        }
        let at = insert_tree(ed, at, level, &tree);
        if let Some(how) = &log {
            refile_note(ed, at, how);
        }
        ed.undo.end(ed.cur.pos());
        ed.set_cursor(h.min(ed.line_count() - 1), 0);
        LAST.with(|l| *l.borrow_mut() = Some((t.file.clone(), at)));
    } else {
        let tfile = t.file.clone();
        let at = s.org_with_file(&tfile, |ted| -> Result<usize, String> {
            let tl = match t.line {
                Some(l) => Some(refind(ted, l, &t.heading).ok_or("Refile target moved")?),
                None => None,
            };
            ted.undo.begin(ted.cur.pos());
            let (at, level) = insertion(ted, tl, reversed);
            let at = insert_tree(ted, at, level, &tree);
            if let Some(how) = &log {
                refile_note(ted, at, how);
            }
            ted.undo.end(ted.cur.pos());
            if super::options::bool("org-auto-align-tags", true) {
                tags::align(ted, at);
            }
            Ok(at)
        })??;
        if !keep {
            let ed = &mut s.ed;
            ed.undo.begin(ed.cur.pos());
            super::delete_lines(ed, h, end - h);
            ed.undo.end(ed.cur.pos());
            ed.set_cursor(h.min(ed.line_count() - 1), 0);
        }
        LAST.with(|l| *l.borrow_mut() = Some((tfile, at)));
    }
    let file = t.file.display().to_string();
    s.ed.set_msg(format!("{msg} to \"{}\" in file {file}: done", t.name));
    Ok(())
}

fn refile_note(ed: &mut Editor, h: usize, how: &str) {
    let how = if how == "note" {
        super::todo::How::Note
    } else {
        super::todo::How::Time
    };
    super::todo::add_log(
        ed,
        h,
        super::todo::Note {
            purpose: "refile".into(),
            state: None,
            prev: None,
            how,
            extra: None,
            time: super::now(),
        },
    );
}

/// org-refile (and org-refile-copy with `keep`).
pub fn refile(ed: &mut Editor, arg: Prefix, keep: bool) -> Result<(), String> {
    if matches!(arg, Prefix::Num(0) | Prefix::U(3)) {
        ed.set_msg("Refile cache cleared");
        return Ok(());
    }
    if arg == Prefix::U(2) {
        return goto_last(ed);
    }
    let keep = keep || arg == Prefix::Num(3) || super::options::bool("org-refile-keep", false);
    let goto = arg == Prefix::U(1);
    let action = if keep && arg == Prefix::Num(3) {
        "Refile (and keep)"
    } else if keep {
        "Copy"
    } else {
        "Refile"
    };
    let st = super::settings(ed);
    let (src, title) = match ed.org_region {
        Some((lo, hi)) if !goto => {
            let text: String = (lo..=hi).map(|l| format!("{}\n", ed.buf.line(l))).collect();
            if !structure::kill_is_subtree(&text) {
                return Err("The region is not a (sequence of) subtree(s)".into());
            }
            ((lo, hi + 1), "region".to_owned())
        }
        _ => {
            let h = fold::back_to_heading(ed, ed.cur.line);
            match (h, goto) {
                (Some(h), _) => {
                    let (_, end) = subtree_text(ed, h);
                    (
                        (h, end),
                        super::links::display_format(&super::links::heading_text(
                            &ed.buf.line(h),
                            &st,
                        )),
                    )
                }
                (None, true) => ((0, 0), String::new()),
                (None, false) => return Err("Before first headline".into()),
            }
        }
    };
    let current = ed.path.clone();
    let prompt = if goto {
        "Goto".to_owned()
    } else if ed.org_region.is_some() {
        format!("{action} region to")
    } else {
        format!("{action} subtree \"{title}\" to")
    };
    let msg = action.to_owned();
    let clock2 = arg == Prefix::Num(2);
    super::effect(ed, move |s| {
        if clock2 {
            match super::clock_target(s) {
                Some(t) => {
                    if let Err(e) = refile_to(s, src, t, keep, &msg) {
                        s.ed.set_err(e);
                    }
                }
                None => s.ed.set_err("No running clock"),
            }
            return;
        }
        let tg = targets(s, current.as_deref());
        if tg.is_empty() {
            return s.ed.set_err("No refile targets");
        }
        let use_path =
            super::sexp::option("org-refile-use-outline-path").is_some_and(|v| v.truthy());
        let names: Vec<String> = tg
            .iter()
            .map(|t| display(t, current.as_deref(), use_path))
            .collect();
        let default = HISTORY.with(|h| h.borrow().first().cloned());
        let p = match &default {
            Some(d) => format!("{prompt} (default {d}): "),
            None => format!("{prompt}: "),
        };
        let mut cands = names.clone();
        if let Some(d) = &default
            && let Some(i) = cands.iter().position(|c| c == d)
        {
            let x = cands.remove(i);
            cands.insert(0, x);
        }
        let allow_create = super::sexp::option("org-refile-allow-creating-parent-nodes")
            .filter(Sexp::truthy)
            .is_some()
            && !goto;
        let require = !allow_create;
        super::complete(&mut s.ed, &p, cands, require, move |ed, choice| {
            let choice = if choice.is_empty() {
                default.unwrap_or_default()
            } else {
                choice
            };
            HISTORY.with(|h| {
                let mut h = h.borrow_mut();
                h.retain(|x| *x != choice);
                h.insert(0, choice.clone());
            });
            let target = names
                .iter()
                .position(|n| {
                    *n == choice || n.trim_end_matches('/') == choice.trim_end_matches('/')
                })
                .map(|i| tg[i].clone());
            let target = match target {
                Some(t) => t,
                None if allow_create => {
                    // A new child under an existing parent path.
                    let Some(slash) = choice.trim_end_matches('/').rfind('/') else {
                        return ed.set_err(format!("Invalid parent node: {choice}"));
                    };
                    let parent = &choice[..slash];
                    let child = choice[slash + 1..].trim_end_matches('/').to_owned();
                    let Some(i) = names.iter().position(|n| n.trim_end_matches('/') == parent)
                    else {
                        return ed.set_err(format!("Invalid parent node: {parent}"));
                    };
                    let p = tg[i].clone();
                    let confirm = super::sexp::option("org-refile-allow-creating-parent-nodes")
                        .and_then(|v| v.sym().map(str::to_owned))
                        == Some("confirm".into());
                    let make = move |ed: &mut Editor| {
                        let pt = p.clone();
                        let child = child.clone();
                        let msg = msg.clone();
                        super::effect(ed, move |s| {
                            let file = pt.file.clone();
                            let r = s.org_with_file(&file, |ted| {
                                let pl = pt.line.and_then(|l| refind(ted, l, &pt.heading));
                                let (at, level) = insertion(ted, pl, false);
                                insert_tree(ted, at, level, &format!("* {child}\n"));
                                (at, ted.buf.line(at))
                            });
                            match r {
                                Ok((at, heading)) => {
                                    let t = Target {
                                        name: child.clone(),
                                        file,
                                        line: Some(at),
                                        heading,
                                    };
                                    if let Err(e) = refile_to(s, src, t, keep, &msg) {
                                        s.ed.set_err(e);
                                    }
                                }
                                Err(e) => s.ed.set_err(e),
                            }
                        });
                    };
                    if confirm {
                        super::yes_or_no(ed, &format!("Create new node \"{choice}\"? "), make);
                    } else {
                        make(ed);
                    }
                    return;
                }
                None => return ed.set_err("No refile target"),
            };
            if goto {
                let file = target.file.clone();
                let line = target.line.unwrap_or(0);
                super::links::mark_ring_push(ed);
                super::effect(ed, move |s| {
                    if let Err(e) = s.org_visit(&file, line) {
                        s.ed.set_err(e);
                    }
                });
                return;
            }
            super::effect(ed, move |s| {
                if let Err(e) = refile_to(s, src, target, keep, &msg) {
                    s.ed.set_err(e);
                }
            });
        });
    });
    Ok(())
}

/// org-refile-goto-last-stored.
fn goto_last(ed: &mut Editor) -> Result<(), String> {
    let Some((f, l)) = LAST.with(|x| x.borrow().clone()) else {
        return Err("No last refile location".into());
    };
    super::links::mark_ring_push(ed);
    super::effect(ed, move |s| {
        if s.org_visit(&f, l).is_ok() {
            s.ed.set_msg("This is the location of the last refile");
        }
    });
    Ok(())
}

// ---- archiving ----

/// org-archive--compute-location: (file, heading).
pub fn archive_location(loc: &str, current: &Path) -> Result<(PathBuf, String), String> {
    let Some(i) = loc.find("::") else {
        return Err(format!("Invalid archive location: {loc:?}"));
    };
    let name = current
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ffmt = &loc[..i];
    let hfmt = &loc[i + 2..];
    let file = if ffmt.trim().is_empty() {
        current.to_path_buf()
    } else {
        let f = super::options::expand(&ffmt.replace("%s", &name));
        if f.is_relative() {
            current.parent().map_or(f.clone(), |d| d.join(&f))
        } else {
            f
        }
    };
    Ok((file, hfmt.replace("%s", &name)))
}

/// org-archive-subtree.
pub fn archive_subtree(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let path = ed.path.clone().ok_or("No file associated to buffer")?;
    let abs = std::path::absolute(&path).unwrap_or(path.clone());
    let st = super::settings(ed);
    let doc = Doc::new(&ed.buf, &st, Some(&abs));
    let loc = doc
        .get(Some(h), "ARCHIVE", Inherit::Yes, false)
        .or_else(|| st.archive.clone())
        .unwrap_or_else(|| super::options::string("org-archive-location", "%s_archive::"));
    let (afile, mut heading) = archive_location(&loc, &abs)?;
    let all_tags = doc.tags(Some(h), false);
    let local = doc.local_tags(h);
    let inherited: Vec<String> = all_tags
        .iter()
        .filter(|t| !local.contains(t))
        .cloned()
        .collect();
    let (y, m, d, hh, mm, wd) = super::localtime(super::now());
    let time = format!(
        "{y:04}-{m:02}-{d:02} {} {hh:02}:{mm:02}",
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wd as usize]
    );
    let home = std::env::var("HOME").unwrap_or_default();
    let file_disp = {
        let s = abs.display().to_string();
        if !home.is_empty() && s.starts_with(&home) {
            format!("~{}", &s[home.len()..])
        } else {
            s
        }
    };
    let todo = doc.get(Some(h), "TODO", Inherit::No, false);
    let category = doc.category(Some(h));
    let olpath = super::refile::outline_path(&ed.buf, &st, h, false).join("/");
    let olid = syntax::parent(&ed.buf, h).and_then(|p| doc.get(Some(p), "ID", Inherit::No, false));
    let closed = doc.get(Some(h), "CLOSED", Inherit::No, false);
    let ctx_items: Vec<String> = super::sexp::option("org-archive-save-context-info")
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.sym().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_else(|| {
            ["time", "file", "olpath", "category", "todo", "itags"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let context = move |k: &str| -> Option<String> {
        match k {
            "category" => Some(category.clone()),
            "file" => Some(file_disp.clone()),
            "itags" => Some(inherited.join(" ")),
            "ltags" => Some(local.join(" ")),
            "olpath" => Some(olpath.clone()),
            "olid" => olid.clone(),
            "time" => Some(time.clone()),
            "todo" => todo.clone(),
            _ => None,
        }
        .filter(|s| !s.trim().is_empty())
    };
    let props: Vec<(String, String)> = ctx_items
        .iter()
        .filter_map(|k| context(k).map(|v| (format!("ARCHIVE_{}", k.to_uppercase()), v)))
        .collect();
    // datetree/ headings.
    let mut datetree = false;
    if let Some(rest) = heading.strip_prefix("datetree/") {
        let nsub = rest.chars().take_while(|&c| c == '*').count();
        let odd = st.opt_bool("org-odd-levels-only", false);
        let base = if odd { 5 } else { 3 } + nsub * if odd { 2 } else { 1 };
        heading = format!("{}{}", "*".repeat(base), &rest[nsub..]);
        datetree = true;
    }
    let (tree, end) = subtree_text(ed, h);
    let infile = afile == abs;
    let add_tags = match super::sexp::option("org-archive-subtree-add-inherited-tags") {
        None => infile,
        Some(Sexp::Sym(s)) if s == "infile" => infile,
        Some(v) => v.truthy(),
    };
    let mark_done = super::sexp::option("org-archive-mark-done").filter(Sexp::truthy);
    let reversed = super::options::bool("org-archive-reversed-order", false);
    let header = super::sexp::option("org-archive-file-header-format").map_or_else(
        || Some("\nArchived entries from file %s\n\n".to_owned()),
        |v| v.str().map(str::to_owned),
    );
    let src_disp = abs.display().to_string();
    let save = super::sexp::option("org-archive-subtree-save-file-p")
        .map_or("from-org".to_owned(), |v| {
            v.sym().map_or("t".to_owned(), str::to_owned)
        });
    let done_kw = mark_done.map(|v| {
        v.str()
            .map(str::to_owned)
            .filter(|k| st.is_done(k))
            .unwrap_or_else(|| {
                st.done_names()
                    .first()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "DONE".into())
            })
    });
    let (cy, cm, cd) = match closed.as_deref() {
        Some(c) if c.len() >= 11 => (
            c[1..5].parse().unwrap_or(y),
            c[6..8].parse().unwrap_or(m),
            c[9..11].parse().unwrap_or(d),
        ),
        _ => (y, m, d),
    };
    super::effect(ed, move |s| {
        let newfile = !afile.exists() && !infile;
        let work = |ted: &mut Editor| -> Result<usize, String> {
            ted.undo.begin(ted.cur.pos());
            if ted.org.is_none() {
                ted.org = Some(Box::default());
            }
            if newfile && let Some(hf) = &header {
                let txt = hf.replace("%s", &src_disp);
                let lines: Vec<String> = txt
                    .strip_suffix('\n')
                    .unwrap_or(&txt)
                    .split('\n')
                    .map(str::to_owned)
                    .collect();
                let n = ted.line_count();
                if n == 1 && ted.buf.len_bytes() == 0 {
                    super::splice(ted, 0, 1, &lines);
                } else {
                    super::insert_lines(ted, n, &lines);
                }
            }
            let (mut lo, mut hi) = (0, ted.line_count());
            if datetree {
                let lv = super::capture::datetree_levels_pub(cy, cm, cd);
                let (day, _) =
                    super::capture::datetree_find_create(ted, &lv, 0, ted.line_count(), 1, None);
                lo = day;
                hi = syntax::subtree_end(&ted.buf, day);
            }
            let hd_level = heading.chars().take_while(|&c| c == '*').count();
            let (at, level) = if !heading.is_empty()
                && hd_level > 0
                && !(datetree && hd_level <= 3 && heading.trim_start_matches('*').trim().is_empty())
            {
                let found = (lo..hi.min(ted.line_count())).find(|&l| {
                    let t = ted.buf.line(l);
                    t == heading
                        || (t.starts_with(&heading)
                            && t[heading.len()..].trim_start().starts_with(':'))
                });
                let hl = match found {
                    Some(l) => l,
                    None => {
                        let mut at = hi.min(ted.line_count());
                        let mut lines = vec![];
                        if !datetree {
                            lines.push(String::new());
                        }
                        lines.push(heading.clone());
                        if at == ted.line_count()
                            && ted.line_count() == 1
                            && ted.buf.len_bytes() == 0
                        {
                            super::splice(ted, 0, 1, &lines);
                            at = 0;
                        } else {
                            super::insert_lines(ted, at, &lines);
                        }
                        at + lines.len() - 1
                    }
                };
                fold::show_subtree(ted, hl);
                let lv = structure::valid_level(
                    ted.org.as_ref().is_some_and(|_| {
                        super::settings(ted).opt_bool("org-odd-levels-only", false)
                    }),
                    hd_level,
                    1,
                );
                let at = if reversed {
                    syntax::next_heading(&ted.buf, hl, usize::MAX).unwrap_or(ted.line_count())
                } else {
                    syntax::subtree_end(&ted.buf, hl)
                };
                (at, lv)
            } else if datetree {
                (syntax::subtree_end(&ted.buf, lo), 4)
            } else if reversed {
                (
                    (0..ted.line_count())
                        .find(|&l| ctx::at_heading(ted, l))
                        .unwrap_or(ted.line_count()),
                    1,
                )
            } else {
                (ted.line_count(), 1)
            };
            let at = insert_tree(ted, at, level, &tree);
            if add_tags && !inherited_empty(&all_tags, &tree) {
                tags::set_tags(ted, at, &all_tags);
            }
            if let Some(kw) = &done_kw {
                let st = super::settings(ted);
                let hl = syntax::headline(&ted.buf.line(at), &st);
                if hl
                    .as_ref()
                    .is_some_and(|x| x.todo.as_deref().is_none_or(|k| !st.is_done(k)))
                {
                    let save = ted.cur;
                    ted.set_cursor(at, 0);
                    let _ = super::todo::todo_to(ted, Some(kw.clone()));
                    ted.cur = save;
                }
            }
            for (k, v) in &props {
                super::props::put(ted, Some(at), k, v)?;
            }
            ted.undo.end(ted.cur.pos());
            Ok(at)
        };
        let r = if infile {
            Ok(work(&mut s.ed))
        } else {
            s.org_with_file(&afile, work)
        };
        match r {
            Ok(Ok(_)) => {
                let ed = &mut s.ed;
                // Remove the subtree from the source (re-found by text when in-file).
                ed.undo.begin(ed.cur.pos());
                let start = if infile {
                    (0..ed.line_count()).find(|&l| {
                        super::lines(ed, l..(l + (end - h)).min(ed.line_count()))
                            .iter()
                            .map(|x| format!("{x}\n"))
                            .collect::<String>()
                            == tree
                    })
                } else {
                    Some(h)
                };
                if let Some(st) = start {
                    super::delete_lines(ed, st, end - h);
                    ed.set_cursor(st.min(ed.line_count() - 1), 0);
                }
                ed.undo.end(ed.cur.pos());
                if !infile
                    && (save == "t" || save == "from-org")
                    && let Ok(i) = s.org_buffer(&afile)
                {
                    let _ = s.org_save(i);
                }
                s.ed.set_msg(format!("Subtree archived in file: {}", afile.display()));
            }
            Ok(Err(e)) | Err(e) => s.ed.set_err(e),
        }
    });
    Ok(())
}

fn inherited_empty(all: &[String], tree: &str) -> bool {
    let first = tree.lines().next().unwrap_or("");
    all.iter().all(|t| first.contains(&format!(":{t}:")))
}

/// org-archive-to-archive-sibling.
pub fn archive_sibling(ed: &mut Editor) -> Result<(), String> {
    let h = fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    let st = super::settings(ed);
    let level = syntax::level(&ed.buf.line(h)).unwrap();
    let leader = format!("{} ", "*".repeat(level));
    let sib = super::options::string("org-archive-sibling-heading", "Archive");
    let tag = "ARCHIVE";
    let (b, e) = match syntax::parent(&ed.buf, h) {
        Some(p) => (p, syntax::subtree_end(&ed.buf, p)),
        None => (0, ed.line_count()),
    };
    ed.undo.begin(ed.cur.pos());
    let found = (b..e).find(|&l| {
        let t = ed.buf.line(l);
        t.starts_with(&leader)
            && syntax::headline(&t, &st)
                .is_some_and(|x| x.title(&t) == sib && x.tags.iter().any(|g| g == tag))
    });
    let mut arch = match found {
        Some(l) => l,
        None => {
            super::insert_lines(ed, e, &[format!("{leader}{sib}")]);
            tags::toggle_tag(ed, e, tag, Some(true));
            e
        }
    };
    let (tree, end) = subtree_text(ed, h);
    let reversed = super::options::bool("org-archive-reversed-order", false);
    let mut at = if reversed {
        syntax::next_heading(&ed.buf, arch, usize::MAX).unwrap_or(ed.line_count())
    } else {
        syntax::subtree_end(&ed.buf, arch)
    };
    super::delete_lines(ed, h, end - h);
    if at > h {
        at -= end - h;
    }
    if arch > h {
        arch -= end - h;
    }
    let odd = st.opt_bool("org-odd-levels-only", false);
    let at = insert_tree(ed, at, structure::valid_level(odd, level, 1), &tree);
    let (y, m, d, hh, mm, wd) = super::localtime(super::now());
    let time = format!(
        "{y:04}-{m:02}-{d:02} {} {hh:02}:{mm:02}",
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wd as usize]
    );
    super::props::put(ed, Some(at), "ARCHIVE_TIME", &time)?;
    fold::fold_subtree(ed, arch, true);
    super::todo::update_parent_statistics(ed, arch);
    ed.undo.end(ed.cur.pos());
    let cur = h.min(ed.line_count() - 1);
    ed.set_cursor(cur, 0);
    if ed.buf.line(cur).trim().is_empty() {
        structure::next_visible_heading(ed, 1);
    }
    Ok(())
}

/// org-toggle-archive-tag.
fn toggle_archive_tag(ed: &mut Editor, find_done: bool) -> Result<(), String> {
    if find_done {
        return archive_all(ed, true);
    }
    let heads: Vec<usize> = match ed.org_region {
        Some((lo, hi)) => (lo..=hi).filter(|&l| ctx::at_heading(ed, l)).collect(),
        None => vec![fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?],
    };
    for h in heads {
        let on = tags::toggle_tag(ed, h, "ARCHIVE", None);
        if on {
            fold::fold_subtree(ed, h, true);
            ed.set_msg("Subtree archived (tag ARCHIVE set)");
        } else {
            ed.set_msg("Subtree unarchived (tag ARCHIVE removed)");
        }
    }
    Ok(())
}

/// org-archive-all-done / org-archive-all-old (tag mode with `tag`).
fn archive_all(ed: &mut Editor, tag: bool) -> Result<(), String> {
    let st = super::settings(ed);
    let l = ed.cur.line;
    let (lo, hi, lvl) = match ctx::at_heading(ed, l).then_some(l) {
        Some(h) => (
            h + 1,
            syntax::subtree_end(&ed.buf, h),
            syntax::level(&ed.buf.line(h)).unwrap() + 1,
        ),
        None => (0, ed.line_count(), 1),
    };
    let candidates: Vec<usize> = (lo..hi)
        .filter(|&i| syntax::level(&ed.buf.line(i)) == Some(lvl))
        .filter(|&i| {
            let end = syntax::subtree_end(&ed.buf, i);
            !(i..end).any(|j| {
                syntax::headline(&ed.buf.line(j), &st)
                    .is_some_and(|x| x.todo.as_deref().is_some_and(|k| !st.is_done(k)))
            })
        })
        .collect();
    if candidates.is_empty() {
        ed.set_msg("No entries to archive");
        return Ok(());
    }
    let first = candidates[0];
    let title = super::links::heading_text(&ed.buf.line(first), &st);
    super::yes_or_no(
        ed,
        &format!(
            "{} \"{title}\"? ",
            if tag { "Set ARCHIVE tag" } else { "Archive" }
        ),
        move |ed| {
            ed.undo.begin(ed.cur.pos());
            if tag {
                tags::toggle_tag(ed, first, "ARCHIVE", Some(true));
                ed.undo.end(ed.cur.pos());
            } else {
                ed.undo.end(ed.cur.pos());
                ed.set_cursor(first, 0);
                if let Err(e) = archive_subtree(ed) {
                    ed.set_err(e);
                }
            }
        },
    );
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-refile" => refile(ed, arg, false),
        "org-refile-copy" => refile(ed, arg, true),
        "org-refile-goto-last-stored" => goto_last(ed),
        "org-refile-cache-clear" => {
            ed.set_msg("Refile cache cleared");
            Ok(())
        }
        "org-refile-reverse" => {
            let saved = super::sexp::option("org-reverse-note-order").is_some_and(|v| v.truthy());
            super::options::put("org-reverse-note-order", toml::Value::Boolean(!saved));
            let r = refile(ed, arg, false);
            super::options::put("org-reverse-note-order", toml::Value::Boolean(saved));
            r
        }
        "org-archive-subtree" => match arg {
            Prefix::U(1) => archive_all(ed, false),
            Prefix::U(2) => archive_all(ed, false),
            _ => {
                if let Some((lo, hi)) = ed.org_region {
                    let first = (lo..=hi).find(|&l| ctx::at_heading(ed, l));
                    if let Some(f) = first {
                        ed.set_cursor(f, 0);
                    }
                }
                archive_subtree(ed)
            }
        },
        "org-archive-subtree-default" => {
            let cmd = super::options::string("org-archive-default-command", "org-archive-subtree");
            super::call(ed, &cmd, arg)
        }
        "org-archive-subtree-default-with-confirmation" => {
            let cmd = super::options::string("org-archive-default-command", "org-archive-subtree");
            let st = super::settings(ed);
            let title = fold::back_to_heading(ed, ed.cur.line)
                .map(|h| super::links::heading_text(&ed.buf.line(h), &st))
                .unwrap_or_default();
            super::yes_or_no(ed, &format!("Archive \"{title}\"? "), move |ed| {
                super::run(ed, &cmd, arg);
            });
            Ok(())
        }
        "org-archive-to-archive-sibling" => archive_sibling(ed),
        "org-toggle-archive-tag" => toggle_archive_tag(ed, !arg.is_none()),
        "org-archive-set-tag" => {
            let h = fold::back_to_heading(ed, ed.cur.line);
            match h {
                Some(h) => {
                    tags::toggle_tag(ed, h, "ARCHIVE", Some(true));
                    Ok(())
                }
                None => Err("Before first headline".into()),
            }
        }
        "org-archive-all-done" => archive_all(ed, false),
        "org-archive-all-old" => archive_all(ed, false),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;
    use super::*;

    #[test]
    fn locations() {
        let (f, h) = archive_location("%s_archive::", Path::new("/d/notes.org")).unwrap();
        assert_eq!(
            (f, h),
            (PathBuf::from("/d/notes.org_archive"), String::new())
        );
        let (f, h) = archive_location("::* Archived Tasks", Path::new("/d/a.org")).unwrap();
        assert_eq!(
            (f, h.as_str()),
            (PathBuf::from("/d/a.org"), "* Archived Tasks")
        );
        assert!(archive_location("bad", Path::new("/a")).is_err());
    }

    #[test]
    fn archive_sibling_moves_under_archive_heading() {
        super::super::set_now(Some(1_780_000_000));
        let e = org("* P\n** A\n** B", "j<C-c><C-x>A");
        let text = e.buf.text();
        assert!(text.starts_with("* P\n** B\n** Archive"), "{text}");
        assert!(
            text.contains("*** A\n:PROPERTIES:\n:ARCHIVE_TIME:"),
            "{text}"
        );
        super::super::set_now(None);
        let e = org("* A", "<C-c><C-x>a");
        assert!(e.buf.line(0).ends_with(":ARCHIVE:"));
    }

    #[test]
    fn outline_paths() {
        let b: Vec<String> = "* A\n** B\n*** C".lines().map(String::from).collect();
        let st = syntax::settings("".lines(), None);
        assert_eq!(outline_path(&b, &st, 2, true), vec!["A", "B", "C"]);
        assert_eq!(outline_path(&b, &st, 2, false), vec!["A", "B"]);
    }
}
