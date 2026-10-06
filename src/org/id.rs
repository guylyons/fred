//! org-id.el: globally unique entry IDs. Creation with every
//! org-id-method (uuid, org, ts), org-id-prefix and
//! org-id-include-domain; org-id-get/-get-create/-copy; the ID location
//! database read from and written to the Emacs-compatible
//! org-id-locations-file (default `~/.emacs.d/.org-id-locations`);
//! org-id-update-id-locations, org-id-find, org-id-goto, ID links
//! (store, open with `::search`, completion and descriptions).
//!
//! Adaptation: Emacs writes the location file when it exits; Fred writes
//! it whenever a location is added, merged with the file's current
//! contents so an Emacs session's entries are kept.

use super::props::{self, Inherit};
use super::sexp::Sexp;
use super::{Prefix, fold, links, options, syntax};
use crate::editor::Editor;
use crate::session::Session;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// org-id-locations (id → abbreviated file) and org-id-files.
#[derive(Default)]
struct Db {
    loaded: bool,
    order: Vec<String>,
    map: HashMap<String, String>,
    files: Vec<String>,
    checksum: Option<Vec<(String, Option<std::time::SystemTime>)>>,
}

impl Db {
    fn put(&mut self, id: &str, file: &str) {
        if self.map.insert(id.to_owned(), file.to_owned()).is_none() {
            self.order.push(id.to_owned());
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.map.clear();
    }

    /// org-id-hash-to-alist.
    fn alist(&self) -> Vec<(String, Vec<String>)> {
        let mut res: Vec<(String, Vec<String>)> = vec![];
        for id in &self.order {
            let f = &self.map[id];
            match res.iter_mut().find(|(x, _)| x == f) {
                Some((_, ids)) => ids.insert(0, id.clone()),
                None => res.insert(0, (f.clone(), vec![id.clone()])),
            }
        }
        res
    }
}

thread_local! {
    static DB: RefCell<Db> = RefCell::new(Db::default());
}

fn track() -> bool {
    options::bool("org-id-track-globally", true)
}

/// org-id-locations-file.
pub fn locations_file() -> PathBuf {
    if cfg!(test) && options::get("org-id-locations-file").is_none() {
        return std::env::temp_dir().join(format!("fred-org-id-{}", std::process::id()));
    }
    options::expand(&options::string(
        "org-id-locations-file",
        "~/.emacs.d/.org-id-locations",
    ))
}

/// abbreviate-file-name.
pub fn abbreviate(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    match s.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => s,
    }
}

/// Read the location file: (file, ids) entries, relative files resolved.
fn read_file() -> Result<Vec<(String, Vec<String>)>, String> {
    let path = locations_file();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let v = super::sexp::read(text.trim()).map_err(|e| e.to_string())?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut out = vec![];
    for e in v.list().unwrap_or(&[]) {
        let Some(items) = e.list() else { continue };
        let Some(f) = items.first().and_then(Sexp::str) else {
            continue;
        };
        let f = if Path::new(f).is_absolute() || f.starts_with('~') {
            f.to_owned()
        } else {
            abbreviate(&dir.join(f))
        };
        let ids = items[1..]
            .iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect();
        out.push((f, ids));
    }
    Ok(out)
}

/// org-id-locations-load: Some(message) when the file is unreadable.
fn load() -> Option<String> {
    DB.with(|d| {
        let mut d = d.borrow_mut();
        d.clear();
        d.loaded = true;
        if !track() {
            return None;
        }
        match read_file() {
            Ok(entries) => {
                d.files = entries.iter().map(|(f, _)| f.clone()).collect();
                for (f, ids) in entries {
                    for id in ids {
                        d.put(&id, &f);
                    }
                }
                None
            }
            Err(_) if !locations_file().exists() => None,
            Err(_) => Some(format!(
                "Could not read `org-id-locations' from {}, setting it to nil",
                locations_file().display()
            )),
        }
    })
}

fn ensure_loaded() {
    if !DB.with(|d| d.borrow().loaded) {
        load();
    }
}

