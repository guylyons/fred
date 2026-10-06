//! `#+INCLUDE` expansion (org-export-expand-include-keyword and its
//! helpers) on text.

use super::macros::in_commented_heading;
use crate::org::element::{self, NodeId, Tree, Type};
use crate::org::syntax::Settings;
use regex::{Regex, RegexBuilder};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// org-export-parse-include-value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IncludeParams {
    pub file: Option<String>,
    pub location: Option<String>,
    pub only_contents: bool,
    pub lines: Option<String>,
    /// `literal` for example, export and src blocks.
    pub literal: bool,
    pub minlevel: Option<usize>,
    pub args: Option<String>,
    pub block: Option<String>,
}

fn re(s: &str) -> Regex {
    RegexBuilder::new(s).case_insensitive(true).build().unwrap()
}

/// org-url-p.
fn url_p(s: &str) -> bool {
    static R: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-zA-Z][-+.a-zA-Z0-9]*://").unwrap());
    R.is_match(s)
}

/// org-strip-quotes.
fn strip_quotes(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// expand-file-name of F relative to DIR.
fn expand(f: &str, dir: Option<&Path>) -> String {
    let p = crate::org::options::expand(f);
    let p = if p.is_absolute() {
        p
    } else {
        dir.map(|d| d.join(&p))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join(&p))
    };
    normalize(&p).to_string_lossy().into_owned()
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// org-export-parse-include-value.
pub fn parse_include_value(
    value: &str,
    dir: Option<&Path>,
    induced: Option<usize>,
) -> IncludeParams {
    let mut v = value.to_owned();
    let mut p = IncludeParams::default();
    let coding = re(r":coding +(\S+)>");
    if let Some(m) = coding.find(&v) {
        v.replace_range(m.range(), "");
    }
    let file_re = Regex::new(r#"^("[^\n]+?"|\S+)(?:\s+|$)"#).unwrap();
    if let Some(c) = file_re.captures(&v) {
        let whole = c.get(0).unwrap().range();
        let mut matched = c[1].to_owned();
        v.replace_range(whole, "");
        let loc = Regex::new(r#"(::(.*?))"?$"#).unwrap();
        if let Some(c2) = loc.captures(&matched) {
            p.location = Some(c2[2].to_owned());
            let g = c2.get(1).unwrap().range();
            matched.replace_range(g, "");
        }
        let stripped = strip_quotes(&matched).to_owned();
        p.file = Some(if url_p(&stripped) {
            stripped
        } else {
            expand(&stripped, dir)
        });
    }
    let only = re(r":only-contents *([^: \r\t\n]\S*)?");
    if let Some(c) = only.captures(&v) {
        p.only_contents = c.get(1).is_some_and(|m| m.as_str() != "nil");
        let r = c.get(0).unwrap().range();
        v.replace_range(r, "");
    }
    let lines = re(r#":lines +"([0-9]*-[0-9]*)""#);
    if let Some(c) = lines.captures(&v) {
        p.lines = Some(c[1].to_owned());
        let r = c.get(0).unwrap().range();
        v.replace_range(r, "");
    }
    // The env cond: its last match gives the block arguments.
    let mut args_group: Option<std::ops::Range<usize>> = None;
    if re(r"\bexample\b").is_match(&v) {
        p.literal = true;
    } else if let Some(c) = re(r"\bexport(?: +(.*))?").captures(&v) {
        p.literal = true;
        args_group = c.get(1).map(|m| m.range());
    } else if let Some(c) = re(r"\bsrc(?: +(.*))?").captures(&v) {
        p.literal = true;
        args_group = c.get(1).map(|m| m.range());
    }
    if !p.literal {
        let ml = re(r":minlevel +([0-9]+)");
        if let Some(c) = ml.captures(&v) {
            p.minlevel = c[1].parse().ok();
            let r = c.get(0).unwrap().range();
            v.replace_range(r, "");
        } else {
            p.minlevel = induced;
        }
    }
    if p.literal
        && let Some(r) = args_group
    {
        p.args = Some(v[r.clone()].to_owned());
        v.replace_range(r, "");
    }
    let quoted = Regex::new(r#""(\S+)""#).unwrap();
    let word = Regex::new(r"\b(\S+)\b").unwrap();
    let c = quoted.captures(&v).or_else(|| word.captures(&v));
    if let Some(c) = c {
        let start = c.get(0).unwrap().start();
        if start == 0 || v.as_bytes()[start - 1] != b':' {
            p.block = Some(c[1].to_owned());
        }
    }
    p
}

/// org-file-contents.
fn file_contents(file: &str) -> Result<String, String> {
    if url_p(file) {
        return Err(format!("Cannot include remote file {file}"));
    }
    std::fs::read_to_string(file).map_err(|_| format!("Cannot include file {file}"))
}

/// The byte range of lines "BEG-END" (1-based, END excluded; 0 = edge).
fn line_range(text: &str, lines: &str) -> (usize, usize) {
    let (a, b) = lines.split_once('-').unwrap_or((lines, ""));
    let lbeg: usize = a.parse().unwrap_or(0);
    let lend: usize = b.parse().unwrap_or(0);
    let line_start = |n: usize| -> usize {
        // forward-line (N-1) from point-min.
        let mut pos = 0;
        for _ in 1..n {
            match text[pos..].find('\n') {
                Some(i) => pos += i + 1,
                None => return text.len(),
            }
        }
        pos
    };
    let beg = if lbeg == 0 { 0 } else { line_start(lbeg) };
    let end = if lend == 0 {
        text.len()
    } else {
        line_start(lend)
    };
    (beg, end.max(beg))
}

fn reduced_level(level: usize) -> usize {
    if crate::org::options::bool("org-odd-levels-only", false) {
        level.div_ceil(2)
    } else {
        level
    }
}

/// State shared by nested inclusions.
pub struct Includer {
    /// org-export--blindly-expand-include's FILE-PREFIX.
    prefixes: Vec<String>,
    /// Footnote definitions outside included line ranges.
    pub footnotes: Vec<(String, String)>,
    pub expand_env: bool,
}

/// org-export-expand-include-keyword on TEXT.  INCLUDED lists (file,
/// lines) being included, DIR resolves relative names, INCLUDER is the
/// file the text belongs to.
pub fn expand_include_keyword(
    text: &str,
    included: &[(String, Option<String>)],
    dir: Option<&Path>,
    includer_file: Option<&Path>,
    state: &mut Includer,
) -> Result<String, String> {
    let include_re = re(r"(?m)^[ \t]*#\+INCLUDE:");
    if !include_re.is_match(text) {
        return Ok(text.to_owned());
    }
    let st = Settings::default();
    // Induced levels of the include lines, before any expansion.
    let mut s = text.to_owned();
    let mut lines: Vec<(usize, usize)> = vec![];
    {
        let mut level = 0;
        let mut pos = 0;
        for l in text.split_inclusive('\n') {
            let stars = l.bytes().take_while(|&b| b == b'*').count();
            if stars > 0
                && l.as_bytes()
                    .get(stars)
                    .is_some_and(|&b| b == b' ' || b == b'\t' || b == b'\n')
            {
                level = stars;
            }
            if include_re.is_match(l) {
                lines.push((pos, 1 + reduced_level(level)));
            }
            pos += l.len();
        }
    }
    let mut shift: isize = 0;
    let top = included.is_empty();
    for (orig, induced) in lines {
        let pos = (orig as isize + shift) as usize;
        let tree = element::parse_buffer(&s, &st);
        let Some(el) = element::at_point(&s, &tree, pos) else {
            continue;
        };
        if in_commented_heading(&tree, el) || tree.kind(el) != Type::Keyword {
            continue;
        }
        let eol = s[pos..].find('\n').map_or(s.len(), |i| pos + i + 1);
        let line = s[pos..eol].to_owned();
        let indentation = line.len() - line.trim_start_matches([' ', '\t']).len();
        let value = tree.str(el, "value").unwrap_or("").to_owned();
        let mut params = parse_include_value(&value, dir, Some(induced));
        if state.expand_env
            && let Some(f) = &params.file
        {
            params.file = Some(substitute_env(f));
        }
        let insert = match params.file.clone() {
            None => String::new(),
            Some(f) if !url_p(&f) && std::fs::metadata(&f).is_err() => {
                return Err(format!("Cannot include file {f}"));
            }
            Some(f) if included.iter().any(|(x, l)| *x == f && *l == params.lines) => {
                return Err(format!("Recursive file inclusion: {f}"));
            }
            Some(f) => blindly_expand(&params, &f, includer_file, included, indentation, state)?,
        };
        s.replace_range(pos..eol, &insert);
        shift += insert.len() as isize - (eol - pos) as isize;
    }
    if top && !state.footnotes.is_empty() {
        for (k, v) in &state.footnotes {
            s.push_str(&format!("\n[fn:{k}] {v}\n"));
        }
    }
    Ok(s)
}

/// substitute-env-in-file-name.
fn substitute_env(f: &str) -> String {
    static VAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))").unwrap()
    });
    VAR.replace_all(f, |c: &regex::Captures| {
        let name = c.get(1).or(c.get(2)).unwrap().as_str();
        std::env::var(name).unwrap_or_else(|_| c[0].to_owned())
    })
    .into_owned()
}

/// org-export--blindly-expand-include.
fn blindly_expand(
    p: &IncludeParams,
    file: &str,
    includer: Option<&Path>,
    included: &[(String, Option<String>)],
    ind: usize,
    state: &mut Includer,
) -> Result<String, String> {
    let ind_str = " ".repeat(ind);
    let block = p.block.clone().unwrap_or_default();
    if p.literal {
        let arg = p.args.as_ref().map(|a| format!(" {a}")).unwrap_or_default();
        let contents = element::escape_code(&prepare_file_contents(
            file,
            p.lines.as_deref(),
            None,
            None,
            None,
            state,
            None,
        )?);
        return Ok(format!(
            "{ind_str}#+BEGIN_{block}{arg}\n{contents}{ind_str}#+END_{block}\n"
        ));
    }
    if p.block.is_some() {
        let contents =
            prepare_file_contents(file, p.lines.as_deref(), None, None, None, state, None)?;
        return Ok(format!(
            "{ind_str}#+BEGIN_{block}\n{contents}{ind_str}#+END_{block}\n"
        ));
    }
    let lines = match &p.location {
        Some(loc) => Some(inclusion_absolute_lines(
            file,
            loc,
            p.only_contents,
            p.lines.as_deref(),
        )?),
        None => p.lines.clone(),
    };
    let id = match state.prefixes.iter().position(|f| f == file) {
        Some(i) => i,
        None => {
            state.prefixes.push(file.to_owned());
            state.prefixes.len() - 1
        }
    };
    let contents = prepare_file_contents(
        file,
        lines.as_deref(),
        Some(ind),
        p.minlevel,
        Some(id),
        state,
        includer,
    )?;
    let mut inc = included.to_vec();
    inc.push((file.to_owned(), p.lines.clone()));
    let fdir = if url_p(file) {
        None
    } else {
        Path::new(file).parent().map(Path::to_path_buf)
    };
    expand_include_keyword(&contents, &inc, fdir.as_deref(), includer, state)
}

/// org-export--inclusion-absolute-lines: "START-END" of the element at
/// LOCATION in FILE (its contents with ONLY_CONTENTS), within LINES.
pub fn inclusion_absolute_lines(
    file: &str,
    location: &str,
    only_contents: bool,
    lines: Option<&str>,
) -> Result<String, String> {
    let text = file_contents(file)?;
    let st = crate::org::settings_of(&text, Path::new(file).parent());
    let tree = element::parse_buffer(&text, &st);
    let Some(target) = link_search(&tree, location) else {
        return Err(format!(
            "No match for fuzzy expression: {location} for {file}::{location}"
        ));
    };
    let el =
        element::at_point(&text, &tree, target).ok_or_else(|| format!("{file}::{location}"))?;
    let cb = if only_contents {
        tree.contents_begin(el)
    } else {
        None
    };
    let mut beg = cb.unwrap_or(tree.begin(el).unwrap_or(0));
    let end = if cb.is_some() {
        tree.contents_end(el).unwrap_or(text.len())
    } else {
        tree.end(el).unwrap_or(text.len())
    };
    if only_contents && tree.is(el, &[Type::Headline, Type::Inlinetask]) {
        let line_end = |p: usize| text[p..].find('\n').map_or(text.len(), |i| p + i + 1);
        let first = &text[beg..line_end(beg).min(end)];
        let t = first.trim_start();
        if ["SCHEDULED:", "DEADLINE:", "CLOSED:"]
            .iter()
            .any(|k| t.contains(k))
        {
            beg = line_end(beg);
        }
        if text[beg..end]
            .trim_start()
            .to_ascii_uppercase()
            .starts_with(":PROPERTIES:")
            && let Some(i) = text[beg..end].to_ascii_uppercase().find(":END:")
        {
            beg = line_end(beg + i);
        }
    }
    let mut region = &text[beg..end];
    let mut base = beg;
    if let Some(l) = lines {
        let skip = region.len() - region.trim_start_matches([' ', '\t', '\n', '\r']).len();
        let bol = region[..skip].rfind('\n').map_or(0, |i| i + 1);
        region = &region[bol..];
        base += bol;
        let (a, b) = line_range(region, l);
        base += a;
        region = &region[a..b];
    }
    let start_line = text[..base].matches('\n').count() + 1;
    let mut count = 0;
    let mut p = 0;
    while p < region.len() {
        count += 1;
        p = region[p..].find('\n').map_or(region.len(), |i| p + i + 1);
    }
    Ok(format!("{}-{}", start_line, start_line + count))
}

/// A simple org-link-search in TREE: `*Heading`, `#custom-id`, a target,
/// a named element, or a heading title.  The byte position found.
fn link_search(tree: &Tree, s: &str) -> Option<usize> {
    let norm = |x: &str| x.split_whitespace().collect::<Vec<_>>().join(" ");
    let find = |types: &[Type], pred: &dyn Fn(NodeId) -> bool| -> Option<usize> {
        tree.map(tree.root, Some(types), |_, n| {
            pred(n).then(|| tree.begin(n)).flatten()
        })
        .into_iter()
        .next()
    };
    if let Some(h) = s.strip_prefix('*') {
        let h = norm(h);
        return find(&[Type::Headline], &|n| {
            norm(tree.str(n, "raw-value").unwrap_or("")) == h
        });
    }
    if let Some(id) = s.strip_prefix('#') {
        return find(&[Type::Headline], &|n| tree.str(n, "CUSTOM_ID") == Some(id));
    }
    let want = norm(s);
    find(&[Type::Target], &|n| {
        norm(tree.str(n, "value").unwrap_or("")).eq_ignore_ascii_case(&want)
    })
    .or_else(|| {
        let mut ty = element::ALL_ELEMENTS.to_vec();
        ty.retain(|t| *t != Type::Headline);
        find(&ty, &|n| {
            tree.str(n, "name")
                .is_some_and(|x| x.eq_ignore_ascii_case(&want))
        })
    })
    .or_else(|| {
        find(&[Type::Headline], &|n| {
            norm(tree.str(n, "raw-value").unwrap_or("")).eq_ignore_ascii_case(&want)
        })
    })
}

/// org-export--prepare-file-contents.
pub fn prepare_file_contents(
    file: &str,
    lines: Option<&str>,
    ind: Option<usize>,
    minlevel: Option<usize>,
    id: Option<usize>,
    state: &mut Includer,
    includer: Option<&Path>,
) -> Result<String, String> {
    let full = file_contents(file)?;
    let (nb, ne) = match lines {
        Some(l) => line_range(&full, l),
        None => (0, full.len()),
    };
    let mut s = full[nb..ne].to_owned();
    // Relative file links follow the includer's directory.
    if let Some(inc) = includer {
        let fdir = Path::new(file)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let idir = inc.parent().map(Path::to_path_buf).unwrap_or_default();
        if normalize(&fdir) != normalize(&idir) {
            s = update_included_links(&s, &fdir, &idir);
        }
    }
    // Blank lines around the contents go.
    let skip = s.len() - s.trim_start_matches([' ', '\t', '\n', '\r']).len();
    let bol = s[..skip].rfind('\n').map_or(0, |i| i + 1);
    s.drain(..bol);
    let kept = s.trim_end_matches([' ', '\r', '\t', '\n']).len();
    let end = s[kept..].find('\n').map_or(s.len(), |i| kept + i + 1);
    s.truncate(end);
    // Indentation up to the first headline.
    if let Some(ind) = ind.filter(|&i| i > 0) {
        let pad = " ".repeat(ind);
        let fn_def = Regex::new(r"^\[fn:([-_[:word:]]+)\]").unwrap();
        let mut out = String::new();
        let mut in_body = true;
        for l in s.split_inclusive('\n') {
            if in_body && headline_line(l) {
                in_body = false;
            }
            if in_body && !fn_def.is_match(l) {
                out.push_str(&pad);
            }
            out.push_str(l);
        }
        if in_body && !s.ends_with('\n') && s.is_empty() {
            out.push_str(&pad);
        }
        s = out;
    }
    // Minimal level.
    if let Some(minlevel) = minlevel {
        let levels: Vec<usize> = s
            .lines()
            .filter(|l| headline_line(l))
            .map(|l| reduced_level(l.bytes().take_while(|&b| b == b'*').count()))
            .collect();
        if let Some(&min) = levels.iter().min() {
            let mut offset = minlevel as isize - min as isize;
            if offset != 0 {
                if crate::org::options::bool("org-odd-levels-only", false) {
                    offset *= 2;
                }
                let mut out = String::new();
                for l in s.split_inclusive('\n') {
                    if headline_line(l) {
                        if offset < 0 {
                            out.push_str(&l[(-offset) as usize..]);
                        } else {
                            out.push_str(&"*".repeat(offset as usize));
                            out.push_str(l);
                        }
                    } else {
                        out.push_str(l);
                    }
                }
                s = out;
            }
        }
    }
    // File-specific footnote labels.
    if let Some(id) = id {
        s = relabel_footnotes(&s, &full, nb, ne, id, state);
    }
    Ok(element::normalize_string(&s, false))
}

fn headline_line(l: &str) -> bool {
    let stars = l.bytes().take_while(|&b| b == b'*').count();
    stars > 0
        && matches!(
            l.as_bytes().get(stars),
            Some(b' ' | b'\t' | b'\n' | b'\r') | None
        )
}

/// Rename every footnote label of S to `-ID-LABEL`; definitions outside
/// the included range [NB, NE) of FULL are kept in STATE.
fn relabel_footnotes(
    s: &str,
    full: &str,
    nb: usize,
    ne: usize,
    id: usize,
    state: &mut Includer,
) -> String {
    let st = Settings::default();
    let tree = element::parse_buffer(s, &st);
    let mut spans: Vec<(usize, String)> = tree.map(
        tree.root,
        Some(&[Type::FootnoteDefinition, Type::FootnoteReference]),
        |t, f| Some((t.begin(f)? + 4, t.str(f, "label")?.to_owned())),
    );
    spans.sort();
    let wide = element::parse_buffer(full, &st);
    let mut seen: Vec<String> = vec![];
    let mut out = s.to_owned();
    for (pos, label) in spans.iter().rev() {
        let new = format!("-{id}-{label}");
        if out[*pos..].starts_with(label.as_str()) {
            out.replace_range(*pos..pos + label.len(), &new);
        }
        if !seen.contains(label) {
            seen.push(label.clone());
            let def = wide.map(wide.root, Some(&[Type::FootnoteDefinition]), |t, d| {
                (t.str(d, "label") == Some(label.as_str())).then_some(d)
            });
            if let Some(&d) = def.first() {
                let b = wide.begin(d).unwrap_or(0);
                if b < nb || b >= ne {
                    let (cb, ce) = (
                        wide.contents_begin(d).unwrap_or(b),
                        wide.contents_end(d).unwrap_or(b),
                    );
                    let body = element::normalize_string(&full[cb..ce], false);
                    if !state.footnotes.iter().any(|(k, _)| *k == new) {
                        state.footnotes.push((new.clone(), body));
                    }
                }
            }
        }
    }
    out
}

/// org-export--update-included-link on every relative file link of S.
fn update_included_links(s: &str, file_dir: &Path, includer_dir: &Path) -> String {
    let st = Settings::default();
    let tree = element::parse_buffer(s, &st);
    let mut links: Vec<(usize, usize, String)> =
        tree.map(tree.root, Some(&[Type::Link]), |t, l| {
            if t.str(l, "type") != Some("file") {
                return None;
            }
            let path = t.str(l, "path")?;
            if path.starts_with('/') || path.starts_with('~') || path.starts_with("/ssh:") {
                return None;
            }
            let abs = normalize(&file_dir.join(path));
            let new = relative(&abs, includer_dir);
            let mut t2 = t.clone();
            t2.put(l, "path", new);
            let contents = match (t.contents_begin(l), t.contents_end(l)) {
                (Some(a), Some(b)) => Some(s[a..b].to_owned()),
                _ => None,
            };
            let text = element::expand(&t2, l, contents, false);
            Some((t.begin(l)?, t.end(l)? - t.post_blank(l), text))
        });
    links.sort_by_key(|x| x.0);
    let mut out = s.to_owned();
    for (b, e, text) in links.into_iter().rev() {
        out.replace_range(b..e, &text);
    }
    out
}

/// file-relative-name.
fn relative(path: &Path, base: &Path) -> String {
    let (pn, bn) = (normalize(path), normalize(base));
    let p: Vec<_> = pn.components().collect();
    let b: Vec<_> = bn.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out.to_string_lossy().into_owned()
}

impl Includer {
    pub fn new(expand_env: bool) -> Includer {
        Includer {
            prefixes: vec![],
            footnotes: vec![],
            expand_env,
        }
    }
}
