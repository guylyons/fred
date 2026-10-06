//! Agenda views: the dispatcher (org-agenda), org-agenda-list,
//! org-todo-list, org-tags-view, org-search-view, stuck projects, block
//! agendas (org-agenda-run-series), building and redoing a [`View`].

use super::collect::{self, Ctx, Src};
use super::format;
use super::{
    Bindings, Block, Face, Item, Kind, Line, Loc, Modes, Restriction, Series, Span, Spec, View,
    opt, opt_bool, opt_string, opt_sym, with_bindings,
};
use crate::editor::Editor;
use crate::org::sexp::{self, Sexp};
use crate::org::time::{self, civil::Tm, stamp};
use crate::org::{Prefix, effect};
use crate::session::Session;
use std::path::PathBuf;

// ---- the dispatcher ----

/// A block agenda's commands (type, match, settings) and global settings.
pub type SeriesSpec = (Vec<(String, Sexp, Bindings)>, Bindings);

/// A normalized custom command: (keys, description, type or series, match, lprops).
#[derive(Clone, Debug)]
pub struct Custom {
    pub key: String,
    pub desc: String,
    /// The type symbol, or None for a block agenda (series).
    pub ty: Option<String>,
    pub matcher: Sexp,
    pub lprops: Bindings,
    /// A series: its commands and global settings.
    pub series: Option<SeriesSpec>,
    /// The files org-store-agenda-views writes the view to.
    pub files: Vec<String>,
}

fn file_list(v: Option<&Sexp>) -> Vec<String> {
    match v {
        Some(Sexp::Str(s)) => vec![s.clone()],
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect(),
        None => vec![],
    }
}

/// A new view of custom command `c` (None for the tree commands).
pub fn custom_view(c: &Custom, arg: Prefix) -> Option<View> {
    let (blocks, series) = match &c.series {
        Some((cmds, gprops)) => (
            cmds.iter()
                .filter_map(|(ty, m, lp)| {
                    Some(Block {
                        spec: spec_of(ty, &super::eval(m), arg)?,
                        lprops: lp.clone(),
                    })
                })
                .collect(),
            Some(Series {
                name: c.desc.clone(),
                cmds: cmds.clone(),
                gprops: gprops.clone(),
            }),
        ),
        None => (
            vec![Block {
                spec: spec_of(c.ty.as_deref()?, &super::eval(&c.matcher), arg)?,
                lprops: c.lprops.clone(),
            }],
            None,
        ),
    };
    Some(View {
        name: buffer_name(Some(&c.key), &blocks, series.is_some()),
        blocks,
        series,
        modes: Modes::start(),
        sticky: super::sticky(),
        keys: Some(c.key.clone()),
        width: super::width(),
        last_arg: arg,
        ..View::default()
    })
}

/// org-agenda-custom-commands, normalized, and the prefix descriptions.
pub fn customs() -> (Vec<Custom>, Vec<(String, String)>) {
    let mut out = vec![];
    let mut prefixes = vec![];
    let list = opt("org-agenda-custom-commands").unwrap_or(Sexp::Nil);
    for e in list.list().unwrap_or(&[]) {
        let Some(key) = e.car().and_then(Sexp::str).map(str::to_owned) else {
            continue;
        };
        let rest = e.cdr();
        if let Sexp::Str(d) = &rest {
            prefixes.push((key, d.clone()));
            continue;
        }
        let items: Vec<Sexp> = rest.list().map(<[Sexp]>::to_vec).unwrap_or_default();
        let (desc, items) = match items.first() {
            Some(Sexp::Str(d)) => (d.clone(), items[1..].to_vec()),
            Some(Sexp::Nil) => (String::new(), items[1..].to_vec()),
            _ => (String::new(), items),
        };
        if items.len() == 1
            && let Sexp::Str(d) = &rest
        {
            prefixes.push((key.clone(), d.clone()));
        }
        let first = items.first().cloned().unwrap_or(Sexp::Nil);
        match &first {
            Sexp::Sym(ty) => out.push(Custom {
                key,
                desc,
                ty: Some(ty.clone()),
                matcher: items.get(1).cloned().unwrap_or(Sexp::Nil),
                lprops: super::bindings(items.get(2)),
                series: None,
                files: file_list(items.get(3)),
            }),
            Sexp::List(cmds) => {
                let cmds = cmds
                    .iter()
                    .filter_map(|c| {
                        let l = c.list()?;
                        Some((
                            l.first()?.sym()?.to_owned(),
                            l.get(1).cloned().unwrap_or(Sexp::Nil),
                            super::bindings(l.get(2)),
                        ))
                    })
                    .collect();
                out.push(Custom {
                    key,
                    desc,
                    ty: None,
                    matcher: Sexp::Nil,
                    lprops: vec![],
                    series: Some((cmds, super::bindings(items.get(1)))),
                    files: file_list(items.get(2)),
                });
            }
            _ => {}
        }
    }
    (out, prefixes)
}

/// The restriction chosen in the dispatcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restrict {
    Buffer,
    Subtree,
    Region,
}

const HEADER: [(&str, &str); 16] = [
    ("a", "Agenda for current week or day"),
    ("<", "Buffer, subtree/region restriction"),
    ("/", "Multi-occur"),
    (">", "Remove restriction"),
    ("t", "List of all TODO entries"),
    ("e", "Export agenda views"),
    ("m", "Match a TAGS/PROP/TODO query"),
    ("T", "Entries with special TODO kwd"),
    ("s", "Search for keywords"),
    ("M", "Like m, but only TODO entries"),
    ("?", "Find :FLAGGED: entries"),
    ("S", "Like s, but only TODO entries"),
    ("*", "Toggle sticky agenda views"),
    ("C", "Configure custom agenda commands"),
    ("#", "List stuck projects"),
    ("!", "Configure stuck projects"),
];

fn type_desc(ty: &str) -> &'static str {
    match ty {
        "agenda" => "Agenda for current week or day",
        "agenda*" => "Appointments for current week or day",
        "alltodo" => "List of all TODO entries",
        "search" => "Word search",
        "stuck" => "List of stuck projects",
        "todo" => "TODO keyword",
        "tags" => "Tags query",
        "tags-todo" => "Tags (TODO)",
        "tags-tree" => "Tags tree",
        "todo-tree" => "TODO kwd tree",
        "occur-tree" => "Occur tree",
        _ => "???",
    }
}