/// prin1 of a string.
fn prin1(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// org-id-locations-save: `merge` keeps entries of the file that this
/// session does not know (Fred writes while Emacs may be running).
fn save(merge: bool) -> Result<(), String> {
    if !track() {
        return Ok(());
    }
    let mut alist = DB.with(|d| d.borrow().alist());
    if merge && let Ok(old) = read_file() {
        let known: std::collections::HashSet<String> =
            DB.with(|d| d.borrow().map.keys().cloned().collect());
        for (f, ids) in old {
            let extra: Vec<String> = ids.into_iter().filter(|i| !known.contains(i)).collect();
            if extra.is_empty() {
                continue;
            }
            match alist.iter_mut().find(|(x, _)| *x == f) {
                Some((_, v)) => v.extend(extra),
                None => alist.push((f, extra)),
            }
        }
    }
    if alist.is_empty() {
        return Ok(());
    }
    let path = locations_file();
    let relative = options::bool("org-id-locations-file-relative", false);
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let entries: Vec<String> = alist
        .iter()
        .map(|(f, ids)| {
            let f = if relative {
                let full = options::expand(f);
                full.strip_prefix(&dir)
                    .map_or_else(|_| f.clone(), |r| r.display().to_string())
            } else {
                f.clone()
            };
            let mut s = format!("({}", prin1(&f));
            for id in ids {
                s.push(' ');
                s.push_str(&prin1(id));
            }
            s.push(')');
            s
        })
        .collect();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&path, format!("\n({})\n", entries.join(" ")))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// org-id-add-location.
pub fn add_location(id: &str, file: Option<&Path>) -> Result<(), String> {
    let file = file.ok_or("`org-id-get' expects a file-visiting buffer")?;
    if !track() {
        return Ok(());
    }
    ensure_loaded();
    let afile = abbreviate(file);
    DB.with(|d| {
        let mut d = d.borrow_mut();
        d.put(id, &afile);
        if !d.files.contains(&afile) {
            d.files.insert(0, afile.clone());
        }
    });
    save(true)
}

/// org-id-paste-tracker: IDs in pasted text now live in `file`.
pub fn paste_tracker(txt: &str, file: Option<&Path>) {
    if !track() {
        return;
    }
    let Some(file) = file else { return };
    let re = crate::org_re!(r"(?m)^[ \t]*:ID:[ \t]+([^ \t\n\r]+)");
    for c in re.captures_iter(txt) {
        let _ = add_location(&c[1], Some(file));
    }
}

// ---- new IDs ----

/// org-id-int-to-b36-one-digit.
fn b36_digit(i: u64) -> char {
    char::from_digit(i as u32, 36).unwrap_or('0')
}

/// org-id-int-to-b36.
pub fn int_to_b36(mut i: u64, length: usize) -> String {
    let mut s = String::new();
    while i > 0 {
        s.insert(0, b36_digit(i % 36));
        i /= 36;
    }
    let length = length.max(1);
    if s.len() < length {
        s = format!("{}{s}", "0".repeat(length - s.len()));
    }
    s
}

/// org-id-b36-to-int.
pub fn b36_to_int(s: &str) -> Result<u64, String> {
    s.chars().try_fold(0u64, |r, c| {
        c.to_digit(36)
            .filter(|_| !c.is_ascii_uppercase())
            .map(|d| r * 36 + d as u64)
            .ok_or_else(|| "Invalid b36 letter".to_owned())
    })
}

/// The current time as (HI LO USEC).
fn time_list() -> (u64, u64, u64) {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = if cfg!(test) {
        super::now() as u64
    } else {
        d.as_secs()
    };
    (secs >> 16, secs & 0xffff, d.subsec_micros() as u64)
}

/// org-id-time-to-b36.
pub fn time_to_b36(t: (u64, u64, u64)) -> String {
    format!(
        "{}{}{}",
        int_to_b36(t.0, 4),
        int_to_b36(t.1, 4),
        int_to_b36(t.2, 4)
    )
}

/// A Lisp time (HI LO USEC).
pub type TimeList = (u64, u64, u64);

/// org-id-decode: (prefix, (HI LO USEC)).
pub fn decode(id: &str) -> Result<(Option<String>, TimeList), String> {
    let parts: Vec<&str> = id.split(':').filter(|p| !p.is_empty()).collect();
    let (prefix, time) = if parts.len() == 2 {
        (Some(parts[0].to_owned()), parts[1])
    } else {
        (None, parts.first().copied().unwrap_or(""))
    };
    let t: String = time.chars().rev().collect();
    let part = |a: usize, b: usize| b36_to_int(t.get(a..b).unwrap_or(""));
    Ok((prefix, (part(0, 4)?, part(4, 8)?, part(8, 12)?)))
}

/// message-make-fqdn.
fn fqdn() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most len bytes into buf.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    if ok && n > 0 {
        String::from_utf8_lossy(&buf[..n]).into_owned()
    } else {
        "localhost".into()
    }
}

