//! ob-exp.el: evaluation during export. The exporter calls
//! [`process_buffer`] on its copy of the document: every block, inline
//! block and call is evaluated or replaced per `:exports`, with
//! references resolved in a pristine copy (org-babel-exp-reference-buffer).

use super::core::{self, Cx, ExecKind, Info, R};
use super::doc::{self, Context, Doc, Kind};
use super::params::{self, get_str};
use super::value;
use crate::org::sexp::Sexp;
use crate::org::options;

/// org-fill-template: `%key` replaced, longest keys first.
pub fn fill_template(tpl: &str, keys: &[(String, String)]) -> String {
    let mut keys = keys.to_vec();
    keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    let mut out = String::new();
    let mut rest = tpl;
    'outer: while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        for (k, v) in &keys {
            if after.starts_with(k.as_str()) {
                out.push_str(v);
                rest = &after[k.len()..];
                continue 'outer;
            }
        }
        out.push('%');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// org-babel-exp-code.
fn exp_code(doc: &mut Doc, cx: &mut Cx, info: &mut Info, inline: bool, parameters: &str) -> R<String> {
    info.body = if get_str(&info.params, ":noweb").as_deref() == Some("strip-export") {
        core::strip_noweb(&info.body)
    } else if params::noweb_p(&info.params, ":export") {
        expand_in_reference(doc, cx, info, ":export")?
    } else {
        info.body.clone()
    };
    let tpl = if inline {
        options::string("org-babel-exp-inline-code-template", "src_%lang[%switches%header-args]{%body}")
    } else {
        options::string("org-babel-exp-code-template", "#+begin_src %lang%switches%header-args\n%body\n#+end_src")
    };
    let mut keys = vec![
        ("lang".to_owned(), info.lang.clone()),
        ("body".to_owned(), if inline { info.body.clone() } else { core::escape(&info.body) }),
        ("switches".to_owned(), if info.switches.trim().is_empty() { String::new() } else { format!(" {}", info.switches) }),
        ("flags".to_owned(), get_str(&info.params, ":flags").map(|f| format!(" {f}")).unwrap_or_default()),
        ("header-args".to_owned(), if parameters.is_empty() { String::new() } else { format!(" {parameters}") }),
    ];
    for (k, v) in &info.params {
        keys.push((k[1..].to_owned(), value::prin1(v)));
    }
    keys.push(("name".to_owned(), info.name.clone().unwrap_or_default()));
    Ok(fill_template(&tpl, &keys))
}

fn expand_in_reference(doc: &mut Doc, cx: &mut Cx, info: &Info, context: &str) -> R<String> {
    match cx.reference.take() {
        Some(mut r) => {
            let out = core::expand_noweb(&mut r, cx, info, context);
            cx.reference = Some(r);
            out
        }
        None => core::expand_noweb(doc, cx, info, context),
    }
}

/// org-babel-exp-results: evaluate for export (results into DOC).
fn exp_results(doc: &mut Doc, cx: &mut Cx, info: &Info, kind: ExecKind, silent: bool, hash: Option<&str>) -> R<()> {
    if let (Some(h), Some(start)) = (hash, info.start)
        && core::current_result_hash(doc, cx, start)?.as_deref() == Some(h)
    {
        return Ok(());
    }
    if !super::lang::executable(&info.lang) {
        cx.message(format!("org-export: No org-babel-execute function for {}.  Not updating exported results.", info.lang));
        return Ok(());
    }
    let mut i = info.clone();
    if params::noweb_p(&i.params, ":eval") {
        i.body = expand_in_reference(doc, cx, info, ":eval")?;
    }
    let extra = vec![(":results".to_owned(), Sexp::Str(if silent { "silent" } else { "replace" }.into()))];
    i.params = params::merge(&[&i.params, &extra])?;
    i.params = core::process_params(doc, cx, std::mem::take(&mut i.params))?;
    core::execute_src_block(doc, cx, false, Some(i), &[], kind)?;
    Ok(())
}