/// org-agenda: the dispatcher menu.
pub fn dispatch(ed: &mut Editor, arg: Prefix, restriction: Option<Restrict>) {
    // A dispatcher restriction lasts for one command (unless locked).
    if super::restriction().is_some_and(|r| r.lock.is_none()) {
        super::set_restriction(None);
    }
    let restrict_ok = ed.org.is_some() && ed.path.is_some();
    let (custom, prefixes) = customs();
    let mut entries: Vec<(String, String)> = vec![];
    let status = if super::restriction().is_some_and(|r| r.lock.is_some()) {
        " (restriction lock active)".to_owned()
    } else if restrict_ok {
        match restriction {
            Some(r) => format!(" (restricted to {})", restrict_name(r)),
            None => " (unrestricted)".to_owned(),
        }
    } else {
        String::new()
    };
    for (k, d) in HEADER {
        entries.push((k.to_owned(), d.to_owned()));
    }
    let show_matcher = opt_bool("org-agenda-menu-show-matcher", true);
    for c in &custom {
        let desc = if !c.desc.trim().is_empty() {
            c.desc.clone()
        } else if let Some(t) = &c.ty {
            type_desc(t).to_owned()
        } else {
            "???".to_owned()
        };
        let m = match &c.matcher {
            Sexp::Str(s) if show_matcher && !s.trim().is_empty() => format!(": {s}"),
            _ => match &c.series {
                Some((cmds, _)) if show_matcher => format!(": set of {} commands", cmds.len()),
                _ => String::new(),
            },
        };
        entries.push((c.key.clone(), format!("{desc}{m}")));
    }
    for (p, d) in &prefixes {
        entries.push((String::new(), format!("{p}   {d}")));
    }
    let title = format!("Press key for agenda command{status}:");
    crate::org::menu(ed, &title, entries, move |ed, key| {
        chosen(ed, &key, arg, restriction, restrict_ok)
    });
}

fn restrict_name(r: Restrict) -> &'static str {
    match r {
        Restrict::Buffer => "buffer",
        Restrict::Subtree => "subtree",
        Restrict::Region => "region",
    }
}

fn chosen(ed: &mut Editor, key: &str, arg: Prefix, restriction: Option<Restrict>, ok: bool) {
    let region = crate::org::region(ed).is_some();
    let next = match key {
        "<" | "1" | "0" | ">" if !ok => {
            ed.set_err("Restriction is only possible in Org buffers");
            return;
        }
        "1" => Some(Restrict::Buffer),
        "0" => Some(if region {
            Restrict::Region
        } else {
            Restrict::Subtree
        }),
        "<" => match restriction {
            Some(Restrict::Buffer) => Some(if region {
                Restrict::Region
            } else {
                Restrict::Subtree
            }),
            Some(_) => None,
            None => Some(Restrict::Buffer),
        },
        ">" => None,
        "*" => {
            let v = !super::sticky();
            super::set_sticky(v);
            ed.set_msg(format!(
                "Sticky agenda {}",
                if v { "enabled" } else { "disabled" }
            ));
            return dispatch(ed, arg, restriction);
        }
        _ => {
            if let Err(e) = run_keys(ed, key, arg, restriction) {
                ed.set_err(e);
            }
            return;
        }
    };
    if matches!(key, "<" | "1" | "0" | ">") {
        if super::restriction().is_some_and(|r| r.lock.is_some()) {
            super::set_restriction(None);
        }
        dispatch(ed, arg, next);
    }
}

/// Establish the dispatcher restriction for the next command.
fn establish(ed: &Editor, r: Option<Restrict>) {
    let Some(r) = r else { return };
    if super::restriction().is_some_and(|x| x.lock.is_some()) {
        return;
    }
    let Some(file) = ed.path.clone().and_then(|p| std::path::absolute(p).ok()) else {
        return;
    };
    let range = match r {
        Restrict::Buffer => None,
        Restrict::Region => crate::org::region(ed).map(|(a, b)| (a, b + 1)),
        Restrict::Subtree => crate::org::syntax::heading_at_or_before(&ed.buf, ed.cur.line)
            .map(|h| (h, crate::org::syntax::subtree_end(&ed.buf, h))),
    };
    super::set_restriction(Some(Restriction {
        file,
        range,
        lock: None,
    }));
}

/// Run the agenda command with dispatcher keys `keys`.
pub fn run_keys(
    ed: &mut Editor,
    keys: &str,
    arg: Prefix,
    restriction: Option<Restrict>,
) -> Result<(), String> {
    establish(ed, restriction);
    let (custom, _) = customs();
    if let Some(c) = custom.into_iter().find(|c| c.key == keys) {
        return run_custom(ed, c, arg);
    }
    let block = |spec: Spec| Block {
        spec,
        lprops: vec![],
    };
    let k = keys.to_owned();
    match keys {
        "a" => start(
            ed,
            vec![block(list_spec(arg, None, None, false))],
            None,
            Some(k),
            arg,
        ),
        "t" | "T" => {
            let a = if keys == "T" && arg.is_none() {
                Prefix::U(1)
            } else {
                arg
            };
            todo_prompt(ed, a, move |ed, spec| {
                start(ed, vec![block(spec)], None, Some(k), a)
            })
        }
        "m" | "M" => {
            let todo_only = keys == "M" || !arg.is_none();
            tags_prompt(ed, move |ed, m| {
                start(
                    ed,
                    vec![block(Spec::Tags {
                        todo_only,
                        matcher: m,
                    })],
                    None,
                    Some(k),
                    arg,
                )
            })
        }
        "s" | "S" => {
            let todo_only = keys == "S" || !arg.is_none();
            search_prompt(ed, todo_only, "", move |ed, s| {
                start(
                    ed,
                    vec![block(Spec::Search {
                        todo_only,
                        string: s,
                    })],
                    None,
                    Some(k),
                    arg,
                )
            })
        }
        "?" => start(
            ed,
            vec![block(Spec::Tags {
                todo_only: false,
                matcher: "+FLAGGED".into(),
            })],
            None,
            Some(k),
            arg,
        ),
        "#" => start(ed, vec![block(Spec::Stuck)], None, Some(k), arg),
        "e" => crate::org::call(ed, "org-store-agenda-views", arg),
        "/" => crate::org::call(ed, "org-occur-in-agenda-files", arg),
        "C" => Err(
            "Customize org-agenda-custom-commands in the [org] table of ~/.config/fred/config.toml"
                .into(),
        ),
        "!" => Err(
            "Customize org-stuck-projects in the [org] table of ~/.config/fred/config.toml".into(),
        ),
        _ => Err("Invalid agenda key".into()),
    }
}

fn list_spec(arg: Prefix, start: Option<i64>, span: Option<Span>, with_hour: bool) -> Spec {
    Spec::List {
        arg: match arg {
            Prefix::Num(n) => Some(n),
            _ => None,
        },
        start,
        span,
        with_hour,
    }
}