/// org-uuidgen-p.
fn uuid_p(s: &str) -> bool {
    crate::org_re!(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").is_match(s)
}

/// The PREFIX argument of org-id-new.
pub enum IdPrefix<'a> {
    Default,
    None,
    Some(&'a str),
}

/// org-id-new.
pub fn new(prefix: IdPrefix) -> Result<String, String> {
    let opt = options::string("org-id-prefix", "nil");
    let prefix = match prefix {
        IdPrefix::None => String::new(),
        IdPrefix::Some(p) => format!("{p}:"),
        IdPrefix::Default if opt != "nil" && !opt.is_empty() => format!("{opt}:"),
        IdPrefix::Default => String::new(),
    };
    let domain = || {
        if options::bool("org-id-include-domain", false) {
            format!("@{}", fqdn())
        } else {
            String::new()
        }
    };
    let method = options::string("org-id-method", "uuid");
    let unique = match method.trim_start_matches('\'') {
        "uuid" | "uuidgen" => {
            let program = options::string("org-id-uuid-program", "nil");
            let u = if program == "nil" || program.is_empty() {
                links::new_id()
            } else {
                std::process::Command::new("sh")
                    .args(["-c", &program])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
                    .unwrap_or_default()
            };
            if uuid_p(&u) { u } else { links::new_id() }
        }
        "org" => {
            let t: String = time_to_b36(time_list()).chars().rev().collect();
            format!("{t}{}", domain())
        }
        "ts" => {
            let fmt = options::string("org-id-ts-format", "%Y%m%dT%H%M%S.%6N");
            let (_, _, usec) = time_list();
            let fmt = fmt
                .replace("%6N", &format!("{usec:06}"))
                .replace("%3N", &format!("{:03}", usec / 1000))
                .replace("%N", &format!("{:09}", usec * 1000));
            let tm = super::time::civil::Tm::from_epoch(super::now());
            format!("{}{}", super::time::civil::format(&fmt, tm), domain())
        }
        _ => return Err("Invalid `org-id-method'".into()),
    };
    Ok(format!("{prefix}{unique}"))
}

// ---- entries ----

/// The entry at line `l`: its heading (None before the first heading).
fn entry(ed: &Editor, l: usize) -> Option<usize> {
    props::entry(ed, l)
}

/// org-id-get on the entry at line `l`: with `inherit`, also IDs of
/// ancestors; returns the ID and the heading it was found on.
pub fn get_at(ed: &Editor, l: usize, inherit: bool) -> Option<(String, Option<usize>)> {
    let mut h = entry(ed, l);
    loop {
        if let Some(id) = props::get(ed, h, "ID", Inherit::No)
            && id.contains(|c: char| !c.is_whitespace())
        {
            return Some((id, h));
        }
        if !inherit {
            return None;
        }
        h = syntax::parent(&ed.buf, h?);
        if h.is_none() {
            // The file-level drawer last.
            return props::get(ed, None, "ID", Inherit::No)
                .filter(|id| id.contains(|c: char| !c.is_whitespace()))
                .map(|id| (id, None));
        }
    }
}

/// org-id-get with CREATE: the ID of the entry at line `l`, made if missing.
pub fn get_create_at(ed: &mut Editor, l: usize, prefix: IdPrefix) -> Result<String, String> {
    if let Some((id, _)) = get_at(ed, l, false) {
        return Ok(id);
    }
    let id = new(prefix)?;
    let h = entry(ed, l);
    props::put(ed, h, "ID", &id)?;
    add_location(&id, ed.path.as_deref())?;
    Ok(id)
}

/// org-id-get-create (FORCE: a new ID).
pub fn get_create(ed: &mut Editor, force: bool) -> Result<String, String> {
    let l = ed.cur.line;
    if force {
        let h = entry(ed, l);
        props::delete(ed, h, "ID");
    }
    get_create_at(ed, l, IdPrefix::Default)
}

// ---- finding ----

/// org-find-entry-with-id in TEXT: the entry's heading line (0 for the
/// file-level property drawer).
pub fn find_in_text(text: &str, id: &str) -> Option<usize> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut in_drawer = false;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.eq_ignore_ascii_case(":PROPERTIES:") {
            in_drawer = true;
        } else if t.eq_ignore_ascii_case(":END:") {
            in_drawer = false;
        } else if in_drawer
            && let Some((k, v)) = props::parse_property(l)
            && k.eq_ignore_ascii_case("ID")
            && v == id
        {
            return Some(syntax::heading_at_or_before(&lines, i).unwrap_or(0));
        }
    }
    None
}

/// The IDs in TEXT (property-drawer `:ID:` values).
fn ids_in_text(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut in_drawer = false;
    for l in text.lines() {
        let t = l.trim();
        if t.eq_ignore_ascii_case(":PROPERTIES:") {
            in_drawer = true;
        } else if t.eq_ignore_ascii_case(":END:") {
            in_drawer = false;
        } else if in_drawer
            && let Some((k, v)) = props::parse_property(l)
            && k.eq_ignore_ascii_case("ID")
            && !v.is_empty()
        {
            out.push(v);
        }
    }
    out
}