/// org-babel-exp-do-export: the replacement text (None: leave as is).
fn do_export(doc: &mut Doc, cx: &mut Cx, mut info: Info, kind: ExecKind, hash: Option<String>, parameters: &str) -> R<Option<String>> {
    let inline = matches!(kind, ExecKind::Inline | ExecKind::InlineCall);
    let silently = |doc: &mut Doc, cx: &mut Cx, info: &Info| -> R<()> {
        if get_str(&info.params, ":session").as_deref() != Some("none") {
            exp_results(doc, cx, info, kind, true, None)?;
        }
        Ok(())
    };
    let clean = |doc: &mut Doc, cx: &mut Cx, info: &Info| -> R<()> {
        if let Some(s) = info.start {
            if inline {
                core::remove_inline_result(doc, s);
            } else {
                core::remove_result(doc, cx, s, false)?;
            }
        }
        Ok(())
    };
    Ok(match get_str(&info.params, ":exports").unwrap_or_else(|| "code".into()).as_str() {
        "none" => {
            silently(doc, cx, &info)?;
            clean(doc, cx, &info)?;
            Some(String::new())
        }
        "code" => {
            silently(doc, cx, &info)?;
            clean(doc, cx, &info)?;
            Some(exp_code(doc, cx, &mut info, inline, parameters)?)
        }
        "results" => {
            exp_results(doc, cx, &info, kind, false, hash.as_deref())?;
            Some(String::new())
        }
        "both" => {
            exp_results(doc, cx, &info, kind, false, hash.as_deref())?;
            Some(exp_code(doc, cx, &mut info, inline, parameters)?)
        }
        other => {
            cx.message(format!("Unknown value of src block parameter :exports {}", value::prin1(&Sexp::Str(other.into()))));
            None
        }
    })
}

/// org-babel-exp-process-buffer on DOC (the export copy).
pub fn process_buffer(doc: &mut Doc, cx: &mut Cx) -> R<()> {
    if !options::bool("org-export-use-babel", true) {
        return Ok(());
    }
    cx.export = true;
    cx.reference = Some(doc.clone());
    let out = process(doc, cx);
    cx.reference = None;
    out
}

fn process(doc: &mut Doc, cx: &mut Cx) -> R<()> {
    let mut l = 0;
    let mut col = 0;
    while l < doc.len() {
        let line = doc.line(l).to_owned();
        let t = line.trim_start().to_ascii_lowercase();
        let block = col == 0 && (t.starts_with("#+begin_src") || t.starts_with("#+call:"));
        let el = doc::element_at(&doc.lines, l);
        let skip = core::in_commented_heading(doc, l) || core::in_archived_heading(doc, l);
        if block && !skip && let Some(el) = el.clone().filter(|e| e.post == l && matches!(e.kind, Kind::Src | Kind::Call)) {
            let next = if el.kind == Kind::Src { process_block(doc, cx, &el)? } else { process_call(doc, cx, &el)? };
            l = next;
            col = 0;
            continue;
        }
        // Inline objects from COL on.
        if !skip && el.as_ref().is_some_and(|e| doc::holds_objects(&e.kind) && l < e.stop) {
            let obj = doc::inlines(l, &line).into_iter().find(|o| o.start >= col);
            if let Some(o) = obj {
                let Some(Context::InlineSrc(_, _) | Context::InlineCall(_, _)) = core::context(doc, (l, o.start)) else {
                    col = o.end;
                    continue;
                };
                col = process_inline(doc, cx, &o)?;
                continue;
            }
        }
        l += 1;
        col = 0;
    }
    Ok(())
}

/// A src block: the line to continue from.
fn process_block(doc: &mut Doc, cx: &mut Cx, el: &doc::El) -> R<usize> {
    let begin = el.begin;
    let mut info = core::src_info(doc, cx, el, false)?;
    let parameters = core::parse_begin(doc.line(el.post)).2;
    let ind = value::indentation(doc.line(el.post));
    let hash = if matches!(get_str(&info.params, ":exports").as_deref(), Some("both") | Some("results")) {
        Some(core::sha1_hash(doc, cx, &info, ":export")?)
    } else {
        None
    };
    info.start = Some((el.post, 0));
    let mark = doc.mark((el.post, 0));
    let rep = do_export(doc, cx, info, ExecKind::Src, hash, &parameters)?;
    let post = doc.marker(mark).0;
    doc.unmark(mark);
    let Some(el) = doc::element_at(&doc.lines, post) else { return Ok(post + 1) };
    match rep {
        None => Ok(el.end.max(post + 1)),
        Some(r) if r.is_empty() => {
            let end = if el.end >= doc.len() { el.stop } else { el.end };
            let begin = begin.min(el.begin);
            doc.splice(begin, end - begin, vec![]);
            Ok(begin)
        }
        Some(r) => {
            let preserve = el.post < doc.len() && core::parse_begin(doc.line(el.post)).1.split_whitespace().any(|s| s == "-i");
            let pad = " ".repeat(ind);
            let mut lines: Vec<String> = r.split('\n').map(str::to_owned).collect();
            let n = lines.len();
            for (i, x) in lines.iter_mut().enumerate() {
                if (!preserve || i == 0 || i + 1 == n) && !x.is_empty() {
                    x.insert_str(0, &pad);
                }
            }
            let old: Vec<String> = doc.lines[el.post..el.stop].to_vec();
            if old != lines {
                doc.splice(el.post, el.stop - el.post, lines.clone());
            }
            Ok(el.post + lines.len())
        }
    }
}