/// Run a custom command.
fn run_custom(ed: &mut Editor, c: Custom, arg: Prefix) -> Result<(), String> {
    let keys = c.key.clone();
    if let Some((cmds, gprops)) = c.series {
        let blocks: Vec<Block> = cmds
            .iter()
            .filter_map(|(ty, m, lp)| {
                Some(Block {
                    spec: spec_of(ty, &super::eval(m), arg)?,
                    lprops: lp.clone(),
                })
            })
            .collect();
        let series = Series {
            name: c.desc.clone(),
            cmds,
            gprops,
        };
        return start(ed, blocks, Some(series), Some(keys), arg);
    }
    let ty = c.ty.unwrap_or_default();
    let m = super::eval(&c.matcher);
    let ms = m.str().unwrap_or("").to_owned();
    match ty.as_str() {
        "tags-tree" => with_bindings(&c.lprops, || {
            crate::org::call_with(ed, "org-match-sparse-tree", arg, ms)
        }),
        "todo-tree" => with_bindings(&c.lprops, || {
            let re = regex::Regex::new(&format!(
                r"(?m)^\*+ [ \t]*{}(?:[\t ]|$)",
                regex::escape(&ms)
            ))
            .map_err(|e| e.to_string())?;
            let n = crate::org::time::occur(ed, &re, |_, _, _| true);
            ed.set_msg(format!("{n} match(es) for regexp {ms}"));
            Ok(())
        }),
        "occur-tree" => with_bindings(&c.lprops, || {
            let re = crate::org::re::compile(&ms, false)?;
            let n = crate::org::time::occur(ed, &re, |_, _, _| true);
            ed.set_msg(format!("{n} match(es) for regexp {ms}"));
            Ok(())
        }),
        _ => match spec_of(&ty, &m, arg) {
            Some(spec) => start(
                ed,
                vec![Block {
                    spec,
                    lprops: c.lprops,
                }],
                None,
                Some(keys),
                arg,
            ),
            None => with_bindings(&c.lprops, || crate::org::call_with(ed, &ty, arg, ms)),
        },
    }
}

/// The block spec of a custom command type with match `m`.
fn spec_of(ty: &str, m: &Sexp, arg: Prefix) -> Option<Spec> {
    let ms = m.str().map(str::to_owned);
    Some(match ty {
        "agenda" => list_spec(arg, None, None, false),
        "agenda*" => list_spec(arg, None, None, true),
        "alltodo" => Spec::Todo {
            keywords: None,
            n: None,
        },
        "search" => Spec::Search {
            todo_only: !arg.is_none(),
            string: ms.unwrap_or_default(),
        },
        "stuck" => Spec::Stuck,
        "tags" => Spec::Tags {
            todo_only: !arg.is_none(),
            matcher: ms.unwrap_or_default(),
        },
        "tags-todo" => Spec::Tags {
            todo_only: true,
            matcher: ms.unwrap_or_default(),
        },
        "todo" => Spec::Todo {
            keywords: ms,
            n: None,
        },
        _ => return None,
    })
}

/// org-todo-list's argument: C-u prompts for keywords, a number picks one.
fn todo_prompt(
    ed: &mut Editor,
    arg: Prefix,
    then: impl FnOnce(&mut Editor, Spec) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    match arg {
        Prefix::U(_) => {
            let kws = agenda_keywords_now();
            crate::org::complete(
                ed,
                "Keyword (or KWD1|KWD2|...): ",
                kws,
                false,
                move |ed, a| {
                    let a = a.trim().to_owned();
                    let spec = Spec::Todo {
                        keywords: (!a.is_empty()).then_some(a),
                        n: None,
                    };
                    if let Err(e) = then(ed, spec) {
                        ed.set_err(e);
                    }
                },
            );
            Ok(())
        }
        Prefix::Num(n) => then(
            ed,
            Spec::Todo {
                keywords: None,
                n: Some(n),
            },
        ),
        _ => then(
            ed,
            Spec::Todo {
                keywords: None,
                n: None,
            },
        ),
    }
}

/// org-todo-keywords-for-agenda from the agenda files on disk (for prompts).
fn agenda_keywords_now() -> Vec<String> {
    let mut kws: Vec<String> = vec![];
    for f in super::files(false, false) {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let st = crate::org::settings_of(&text, f.parent());
        for k in st.todo_names() {
            if !kws.iter().any(|x| x == k) {
                kws.insert(0, k.to_owned());
            }
        }
    }
    kws.reverse();
    kws
}