/// org-id-find-id-file.
pub fn find_id_file(s: &Session, id: &str) -> Option<PathBuf> {
    ensure_loaded();
    if let Some(f) = DB.with(|d| d.borrow().map.get(id).cloned()) {
        return Some(options::expand(&f));
    }
    s.ed.org.as_ref()?;
    s.ed.path.clone()
}

/// org-id-find-id-in-file: the heading line.
pub fn find_in_file(s: &Session, id: &str, file: &Path) -> Option<usize> {
    let text = s.org_text(file).ok()?;
    find_in_text(&text, id)
}

/// org-id-find: (file, heading line).
pub fn find(s: &mut Session, id: &str) -> Option<(PathBuf, usize)> {
    if let Some(f) = find_id_file(s, id)
        && let Some(l) = find_in_file(s, id, &f)
    {
        return Some((f, l));
    }
    let _ = update_id_locations(s, &[], true);
    let f = find_id_file(s, id)?;
    find_in_file(s, id, &f).map(|l| (f, l))
}

/// The archive files of FILE (org-add-archive-files).
fn archive_files(f: &Path) -> Vec<PathBuf> {
    let loc = options::string("org-archive-location", "%s_archive::");
    super::refile::archive_location(&loc, f)
        .ok()
        .map(|(p, _)| p)
        .filter(|p| p != f && p.exists())
        .into_iter()
        .collect()
}

