//! ob-tangle.el: writing blocks to files, link comments, detangling and
//! jumping back from tangled code.

use super::core::{self, Cx, Info, R, err};
use super::params::get_str;
use super::doc::{self, Doc, Kind};
use super::lang;
use super::params::{self, get};
use super::value::{self, as_string};
use crate::org::{options, syntax};
use std::path::{Path, PathBuf};

/// Which blocks org-babel-tangle writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    /// `C-u`: the block at point.
    Block,
    /// `C-u C-u`: blocks for the target file of the block at point.
    File,
}

/// A block to write (org-babel-tangle-single-block's spec).
#[derive(Clone, Debug)]
pub struct Spec {
    pub lang: String,
    pub start_line: usize,
    pub file: String,
    pub link: Option<String>,
    pub source_name: String,
    pub params: params::Params,
    pub body: String,
    pub comment: Option<String>,
}

fn source_file(doc: &Doc) -> String {
    let p = doc.path.clone().unwrap_or_default();
    if options::bool("org-babel-tangle-use-relative-file-links", true) {
        core::relative(&p, &std::env::current_dir().unwrap_or_default())
    } else {
        p.display().to_string()
    }
}

/// abbreviate-file-name.
fn abbreviate(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&format!("{h}/")) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

/// org-link-heading-search-string for heading `h`.
fn heading_search(doc: &Doc, h: usize) -> String {
    let st = syntax::Settings::default();
    let line = doc.line(h);
    let title = syntax::headline(line, &st).map(|hl| hl.title(line).to_owned()).unwrap_or_default();
    let re = regex::Regex::new(r"\[[0-9]*%\]|\[[0-9]*/[0-9]*\]").unwrap();
    let t = re.replace_all(&title, "");
    format!("*{}", t.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', ""))
}

/// org-babel-tangle--unbracketed-link: `file:PATH::SEARCH` for the block.
pub fn unbracketed_link(doc: &Doc, info: &Info) -> Option<String> {
    if get_str(&info.params, ":comments").as_deref() == Some("no") {
        return None;
    }
    let path = doc.path.as_ref()?;
    let line = info.start.map_or(0, |s| s.0);
    let search = if let Some(n) = &info.name {
        Some(n.clone())
    } else {
        match syntax::heading_at_or_before(&doc.lines, line.min(doc.len().saturating_sub(1))) {
            None => {
                let s = doc.line(line).split_whitespace().collect::<Vec<_>>().join(" ");
                (!s.is_empty()).then_some(s)
            }
            Some(h) => {
                let (_, ids) = core::local_values(doc, Some(h), "CUSTOM_ID");
                Some(ids.first().map(|id| format!("#{id}")).unwrap_or_else(|| heading_search(doc, h)))
            }
        }
    };
    let bare = format!("file:{}{}", abbreviate(path), search.map(|s| format!("::{s}")).unwrap_or_default());
    if options::bool("org-babel-tangle-use-relative-file-links", true)
        && let Some(Sexp::Str(t)) = get(&info.params, ":tangle").cloned()
    {
        let target_dir = Path::new(&t).parent().map(|d| doc.dir().join(d)).unwrap_or_else(|| doc.dir());
        let rest = &bare[5..];
        let (file, search) = rest.split_once("::").map_or((rest, None), |(f, s)| (f, Some(s)));
        let abs = options::expand(file);
        return Some(format!("file:{}{}", core::relative(&abs, &target_dir), search.map(|s| format!("::{s}")).unwrap_or_default()));
    }
    Some(bare)
}

use crate::org::sexp::Sexp;

/// org-fill-template with link data.
fn fill(tpl: &str, start: usize, file: &str, link: &str, source: &str) -> String {
    tpl.replace("%start-line", &start.to_string()).replace("%file", file).replace("%link", link).replace("%source-name", source)
}

fn fmt_beg() -> String {
    options::string("org-babel-tangle-comment-format-beg", "[[%link][%source-name]]")
}

fn fmt_end() -> String {
    options::string("org-babel-tangle-comment-format-end", "%source-name ends here")
}

/// org-babel-tangle-comment-links.
pub fn comment_links(doc: &Doc, info: &Info) -> (String, String) {
    let start = info.start.map_or(1, |s| s.0 + 1);
    let file = doc.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let link = unbracketed_link(doc, info).unwrap_or_default();
    let name = info.name.clone().unwrap_or_default();
    (fill(&fmt_beg(), start, &file, &link, &name), fill(&fmt_end(), start, &file, &link, &name))
}

/// org-babel-effective-tangled-filename.
pub fn effective_filename(buffer_fn: &Path, lang: &str, tfile: &str) -> Option<PathBuf> {
    let dir = buffer_fn.parent().unwrap_or(Path::new(""));
    let base = match tfile {
        "yes" => buffer_fn.with_extension(""),
        "no" => return None,
        "" => return None,
        f => {
            let f = options::expand(f);
            if f.is_absolute() { f } else { dir.join(f) }
        }
    };
    if tfile == "yes" {
        let ext = lang::tangle_ext(lang);
        return Some(PathBuf::from(format!("{}.{ext}", base.display())));
    }
    Some(base)
}

/// org-babel-tangle--compute-targets.
fn compute_targets(buffer_fn: &Path, info: &Info) -> Vec<PathBuf> {
    let targets: Vec<String> = match get(&info.params, ":tangle") {
        Some(Sexp::Str(s)) if s == "yes" => {
            vec![effective_filename(buffer_fn, &info.lang, "yes").and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()]
        }
        Some(Sexp::Str(s)) if s == "no" => vec![],
        Some(v) if value::listp(v) => value::items(v).unwrap_or(&[]).iter().filter_map(as_string).collect(),
        Some(v) => as_string(v).into_iter().collect(),
        None => vec![],
    };
    let dirs: Vec<String> = match get(&info.params, ":tangle-directory") {
        Some(v) if value::listp(v) => value::items(v).unwrap_or(&[]).iter().filter_map(as_string).collect(),
        Some(v) => as_string(v).into_iter().collect(),
        None => vec![],
    };
    let mut files = vec![];
    for f in targets {
        if Path::new(&f).is_absolute() || dirs.is_empty() {
            files.push(f);
        } else {
            for d in &dirs {
                files.push(Path::new(d).join(&f).display().to_string());
            }
        }
    }
    let mut out: Vec<PathBuf> = vec![];
    for f in files {
        if let Some(p) = effective_filename(buffer_fn, &info.lang, &f)
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// org-babel-tangle-single-block for the block element at `el`.
fn single_block(doc: &mut Doc, cx: &mut Cx, el: &doc::El, counter: usize) -> R<(Info, Spec)> {
    let info = core::src_info(doc, cx, el, false)?;
    let p = &info.params;
    let start_line = el.post + 2;
    let link = unbracketed_link(doc, &info);
    let source_name = info.name.clone().unwrap_or_else(|| {
        let st = syntax::Settings::default();
        let h = syntax::heading_at_or_before(&doc.lines, el.post);
        let title = h
            .and_then(|h| {
                let l = doc.line(h);
                syntax::headline(l, &st).map(|hl| hl.title(l).to_owned())
            })
            .unwrap_or_else(|| "No heading".into());
        format!("{title}:{counter}")
    });
    let body = if params::noweb_p(p, ":tangle") {
        if get_str(p, ":noweb").as_deref() == Some("strip-tangle") {
            core::strip_noweb(&info.body)
        } else {
            core::expand_noweb(doc, cx, &info, ":tangle")?
        }
    } else {
        info.body.clone()
    };
    let mut expanded = if params::has(p, ":no-expand") {
        body
    } else {
        let dir = doc.dir();
        let mut env = core::Env { doc, cx, at: el.post, dir };
        core::expand_full(&mut env, &info.lang, &body, p)?
    };
    if info.switches.contains("-r")
        && let Some(f) = &info.coderef
    {
        expanded = core::strip_coderefs(&expanded, f);
    }
    let comments = get_str(p, ":comments");
    let comment = if matches!(comments.as_deref(), Some("both") | Some("org")) {
        // From the previous heading or code block end to the block.
        let heading = syntax::heading_at_or_before(&doc.lines, el.post).map(|h| (h, syntax::level(doc.line(h)).unwrap_or(0) + 1));
        let prev_end = (0..el.post)
            .rev()
            .find(|&i| doc.line(i).trim().eq_ignore_ascii_case("#+end_src") && (0..i).any(|j| doc.line(j).trim_start().to_ascii_lowercase().starts_with("#+begin_src")))
            .map(|i| (i, doc.line(i).len()));
        let from = heading.into_iter().chain(prev_end).max().unwrap_or((0, 0));
        let mut text = doc.line(from.0)[from.1.min(doc.line(from.0).len())..].to_owned();
        if from.0 < el.post {
            text.push('\n');
            for l in from.0 + 1..el.post {
                text.push_str(doc.line(l));
                text.push('\n');
            }
        }
        Some(value::remove_indentation(&text))
    } else {
        None
    };
    let preserve = info.preserve_indent;
    let spec = Spec {
        lang: info.lang.clone(),
        start_line,
        file: source_file(doc),
        link,
        source_name,
        params: info.params.clone(),
        body: if preserve { expanded } else { value::remove_indentation(&expanded) },
        comment,
    };
    Ok((info, spec))
}

/// org-babel-spec-to-string.
fn spec_to_string(spec: &Spec, out: &mut String) {
    let comments = get_str(&spec.params, ":comments");
    let link = matches!(comments.as_deref(), Some("both") | Some("link") | Some("yes") | Some("noweb"));
    let insert_comment = |text: &str, out: &mut String| {
        if comments.is_some() && comments.as_deref() != Some("no") && !text.trim().is_empty() {
            if options::bool("org-babel-tangle-uncomment-comments", false) {
                out.push_str(text);
            } else {
                out.push_str(&lang::comment_region(&spec.lang, text));
                out.push('\n');
            }
        }
    };
    if let Some(c) = &spec.comment {
        insert_comment(c, out);
    }
    let l = spec.link.clone().unwrap_or_default();
    if link {
        insert_comment(&fill(&fmt_beg(), spec.start_line, &spec.file, &l, &spec.source_name), out);
    }
    out.push_str(&spec.body);
    out.push('\n');
    if link {
        insert_comment(&fill(&fmt_end(), spec.start_line, &spec.file, &l, &spec.source_name), out);
    }
}

/// org-babel-interpret-file-mode.
pub fn interpret_file_mode(mode: &Sexp) -> R<u32> {
    match mode {
        Sexp::Int(i) => {
            let o = format!("{i:o}");
            if o.len() == 3 { Ok(*i as u32) } else { err(format!("{i:o} is not a valid file mode octal.  Did you give the decimal value {i} by mistake?")) }
        }
        Sexp::Str(s) => {
            if let Some(o) = s.strip_prefix('o')
                && (o.len() == 3 || (o.len() == 4 && o.starts_with('0')))
                && o.bytes().all(|b| (b'0'..=b'7').contains(&b))
            {
                return Ok(u32::from_str_radix(o, 8).unwrap_or(0o644));
            }
            if s.len() == 9 && regex::Regex::new("^[r-][w-][xs-][r-][w-][xs-][r-][w-][x-]$").unwrap().is_match(s) {
                let mut m = 0;
                for (i, c) in s.chars().enumerate() {
                    if c != '-' {
                        m |= 1 << (8 - i);
                    }
                }
                return Ok(m);
            }
            if regex::Regex::new(r"^[ugoa]*(?:[+=-][rwxXstugo]*)+(,[ugoa]*(?:[+=-][rwxXstugo]*)+)*$").unwrap().is_match(s) {
                return Ok(symbolic_mode(s, options::int("org-babel-tangle-default-file-mode", 0o644) as u32));
            }
            err(format!("File mode {} not recognized as a valid format.  See `org-babel-interpret-file-mode'", value::prin1(mode)))
        }
        m => err(format!("File mode {} not recognized as a valid format", value::prin1(m))),
    }
}

/// file-modes-symbolic-to-number (rwx and ugoa only).
fn symbolic_mode(s: &str, base: u32) -> u32 {
    let mut mode = base;
    for clause in s.split(',') {
        let who_end = clause.find(['+', '-', '=']).unwrap_or(clause.len());
        let who = &clause[..who_end];
        let mask = if who.is_empty() || who.contains('a') {
            0o777
        } else {
            let mut m = 0;
            if who.contains('u') {
                m |= 0o700;
            }
            if who.contains('g') {
                m |= 0o070;
            }
            if who.contains('o') {
                m |= 0o007;
            }
            m
        };
        let mut rest = &clause[who_end..];
        while let Some(op) = rest.chars().next() {
            let perms_end = rest[1..].find(['+', '-', '=']).map_or(rest.len(), |i| i + 1);
            let perms = &rest[1..perms_end];
            let mut bits = 0;
            if perms.contains('r') {
                bits |= 0o444;
            }
            if perms.contains('w') {
                bits |= 0o222;
            }
            if perms.contains('x') || (perms.contains('X') && mode & 0o111 != 0) {
                bits |= 0o111;
            }
            bits &= mask;
            match op {
                '+' => mode |= bits,
                '-' => mode &= !bits,
                _ => mode = (mode & !mask) | bits,
            }
            rest = &rest[perms_end..];
        }
    }
    mode
}

/// org-babel-tangle-collect-blocks: (file, [(lang, spec)]) in order.
fn collect(doc: &mut Doc, cx: &mut Cx, lang_re: Option<&str>, tangle_file: Option<&Path>) -> R<Vec<(PathBuf, Vec<Spec>)>> {
    let Some(buffer_fn) = doc.path.clone() else { return err("Buffer is not visiting a file") };
    let mut groups: Vec<(PathBuf, Vec<Spec>)> = vec![];
    let mut counter = 0;
    let mut last_heading = None;
    let re = lang_re.map(regex::Regex::new).transpose().map_err(|e| e.to_string())?;
    let with_archived = options::bool("org-tangle-with-archived-trees", false);
    for el in doc::all_elements(&doc.lines) {
        if el.kind != Kind::Src {
            continue;
        }
        let h = syntax::heading_at_or_before(&doc.lines, el.post);
        if h == last_heading {
            counter += 1;
        } else {
            counter = 1;
            last_heading = h;
        }
        if core::in_commented_heading(doc, el.post) || (!with_archived && core::in_archived_heading(doc, el.post)) {
            continue;
        }
        let (info, spec) = single_block(doc, cx, &el, counter)?;
        for f in compute_targets(&buffer_fn, &info) {
            if info.lang.is_empty() {
                continue;
            }
            if tangle_file.is_some_and(|t| t != f) {
                continue;
            }
            if let Some(re) = &re
                && !re.is_match(&info.lang)
            {
                continue;
            }
            match groups.iter_mut().find(|(g, _)| *g == f) {
                Some((_, v)) => v.push(spec.clone()),
                None => groups.push((f, vec![spec.clone()])),
            }
        }
    }
    Ok(groups)
}

/// org-babel-tangle: write the files; their names.
pub fn tangle(doc: &mut Doc, cx: &mut Cx, scope: Scope, target_file: Option<&str>, lang_re: Option<&str>) -> R<Vec<PathBuf>> {
    let Some(buffer_fn) = doc.path.clone() else { return err("Buffer is not visiting a file") };
    let point = doc.point;
    let groups = match scope {
        Scope::Block => {
            let Some(head) = core::where_is_head(doc, point.0) else { return err("Point is not in a source code block") };
            let el = doc::element_at(&doc.lines, head).ok_or("Point is not in a source code block")?;
            let (info, spec) = single_block(doc, cx, &el, 1)?;
            let info = with_target(info, target_file);
            compute_targets(&buffer_fn, &info).into_iter().map(|f| (f, vec![spec.clone()])).collect()
        }
        Scope::File => {
            let info = core::get_info(doc, cx, point, true)?.ok_or("Point is not in a source code block")?;
            let info = with_target(info, target_file);
            let t = compute_targets(&buffer_fn, &info);
            let t = t.first().cloned().ok_or("Point is not in a source code block")?;
            collect(doc, cx, lang_re, Some(&t))?
        }
        Scope::All => {
            if let Some(t) = target_file {
                set_default_target(doc, cx, t, lang_re)?
            } else {
                collect(doc, cx, lang_re, None)?
            }
        }
    };
    let mut written = vec![];
    let mut count = 0;
    for (file, specs) in groups {
        let mut out = String::new();
        let mut modes: Vec<u32> = vec![];
        let mut make_dir = false;
        let mut she_banged = false;
        for spec in &specs {
            let she_bang = get_str(&spec.params, ":shebang").filter(|s| !s.is_empty());
            let mut tm = get(&spec.params, ":tangle-mode").cloned().filter(|v| !v.is_nil());
            if she_bang.is_some() && tm.is_none() {
                tm = Some(Sexp::Int(0o755));
            }
            if let Some(m) = tm {
                let m = interpret_file_mode(&m)?;
                if !modes.contains(&m) {
                    modes.insert(0, m);
                }
            }
            if get_str(&spec.params, ":mkdirp").is_some_and(|m| m != "no") {
                make_dir = true;
            }
            if get_str(&spec.params, ":padline").as_deref() != Some("no") && !out.is_empty() {
                out.push('\n');
            }
            if let Some(sb) = &she_bang
                && !she_banged
            {
                out.push_str(sb);
                out.push('\n');
                she_banged = true;
            }
            spec_to_string(spec, &mut out);
            count += 1;
        }
        if make_dir && let Some(d) = file.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let same = std::fs::read_to_string(&file).is_ok_and(|old| old == out);
        if !same {
            if std::path::absolute(&file).ok() == std::path::absolute(&buffer_fn).ok() {
                return err("Not allowed to tangle into the same file as self");
            }
            let remove = options::string("org-babel-tangle-remove-file-before-write", "auto");
            if file.exists() {
                let readonly = std::fs::metadata(&file).is_ok_and(|m| m.permissions().readonly());
                if remove == "t" || (remove == "auto" && readonly) {
                    let _ = std::fs::remove_file(&file);
                }
            }
            std::fs::write(&file, &out).map_err(|e| format!("{}: {e}", file.display()))?;
            for m in &modes {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(*m));
            }
        }
        written.insert(0, file);
    }
    cx.message(format!(
        "Tangled {count} code block{} from {}",
        if count == 1 { "" } else { "s" },
        buffer_fn.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    Ok(written)
}

fn with_target(mut info: Info, target: Option<&str>) -> Info {
    if let Some(t) = target
        && get_str(&info.params, ":tangle").is_none_or(|x| x == "no")
    {
        params::set(&mut info.params, ":tangle", Sexp::Str(t.into()));
    }
    info
}

/// TARGET-FILE as the default `:tangle` (merged into org-babel-default-header-args).
fn set_default_target(doc: &mut Doc, cx: &mut Cx, target: &str, lang_re: Option<&str>) -> R<Vec<(PathBuf, Vec<Spec>)>> {
    params::with_default(":tangle", Sexp::Str(target.into()), || collect(doc, cx, lang_re, None))
}

/// A tangled-code position's origin (org-babel-tangle-jump-to-org).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub link: String,
    pub block_name: String,
    /// Lines of the body in the tangled file.
    pub body_start: usize,
    pub body_end: usize,
    /// Offset of point from the body start: (lines, column).
    pub offset: (usize, usize),
}

/// The `[[link][name]]` ... `name ends here` pair around point.
pub fn origin(code: &Doc) -> R<Origin> {
    let mid = code.point.0;
    let link_re = regex::Regex::new(r"\[\[([^\]]+)\](?:\[([^\]]+)\])?\]").unwrap();
    for start in (0..=mid.min(code.len().saturating_sub(1))).rev() {
        let Some(c) = link_re.captures(code.line(start)) else { continue };
        let Some(name) = c.get(2).map(|m| m.as_str().to_owned()) else { continue };
        let end_marker = format!(" {name} ends here");
        if let Some(end) = (start + 1..code.len()).find(|&l| code.line(l).contains(&end_marker))
            && start < mid
            && mid < end
        {
            return Ok(Origin {
                link: c[1].to_owned(),
                block_name: name,
                body_start: start + 1,
                body_end: end,
                offset: (mid - start - 1, code.point.1),
            });
        }
    }
    err("Not in tangled code")
}

/// The org file and the block line an origin points to, in ORG.
pub fn find_block(org: &Doc, o: &Origin) -> Option<usize> {
    let search = o.link.split_once("::").map(|(_, s)| s.to_owned());
    let blocks: Vec<usize> = doc::all_elements(&org.lines).into_iter().filter(|e| e.kind == Kind::Src).map(|e| e.post).collect();
    let n_re = regex::Regex::new(r"[^ \t\n\r]:(\d+)$").unwrap();
    if let Some(c) = n_re.captures(&o.block_name) {
        let n: usize = c[1].parse().ok()?;
        // The heading the link searches, else the buffer start.
        let start = match search.as_deref() {
            Some(s) if s.starts_with('*') => {
                let want = s[1..].trim();
                (0..org.len()).find(|&l| syntax::level(org.line(l)).is_some() && heading_search(org, l)[1..].trim() == want)
            }
            Some(s) => org.lines.iter().position(|l| l.contains(s)),
            None => None,
        };
        let from = match start {
            Some(h) => syntax::heading_at_or_before(&org.lines, h).unwrap_or(0),
            None => 0,
        };
        return blocks.iter().filter(|&&b| b >= from).nth(n.saturating_sub(1)).copied();
    }
    doc::all_elements(&org.lines).into_iter().find(|e| e.kind == Kind::Src && e.name.as_deref() == Some(o.block_name.as_str())).map(|e| e.post)
}

/// The file a link names (relative to the tangled file's directory).
pub fn link_file(code: &Doc, o: &Origin) -> PathBuf {
    let path = o.link.strip_prefix("file:").unwrap_or(&o.link);
    let path = path.split("::").next().unwrap_or(path);
    let p = options::expand(path);
    if p.is_absolute() { p } else { code.dir().join(p) }
}

/// The target line and column in ORG for point in CODE.
pub fn jump_position(org: &Doc, o: &Origin, block: usize) -> (usize, usize) {
    let body = block + 1;
    let end = doc::element_at(&org.lines, block).map_or(body, |e| e.stop.saturating_sub(1));
    if body + o.offset.0 < end { (body + o.offset.0, o.offset.1) } else { (body, 0) }
}

/// org-babel-update-block-body for the block whose head is `head`.
pub fn update_block_body(org: &mut Doc, head: usize, new_body: &str) {
    let Some(el) = doc::element_at(&org.lines, head) else { return };
    let begin = org.line(el.post).to_owned();
    let preserve = core::parse_begin(&begin).1.split_whitespace().any(|s| s == "-i") || options::bool("org-src-preserve-indentation", false);
    let ind = value::indentation(&begin) + options::int("org-edit-src-content-indentation", 2) as usize;
    let body = if preserve {
        new_body.to_owned()
    } else {
        value::remove_indentation(new_body)
            .split('\n')
            .map(|l| if l.trim().is_empty() { String::new() } else { format!("{}{l}", " ".repeat(ind)) })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let body = body.trim_end_matches('\n');
    let lines: Vec<String> = if body.is_empty() { vec![] } else { body.split('\n').map(str::to_owned).collect() };
    org.splice(el.post + 1, el.stop - 1 - (el.post + 1), lines);
}

/// The detangle pairs of a tangled file: (origin, body).
pub fn detangle_pairs(code: &Doc) -> Vec<(Origin, String)> {
    let mut out = vec![];
    let link_re = regex::Regex::new(r"\[\[([^\]]+)\]\[([^\]]+)\]\]").unwrap();
    let mut l = 0;
    while l < code.len() {
        if let Some(c) = link_re.captures(code.line(l)) {
            let name = c[2].to_owned();
            let marker = format!(" {name} ends here");
            if let Some(end) = (l + 1..code.len()).find(|&k| code.line(k).contains(&marker)) {
                let body = code.lines[l + 1..end].join("\n");
                out.push((Origin { link: c[1].to_owned(), block_name: name, body_start: l + 1, body_end: end, offset: (0, 0) }, body));
                l = end + 1;
                continue;
            }
        }
        l += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_modes() {
        assert_eq!(interpret_file_mode(&Sexp::Int(0o755)).unwrap(), 0o755);
        assert_eq!(interpret_file_mode(&Sexp::Str("o644".into())).unwrap(), 0o644);
        assert_eq!(interpret_file_mode(&Sexp::Str("rwxr-xr--".into())).unwrap(), 0o754);
        assert_eq!(interpret_file_mode(&Sexp::Str("u+x".into())).unwrap(), 0o744);
        assert_eq!(interpret_file_mode(&Sexp::Str("a=rw".into())).unwrap(), 0o666);
        assert!(interpret_file_mode(&Sexp::Int(755)).is_err());
    }
}