fn tags_prompt(
    ed: &mut Editor,
    then: impl FnOnce(&mut Editor, String) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let mut tags: Vec<String> = vec![];
    for f in super::files(false, false) {
        if let Ok(text) = std::fs::read_to_string(&f) {
            let st = crate::org::settings_of(&text, f.parent());
            for t in st.tag_names().into_iter().chain(st.file_tags.clone()) {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
            for l in text.lines() {
                if let Some(h) = crate::org::syntax::headline(l, &st) {
                    for t in h.tags {
                        if !tags.contains(&t) {
                            tags.push(t);
                        }
                    }
                }
            }
        }
    }
    crate::org::complete(ed, "Match: ", tags, false, move |ed, m| {
        if let Err(e) = then(ed, m) {
            ed.set_err(e);
        }
    });
    Ok(())
}

fn search_prompt(
    ed: &mut Editor,
    _todo_only: bool,
    initial: &str,
    then: impl FnOnce(&mut Editor, String) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let prompt = if opt_bool("org-agenda-search-view-always-boolean", false) {
        "[+-]Word/{Regexp} ...: "
    } else {
        "Phrase or [+-]Word/{Regexp} ...: "
    };
    crate::org::read(ed, prompt, initial, move |ed, s| {
        if let Err(e) = then(ed, s) {
            ed.set_err(e);
        }
    });
    Ok(())
}

/// The buffer name of a new view (org-agenda-buffer-name, sticky names).
fn buffer_name(keys: Option<&str>, blocks: &[Block], series: bool) -> String {
    if let Some(n) = opt("org-agenda-buffer-name").and_then(|v| v.str().map(str::to_owned))
        && !super::sticky()
    {
        return n;
    }
    if !super::sticky() {
        return "*Org Agenda*".into();
    }
    let m = if series {
        None
    } else {
        blocks.first().and_then(|b| match &b.spec {
            Spec::Tags { matcher, .. } => Some(matcher.clone()),
            Spec::Search { string, .. } => Some(string.clone()),
            Spec::Todo {
                keywords: Some(k), ..
            } => Some(k.clone()),
            _ => None,
        })
    };
    let k = keys.map(str::to_owned).unwrap_or_else(|| {
        match blocks.first().map(|b| &b.spec) {
            Some(Spec::Todo { .. }) => "t",
            Some(Spec::Tags {
                todo_only: true, ..
            }) => "M",
            Some(Spec::Tags { .. }) => "m",
            Some(Spec::Search {
                todo_only: true, ..
            }) => "S",
            Some(Spec::Search { .. }) => "s",
            _ => "a",
        }
        .to_owned()
    });
    match m {
        Some(m) => format!("*Org Agenda({k}:{m})*"),
        None => format!("*Org Agenda({k})*"),
    }
}

/// Build and show a new view of `blocks`.
pub fn start(
    ed: &mut Editor,
    blocks: Vec<Block>,
    series: Option<Series>,
    keys: Option<String>,
    arg: Prefix,
) -> Result<(), String> {
    let name = buffer_name(keys.as_deref(), &blocks, series.is_some());
    let sticky = super::sticky();
    // org-agenda-append-agenda: add to the view of this buffer.
    if ed.org_view.as_ref().is_some_and(|v| v.appending) {
        let mut v = super::view_mut(ed)?.clone();
        v.appending = false;
        for b in blocks {
            v.blocks.push(b);
        }
        v.series = None;
        effect(ed, move |s| {
            let v = build(s, v);
            show(s, v, true);
        });
        return Ok(());
    }
    let v = View {
        name,
        blocks,
        series,
        modes: Modes::start(),
        sticky,
        keys,
        width: super::width(),
        last_arg: arg,
        ..View::default()
    };
    effect(ed, move |s| {
        // Sticky: show the existing buffer (org-agenda-use-sticky-p).
        if sticky
            && s.org_agenda_find(&v.name).is_some_and(|i| {
                s.org_with_buffer(i, |e| {
                    e.org_view.as_ref().is_some_and(|x| x.last_arg == arg)
                })
            })
        {
            let i = s.org_agenda_find(&v.name).unwrap();
            s.org_show(i);
            s.ed.set_msg("Sticky Agenda buffer, use `r' to refresh");
            return;
        }
        let v = build(s, v);
        show(s, v, false);
    });
    Ok(())
}

/// Show `v` in its agenda buffer (`keep`: keep the cursor's line).
pub fn show(s: &mut Session, mut v: View, keep: bool) {
    super::filter::apply(&mut v);
    s.org_agenda_show(v, keep);
}

// ---- building ----

/// The sources of a build: every agenda file's text.
fn sources(s: &mut Session, archives: Option<&'static str>) -> (Vec<Src>, Vec<String>) {
    let mut out = vec![];
    let mut errs = vec![];
    let r = super::restriction();
    let mut files = super::files(false, archives == Some("t"));
    if archives == Some("t") && r.is_none() {
        let extra: Vec<PathBuf> = files.iter().filter_map(|f| archive_file(f)).collect();
        for e in extra {
            if !files.contains(&e) {
                files.push(e);
            }
        }
    }
    for f in files {
        match s.org_text(&f) {
            Ok(text) => {
                let range = r
                    .as_ref()
                    .filter(|r| same(&r.file, &f))
                    .and_then(|r| r.range);
                out.push(Src::new(f, &text, range));
            }
            Err(_) => {
                if !opt_bool("org-agenda-skip-unavailable-files", false) {
                    errs.push(format!("Non-existent agenda file {}", f.display()));
                }
            }
        }
    }
    (out, errs)
}

fn same(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(x, y)| x == y)
}

/// The archive file of an agenda file (org-archive-location's file part).
pub fn archive_file(f: &std::path::Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(f).ok()?;
    let st = crate::org::settings_of(&text, f.parent());
    let loc = st
        .archive
        .clone()
        .unwrap_or_else(|| opt_string("org-archive-location", "%s_archive::"));
    let file_part = loc.split("::").next().unwrap_or("");
    if file_part.is_empty() {
        return None;
    }
    let name = f.file_name()?.to_string_lossy().into_owned();
    let p = file_part.replace("%s", &f.display().to_string());
    let p = if p.contains('/') {
        PathBuf::from(p)
    } else {
        f.with_file_name(file_part.replace("%s", &name))
    };
    p.is_file().then_some(p)
}

/// Rebuild every block of `v` from the sources.
pub fn build(s: &mut Session, mut v: View) -> View {
    let (srcs, errs) = sources(s, v.modes.archives);
    if let Some(e) = errs.first() {
        s.ed.set_err(e.clone());
    }
    v.lines.clear();
    v.width = super::width();
    let mut kws: Vec<String> = vec![];
    for src in &srcs {
        for k in src.st.todo_names() {
            if !kws.iter().any(|x| x == k) {
                kws.insert(0, k.to_owned());
            }
        }
    }
    v.todo_kwds = kws;
    v.tag_alist = tag_alist(&srcs);
    let gprops = v
        .series
        .as_ref()
        .map(|x| x.gprops.clone())
        .unwrap_or_default();
    let multi = v.blocks.len() > 1 || v.series.is_some();
    let mut goto = None;
    let blocks = v.blocks.clone();
    for (bi, b) in blocks.iter().enumerate() {
        let mut props = gprops.clone();
        props.extend(b.lprops.iter().cloned());
        let (mut lines, g) = with_bindings(&props, || block(s, &srcs, &v, &b.spec, multi));
        if bi > 0 && !opt_bool("org-agenda-compact-blocks", false) {
            let sep = match opt("org-agenda-block-separator") {
                Some(Sexp::Str(s)) => Some(s),
                Some(Sexp::Nil) => None,
                Some(Sexp::Int(c)) => Some(
                    char::from_u32(c as u32)
                        .unwrap_or('─')
                        .to_string()
                        .repeat(v.width),
                ),
                _ => Some("─".repeat(v.width)),
            };
            if let Some(sep) = sep {
                // Upstream inserts "\n" SEPARATOR "\n" after the block's final newline.
                v.lines.push(Line {
                    block: bi,
                    ..Line::default()
                });
                v.lines.push(Line {
                    text: sep,
                    face: Face::StructureSecondary,
                    block: bi,
                    ..Line::default()
                });
            }
        }
        if let Some(g) = g
            && goto.is_none()
        {
            goto = Some(v.lines.len() + g);
        }
        for l in &mut lines {
            l.block = bi;
        }
        v.lines.extend(lines);
    }
    if v.series.is_some() && !v.lines.is_empty() {
        goto = Some(0);
    }
    v.goto = goto;
    finalize(&mut v, &srcs);
    v.mode_name = mode_name(&v);
    v
}

/// org-tag-alist-for-agenda: tags and their fast keys.
fn tag_alist(srcs: &[Src]) -> Vec<(String, Option<char>)> {
    let mut out: Vec<(String, Option<char>)> = vec![];
    for s in srcs {
        for t in &s.st.tags {
            if let crate::org::syntax::TagEntry::Tag(n, k) = t
                && !out.iter().any(|(x, _)| x == n)
            {
                out.push((n.clone(), *k));
            }
        }
    }
    out
}

