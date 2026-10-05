//! Port of upstream Org Babel (ob-*.el): see docs/org-port-notes.md.
//!
//! Babel works on a [`Doc`] (the buffer's lines, file and point): the
//! commands here copy the buffer into one, run the upstream logic, and
//! apply the changed lines back as one undoable splice. Evaluation that
//! needs the user's consent (org-confirm-babel-evaluate) stops with
//! [`Fail::Confirm`]; the command asks and runs again with the answer.
//!
//! API for other modules:
//! - [`execute_maybe`]: org-babel-execute-maybe for org-ctrl-c-ctrl-c
//!   (true when point was on a block or call).
//! - [`export_process`] / [`export_process_text`]: ob-exp's
//!   org-babel-exp-process-buffer for the exporter.
//! - [`sbe`]: org-sbe for table formulas.
//! - [`set_export_string`]: lets the exporter serve ob-org's
//!   `:results latex/html/ascii`.

pub mod core;
pub mod doc;
pub mod exp;
pub mod lang;
pub mod params;
pub mod session;
pub mod sha1;
pub mod tangle;
pub mod value;

#[cfg(test)]
mod tests;

use self::core::{Cx, ExecKind, Fail, R};
use self::doc::{Context, Doc, Kind};
use super::Prefix;
use crate::editor::Editor;
use crate::org::sexp::Sexp;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

// ---- editor glue ----

/// The buffer as a [`Doc`].
pub fn doc_of(ed: &Editor) -> Doc {
    let lines = (0..ed.line_count()).map(|l| ed.buf.line(l)).collect::<Vec<_>>();
    let lines = if lines.len() == 1 && lines[0].is_empty() { vec![] } else { lines };
    let path = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    let mut d = Doc { lines, path, ..Doc::default() };
    d.point = (ed.cur.line, ed.cur.byte);
    d
}

/// Make the buffer DOC's text (one splice over the changed lines).
pub fn apply(ed: &mut Editor, doc: &Doc) {
    let old: Vec<String> = doc_of(ed).lines;
    let new = &doc.lines;
    if old != *new {
        let p = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let max_s = old.len().min(new.len()) - p;
        let s = old.iter().rev().zip(new.iter().rev()).take(max_s).take_while(|(a, b)| a == b).count();
        let with: Vec<String> = new[p..new.len() - s].to_vec();
        let remove = old.len() - p - s;
        if old.is_empty() {
            super::insert_lines(ed, 0, &with);
        } else if new.is_empty() {
            super::splice(ed, 0, old.len(), &[String::new()]);
        } else {
            super::splice(ed, p, remove, &with);
        }
    }
    let l = doc.point.0.min(ed.line_count().saturating_sub(1));
    let col = doc.point.1.min(ed.buf.line(l).len());
    ed.set_cursor(l, col);
}

type Job = Arc<dyn Fn(&mut Doc, &mut Cx) -> R<()> + Send + Sync>;
type Then = Box<dyn FnOnce(&mut Editor) + Send>;

/// Run JOB on the buffer; ask and rerun for confirmations; THEN after.
fn run_job(ed: &mut Editor, approved: HashSet<String>, denied: HashSet<String>, job: Job, then: Option<Then>) {
    let mut doc = doc_of(ed);
    let mut cx = Cx { approved: approved.clone(), denied: denied.clone(), ..Cx::default() };
    match job(&mut doc, &mut cx) {
        Ok(()) => {
            apply(ed, &doc);
            finish(ed, &cx);
            if let Some(t) = then {
                t(ed);
            }
        }
        Err(Fail::Error(e)) => {
            apply(ed, &doc);
            ed.set_err(e);
        }
        Err(Fail::Confirm { question, key }) => {
            let entries = vec![("y".to_owned(), "yes".to_owned()), ("n".to_owned(), "no".to_owned())];
            super::menu(ed, &question, entries, move |ed, k| {
                let (mut a, mut d) = (approved, denied);
                if k == "y" {
                    a.insert(key);
                } else {
                    d.insert(key);
                }
                run_job(ed, a, d, job, then);
            });
        }
    }
}

fn finish(ed: &mut Editor, cx: &Cx) {
    if let Some((name, text)) = &cx.show {
        if let Some(link) = name.strip_prefix("\0open:") {
            open_link(ed, link);
        } else {
            show_buffer(ed, name, text);
        }
    }
    if !cx.error_output.is_empty() {
        ERROR_OUTPUT.with(|e| *e.borrow_mut() = cx.error_output.clone());
    }
    if let Some(m) = &cx.msg {
        if m.starts_with("Babel evaluation exited") {
            ed.set_err(m.clone());
        } else {
            ed.set_msg(m.clone());
        }
    }
}

thread_local! {
    /// *Org-Babel Error Output*.
    static ERROR_OUTPUT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static EXPORT_STRING: std::cell::Cell<Option<fn(&str, &str) -> Option<String>>> = const { std::cell::Cell::new(None) };
}

/// The exporter registers org-export-string-as here (for ob-org).
pub fn set_export_string(f: fn(&str, &str) -> Option<String>) {
    EXPORT_STRING.with(|e| e.set(Some(f)));
}

pub fn export_string(body: &str, backend: &str) -> Option<String> {
    EXPORT_STRING.with(|e| e.get()).and_then(|f| f(body, backend))
}

/// Show TEXT in a buffer named NAME (Fred's stand-in for help and
/// session windows): a file in the Babel temporary directory.
fn show_buffer(ed: &mut Editor, name: &str, text: &str) {
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || " .-_[]".contains(c) { c } else { '_' }).collect();
    let path = core::temp_dir().join(file.trim_matches('_'));
    if let Err(e) = std::fs::write(&path, text) {
        ed.set_err(format!("{}: {e}", path.display()));
        return;
    }
    super::effect(ed, move |s| {
        s.perform(crate::ex::ExEffect::Open { path, line: 0, col: 0, pattern: None });
    });
}

/// Follow a `[[link]]` from results (org-open-at-point, else visit the file).
fn open_link(ed: &mut Editor, link: &str) {
    if super::call(ed, "org-open-at-point", Prefix::None).is_ok() {
        return;
    }
    let file = link.strip_prefix("file:").unwrap_or(link);
    let file = file.split("::").next().unwrap_or(file);
    let path = doc_of(ed).dir().join(super::options::expand(file));
    if path.exists() {
        super::effect(ed, move |s| s.perform(crate::ex::ExEffect::Open { path, line: 0, col: 0, pattern: None }));
    } else {
        ed.set_err(format!("No such file: {}", path.display()));
    }
}

fn job(f: impl Fn(&mut Doc, &mut Cx) -> R<()> + Send + Sync + 'static) -> Job {
    Arc::new(f)
}

// ---- API ----