fn process_call(doc: &mut Doc, cx: &mut Cx, el: &doc::El) -> R<usize> {
    let Some(info) = core::call_info(doc, cx, el, false)? else {
        let v = doc.line(el.post).split_once(':').map_or("", |(_, v)| v.trim()).to_owned();
        return Err(format!("Unknown Babel reference: {}", core::parse_call(&v).0).into());
    };
    let mark = doc.mark((el.post, 0));
    do_export(doc, cx, info, ExecKind::Call, None, "")?;
    let post = doc.marker(mark).0;
    doc.unmark(mark);
    let rep = fill_template(&options::string("org-babel-exp-call-line-template", ""), &[("line".into(), doc.line(post).trim().to_owned())]);
    let Some(e) = doc::element_at(&doc.lines, post) else { return Ok(post + 1) };
    if rep.is_empty() {
        let end = if e.end >= doc.len() { e.stop } else { e.end };
        doc.splice(e.begin, end - e.begin, vec![]);
        Ok(e.begin)
    } else {
        doc.splice(e.post, e.stop - e.post, rep.split('\n').map(str::to_owned).collect());
        Ok(e.post + rep.split('\n').count())
    }
}

/// An inline object: the column to continue from on its line.
fn process_inline(doc: &mut Doc, cx: &mut Cx, o: &doc::Inline) -> R<usize> {
    let (l, start) = (o.line, o.start);
    if o.call {
        let Some(info) = core::inline_call_info(doc, cx, o, false)? else { return Err(format!("Unknown Babel reference: {}", o.name).into()) };
        do_export(doc, cx, info, ExecKind::InlineCall, None, "")?;
        let rep = fill_template(&options::string("org-babel-exp-call-line-template", ""), &[("line".into(), doc.line(l)[o.start..o.end].to_owned())]);
        let line = doc.line(l).to_owned();
        if rep.is_empty() {
            let rest = line[o.end..].trim_start_matches([' ', '\t']);
            doc.set(l, format!("{}{rest}", &line[..start]));
            return Ok(start);
        }
        doc.set(l, format!("{}{rep}{}", &line[..start], &line[o.end..]));
        return Ok(start + rep.len());
    }
    let Some(Context::InlineSrc(obj, _)) = core::context(doc, (l, start)) else { return Ok(o.end) };
    let mut info = core::get_info(doc, cx, (l, start), false)?.ok_or("No inline block")?;
    if get_str(&info.params, ":noweb").as_deref() == Some("yes") {
        info.body = expand_in_reference(doc, cx, &info, ":export")?;
    }
    let rep = do_export(doc, cx, info, ExecKind::Inline, None, obj.inside.as_deref().unwrap_or(""))?;
    let line = doc.line(l).to_owned();
    let end = obj.end.min(line.len());
    Ok(match rep {
        Some(r) if r.is_empty() => {
            let rest = line[end..].trim_start_matches([' ', '\t']);
            doc.set(l, format!("{}{rest}", &line[..start]));
            start
        }
        None => end,
        Some(r) if r != line[start..end] => {
            doc.set(l, format!("{}{r}{}", &line[..start], &line[end..]));
            start + r.len()
        }
        Some(_) => end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        let k = vec![("lang".to_owned(), "sh".to_owned()), ("language".to_owned(), "x".to_owned())];
        assert_eq!(fill_template("%lang %language %other", &k), "sh x %other");
    }
}