/// One block's lines; the line to put the cursor on.
fn block(
    s: &mut Session,
    srcs: &[Src],
    v: &View,
    spec: &Spec,
    multi: bool,
) -> (Vec<Line>, Option<usize>) {
    match spec {
        Spec::List {
            arg,
            start,
            span,
            with_hour,
        } => agenda_list(s, srcs, v, *arg, *start, *span, *with_hour),
        Spec::Todo { keywords, n } => (todo_list(srcs, v, keywords.clone(), *n, multi), None),
        Spec::Tags { todo_only, matcher } => {
            (tags_view(srcs, v, *todo_only, matcher, multi, None), None)
        }
        Spec::Search { todo_only, string } => {
            (search_view(srcs, v, *todo_only, string, multi), None)
        }
        Spec::Stuck => (stuck(srcs, v, multi), None),
    }
}

fn header_line(text: &str, face: Face) -> Line {
    Line {
        text: text.to_owned(),
        face,
        structural: true,
        ..Line::default()
    }
}

/// org-agenda--insert-overriding-header: Some(lines) when overridden.
fn overriding_header() -> Option<Vec<Line>> {
    match opt("org-agenda-overriding-header") {
        None | Some(Sexp::Nil) => None,
        Some(Sexp::Str(s)) if s.is_empty() => Some(vec![]),
        Some(v) => {
            let s = v.str().map_or_else(|| v.to_string(), str::to_owned);
            Some(
                s.trim_end_matches('\n')
                    .split('\n')
                    .map(|l| header_line(l, Face::Structure))
                    .collect(),
            )
        }
    }
}

fn item_line(it: Item, day: Option<i64>) -> Line {
    Line {
        text: it.line.clone(),
        face: it.face,
        day,
        item: Some(it),
        ..Line::default()
    }
}

/// The entry types of the agenda list (org-agenda-entry-types and modes).
fn entry_types(with_hour: bool, show_log: Option<&Sexp>) -> Vec<String> {
    let mut types: Vec<String> = match opt("org-agenda-entry-types") {
        Some(v) => v
            .list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect(),
        None => [":deadline", ":scheduled", ":timestamp", ":sexp"]
            .map(String::from)
            .to_vec(),
    };
    if types.iter().any(|t| t == ":deadline*") {
        types.retain(|t| t != ":deadline");
    }
    if types.iter().any(|t| t == ":scheduled*") {
        types.retain(|t| t != ":scheduled");
    }
    if with_hour {
        for (a, b) in [(":deadline", ":deadline*"), (":scheduled", ":scheduled*")] {
            if let Some(i) = types.iter().position(|t| t == a) {
                types.remove(i);
                types.insert(0, b.into());
            }
        }
    }
    if !opt_bool("org-agenda-include-deadlines", true) {
        types.retain(|t| t != ":deadline" && t != ":deadline*");
    }
    match show_log {
        Some(v) if matches!(v.sym(), Some("only" | "clockcheck")) => vec![":closed".into()],
        Some(v) if v.truthy() => {
            let mut t = vec![":closed".to_owned()];
            t.extend(types);
            t
        }
        _ => types,
    }
}

/// org-agenda-start-day as an absolute day.
fn start_day_option() -> Option<i64> {
    match opt("org-agenda-start-day")? {
        Sexp::Str(s) if !s.is_empty() => {
            let a = time::read::analyze(&s, time::current_effective_time());
            Some(a.tm.absolute())
        }
        Sexp::Int(n) => Some(n),
        _ => None,
    }
}

/// The default span (org-agenda-span).
pub fn default_span() -> Span {
    opt("org-agenda-span")
        .and_then(|v| Span::from_sexp(&v))
        .unwrap_or(Span::Week)
}

fn start_weekday() -> Option<i64> {
    match opt("org-agenda-start-on-weekday") {
        None => Some(1),
        Some(Sexp::Int(n)) => Some(n),
        _ => None,
    }
}

/// org-agenda-list.
#[allow(clippy::too_many_arguments)]
fn agenda_list(
    s: &mut Session,
    srcs: &[Src],
    v: &View,
    arg: Option<i64>,
    start: Option<i64>,
    span: Option<Span>,
    with_hour: bool,
) -> (Vec<Line>, Option<usize>) {
    let span = match arg {
        Some(n) if n > 0 => Some(Span::from_days(n)),
        _ => span,
    }
    .unwrap_or_else(default_span);
    let today = time::today();
    let sd = start.or_else(start_day_option).unwrap_or(today);
    let ndays = span.ndays(sd);
    let wd = if ndays == 7 || ndays == 14 {
        start_weekday()
    } else {
        None
    };
    let first = match wd {
        Some(n1) if ndays >= 7 => {
            let nt = Tm::from_absolute(sd).weekday();
            let d = nt - n1;
            sd - (if d < 0 { 7 } else { 0 } + d)
        }
        _ => sd,
    };
    let mut lines = vec![];
    let mut ctx = Ctx::new("agenda", today);
    ctx.archives = v.modes.archives.is_some();
    ctx.include_inactive |= v.modes.inactive;
    let show_log = v.modes.show_log.clone();
    if let Some(l) = &show_log {
        ctx.log_items = match l {
            Sexp::List(items) => items
                .iter()
                .filter_map(|x| x.str().map(str::to_owned))
                .collect(),
            v if v.sym() == Some("clockcheck") => vec!["clock".into()],
            _ => opt("org-agenda-log-mode-items")
                .and_then(|v| {
                    v.list().map(|l| {
                        l.iter()
                            .filter_map(|x| x.str().map(str::to_owned))
                            .collect()
                    })
                })
                .unwrap_or_else(|| vec!["closed".into(), "clock".into()]),
        };
    }
    let types = entry_types(with_hour, show_log.as_ref());
    let last = first + ndays - 1;
    if !opt_bool("org-agenda-compact-blocks", false) {
        match overriding_header() {
            Some(h) => lines.extend(h),
            None => {
                let (w1, w2) = (
                    stamp::days_to_iso_week(first),
                    stamp::days_to_iso_week(last),
                );
                let wk = if last - first >= 350 {
                    String::new()
                } else if w1 == w2 {
                    format!(" (W{w1:02})")
                } else {
                    format!(" (W{w1:02}-W{w2:02})")
                };
                let mut l = header_line(&format!("{}-agenda{wk}:", span.name()), Face::Structure);
                l.date_header = false;
                lines.push(l);
            }
        }
    }
    let mut goto = None;
    let mut day_cnt = 0;
    let include_diary = opt_bool("org-agenda-include-diary", false);
    for d in first..=last {
        let todayp = d == today;
        if goto.is_none() && (todayp || d == sd) {
            goto = Some(lines.len());
        }
        let mut all: Vec<Item> = vec![];
        for src in srcs {
            all.extend(collect::day_entries(src, &ctx, d, &types));
        }
        if include_diary {
            all.extend(super::diary::entries(&ctx, d));
        }
        if all.is_empty() && !opt_bool("org-agenda-show-all-dates", true) {
            continue;
        }
        day_cnt += 1;
        let face = format::day_face(d, today);
        lines.push(Line {
            text: format::format_date(d),
            face,
            day: Some(d),
            day_cnt: Some(day_cnt),
            date_header: true,
            today: todayp,
            ..Line::default()
        });
        let all = format::add_grid(&ctx.prefix, all, ndays, todayp, &ctx.strategy);
        let done: Vec<String> = all
            .iter()
            .flat_map(|x| x.done_kwds.iter().cloned())
            .collect();
        for it in format::finalize_entries(all, "agenda", &ctx.strategy, &done) {
            let mut l = item_line(it, Some(d));
            l.day_cnt = Some(day_cnt);
            lines.push(l);
        }
    }
    if v.modes.clockreport {
        lines.extend(clock_report(s, first, last + 1));
    }
    if show_log.as_ref().and_then(Sexp::sym) == Some("clockcheck") {
        lines = super::cmds::clocking_issues(lines);
    }
    (lines, goto)
}