/// org-id-extra-files (a list, or the name of an option holding one).
fn extra_files() -> Vec<String> {
    let v = super::sexp::option("org-id-extra-files")
        .unwrap_or_else(|| Sexp::Sym("org-agenda-text-search-extra-files".into()));
    let v = match &v {
        Sexp::Sym(name) | Sexp::Str(name)
            if !name.contains('/') && options::get(name).is_some() =>
        {
            super::sexp::option(name).unwrap_or(Sexp::Nil)
        }
        Sexp::Sym(_) => Sexp::Nil,
        _ => v,
    };
    match v {
        Sexp::Str(s) => vec![s],
        v => v
            .list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| match x {
                Sexp::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
    }
}

/// org-id-update-id-locations: the summary message.
pub fn update_id_locations(
    s: &mut Session,
    files: &[PathBuf],
    silent: bool,
) -> Result<String, String> {
    if !track() {
        return Err("Please turn on `org-id-track-globally' if you want to track IDs".into());
    }
    ensure_loaded();
    let mut all: Vec<PathBuf> = vec![];
    for f in super::agenda_files() {
        if options::bool("org-id-search-archives", true) {
            all.extend(archive_files(&f));
        }
        all.push(f);
    }
    all.extend(extra_files().iter().map(|f| options::expand(f)));
    all.extend(
        DB.with(|d| d.borrow().files.clone())
            .iter()
            .map(|f| options::expand(f)),
    );
    for i in 0..s.org_buffer_count() {
        if let Some(p) = s.org_buffer_path(i)
            && p.extension()
                .is_some_and(|e| e == "org" || e == "org_archive")
        {
            all.push(p);
        }
    }
    all.extend(files.iter().cloned());
    let mut seen = std::collections::HashSet::new();
    let files: Vec<PathBuf> = all
        .into_iter()
        .map(|f| std::fs::canonicalize(&f).unwrap_or(f))
        .filter(|f| seen.insert(f.clone()))
        .collect();
    let mut sorted: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
    sorted.sort();
    let checksum: Vec<(String, Option<std::time::SystemTime>)> = sorted
        .into_iter()
        .map(|f| {
            let m = std::fs::metadata(&f).and_then(|m| m.modified()).ok();
            (f, m)
        })
        .collect();
    let modified_buffers = (0..s.org_buffer_count()).any(|i| {
        s.org_buffer_path(i).is_some_and(|p| {
            files
                .iter()
                .any(|f| std::fs::canonicalize(&p).ok().as_ref() == Some(f))
        })
    });
    if !modified_buffers && DB.with(|d| d.borrow().checksum.as_ref() == Some(&checksum)) {
        let (nf, ni) = DB.with(|d| {
            let d = d.borrow();
            (d.files.len(), d.map.len())
        });
        return Ok(format!(
            "{} files scanned, {nf} files contains IDs, and {ni} IDs found.",
            files.len()
        ));
    }
    let mut entries: Vec<(String, Vec<String>)> = vec![];
    let mut seen_ids = std::collections::HashSet::new();
    let mut ndup = 0;
    let mut dups = vec![];
    for f in &files {
        if !f.exists() {
            continue;
        }
        let Ok(text) = s.org_text(f) else { continue };
        let mut ids = ids_in_text(&text);
        ids.reverse();
        if ids.is_empty() {
            continue;
        }
        for id in &ids {
            if !seen_ids.insert(id.clone()) && !silent {
                dups.push(format!("Duplicate ID {id:?}"));
                ndup += 1;
            }
        }
        entries.insert(0, (abbreviate(f), ids));
    }
    DB.with(|d| {
        let mut d = d.borrow_mut();
        d.clear();
        d.files = entries.iter().map(|(f, _)| f.clone()).collect();
        for (f, ids) in &entries {
            for id in ids {
                d.put(id, f);
            }
        }
        d.checksum = Some(checksum);
    });
    save(false)?;
    let (nf, ni) = DB.with(|d| {
        let d = d.borrow();
        (d.files.len(), d.map.len())
    });
    if !silent && ndup > 0 {
        return Ok(format!(
            "WARNING: {ndup} duplicate IDs found, check *Messages* buffer ({})",
            dups.join("; ")
        ));
    }
    Ok(format!(
        "{} files scanned, {nf} files contains IDs, and {ni} IDs found.",
        files.len()
    ))
}

// ---- commands ----

/// org-id-goto.
pub fn goto(ed: &mut Editor, id: String) {
    super::effect(ed, move |s| match find(s, &id) {
        Some((path, line)) => {
            if let Err(e) = s.org_visit(&path, line) {
                return s.ed.set_err(e);
            }
            fold::show_context_for(&mut s.ed, line, "default");
        }
        None => s.ed.set_err(format!("Cannot find entry with ID \"{id}\"")),
    });
}

/// org-id-open: the link path `ID[::SEARCH]`.
pub fn open(ed: &mut Editor, link: &str) -> Result<(), String> {
    let (id, option) = match link.find("::") {
        Some(i) => (link[..i].to_owned(), Some(link[i + 2..].to_owned())),
        None => (link.to_owned(), None),
    };
    links::mark_ring_push(ed);
    // This buffer, when the database does not place the ID elsewhere.
    ensure_loaded();
    let elsewhere = DB
        .with(|d| d.borrow().map.get(&id).cloned())
        .is_some_and(|f| ed.path.as_deref().map(abbreviate).as_deref() != Some(f.as_str()));
    if !elsewhere && ed.org.is_some() {
        let text = ed.buf.text();
        let hit = find_in_text(&text, &id)
            .map(|l| (l, option.clone()))
            .or_else(|| find_in_text(&text, link).map(|l| (l, None)));
        if let Some((l, option)) = hit {
            return arrive(ed, l, option.as_deref());
        }
    }
    let full = link.to_owned();
    super::effect(ed, move |s| {
        let (hit, option) = match find(s, &id) {
            Some(h) => (Some(h), option),
            None if option.is_some() => (find(s, &full), None),
            None => (None, None),
        };
        let Some((path, line)) = hit else {
            return s.ed.set_err(format!("Cannot find entry with ID \"{id}\""));
        };
        if let Err(e) = s.org_visit(&path, line) {
            return s.ed.set_err(e);
        }
        if let Err(e) = arrive(&mut s.ed, line, option.as_deref()) {
            s.ed.set_err(e);
        }
    });
    Ok(())
}

/// At the entry of an ID link: run its search inside the subtree.
fn arrive(ed: &mut Editor, l: usize, option: Option<&str>) -> Result<(), String> {
    ed.set_cursor(l, 0);
    if let Some(opt) = option {
        let st = super::settings(ed);
        let range = if syntax::level(&ed.buf.line(l)).is_some() {
            l..syntax::subtree_end(&ed.buf, l)
        } else {
            0..ed.line_count()
        };
        let _ = st;
        links::search_in(ed, opt, None, range)?;
    }
    let cl = ed.cur.line;
    fold::show_context_for(ed, cl, "default");
    Ok(())
}

/// org-id-store-link: (link, description).
pub fn store_link(ed: &mut Editor) -> Result<(String, Option<String>), String> {
    let consider_parent = options::bool("org-id-link-consider-parent-id", false);
    let use_context = options::bool("org-id-link-use-context", true);
    let context = options::bool("org-link-context-for-files", true);
    let inherit = consider_parent && use_context && context;
    let l = ed.cur.line;
    let (id, from) = match get_at(ed, l, inherit) {
        Some(x) => x,
        None => {
            let id = get_create_at(ed, l, IdPrefix::Default)?;
            (id, entry(ed, l))
        }
    };
    let mut link = format!("id:{id}");
    let st = super::settings(ed);
    let id_line = from.unwrap_or(0);
    let mut desc = match from {
        None => st
            .value("TITLE")
            .map(str::to_owned)
            .or_else(|| {
                ed.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| link.clone()),
        Some(h) => {
            let line = ed.buf.line(h);
            match syntax::headline(&line, &st) {
                Some(hl) if !hl.title.is_empty() => hl.title(&line).to_owned(),
                _ => line,
            }
        }
    };
    if context
        && use_context
        && let Some((search, sdesc, line)) = links::precise_target(ed)
        && line > id_line
    {
        link = format!("{link}::{search}");
        if let Some(d) = sdesc {
            desc = d;
        }
    }
    Ok((link, Some(desc)))
}

/// org-id-store-link-maybe: the ID link org-store-link uses, if any.
pub fn store_link_maybe(
    ed: &mut Editor,
    interactive: bool,
) -> Result<Option<(String, Option<String>)>, String> {
    if ed.path.is_none() || ed.org.is_none() {
        return Ok(None);
    }
    let mode = options::string("org-id-link-to-org-use-id", "nil");
    let mode = mode.trim_start_matches('\'');
    let custom = || props::get(ed, entry(ed, ed.cur.line), "CUSTOM_ID", Inherit::No).is_some();
    let consider = options::bool("org-id-link-consider-parent-id", false)
        && options::bool("org-id-link-use-context", true)
        && options::bool("org-link-context-for-files", true);
    let yes = mode == "t"
        || (interactive
            && (mode == "create-if-interactive"
                || (mode == "create-if-interactive-and-no-custom-id" && !custom())))
        || (mode != "nil" && get_at(ed, ed.cur.line, consider).is_some());
    if !yes {
        return Ok(None);
    }
    store_link(ed).map(Some)
}

/// Outline-path completion targets (org-id-completion-targets): this
/// buffer's headings, then those of org-id-files.
fn completion_targets(ed: &Editor) -> Vec<(String, Option<PathBuf>, usize)> {
    let st = super::settings(ed);
    let lines: Vec<String> = (0..ed.line_count()).map(|i| ed.buf.line(i)).collect();
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        if syntax::level(l).is_some() {
            let p = super::refile::outline_path(&lines, &st, i, true).join("/");
            out.push((p, None, i));
        }
    }
    ensure_loaded();
    let mine = ed.path.as_deref().map(abbreviate);
    for f in DB.with(|d| d.borrow().files.clone()) {
        if Some(&f) == mine.as_ref() {
            continue;
        }
        let path = options::expand(&f);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let st = super::settings_of(&text, path.parent());
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (i, l) in lines.iter().enumerate() {
            if syntax::level(l).is_some() {
                let p = super::refile::outline_path(&lines, &st, i, true).join("/");
                out.push((format!("{name}/{p}"), Some(path.clone()), i));
            }
        }
    }
    out
}

/// org-id-complete (the `id:` type's completion in org-insert-link):
/// `then` gets the link.
pub fn complete(ed: &mut Editor, then: impl FnOnce(&mut Editor, String) + Send + 'static) {
    let targets = completion_targets(ed);
    if targets.is_empty() {
        super::read(ed, "Link: ", "id:", then);
        return;
    }
    let names: Vec<String> = targets.iter().map(|(n, _, _)| n.clone()).collect();
    super::complete(ed, "Entry: ", names, true, move |ed, pick| {
        let Some((_, file, line)) = targets.into_iter().find(|(n, _, _)| *n == pick) else {
            return;
        };
        match file {
            None => {
                ed.undo.begin(ed.cur.pos());
                match get_create_at(ed, line, IdPrefix::Default) {
                    Ok(id) => then(ed, format!("id:{id}")),
                    Err(e) => ed.set_err(e),
                }
                ed.undo.end(ed.cur.pos());
            }
            Some(path) => super::effect(ed, move |s| {
                let r = s.org_with_file(&path, |e| {
                    e.undo.begin(e.cur.pos());
                    let r = get_create_at(e, line, IdPrefix::Default);
                    e.undo.end(e.cur.pos());
                    r
                });
                match r.and_then(|r| r) {
                    Ok(id) => then(&mut s.ed, format!("id:{id}")),
                    Err(e) => s.ed.set_err(e),
                }
            }),
        }
    });
}

/// org-id-description: the linked heading (keywords, priority and tags
/// removed), for an ID link inserted without a description.
pub fn description(ed: &Editor, link: &str, desc: Option<&str>) -> Option<String> {
    if let Some(d) = desc.filter(|d| d.contains(|c: char| !c.is_whitespace())) {
        return Some(d.to_owned());
    }
    let id = link.strip_prefix("id:").unwrap_or(link);
    let heading = |text: &str, st: &super::syntax::Settings| {
        let l = find_in_text(text, id)?;
        let line = text.lines().nth(l)?;
        let hl = syntax::headline(line, st)?;
        Some(links::display_format(hl.title(line)))
    };
    if let Some(h) = heading(&ed.buf.text(), &super::settings(ed)) {
        return Some(h);
    }
    ensure_loaded();
    let f = DB.with(|d| d.borrow().map.get(id).cloned())?;
    let path = options::expand(&f);
    let text = std::fs::read_to_string(&path).ok()?;
    let st = super::settings_of(&text, path.parent());
    heading(&text, &st).or_else(|| desc.map(str::to_owned))
}

/// The ID commands.
pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-id-get-create" => get_create(ed, !arg.is_none()).map(|_| ()),
        "org-id-copy" => get_create_at(ed, ed.cur.line, IdPrefix::Default).map(|id| {
            super::structure::kill_new(ed, &id);
        }),
        "org-id-store-link" => store_link(ed).map(|(l, d)| links::add_stored(ed, l, d)),
        "org-id-goto" => {
            match super::take_call_arg() {
                Some(id) => goto(ed, id),
                None => super::read(ed, "ID: ", "", goto),
            }
            Ok(())
        }
        "org-id-update-id-locations" => {
            super::effect(ed, |s| match update_id_locations(s, &[], false) {
                Ok(m) => s.ed.set_msg(m),
                Err(e) => s.ed.set_err(e),
            });
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::tests::org;

    #[test]
    fn b36_and_decode() {
        assert_eq!(int_to_b36(0, 4), "0000");
        assert_eq!(int_to_b36(35, 1), "z");
        assert_eq!(int_to_b36(36, 0), "10");
        assert_eq!(b36_to_int("10").unwrap(), 36);
        assert!(b36_to_int("A").is_err());
        let t = (25_000, 12_345, 678_901);
        let s: String = time_to_b36(t).chars().rev().collect();
        assert_eq!(
            decode(&format!("Org:{s}")).unwrap(),
            (Some("Org".into()), t)
        );
        assert_eq!(decode(&s).unwrap(), (None, t));
    }

    #[test]
    fn new_ids_by_method_and_prefix() {
        let u = new(IdPrefix::Default).unwrap();
        assert!(uuid_p(&u), "{u}");
        options::put("org-id-prefix", toml::Value::String("Fred".into()));
        assert!(new(IdPrefix::Default).unwrap().starts_with("Fred:"));
        assert!(!new(IdPrefix::None).unwrap().contains(':'));
        assert!(new(IdPrefix::Some("X")).unwrap().starts_with("X:"));
        options::remove("org-id-prefix");
        options::put("org-id-method", toml::Value::String("org".into()));
        let o = new(IdPrefix::Default).unwrap();
        assert_eq!(o.len(), 12, "{o}");
        assert!(decode(&o).is_ok());
        options::put("org-id-method", toml::Value::String("ts".into()));
        options::put("org-id-include-domain", toml::Value::Boolean(true));
        let t = new(IdPrefix::Default).unwrap();
        assert!(crate::org_re!(r"^\d{8}T\d{6}\.\d{6}@").is_match(&t), "{t}");
        options::put("org-id-method", toml::Value::String("bogus".into()));
        assert!(new(IdPrefix::Default).is_err());
        options::remove("org-id-method");
        options::remove("org-id-include-domain");
    }

    #[test]
    fn get_create_copy_and_locations_file() {
        let dir = tempfile::tempdir().unwrap();
        let loc = dir.path().join("locs");
        options::put(
            "org-id-locations-file",
            toml::Value::String(loc.display().to_string()),
        );
        DB.with(|d| *d.borrow_mut() = Db::default());
        let mut e = org("* A\nbody", "j");
        e.path = Some(dir.path().join("a.org"));
        crate::org::run(&mut e, "org-id-get-create", Prefix::None);
        let id = props::get(&e, Some(0), "ID", Inherit::No).unwrap();
        assert_eq!(e.buf.line(1), ":PROPERTIES:");
        // Again: the same ID; C-u: a new one.
        crate::org::run(&mut e, "org-id-get-create", Prefix::None);
        assert_eq!(props::get(&e, Some(0), "ID", Inherit::No).unwrap(), id);
        crate::org::run(&mut e, "org-id-get-create", Prefix::U(1));
        let id2 = props::get(&e, Some(0), "ID", Inherit::No).unwrap();
        assert_ne!(id, id2);
        // The Emacs-format location file.
        let text = std::fs::read_to_string(&loc).unwrap();
        assert!(text.starts_with("\n((\""), "{text}");
        assert!(text.contains(&format!("\"{id2}\"")), "{text}");
        let v = super::super::sexp::read(text.trim()).unwrap();
        assert_eq!(v.list().unwrap().len(), 1);
        // Entries the file has but this session does not are kept.
        std::fs::write(&loc, "((\"/x/other.org\" \"keep-me\"))").unwrap();
        crate::org::run(&mut e, "org-id-get-create", Prefix::U(1));
        let text = std::fs::read_to_string(&loc).unwrap();
        assert!(text.contains("keep-me"), "{text}");
        // org-id-copy.
        crate::org::run(&mut e, "org-id-copy", Prefix::None);
        let id3 = props::get(&e, Some(0), "ID", Inherit::No).unwrap();
        assert_eq!(super::super::structure::current_kill(&mut e), id3);
        options::remove("org-id-locations-file");
    }

    #[test]
    fn find_in_text_and_store_link() {
        let text =
            ":PROPERTIES:\n:ID: top\n:END:\n#+TITLE: T\n* A\n:PROPERTIES:\n:ID: abc\n:END:\n** B\n";
        assert_eq!(find_in_text(text, "abc"), Some(4));
        assert_eq!(find_in_text(text, "top"), Some(0));
        assert_eq!(find_in_text(text, "zzz"), None);
        let dir = tempfile::tempdir().unwrap();
        options::put(
            "org-id-locations-file",
            toml::Value::String(dir.path().join("l").display().to_string()),
        );
        let mut e = org(text, "jjjjjjjj");
        e.path = Some(dir.path().join("t.org"));
        // On B, with the parent's ID considered: a search link.
        options::put("org-id-link-consider-parent-id", toml::Value::Boolean(true));
        let (l, d) = store_link(&mut e).unwrap();
        assert_eq!(l, "id:abc::*B");
        assert_eq!(d.as_deref(), Some("B"));
        options::remove("org-id-link-consider-parent-id");
        // Otherwise B gets its own ID.
        let (l, d) = store_link(&mut e).unwrap();
        assert!(l.starts_with("id:") && !l.contains("::"), "{l}");
        assert_eq!(d.as_deref(), Some("B"));
        // Before the first heading: the title.
        e.set_cursor(0, 0);
        let (l, d) = store_link(&mut e).unwrap();
        assert_eq!((l.as_str(), d.as_deref()), ("id:top", Some("T")));
        options::remove("org-id-locations-file");
    }

    #[test]
    fn open_id_link_in_this_buffer_with_search() {
        let text = "* A\n:PROPERTIES:\n:ID: abc\n:END:\n** B\n** C\n* D [[id:abc::*C]]";
        let mut e = org(text, "");
        e.set_cursor(6, 6);
        crate::org::run(&mut e, "org-open-at-point", Prefix::None);
        assert_eq!(e.cur.line, 5, "{:?}", e.msg);
        let mut e = org(text, "");
        open(&mut e, "abc").unwrap();
        assert_eq!(e.cur.line, 0);
    }

    #[test]
    fn update_locations_and_find_across_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.org");
        let b = dir.path().join("b.org");
        std::fs::write(&a, "* A\n[[id:bbb]]\n").unwrap();
        std::fs::write(&b, "* Target\n:PROPERTIES:\n:ID: bbb\n:END:\n").unwrap();
        let loc = dir.path().join("locs");
        let mut cfg = crate::config::Config::default();
        cfg.org.0.insert(
            "org-id-locations-file".into(),
            toml::Value::String(loc.display().to_string()),
        );
        cfg.org.0.insert(
            "org-agenda-files".into(),
            toml::Value::Array(vec![toml::Value::String(b.display().to_string())]),
        );
        DB.with(|d| *d.borrow_mut() = Db::default());
        let (mut s, _) = Session::open(Some(a.clone()), &cfg, &dir.path().join("swap")).unwrap();
        crate::org::options::set(cfg.org.0.clone());
        let msg = update_id_locations(&mut s, &[], false).unwrap();
        assert_eq!(
            msg,
            "2 files scanned, 1 files contains IDs, and 1 IDs found."
        );
        let saved = std::fs::read_to_string(&loc).unwrap();
        assert!(saved.contains("\"bbb\""), "{saved}");
        assert_eq!(find(&mut s, "bbb").map(|(_, l)| l), Some(0));
        // C-c C-o on the link visits b.org.
        for k in crate::key::parse_keys("j<C-c><C-o>") {
            s.handle_key(k);
        }
        assert_eq!(
            s.ed.path
                .as_deref()
                .map(std::fs::canonicalize)
                .map(Result::unwrap),
            Some(std::fs::canonicalize(&b).unwrap())
        );
        crate::org::options::set(toml::Table::new());
    }
}