/// org-babel-execute-maybe: execute the block, inline block or call at
/// point. False when there is none.
pub fn execute_maybe(ed: &mut Editor, arg: Prefix) -> bool {
    let doc = doc_of(ed);
    if !matches!(core::context(&doc, doc.point), Some(Context::Src(_) | Context::InlineSrc(..) | Context::Call(_) | Context::InlineCall(..))) {
        return false;
    }
    let force = arg.universal() > 0;
    run_job(ed, HashSet::new(), HashSet::new(), job(move |d, cx| execute_at(d, cx, d.point, force)), None);
    true
}

/// Execute what is at POS (org-babel-execute-src-block-maybe, then
/// org-babel-lob-execute-maybe).
fn execute_at(d: &mut Doc, cx: &mut Cx, pos: (usize, usize), force: bool) -> R<()> {
    match core::context(d, pos) {
        Some(Context::Src(el)) => {
            let info = core::src_info(d, cx, &el, false)?;
            core::execute_src_block(d, cx, force, Some(info), &[], ExecKind::Src)?;
        }
        Some(Context::InlineSrc(o, _)) => {
            let info = core::get_info(d, cx, (o.line, o.start), false)?;
            core::execute_src_block(d, cx, force, info, &[], ExecKind::Inline)?;
        }
        Some(Context::Call(el)) => {
            if let Some(info) = core::call_info(d, cx, &el, false)? {
                core::execute_src_block(d, cx, force, Some(info), &[], ExecKind::Call)?;
            }
        }
        Some(Context::InlineCall(o, _)) => {
            if let Some(info) = core::inline_call_info(d, cx, &o, false)? {
                core::execute_src_block(d, cx, force, Some(info), &[], ExecKind::InlineCall)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// org-babel-exp-process-buffer on TEXT without prompting: blocks that
/// need confirmation must be in APPROVED (or DENIED); otherwise the
/// returned error is the [`Fail::Confirm`] to ask about.
pub fn export_process_text(text: &str, path: Option<PathBuf>, approved: &HashSet<String>, denied: &HashSet<String>) -> Result<String, Fail> {
    let mut doc = Doc::new(text, path);
    let mut cx = Cx { approved: approved.clone(), denied: denied.clone(), ..Cx::default() };
    exp::process_buffer(&mut doc, &mut cx)?;
    Ok(doc.text())
}

/// org-babel-exp-process-buffer for the exporter: evaluates TEXT (the
/// export copy of this buffer), asking for confirmations, then calls
/// THEN with the processed text.
pub fn export_process(ed: &mut Editor, text: String, then: impl FnOnce(&mut Editor, Result<String, String>) + Send + 'static) {
    let path = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    export_loop(ed, text, path, HashSet::new(), HashSet::new(), Box::new(then));
}

type ExportThen = Box<dyn FnOnce(&mut Editor, Result<String, String>) + Send>;

fn export_loop(ed: &mut Editor, text: String, path: Option<PathBuf>, a: HashSet<String>, d: HashSet<String>, then: ExportThen) {
    match export_process_text(&text, path.clone(), &a, &d) {
        Ok(t) => then(ed, Ok(t)),
        Err(Fail::Error(e)) => then(ed, Err(e)),
        Err(Fail::Confirm { question, key }) => {
            let entries = vec![("y".to_owned(), "yes".to_owned()), ("n".to_owned(), "no".to_owned())];
            super::menu(ed, &question, entries, move |ed, k| {
                let (mut a, mut d) = (a, d);
                if k == "y" { a.insert(key) } else { d.insert(key) };
                export_loop(ed, text, path, a, d, then);
            });
        }
    }
}

/// org-sbe: the result of calling SOURCE_BLOCK with HEADER_ARGS and
/// VARIABLES (name, values; a value prefixed with `$` is a string).
pub fn sbe(ed: &Editor, source_block: &str, header_args: &str, variables: &[(String, Vec<String>)], approved: &HashSet<String>) -> Result<String, Fail> {
    let mut doc = doc_of(ed);
    let mut cx = Cx { approved: approved.clone(), ..Cx::default() };
    sbe_doc(&mut doc, &mut cx, source_block, header_args, variables)
}

/// org-sbe on a document.
pub fn sbe_doc(doc: &mut Doc, cx: &mut Cx, source_block: &str, header_args: &str, variables: &[(String, Vec<String>)]) -> Result<String, Fail> {
    if source_block.is_empty() {
        return Ok(String::new());
    }
    let vars: Vec<String> = variables
        .iter()
        .map(|(n, vals)| {
            let mut out = vec![];
            let mut quote = false;
            for v in vals {
                if v == "$" {
                    quote = true;
                    continue;
                }
                out.push(if quote { value::prin1(&Sexp::Str(v.clone())) } else { v.clone() });
                quote = false;
            }
            if out.len() > 1 {
                let read: Vec<String> = out.iter().map(|x| value::prin1(&value::read(x, true).unwrap_or(Sexp::Str(x.clone())))).collect();
                format!("{n}='({})", read.join(" "))
            } else {
                format!("{n}={}", out.first().cloned().unwrap_or_default())
            }
        })
        .collect();
    let spec = format!(":var results={source_block}[{header_args}]({})", vars.join(", "));
    let p = params::parse_header_arguments(&spec, false)?;
    let info = core::Info { lang: "emacs-lisp".into(), body: "results".into(), params: p, ..core::Info::default() };
    let silent = vec![(":results".to_owned(), Sexp::Str("silent".into()))];
    let r = core::execute_src_block(doc, cx, false, Some(info), &silent, ExecKind::Src)?;
    let r = r.unwrap_or(Sexp::Nil);
    Ok(value::trim(&value::echo(&r)).to_owned())
}

// ---- commands ----

const COMMANDS: &[&str] = &[
    "org-babel-execute-maybe",
    "org-babel-execute-safely-maybe",
    "org-babel-execute-src-block-maybe",
    "org-babel-execute-src-block",
    "org-babel-lob-execute-maybe",
    "org-babel-execute-buffer",
    "org-babel-execute-subtree",
    "org-babel-open-src-block-result",
    "org-babel-remove-result",
    "org-babel-remove-inline-result",
    "org-babel-remove-result-one-or-many",
    "org-babel-goto-named-src-block",
    "org-babel-goto-named-result",
    "org-babel-goto-src-block-head",
    "org-babel-next-src-block",
    "org-babel-previous-src-block",
    "org-babel-mark-block",
    "org-babel-demarcate-block",
    "org-babel-insert-header-arg",
    "org-babel-check-src-block",
    "org-babel-view-src-block-info",
    "org-babel-expand-src-block",
    "org-babel-expand-src-block-maybe",
    "org-babel-sha1-hash",
    "org-babel-hash-at-point",
    "org-babel-load-in-session",
    "org-babel-load-in-session-maybe",
    "org-babel-initiate-session",
    "org-babel-switch-to-session",
    "org-babel-pop-to-session",
    "org-babel-pop-to-session-maybe",
    "org-babel-switch-to-session-with-code",
    "org-babel-do-key-sequence-in-edit-buffer",
    "org-babel-describe-bindings",
    "org-babel-lob-ingest",
    "org-babel-tangle",
    "org-babel-tangle-file",
    "org-babel-detangle",
    "org-babel-tangle-jump-to-org",
    "org-babel-tangle-clean",
    "org-babel-hide-result-toggle",
    "org-babel-hide-result-toggle-maybe",
    "org-babel-result-hide-all",
    "org-babel-show-result-all",
    "org-babel-examplify-region",
    "org-babel-do-load-languages",
    "org-babel-exp-process-buffer",
    "org-babel-exp-src-block",
    "org-babel-shell-initialize",
    "org-babel-lilypond-toggle-midi-play",
    "org-babel-lilypond-toggle-pdf-display",
    "org-babel-lilypond-toggle-png-generation",
    "org-babel-lilypond-toggle-html-generation",
    "org-babel-lilypond-toggle-pdf-generation",
    "org-babel-lilypond-toggle-arrange-mode",
    "org-babel-lilypond-tangle",
    "org-babel-screen-test",
    "org-babel-processing-view-sketch",
    "org-babel-haskell-export-to-lhs",
    "org-babel-show-error-output",
];

/// Babel's interactive commands, by upstream name.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    if !COMMANDS.contains(&name) {
        return None;
    }
    Some(run_command(ed, name, arg))
}

fn here(ed: &Editor) -> (Doc, (usize, usize)) {
    let d = doc_of(ed);
    let p = d.point;
    (d, p)
}

fn run_command(ed: &mut Editor, name: &str, arg: Prefix) -> Result<(), String> {
    let force = arg.universal() > 0 && arg != Prefix::None;
    match name {
        "org-babel-execute-maybe" | "org-babel-execute-src-block-maybe" | "org-babel-lob-execute-maybe" | "org-babel-execute-safely-maybe" => {
            if name == "org-babel-execute-safely-maybe" && super::options::bool("org-babel-no-eval-on-ctrl-c-ctrl-c", false) {
                return Ok(());
            }
            let (d, p) = here(ed);
            let ok = match core::context(&d, p) {
                Some(Context::Src(_) | Context::InlineSrc(..)) => name != "org-babel-lob-execute-maybe",
                Some(Context::Call(_) | Context::InlineCall(..)) => name != "org-babel-execute-src-block-maybe",
                _ => false,
            };
            if ok {
                run_job(ed, HashSet::new(), HashSet::new(), job(move |d, cx| execute_at(d, cx, d.point, force)), None);
            }
            Ok(())
        }
        "org-babel-execute-src-block" => {
            run_job(
                ed,
                HashSet::new(),
                HashSet::new(),
                job(move |d, cx| {
                    let info = core::get_info(d, cx, d.point, false)?;
                    let kind = if matches!(core::context(d, d.point), Some(Context::InlineSrc(..))) { ExecKind::Inline } else { ExecKind::Src };
                    core::execute_src_block(d, cx, force, info, &[], kind)?;
                    Ok(())
                }),
                None,
            );
            Ok(())
        }
        "org-babel-execute-buffer" => {
            execute_range(ed, 0, None, force);
            Ok(())
        }
        "org-babel-execute-subtree" => {
            let (d, p) = here(ed);
            let h = crate::org::syntax::heading_at_or_before(&d.lines, p.0.min(d.len().saturating_sub(1)));
            match h {
                Some(h) => execute_range(ed, 0, Some(h), force),
                None => execute_range(ed, 0, None, force),
            }
            Ok(())
        }
        "org-babel-open-src-block-result" => {
            let (d, p) = here(ed);
            if !matches!(core::context(&d, p), Some(Context::Src(_) | Context::InlineSrc(..))) {
                return Ok(());
            }
            run_job(
                ed,
                HashSet::new(),
                HashSet::new(),
                job(move |d, cx| {
                    let p = d.point;
                    let Some(info) = core::get_info(d, cx, p, true)? else { return Ok(()) };
                    let start = info.start.unwrap_or(p);
                    let mut loc = if force { None } else { core::where_is_result(d, cx, start, false, None)? };
                    if loc.is_none() {
                        core::execute_src_block(d, cx, false, None, &[], ExecKind::Src)?;
                        loc = core::where_is_result(d, cx, start, false, None)?;
                    }
                    let Some(core::Loc::Line(l)) = loc else { return Ok(()) };
                    let mut r = l + 1;
                    while r < d.len() && d.line(r).trim().is_empty() {
                        r += 1;
                    }
                    let t = d.line(r).trim().to_owned();
                    if t.starts_with("[[") && t.ends_with("]]") {
                        cx.show = Some((format!("\0open:{}", &t[2..t.find(']').unwrap_or(t.len())]), String::new()));
                        return Ok(());
                    }
                    let res = core::read_result(d, r)?.unwrap_or(Sexp::Nil);
                    let text = core::format_result(&res, params::get_str(&info.params, ":sep").as_deref());
                    cx.show = Some((super::options::string("org-babel-results-buffer-name", "*Org Babel Results*"), text));
                    Ok(())
                }),
                None,
            );
            Ok(())
        }
        "org-babel-remove-result" => {
            let (mut d, p) = here(ed);
            let mut cx = Cx::default();
            core::remove_result(&mut d, &mut cx, p, false).map_err(err_text)?;
            apply(ed, &d);
            Ok(())
        }
        "org-babel-remove-inline-result" => {
            let (mut d, p) = here(ed);
            core::remove_inline_result(&mut d, p);
            apply(ed, &d);
            Ok(())
        }
        "org-babel-remove-result-one-or-many" => {
            let (mut d, p) = here(ed);
            let mut cx = Cx::default();
            if arg.is_none() {
                core::remove_result(&mut d, &mut cx, p, false).map_err(err_text)?;
            } else {
                // Every block and call line, bottom first so lines stay put.
                let heads: Vec<usize> = doc::all_elements(&d.lines).into_iter().filter(|e| matches!(e.kind, Kind::Src | Kind::Call)).map(|e| e.post).collect();
                for h in heads.into_iter().rev() {
                    core::remove_result(&mut d, &mut cx, (h, 0), false).map_err(err_text)?;
                }
            }
            apply(ed, &d);
            Ok(())
        }
        "org-babel-goto-named-src-block" => {
            let (d, p) = here(ed);
            let names: Vec<String> = doc::all_elements(&d.lines).into_iter().filter(|e| e.kind == Kind::Src).filter_map(|e| e.name).collect();
            let _ = (p, names.clone());
            super::complete(ed, "source-block name: ", names, true, |ed, name| {
                let d = doc_of(ed);
                match doc::all_elements(&d.lines).into_iter().find(|e| e.kind == Kind::Src && e.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&name))) {
                    Some(e) => {
                        ed.set_cursor(e.begin, 0);
                        crate::org::fold::show_context(ed, e.begin);
                    }
                    None => ed.set_msg(format!("source-code block `{name}' not found in this buffer")),
                }
            });
            Ok(())
        }
        "org-babel-goto-named-result" => {
            let d = doc_of(ed);
            let names: Vec<String> = d.lines.iter().filter(|l| doc::is_result_line(l)).map(|l| doc::result_value(l).to_owned()).filter(|n| !n.is_empty()).collect();
            super::complete(ed, "Source-block name: ", names, true, |ed, name| {
                let d = doc_of(ed);
                match core::find_named_result(&d, &name) {
                    Some(l) => {
                        ed.set_cursor(l, 0);
                        crate::org::fold::show_context(ed, l);
                    }
                    None => ed.set_msg(format!("result `{name}' not found in this buffer")),
                }
            });
            Ok(())
        }
        "org-babel-goto-src-block-head" => {
            let (d, p) = here(ed);
            match core::where_is_head(&d, p.0) {
                Some(h) => {
                    ed.set_cursor(h, 0);
                    Ok(())
                }
                None => Err("Not currently in a code block".into()),
            }
        }
        "org-babel-next-src-block" | "org-babel-previous-src-block" => {
            let (d, p) = here(ed);
            let n = arg.value().max(1) as usize;
            let back = name.contains("previous");
            let heads: Vec<usize> = (0..d.len()).filter(|&l| d.line(l).trim_start().to_ascii_lowercase().starts_with("#+begin_src")).collect();
            let target = if back { heads.iter().rev().filter(|&&h| h < p.0).nth(n - 1) } else { heads.iter().filter(|&&h| h > p.0).nth(n - 1) };
            match target {
                Some(&h) => {
                    ed.set_cursor(h, 0);
                    crate::org::fold::show_context(ed, h);
                    Ok(())
                }
                None => Err(format!("No {} code blocks", if back { "previous" } else { "further" })),
            }
        }
        "org-babel-mark-block" => {
            let (d, p) = here(ed);
            if let Some(h) = core::where_is_head(&d, p.0)
                && let Some(el) = doc::element_at(&d.lines, h)
                && el.stop >= h + 2
            {
                let last = el.stop - 2;
                let first = (h + 1).min(last);
                ed.mode = crate::editor::Mode::VisualLine { anchor: last.max(first) };
                ed.set_cursor(first, 0);
            }
            Ok(())
        }
        "org-babel-demarcate-block" => demarcate(ed, arg),
        "org-babel-insert-header-arg" => insert_header_arg(ed),
        "org-babel-check-src-block" => {
            let (d, p) = here(ed);
            let Some(h) = core::where_is_head(&d, p.0) else {
                ed.set_msg("No suspicious header arguments found.");
                return Ok(());
            };
            let params = core::parse_begin(d.line(h)).2;
            let parsed = params::parse_header_arguments(&params, true).map_err(|e| e.to_string())?;
            let names: Vec<&str> = params::COMMON.iter().map(|(n, _)| *n).collect();
            for (k, _) in parsed {
                let header = &k[1..];
                for n in &names {
                    if header != *n && params::distance(header, n) <= 2 && !names.contains(&header) {
                        return Err(format!("Supplied header \"{header}\" is suspiciously close to \"{n}\""));
                    }
                }
            }
            ed.set_msg("No suspicious header arguments found.");
            Ok(())
        }
        "org-babel-view-src-block-info" => {
            let (mut d, p) = here(ed);
            let mut cx = Cx::default();
            let Some(info) = core::get_info(&mut d, &mut cx, p, true).map_err(err_text)? else { return Ok(()) };
            let mut s = String::new();
            if let Some(n) = &info.name {
                s.push_str(&format!("Name: {n}\n"));
            }
            s.push_str(&format!("Language: {}\n", info.lang));
            s.push_str("Properties:\n");
            let line = info.start.map_or(p.0, |x| x.0);
            let a = core::entry_get(&d, &mut cx, line, "header-args", true).unwrap_or_else(|| "nil".into());
            let b = core::entry_get(&d, &mut cx, line, &format!("header-args:{}", info.lang), true).unwrap_or_else(|| "nil".into());
            s.push_str(&format!("\t:header-args \t{a}\n"));
            s.push_str(&format!("\t:header-args:{} \t{b}\n", info.lang));
            if !info.switches.is_empty() {
                s.push_str(&format!("Switches: {}\n", info.switches));
            }
            s.push_str("Header Arguments:\n");
            let mut ps = info.params.clone();
            ps.sort_by(|x, y| x.0.cmp(&y.0));
            for (k, v) in ps {
                let shown = value::princ(&v);
                if !shown.is_empty() && !v.is_nil() {
                    s.push_str(&format!("\t{k}{}\t{shown}\n", if k.len() > 7 { "" } else { "\t" }));
                }
            }
            show_buffer(ed, "*Help*", &s);
            Ok(())
        }
        "org-babel-expand-src-block" | "org-babel-expand-src-block-maybe" => {
            let (d0, p) = here(ed);
            if !matches!(core::context(&d0, p), Some(Context::Src(_) | Context::InlineSrc(..))) {
                return if name.ends_with("maybe") { Ok(()) } else { Err("No source block at point".into()) };
            }
            let file = d0.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            run_job(
                ed,
                HashSet::new(),
                HashSet::new(),
                job(move |d, cx| {
                    let Some(info) = core::get_info(d, cx, d.point, false)? else { return Ok(()) };
                    let mut ps = info.params.clone();
                    ps.sort_by(|x, y| x.0.cmp(&y.0));
                    let body = if params::noweb_p(&ps, ":eval") { core::expand_noweb(d, cx, &info, ":eval")? } else { info.body.clone() };
                    let dir = d.dir();
                    let at = info.start.map_or(d.point.0, |s| s.0);
                    let expanded = core::expand_full(&mut core::Env { doc: d, cx, at, dir }, &info.lang, &body, &ps)?;
                    let ext = lang::tangle_ext(&info.lang);
                    cx.show = Some((format!("Org-Babel Preview {file}[ {} ].{ext}", info.lang), expanded));
                    Ok(())
                }),
                None,
            );
            Ok(())
        }
        "org-babel-sha1-hash" => {
            run_job(
                ed,
                HashSet::new(),
                HashSet::new(),
                job(|d, cx| {
                    let Some(info) = core::get_info(d, cx, d.point, false)? else { return Ok(()) };
                    let h = core::sha1_hash(d, cx, &info, ":eval")?;
                    cx.message(h);
                    Ok(())
                }),
                None,
            );
            Ok(())
        }
        "org-babel-hash-at-point" => {
            let (d, p) = here(ed);
            if let Some(h) = doc::result_hash(d.line(p.0)) {
                ed.reg = crate::editor::Register { text: h.clone(), linewise: false };
                ed.set_msg(h);
            }
            Ok(())
        }
        "org-babel-load-in-session" | "org-babel-load-in-session-maybe" | "org-babel-initiate-session" | "org-babel-switch-to-session"
        | "org-babel-pop-to-session" | "org-babel-pop-to-session-maybe" | "org-babel-switch-to-session-with-code" => {
            let (d, p) = here(ed);
            if !matches!(core::context(&d, p), Some(Context::Src(_) | Context::InlineSrc(..))) {
                return if name.ends_with("maybe") { Ok(()) } else { Err("No src code block at point".into()) };
            }
            let load = name.starts_with("org-babel-load-in-session");
            let prep = arg != Prefix::None || load;
            let name = name.to_owned();
            run_job(
                ed,
                HashSet::new(),
                HashSet::new(),
                job(move |d, cx| {
                    let Some(info) = core::get_info(d, cx, d.point, !prep)? else { return err_r("No src code block at point") };
                    let (kind, sname) = session_kind(&info)?;
                    let dir = params::get_str(&info.params, ":dir").map(|x| d.dir().join(x)).unwrap_or_else(|| d.dir());
                    session::initiate(&kind, &sname, &dir)?;
                    if prep {
                        let at = info.start.map_or(0, |s| s.0);
                        let lines = lang::assignments(&mut core::Env { doc: d, cx, at, dir: dir.clone() }, &info.lang, &info.params)?.unwrap_or_default();
                        for l in lines {
                            session::eval(&kind, &sname, &dir, &l)?;
                        }
                    }
                    if load {
                        let body = if params::noweb_p(&info.params, ":eval") { core::expand_noweb(d, cx, &info, ":eval")? } else { info.body.clone() };
                        session::eval(&kind, &sname, &dir, &body)?;
                    }
                    if name == "org-babel-initiate-session" {
                        cx.message(format!("session {sname} ready"));
                    }
                    cx.show = Some((sname.clone(), session::transcript(&kind, &sname).unwrap_or_default()));
                    Ok(())
                }),
                None,
            );
            Ok(())
        }
        "org-babel-do-key-sequence-in-edit-buffer" => {
            super::read(ed, "Enter key-sequence to execute in edit buffer: ", "", |ed, keys| {
                if let Err(e) = super::call(ed, "org-edit-special", Prefix::None) {
                    ed.set_err(e);
                    return;
                }
                for k in crate::key::parse_keys(&keys) {
                    ed.handle_key(k);
                }
                if let Err(e) = super::call(ed, "org-edit-src-exit", Prefix::None) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        "org-babel-describe-bindings" => {
            let mut s = String::from("Key             Binding\n\n");
            for (k, c) in super::keymap::ORG_MODE_MAP.iter().filter(|(k, _)| k.starts_with("C-c C-v")) {
                s.push_str(&format!("{k:<16}{c}\n"));
            }
            show_buffer(ed, "*Help*", &s);
            Ok(())
        }
        "org-babel-lob-ingest" => {
            let initial = ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            super::read(ed, "File: ", &initial, |ed, file| {
                let path = super::options::expand(&file);
                let (mut d, cx_doc) = match std::fs::read_to_string(&path) {
                    Ok(t) => (Doc::new(&t, std::path::absolute(&path).ok()), true),
                    Err(_) if file.is_empty() => (doc_of(ed), false),
                    Err(e) => {
                        ed.set_err(format!("{file}: {e}"));
                        return;
                    }
                };
                let _ = cx_doc;
                let mut cx = Cx::default();
                match core::lob_ingest(&mut d, &mut cx) {
                    Ok(_) => finish(ed, &cx),
                    Err(e) => ed.set_err(err_text(e)),
                }
            });
            Ok(())
        }
        "org-babel-tangle" => {
            let scope = match arg.universal() {
                1 => tangle::Scope::Block,
                2 => tangle::Scope::File,
                _ => tangle::Scope::All,
            };
            let (mut d, _) = here(ed);
            let mut cx = Cx::default();
            tangle::tangle(&mut d, &mut cx, scope, None, None).map_err(err_text)?;
            finish(ed, &cx);
            Ok(())
        }
        "org-babel-tangle-file" => {
            let initial = ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            super::read(ed, "File to tangle: ", &initial, |ed, file| {
                let path = super::options::expand(&file);
                let abs = std::path::absolute(&path).unwrap_or(path.clone());
                let mut d = if ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()) == Some(abs.clone()) {
                    doc_of(ed)
                } else {
                    match std::fs::read_to_string(&path) {
                        Ok(t) => Doc::new(&t, Some(abs)),
                        Err(e) => {
                            ed.set_err(format!("{file}: {e}"));
                            return;
                        }
                    }
                };
                let mut cx = Cx::default();
                match tangle::tangle(&mut d, &mut cx, tangle::Scope::All, None, None) {
                    Ok(_) => finish(ed, &cx),
                    Err(e) => ed.set_err(err_text(e)),
                }
            });
            Ok(())
        }
        "org-babel-tangle-jump-to-org" => {
            let code = doc_of(ed);
            let o = tangle::origin(&code).map_err(err_text)?;
            let org_path = tangle::link_file(&code, &o);
            let text = std::fs::read_to_string(&org_path).map_err(|e| format!("{}: {e}", org_path.display()))?;
            let org = Doc::new(&text, Some(org_path.clone()));
            let block = tangle::find_block(&org, &o).ok_or("Not in tangled code")?;
            let (line, col) = tangle::jump_position(&org, &o, block);
            super::effect(ed, move |s| s.perform(crate::ex::ExEffect::Open { path: org_path, line, col, pattern: None }));
            Ok(())
        }
        "org-babel-detangle" => {
            let code = doc_of(ed);
            let source = code.path.clone();
            let pairs = tangle::detangle_pairs(&code);
            let mut by_file: Vec<(PathBuf, Vec<(tangle::Origin, String)>)> = vec![];
            for (o, body) in pairs {
                let f = tangle::link_file(&code, &o);
                match by_file.iter_mut().find(|(p, _)| *p == f) {
                    Some((_, v)) => v.push((o, body)),
                    None => by_file.push((f, vec![(o, body)])),
                }
            }
            let count: usize = by_file.iter().map(|(_, v)| v.len()).sum();
            super::effect(ed, move |s| {
                for (file, items) in by_file {
                    s.perform(crate::ex::ExEffect::Open { path: file, line: 0, col: 0, pattern: None });
                    let mut d = doc_of(&s.ed);
                    for (o, body) in &items {
                        if let Some(b) = tangle::find_block(&d, o) {
                            tangle::update_block_body(&mut d, b, body);
                        }
                    }
                    s.ed.undo.begin(s.ed.cur.pos());
                    apply(&mut s.ed, &d);
                    s.ed.undo.end(s.ed.cur.pos());
                }
                if let Some(src) = source {
                    s.perform(crate::ex::ExEffect::Open { path: src, line: 0, col: 0, pattern: None });
                }
                s.ed.set_msg(format!("Detangled {count} code blocks"));
            });
            Ok(())
        }
        "org-babel-tangle-clean" => {
            let mut d = doc_of(ed);
            let link_re = regex::Regex::new(r"\[\[file:.*\]\[.*\]\]").unwrap();
            let (s, e) = (core::noweb_start(), core::noweb_end());
            d.lines.retain(|l| !link_re.is_match(l) && !(l.contains(&s) && l.contains(&e)));
            apply(ed, &d);
            Ok(())
        }
        "org-babel-hide-result-toggle" | "org-babel-hide-result-toggle-maybe" => {
            let (d, p) = here(ed);
            if !doc::is_result_line(d.line(p.0)) {
                return if name.ends_with("maybe") { Ok(()) } else { Err("Not looking at a result line".into()) };
            }
            toggle_result(ed, &d, p.0, None);
            Ok(())
        }
        "org-babel-result-hide-all" => {
            let d = doc_of(ed);
            for l in (0..d.len()).filter(|&l| doc::is_result_line(d.line(l))) {
                toggle_result(ed, &d, l, Some(true));
            }
            Ok(())
        }
        "org-babel-show-result-all" => {
            let d = doc_of(ed);
            for l in (0..d.len()).filter(|&l| doc::is_result_line(d.line(l))) {
                toggle_result(ed, &d, l, Some(false));
            }
            Ok(())
        }
        "org-babel-examplify-region" => {
            let (lo, hi) = match ed.mode {
                crate::editor::Mode::VisualLine { anchor } => (anchor.min(ed.cur.line), anchor.max(ed.cur.line)),
                _ => (ed.cur.line, ed.cur.line),
            };
            let mut d = doc_of(ed);
            let text = d.lines[lo..=hi].join("\n") + "\n";
            let min = super::options::int("org-babel-min-lines-for-block-output", 10) as usize;
            let out: Vec<String> = if hi - lo + 1 < min {
                text.lines().map(|l| format!(": {l}")).collect()
            } else {
                let mut v = vec!["#+begin_example".to_owned()];
                v.extend(core::escape(text.trim_end_matches('\n')).split('\n').map(str::to_owned));
                v.push("#+end_example".into());
                v
            };
            d.splice(lo, hi - lo + 1, out);
            ed.mode = crate::editor::Mode::Normal;
            apply(ed, &d);
            Ok(())
        }
        "org-babel-do-load-languages" => {
            ed.set_msg(format!("Babel languages: {}", lang::loaded().join(" ")));
            Ok(())
        }
        "org-babel-exp-process-buffer" => {
            run_job(ed, HashSet::new(), HashSet::new(), job(|d, cx| exp::process_buffer(d, cx)), None);
            Ok(())
        }
        "org-babel-exp-src-block" => Err("org-babel-exp-src-block runs during export (see org-babel-exp-process-buffer)".into()),
        "org-babel-shell-initialize" => {
            ed.set_msg(format!("Shells: {}", lang::shell_names().join(" ")));
            Ok(())
        }
        n if n.starts_with("org-babel-lilypond-toggle-") => {
            let (opt, what) = match n {
                "org-babel-lilypond-toggle-midi-play" => ("org-babel-lilypond-play-midi-post-tangle", "Post-Tangle MIDI play"),
                "org-babel-lilypond-toggle-pdf-display" => ("org-babel-lilypond-display-pdf-post-tangle", "Post-Tangle PDF display"),
                "org-babel-lilypond-toggle-png-generation" => ("org-babel-lilypond-gen-png", "PNG image generation"),
                "org-babel-lilypond-toggle-html-generation" => ("org-babel-lilypond-gen-html", "HTML generation"),
                "org-babel-lilypond-toggle-pdf-generation" => ("org-babel-lilypond-gen-pdf", "PDF generation"),
                _ => ("org-babel-lilypond-arrange-mode", "Arrange mode"),
            };
            let default = matches!(opt, "org-babel-lilypond-play-midi-post-tangle" | "org-babel-lilypond-display-pdf-post-tangle");
            let on = !super::options::bool(opt, default);
            super::options::put(opt, toml::Value::Boolean(on));
            ed.set_msg(format!("{what} has been {}", if on { "ENABLED." } else { "DISABLED." }));
            Ok(())
        }
        "org-babel-lilypond-tangle" => {
            let (mut d, _) = here(ed);
            let mut cx = Cx::default();
            tangle::tangle(&mut d, &mut cx, tangle::Scope::All, Some("yes"), Some("lilypond")).map_err(err_text)?;
            finish(ed, &cx);
            Ok(())
        }
        "org-babel-screen-test" => {
            let mut d = Doc::default();
            let mut cx = Cx::default();
            let tmp = core::temp_file("ob-screen-test-", "");
            let _ = std::fs::remove_file(&tmp);
            let n = std::process::id() % 99999;
            let body = format!("echo '{n}' > {}\nexit\n", tmp.display());
            let p = params::default_args("org-babel-default-header-args:screen");
            let dir = d.dir();
            lang::execute(&mut core::Env { doc: &mut d, cx: &mut cx, at: 0, dir }, "screen", &body, &p).map_err(err_text)?;
            std::thread::sleep(std::time::Duration::from_millis(500));
            let ok = std::fs::read_to_string(&tmp).is_ok_and(|s| s.contains(&n.to_string()));
            ed.set_msg(format!("org-babel-screen: Setup {}", if ok { "WORKS." } else { "DOESN'T work." }));
            Ok(())
        }
        "org-babel-processing-view-sketch" => Err("org-babel-processing-view-sketch: Processing's sketch runner is an Emacs mode; not available in Fred".into()),
        "org-babel-haskell-export-to-lhs" => Err("org-babel-haskell-export-to-lhs needs the LaTeX exporter".into()),
        "org-babel-show-error-output" => {
            let t = ERROR_OUTPUT.with(|e| e.borrow().clone());
            show_buffer(ed, "*Org-Babel Error Output*", &t);
            Ok(())
        }
        _ => Err(format!("{name}: not yet ported to fred")),
    }
}

fn err_text(f: Fail) -> String {
    match f {
        Fail::Error(e) => e,
        Fail::Confirm { question, .. } => question,
    }
}

fn err_r<T>(s: &str) -> R<T> {
    Err(Fail::Error(s.into()))
}

/// The session a block's language runs in, and its name.
fn session_kind(info: &core::Info) -> R<(session::Kind, String)> {
    let s = params::get_str(&info.params, ":session").filter(|s| s != "none");
    let Some(s) = s else { return err_r("This block is not using a session!") };
    let o = |n: &str, d: &str| super::options::string(n, d);
    let named = |d: &str| if s.is_empty() { d.to_owned() } else { s.clone() };
    Ok(match info.lang.as_str() {
        "python" => (session::Kind::Python(o("org-babel-python-command", "auto").replace("auto", "python3")), if s.is_empty() { "*Python*".into() } else if s.starts_with('*') { s.clone() } else { format!("*{s}*") }),
        "ruby" => (session::Kind::Ruby(o("org-babel-ruby-command", "ruby")), named("ruby")),
        "R" => (session::Kind::R(o("org-babel-R-command", "R --slave --no-save")), named("*R*")),
        "julia" => (session::Kind::Julia(o("org-babel-julia-command", "julia")), named("*Julia*")),
        "js" => (session::Kind::Node(o("org-babel-js-cmd", "node")), named("*Javascript REPL*")),
        "haskell" => (session::Kind::Haskell, named("*ob-haskell*")),
        "ocaml" => (session::Kind::Ocaml(o("org-babel-ocaml-command", "ocaml")), named("*ocaml-toplevel*")),
        "octave" | "matlab" => (
            session::Kind::Plain(o("org-babel-octave-shell-command", "octave -q").split_whitespace().map(str::to_owned).collect(), "disp('org_babel_eoe')".into(), "org_babel_eoe".into()),
            named("*Inferior Octave*"),
        ),
        "shell" | "eshell" => (session::Kind::Shell(core::shell_file_name()), named("*shell*")),
        l if lang::shell_names().iter().any(|x| x == l) => (session::Kind::Shell(l.to_owned()), named(&format!("*{l}*"))),
        l => return err_r(&format!("No org-babel-initiate-session function for {l}!")),
    })
}

/// org-babel-hide-result-toggle at the results line `l`.
fn toggle_result(ed: &mut Editor, d: &Doc, l: usize, force: Option<bool>) {
    let mut start = l + 1;
    while start < d.len() && doc::keyword(d.line(start)).is_some_and(|(k, _)| k == "HEADER" || k == "HEADERS") {
        start += 1;
    }
    let end = core::result_end(d, start);
    if end <= start {
        return;
    }
    let hidden = ed.folds.hidden(start);
    let hide = force.unwrap_or(!hidden);
    crate::org::fold::region(ed, start, end - 1, hide, crate::org::fold::Spec::Block);
}

/// Execute every executable in a range (org-babel-execute-buffer and
/// -subtree), one at a time so each can ask for confirmation.
fn execute_range(ed: &mut Editor, n: usize, heading: Option<usize>, force: bool) {
    let d = doc_of(ed);
    let (lo, hi) = match heading {
        Some(h) => (h, crate::org::syntax::subtree_end(&d.lines, h)),
        None => (0, d.len()),
    };
    let execs = executables(&d, lo, hi);
    let Some(&pos) = execs.get(n) else { return };
    let _ = pos;
    run_job(
        ed,
        HashSet::new(),
        HashSet::new(),
        job(move |d, cx| {
            let (lo, hi) = match heading {
                Some(h) => (h, crate::org::syntax::subtree_end(&d.lines, h)),
                None => (0, d.len()),
            };
            let point = d.point;
            if let Some(&p) = executables(d, lo, hi).get(n) {
                execute_at(d, cx, p, force)?;
            }
            d.point = point;
            Ok(())
        }),
        Some(Box::new(move |ed: &mut Editor| execute_range(ed, n + 1, heading, force))),
    );
}

/// Positions of blocks, calls and inline objects in lines `lo..hi`
/// (org-babel-map-executables), skipping commented subtrees? no: as upstream.
fn executables(d: &Doc, lo: usize, hi: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    for el in doc::all_elements(&d.lines) {
        if el.post < lo || el.post >= hi {
            continue;
        }
        match el.kind {
            Kind::Src | Kind::Call => out.push((el.post, 0)),
            ref k if doc::holds_objects(k) => {
                for l in el.post..el.stop {
                    for o in doc::inlines(l, d.line(l)) {
                        out.push((l, o.start));
                    }
                }
            }
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// org-babel-demarcate-block.
fn demarcate(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let mut d = doc_of(ed);
    let mut cx = Cx::default();
    let p = d.point;
    let region = match ed.mode {
        crate::editor::Mode::VisualLine { anchor } => Some((anchor.min(p.0), anchor.max(p.0))),
        _ => None,
    };
    let level = crate::org::syntax::heading_at_or_before(&d.lines, p.0.min(d.len().saturating_sub(1))).and_then(|h| crate::org::syntax::level(d.line(h))).unwrap_or(1);
    let stars = format!("{} ", "*".repeat(level));
    let head = core::where_is_head(&d, p.0);
    let info = core::get_info(&mut d, &mut cx, p, true).map_err(err_text)?;
    if let (Some(h), Some(_)) = (head, &info) {
        let el = doc::element_at(&d.lines, h).ok_or("No block")?;
        let (body_lo, body_hi) = (el.post + 1, el.stop - 1);
        let upper = d.line(el.post).contains("#+BEGIN_SRC");
        // Split points as (line, col) inside the body.
        let cuts: Vec<(usize, usize)> = match region {
            Some((a, b)) => vec![(a, 0), (b + 1, 0)],
            None => vec![p],
        };
        let inside = cuts.iter().all(|&(l, c)| (l > body_lo || (l == body_lo && c == 0) || l == body_lo) && (l < body_hi || (l == body_hi && c == 0)));
        if !inside || cuts.iter().any(|&(l, _)| l < body_lo || l > body_hi) {
            return Err("Select within the source block body to split it".into());
        }
        let text: Vec<String> = d.lines[body_lo..body_hi].to_vec();
        // The body as one string with cut offsets.
        let mut joined = String::new();
        let mut offsets = vec![];
        let mut pads = vec![0];
        for (i, l) in text.iter().enumerate() {
            for &(cl, cc) in &cuts {
                if cl == body_lo + i {
                    offsets.push(joined.len() + cc.min(l.len()));
                    pads.push(value::indentation(&l[..cc.min(l.len())]).max(unicode_width::UnicodeWidthStr::width(&l[..cc.min(l.len())])));
                }
            }
            joined.push_str(l);
            joined.push('\n');
        }
        for &(cl, _) in &cuts {
            if cl == body_hi {
                offsets.push(joined.len());
                pads.push(0);
            }
        }
        let mut bounds = vec![0];
        bounds.extend(offsets);
        bounds.push(joined.len());
        let parts: Vec<String> = bounds.windows(2).enumerate().map(|(i, w)| format!("{}{}", " ".repeat(pads.get(i).copied().unwrap_or(0)), &joined[w[0]..w[1]])).collect();
        let affiliated: Vec<String> = d.lines[el.begin..el.post].to_vec();
        let begin_line = d.line(el.post).to_owned();
        let end_line = d.line(el.stop - 1).to_owned();
        let preserve = core::parse_begin(&begin_line).1.split_whitespace().any(|s| s == "-i") || super::options::bool("org-src-preserve-indentation", false);
        let ind = value::indentation(&begin_line);
        let content = ind + super::options::int("org-edit-src-content-indentation", 2) as usize;
        let body_lines = |part: &str| -> Vec<String> {
            let part = part.strip_suffix('\n').unwrap_or(part);
            if part.trim().is_empty() {
                return vec![];
            }
            let b = if preserve { part.to_owned() } else { value::remove_indentation(part) };
            b.split('\n').map(|l| if preserve || l.trim().is_empty() { l.to_owned() } else { format!("{}{l}", " ".repeat(content)) }).collect()
        };
        let mut out: Vec<String> = vec![];
        let n = parts.len();
        let mut last_start = 0;
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.push(if arg != Prefix::None { stars.clone() } else { String::new() });
            }
            last_start = out.len();
            if i == 0 {
                out.extend(affiliated.iter().cloned());
            } else {
                out.extend(affiliated.iter().filter(|l| doc::keyword(l).is_none_or(|(k, _)| k != "NAME" && k != "CAPTION")).cloned());
            }
            last_start += out.len() - last_start - 0;
            let begin = if upper { begin_line.clone() } else { begin_line.clone() };
            out.push(begin);
            out.extend(body_lines(part));
            out.push(end_line.clone());
            let _ = n;
        }
        // Point at the last block's #+begin line.
        let last_begin = out.iter().rposition(|l| l.trim_start().to_ascii_lowercase().starts_with("#+begin_src")).unwrap_or(last_start);
        d.splice(el.begin, el.stop - el.begin, out);
        d.point = (el.begin + last_begin, 0);
        ed.mode = crate::editor::Mode::Normal;
        apply(ed, &d);
        return Ok(());
    }
    // Wrap the region (or point) in a new block.
    let lang = info.map(|i| i.lang).or_else(|| {
        // Within blank lines after a block: its language.
        doc::element_at(&d.lines, p.0).filter(|e| e.kind == Kind::Src).map(|e| core::parse_begin(d.line(e.post)).0)
    });
    let upper = doc::element_at(&d.lines, p.0).is_some_and(|e| e.kind == Kind::Src && d.line(e.post).contains("#+BEGIN_SRC"));
    let insert = move |ed: &mut Editor, lang: String| {
        let mut d = doc_of(ed);
        let p = d.point;
        let (body, at): (Vec<String>, usize) = match region {
            Some((a, b)) => {
                let body = d.lines[a..=b].to_vec();
                d.splice(a, b - a + 1, vec![]);
                (body, a)
            }
            None => {
                let line = d.line(p.0).to_owned();
                let col = p.1.min(line.len());
                if col > 0 && col < line.len() {
                    d.set(p.0, line[..col].to_owned());
                    d.insert(p.0 + 1, vec![line[col..].to_owned()]);
                    (vec![], p.0 + 1)
                } else if col > 0 {
                    (vec![], p.0 + 1)
                } else {
                    (vec![], p.0)
                }
            }
        };
        let mut lines = vec![];
        if arg != Prefix::None {
            lines.push(stars.clone());
        }
        lines.push(format!("{}{lang}", if upper { "#+BEGIN_SRC " } else { "#+begin_src " }));
        lines.extend(body);
        lines.push(if upper { "#+END_SRC" } else { "#+end_src" }.into());
        let first = at + usize::from(arg != Prefix::None);
        d.insert(at, lines);
        d.point = (first, d.line(first).len());
        ed.mode = crate::editor::Mode::Normal;
        apply(ed, &d);
    };
    match lang {
        Some(l) => insert(ed, l),
        None => {
            let mut langs = lang::loaded();
            for l in ["C", "C++", "asymptote", "beamer", "calc", "cpp", "ditaa", "desktop", "dot", "elisp", "ocaml", "screen", "sqlite", "toml", "shell"] {
                if !langs.iter().any(|x| x == l) {
                    langs.push(l.into());
                }
            }
            super::complete(ed, "Lang: ", langs, false, move |ed, l| insert(ed, l));
        }
    }
    Ok(())
}

/// org-babel-insert-header-arg.
fn insert_header_arg(ed: &mut Editor) -> Result<(), String> {
    let (mut d, p) = here(ed);
    let mut cx = Cx::default();
    let Some(info) = core::get_info(&mut d, &mut cx, p, true).map_err(err_text)? else { return Err("No source block at point".into()) };
    let mut headers: Vec<(&'static str, Option<&'static [&'static [&'static str]]>)> = params::COMMON.to_vec();
    for h in params::lang_headers(&info.lang) {
        headers.retain(|(n, _)| *n != h.0);
        headers.insert(0, h);
    }
    let names: Vec<String> = headers.iter().map(|(n, _)| (*n).to_owned()).collect();
    let begin = info.start.map_or(p.0, |s| s.0);
    super::complete(ed, "Header Arg: ", names, false, move |ed, header| {
        let vals = headers.iter().find(|(n, _)| *n == header).and_then(|(_, v)| *v);
        let put = move |ed: &mut Editor, value: String| {
            let mut d = doc_of(ed);
            let mut l = d.line(begin).to_owned();
            if !l.ends_with(' ') {
                l.push(' ');
            }
            l.push(':');
            l.push_str(&header);
            if !value.is_empty() {
                l.push(' ');
                l.push_str(value.trim_end());
            }
            d.set(begin, l);
            apply(ed, &d);
        };
        match vals {
            None => super::read(ed, "value: ", "", put),
            Some(groups) => ask_groups(ed, groups.to_vec(), String::new(), Box::new(put)),
        }
    });
    Ok(())
}

type Put = Box<dyn FnOnce(&mut Editor, String) + Send>;

fn ask_groups(ed: &mut Editor, mut groups: Vec<&'static [&'static str]>, acc: String, put: Put) {
    if groups.is_empty() {
        put(ed, acc);
        return;
    }
    let g = groups.remove(0);
    let mut cands = vec!["default".to_owned()];
    cands.extend(g.iter().map(|s| (*s).to_owned()));
    super::complete(ed, "Value: ", cands, false, move |ed, v| {
        let acc = if v.is_empty() || v == "default" { acc } else { format!("{acc}{v} ") };
        ask_groups(ed, groups, acc, put);
    });
}