/// The clock report of the agenda span (org-agenda-clockreport-mode),
/// computed by the clock module (`org-clock-get-clocktable`).
fn clock_report(s: &mut Session, start: i64, end: i64) -> Vec<Line> {
    let params = opt("org-agenda-clockreport-parameter-plist")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "(:link t :maxlevel 2)".into());
    let p = format!(
        "{} :tstart {start} :tend {end} :scope agenda",
        params.trim_end_matches(')')
    ) + ")";
    let r = crate::org::call_with(&mut s.ed, "org-clock-get-clocktable", Prefix::None, p);
    let mut out = vec![];
    match (r, super::take_result()) {
        (Ok(()), Some(tbl)) => {
            if let Some(h) =
                opt("org-agenda-clock-report-header").and_then(|v| v.str().map(str::to_owned))
            {
                out.push(header_line(h.trim_end_matches('\n'), Face::Structure));
            }
            for l in tbl.lines() {
                out.push(Line {
                    text: l.to_owned(),
                    ..Line::default()
                });
            }
        }
        (Err(e), _) => s.ed.set_err(e),
        _ => {}
    }
    out
}

/// org-agenda-propertize-selected-todo-keywords and the TODO list.
fn todo_list(
    srcs: &[Src],
    v: &View,
    keywords: Option<String>,
    n: Option<i64>,
    multi: bool,
) -> Vec<Line> {
    let today = time::today();
    let kws = &v.todo_kwds;
    let select = keywords.or_else(|| match n {
        Some(n) if n > 0 => kws.get(n as usize - 1).cloned(),
        _ => None,
    });
    let ctx = Ctx::new("todo", today);
    let mut all = vec![];
    for src in srcs {
        all.extend(collect::todos(src, &ctx, select.as_deref()));
    }
    let mut lines = vec![];
    match overriding_header() {
        Some(h) => lines.extend(h),
        None => {
            let mut l = header_line(
                &format!(
                    "Global list of TODO items of type: {}",
                    select.as_deref().unwrap_or("ALL")
                ),
                Face::Structure,
            );
            let at = "Global list of TODO items of type: ".len();
            l.spans.push((at..l.text.len(), Face::StructureFilter));
            lines.push(l);
            if !multi {
                let mut s = "Press ‘N r’ (e.g. ‘0 r’) to search again: (0)[ALL]".to_owned();
                let mut cur = s.chars().count();
                let mut out = vec![];
                for (i, k) in kws.iter().enumerate() {
                    let item = format!("({}){k}", i + 1);
                    if cur + item.chars().count() + 1 > v.width {
                        out.push(std::mem::take(&mut s));
                        s = " ".repeat(21);
                        cur = 21;
                    }
                    s.push(' ');
                    s.push_str(&item);
                    cur += item.chars().count() + 1;
                }
                out.push(s);
                for o in out {
                    lines.push(header_line(&o, Face::StructureSecondary));
                }
            }
        }
    }
    let done: Vec<String> = all.iter().flat_map(|x| x.done_kwds.clone()).collect();
    for it in format::finalize_entries(all, "todo", &ctx.strategy, &done) {
        lines.push(item_line(it, None));
    }
    lines
}

/// org-tags-view (`skip`: org-agenda-list-stuck-projects' skip regexp).
fn tags_view(
    srcs: &[Src],
    v: &View,
    todo_only: bool,
    matcher: &str,
    multi: bool,
    skip: Option<&regex::Regex>,
) -> Vec<Line> {
    let _ = v;
    let today = time::today();
    let ctx = Ctx::new("tags", today);
    let mut all = vec![];
    let now = crate::org::now();
    for src in srcs {
        let m = crate::org::tags::Matcher::new(matcher, &src.st, now);
        let items = collect::tags_match(src, &ctx, &m, todo_only || m.todo_only);
        match skip {
            Some(re) => {
                let mut skip_to = 0;
                for it in items {
                    let h = it.hd.as_ref().map_or(0, |l| l.line);
                    if h < skip_to {
                        continue;
                    }
                    let end = src.subtree_end(h);
                    let body = src.lines[h + 1..end.max(h + 1)].join("\n");
                    let own = src.lines[h].clone();
                    if re.is_match(&format!("{own}\n{body}")) {
                        // Skip the entry (outline-next-heading), not the subtree.
                        skip_to = h + 1;
                        continue;
                    }
                    all.push(it);
                }
            }
            None => all.extend(items),
        }
    }
    let mut lines = vec![];
    match overriding_header() {
        Some(h) => lines.extend(h),
        None => {
            let mut l = header_line(
                &format!("Headlines with TAGS match: {matcher}"),
                Face::Structure,
            );
            let at = "Headlines with TAGS match: ".len();
            l.spans.push((at..l.text.len(), Face::StructureFilter));
            lines.push(l);
            if !multi {
                lines.push(header_line(
                    "Press ‘C-u r’ to search again",
                    Face::StructureSecondary,
                ));
            }
        }
    }
    let done: Vec<String> = all.iter().flat_map(|x| x.done_kwds.clone()).collect();
    for it in format::finalize_entries(all, "tags", &ctx.strategy, &done) {
        lines.push(item_line(it, None));
    }
    lines
}

/// org-search-view.
fn search_view(srcs: &[Src], v: &View, todo_only: bool, string: &str, multi: bool) -> Vec<Line> {
    let _ = v;
    let today = time::today();
    let ctx = Ctx::new("search", today);
    let parsed = collect::parse_search(string, todo_only);
    let mut all = vec![];
    for src in srcs {
        all.extend(collect::search(src, &ctx, &parsed));
    }
    let mut extra: Vec<Src> = vec![];
    if super::restriction().is_none() {
        let files = opt("org-agenda-text-search-extra-files")
            .map(|v| {
                v.list()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|x| x.str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for f in files {
            let paths: Vec<PathBuf> = if f == "agenda-archives" {
                super::files(false, false)
                    .iter()
                    .filter_map(|p| archive_file(p))
                    .collect()
            } else {
                vec![crate::org::options::expand(&f)]
            };
            for p in paths {
                if srcs.iter().any(|s| same(&s.path, &p)) {
                    continue;
                }
                if let Ok(t) = std::fs::read_to_string(&p) {
                    extra.push(Src::new(p, &t, None));
                }
            }
        }
    }
    for src in &extra {
        all.extend(collect::search(src, &ctx, &parsed));
    }
    let mut lines = vec![];
    match overriding_header() {
        Some(h) => lines.extend(h),
        None => {
            let mut l = header_line(&format!("Search words: {string}"), Face::Structure);
            l.spans
                .push(("Search words: ".len()..l.text.len(), Face::StructureFilter));
            lines.push(l);
            if !multi {
                lines.push(header_line(
                    "Press ‘[’, ‘]’ to add/sub word, ‘{’, ‘}’ to add/sub regexp, ‘C-u r’ for a fresh search",
                    Face::StructureSecondary,
                ));
            }
        }
    }
    let done: Vec<String> = all.iter().flat_map(|x| x.done_kwds.clone()).collect();
    for it in format::finalize_entries(all, "search", &ctx.strategy, &done) {
        lines.push(item_line(it, None));
    }
    lines
}

/// org-agenda-list-stuck-projects.
fn stuck(srcs: &[Src], v: &View, multi: bool) -> Vec<Line> {
    let spec = opt("org-stuck-projects").unwrap_or_else(|| {
        sexp::read(r#"("+LEVEL=2/-DONE" ("TODO" "NEXT" "NEXTACTION") nil "")"#).unwrap()
    });
    let l = spec.list().unwrap_or(&[]);
    let matcher = l.first().and_then(Sexp::str).unwrap_or("").to_owned();
    let strs = |x: Option<&Sexp>| -> Vec<String> {
        x.and_then(Sexp::list)
            .unwrap_or(&[])
            .iter()
            .filter_map(|y| y.str().map(str::to_owned))
            .collect()
    };
    let todo = strs(l.get(1));
    let tags = strs(l.get(2));
    let gen_re = l
        .get(3)
        .and_then(Sexp::str)
        .filter(|s| !s.trim().is_empty());
    let todo_wds: Vec<String> = if todo.iter().any(|t| t == "*") {
        let mut v: Vec<String> = vec![];
        for s in srcs {
            for k in s.st.not_done_names() {
                if !v.iter().any(|x| x == k) {
                    v.push(k.to_owned());
                }
            }
        }
        v
    } else {
        todo
    };
    let mut parts: Vec<String> = vec![];
    if !todo_wds.is_empty() {
        parts.push(format!(
            r"(?m)^\*+[ \t]+({})(?:[ \t]|$)",
            todo_wds
                .iter()
                .map(|w| regex::escape(w))
                .collect::<Vec<_>>()
                .join("|")
        ));
    }
    if tags.iter().any(|t| t == "*") {
        parts.push(r"(?m)^\*+ .*[ \t]:[\w@#%:]+:[ \t]*$".into());
    } else if !tags.is_empty() {
        parts.push(format!(
            r"(?m)^\*+ .*?[ \t]:(?:[\w@#%]+:)*({}):(?:[\w@#%]+:)*[ \t]*$",
            tags.iter()
                .map(|w| regex::escape(w))
                .collect::<Vec<_>>()
                .join("|")
        ));
    }
    if let Some(g) = gen_re {
        parts.push(crate::org::re::translate(g));
    }
    let re = regex::Regex::new(&parts.join("|")).ok();
    let header_set = opt("org-agenda-overriding-header").is_some_and(|x| x.truthy());
    let mut b: Bindings = vec![];
    if !header_set {
        b.push((
            "org-agenda-overriding-header".into(),
            Sexp::Str("List of stuck projects: ".into()),
        ));
    }
    with_bindings(&b, || match &re {
        Some(re) if !parts.is_empty() => tags_view(srcs, v, false, &matcher, multi, Some(re)),
        _ => vec![header_line(
            "Missing information to identify unstuck projects",
            Face::Warning,
        )],
    })
}

// ---- finalize ----

/// org-agenda-finalize: align tags, habits, entry text, mark the clock.
fn finalize(v: &mut View, srcs: &[Src]) {
    let remove = opt_sym("org-agenda-remove-tags", "nil");
    let today = time::today();
    for l in &mut v.lines {
        let Some(it) = &l.item else { continue };
        if remove != "t" {
            l.text = format::align_tags(&l.text, v.width);
        }
        if let Some(h) = &it.habit {
            let (t, spans) = super::habit::insert_graph(&l.text, h, today);
            l.text = t;
            l.spans.extend(spans);
        }
        if it.clocking {
            l.face = Face::Clocking;
        }
    }
    if let Some(n) = v.modes.entry_text {
        super::cmds::entry_text_show(v, srcs, n);
    }
}

/// org-agenda-set-mode-name.
pub fn mode_name(v: &View) -> String {
    let mut s = String::from("Org-Agenda");
    if super::restriction().is_some() {
        s.push_str(" []");
    }
    s.push(' ');
    let span = v.blocks.iter().find_map(|b| match &b.spec {
        Spec::List { span, arg, .. } => Some(
            arg.filter(|&n| n > 0)
                .map(Span::from_days)
                .or(*span)
                .unwrap_or_else(default_span),
        ),
        _ => None,
    });
    if let Some(sp) = span {
        s.push_str(&sp.name());
    }
    let m = &v.modes;
    for (on, t) in [
        (m.follow, " Follow"),
        (m.entry_text.is_some(), " ETxt"),
        (opt_bool("org-agenda-include-diary", false), " Diary"),
        (opt_bool("org-agenda-include-deadlines", true), " Ddl"),
        (opt_bool("org-agenda-use-time-grid", true), " Grid"),
        (
            super::habit::loaded() && opt_bool("org-habit-show-habits", true),
            " Habit",
        ),
    ] {
        if on {
            s.push_str(t);
        }
    }
    match &m.show_log {
        Some(Sexp::List(_)) => s.push_str(" LogAll"),
        Some(x) if x.sym() == Some("clockcheck") => s.push_str(" ClkCk"),
        Some(x) if x.truthy() => s.push_str(" Log"),
        _ => {}
    }
    let f = &v.filters;
    let p = &v.presets;
    if f.any() || p.any() {
        s.push(' ');
    }
    let cats: Vec<String> = p.category.iter().chain(&f.category).cloned().collect();
    if !cats.is_empty() {
        s.push_str(&format!("[{}]", cats.concat()));
    }
    s.push_str(&p.tag.iter().chain(&f.tag).cloned().collect::<String>());
    s.push_str(
        &p.effort
            .iter()
            .chain(&f.effort)
            .cloned()
            .collect::<String>(),
    );
    for r in p.regexp.iter().chain(&f.regexp) {
        s.push_str(&format!("{}/{}/", &r[..1], &r[1..]));
    }
    match m.archives {
        Some("t") => s.push_str(" Archives"),
        Some(_) => s.push_str(&format!(" :{}:", opt_string("org-archive-tag", "ARCHIVE"))),
        None => {}
    }
    if m.clockreport {
        s.push_str(" Clock");
    }
    s
}

// ---- redo ----

/// org-agenda-check-type.
pub fn check_type(ed: &Editor, types: &[Kind]) -> Result<(), String> {
    let v = super::view(ed)?;
    let i = super::cur_line(ed).unwrap_or(0);
    let k = v
        .kind_at(i)
        .ok_or_else(|| "No Org agenda currently displayed".to_owned())?;
    if types.contains(&k) {
        Ok(())
    } else {
        Err(format!(
            "Not allowed in `{}'-type agenda buffer or component",
            k.name()
        ))
    }
}

/// org-agenda-redo: rebuild the view, keeping the cursor line.
pub fn redo(ed: &mut Editor, _all: bool) {
    let Some(v) = ed.org_view.as_deref().cloned() else {
        return;
    };
    ed.set_msg("Rebuilding agenda buffer...");
    effect(ed, move |s| {
        let mut v = build(s, v);
        v.goto = None;
        show(s, v, true);
        if !s.ed.msg.as_ref().is_some_and(|m| m.1) {
            s.ed.set_msg("Rebuilding agenda buffer...done");
        }
    });
}

/// Redo with the block at the cursor changed by `f` (org-agenda-later,
/// span changes), then go to `cnt` (org-agenda-find-same-or-today-or-agenda).
pub fn redo_block(
    ed: &mut Editor,
    f: impl FnOnce(&mut Spec),
    day_cnt: Option<usize>,
) -> Result<(), String> {
    let i = super::cur_line(ed).unwrap_or(0);
    let v = super::view_mut(ed)?;
    let b = v.lines.get(i).map_or(0, |l| l.block);
    let b = if v
        .blocks
        .get(b)
        .is_some_and(|x| matches!(x.spec, Spec::List { .. }))
    {
        b
    } else {
        v.blocks
            .iter()
            .position(|x| matches!(x.spec, Spec::List { .. }))
            .ok_or("No Org agenda currently displayed")?
    };
    f(&mut v.blocks[b].spec);
    let v = v.clone();
    effect(ed, move |s| {
        let mut v = build(s, v);
        v.goto = day_cnt
            .and_then(|c| {
                v.lines
                    .iter()
                    .position(|l| l.date_header && l.day_cnt == Some(c))
            })
            .or_else(|| v.lines.iter().position(|l| l.today))
            .or(v.goto);
        show(s, v, false);
    });
    Ok(())
}

/// Read a Loc's current line in its buffer text (for remote edits).
pub fn find_line(s: &Session, at: &Loc) -> Option<(usize, Vec<String>)> {
    let text = s.org_text(&at.path).ok()?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    at.find(&lines).map(|l| (l, lines))
}

/// org-agenda-compute-starting-span.
pub fn starting_span(sd: i64, span: Span, n: Option<i64>) -> i64 {
    let t = Tm::from_absolute(sd);
    match span {
        Span::Day => match n {
            Some(n) => Tm::date(t.year, t.month, 1).absolute() + n - 1,
            None => sd,
        },
        Span::Week | Span::Fortnight => {
            let nt = t.weekday();
            let d = start_weekday().map_or(0, |w| nt - w);
            let mut sd = sd - (if d < 0 { 7 } else { 0 } + d);
            if let Some(mut n) = n {
                let mut y1 = None;
                if n > 99 {
                    y1 = Some(stamp::small_year_to_year(n / 100));
                    n %= 100;
                }
                let year = y1.unwrap_or_else(|| iso_year(sd));
                sd = iso_to_absolute(n, 1, year);
            }
            sd
        }
        Span::Month => {
            let (mut y, mut m) = (t.year, t.month);
            if let Some(mut n) = n {
                if n > 99 {
                    y = stamp::small_year_to_year(n / 100);
                    n %= 100;
                }
                m = n;
            }
            Tm::date(y, m, 1).absolute()
        }
        Span::Year => Tm::date(n.unwrap_or(t.year), 1, 1).absolute(),
        Span::Days(_) => sd,
    }
}

fn iso_year(abs: i64) -> i64 {
    iso_of(abs).0
}

/// calendar-iso-from-absolute: (year, week, weekday 1..7).
fn iso_of(abs: i64) -> (i64, i64, i64) {
    let t = Tm::from_absolute(abs);
    let mut y = t.year;
    if abs >= iso_to_absolute(1, 1, y + 1) {
        y += 1;
    } else if abs < iso_to_absolute(1, 1, y) {
        y -= 1;
    }
    let week = 1 + (abs - iso_to_absolute(1, 1, y)) / 7;
    let wd = t.weekday();
    (y, week, if wd == 0 { 7 } else { wd })
}

/// calendar-iso-to-absolute (week, day 1..7, year).
pub fn iso_to_absolute(week: i64, day: i64, year: i64) -> i64 {
    // The Monday of week 1 is the Monday on or before January 4.
    let jan4 = Tm::date(year, 1, 4).absolute();
    let wd = Tm::from_absolute(jan4).weekday();
    let monday = jan4 - (if wd == 0 { 6 } else { wd - 1 });
    monday + (week - 1) * 7 + (day - 1)
}

/// An agenda for `start` (org-calendar-goto-agenda, org-agenda-goto-date).
pub fn list_at(start: i64, span: Option<Span>) -> Spec {
    Spec::List {
        arg: None,
        start: Some(start),
        span,
        with_hour: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_weeks() {
        // 2026-01-01 is a Thursday: ISO week 1 starts Monday 2025-12-29.
        assert_eq!(
            iso_to_absolute(1, 1, 2026),
            Tm::date(2025, 12, 29).absolute()
        );
        assert_eq!(iso_of(Tm::date(2026, 10, 6).absolute()).1, 41);
    }

    #[test]
    fn starting_spans() {
        let d = Tm::date(2026, 10, 7).absolute(); // Wednesday
        assert_eq!(
            starting_span(d, Span::Week, None),
            Tm::date(2026, 10, 5).absolute()
        );
        assert_eq!(
            starting_span(d, Span::Month, None),
            Tm::date(2026, 10, 1).absolute()
        );
        assert_eq!(
            starting_span(d, Span::Year, None),
            Tm::date(2026, 1, 1).absolute()
        );
        assert_eq!(
            starting_span(d, Span::Day, Some(3)),
            Tm::date(2026, 10, 3).absolute()
        );
    }
}
