//! MobileOrg (org-mobile.el): staging Org files for a mobile viewer
//! (org-mobile-push) and integrating its captures and flagged edits
//! (org-mobile-pull, org-mobile-apply, org-mobile-edit).
//!
//! Emacs hooks (org-mobile-pre-push-hook & co.) are shell commands here:
//! each string in the option runs with `sh -c` (rsync/scp to the WebDAV
//! server, as the upstream docstrings suggest).

use super::Prefix;
use super::sexp::Sexp;
use super::syntax;
use crate::editor::Editor;
use crate::session::Session;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// org-mobile-capture-file.
pub const CAPTURE_FILE: &str = "mobileorg.org";

/// org-mobile-encryption-password-session.
static PASSWORD: Mutex<Option<String>> = Mutex::new(None);

pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    let job: fn(&mut Session) -> Result<(), String> = match name {
        "org-mobile-push" => push,
        "org-mobile-pull" => pull,
        "org-mobile-create-sumo-agenda" => |s| create_sumo_agenda(s).map(|_| ()),
        "org-mobile-move-capture" => |s| {
            if move_capture(s)?.is_none() {
                s.ed.set_msg("No new items");
            }
            Ok(())
        },
        "org-mobile-apply" => |s| {
            let i = s.org_current();
            apply(s, i, 0).map(|_| ())
        },
        _ => return None,
    };
    with_password(ed, move |ed| {
        super::effect(ed, move |s| {
            if let Err(e) = job(s) {
                s.ed.set_err(e);
            }
        })
    });
    Some(Ok(()))
}

// ---- options ----

fn opt_path(name: &str, default: &str) -> PathBuf {
    super::options::expand(&super::options::string(name, default))
}

fn mobile_dir() -> PathBuf {
    opt_path("org-mobile-directory", "")
}

fn inbox() -> PathBuf {
    opt_path("org-mobile-inbox-for-pull", "~/org/from-mobile.org")
}

fn tempfile() -> PathBuf {
    opt_path("org-mobile-encryption-tempfile", "~/orgtmpcrypt")
}

fn index_file() -> String {
    super::options::string("org-mobile-index-file", "index.org")
}

fn use_encryption() -> bool {
    super::options::bool("org-mobile-use-encryption", false)
}

fn which(prog: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(prog))
        .find(|p| p.is_file())
}

/// org-mobile-checksum-binary (default: the first of shasum, sha1sum,
/// md5sum, md5 on PATH).
fn checksum_binary() -> Option<String> {
    match super::options::get("org-mobile-checksum-binary") {
        Some(toml::Value::String(s)) => Some(s),
        Some(_) => None,
        None => ["shasum", "sha1sum", "md5sum", "md5"]
            .iter()
            .find_map(|p| which(p))
            .map(|p| p.display().to_string()),
    }
}

/// org-mobile-encryption-password.
fn password() -> Option<String> {
    let p = super::options::string("org-mobile-encryption-password", "");
    if !p.trim().is_empty() {
        return Some(p);
    }
    PASSWORD
        .lock()
        .ok()?
        .clone()
        .filter(|p| !p.trim().is_empty())
}

/// Ask for the session password first when encryption needs one.
fn with_password(ed: &mut Editor, then: impl FnOnce(&mut Editor) + Send + 'static) {
    if !use_encryption() || password().is_some() {
        return then(ed);
    }
    super::read(
        ed,
        "Password for mobile application: ",
        "",
        move |ed, pw| {
            if let Ok(mut p) = PASSWORD.lock() {
                *p = Some(pw);
            }
            then(ed)
        },
    );
}

/// org-mobile-force-mobile-change for an edit kind.
fn force_change(kind: &str) -> bool {
    match super::sexp::option("org-mobile-force-mobile-change") {
        Some(Sexp::T) => true,
        Some(v) => v
            .list()
            .is_some_and(|l| l.iter().any(|x| x.str() == Some(kind))),
        None => false,
    }
}

/// Run the shell commands of a hook option.
fn run_hooks(name: &str) -> Result<(), String> {
    for cmd in super::options::strings(name, &[]) {
        let ok = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .current_dir(super::options::directory())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("{name}: `{cmd}' failed"));
        }
    }
    Ok(())
}

// ---- files ----

/// org-mobile-files-alist: (file, link name) for each staged file.
pub fn files_alist() -> Vec<(PathBuf, String)> {
    let spec = super::sexp::option("org-mobile-files")
        .unwrap_or_else(|| Sexp::List(vec![Sexp::Sym("org-agenda-files".into())]));
    let spec: Vec<Sexp> = match spec.list() {
        Some(l) => l.to_vec(),
        None => vec![spec],
    };
    let extra = super::sexp::option("org-agenda-text-search-extra-files");
    let extra: Vec<Sexp> = extra
        .and_then(|v| v.list().map(<[Sexp]>::to_vec))
        .unwrap_or_default();
    let include_archives = spec
        .iter()
        .any(|f| f.str() == Some("org-agenda-text-search-extra-files"))
        && extra.iter().any(|f| f.str() == Some("agenda-archives"));
    let mut files: Vec<PathBuf> = vec![];
    for f in &spec {
        match f.str() {
            Some("org-agenda-files") => {
                for a in super::agenda_files() {
                    // ponytail: archives are `FILE_archive` siblings, not
                    // every ARCHIVE location (org-add-archive-files).
                    let arch = PathBuf::from(format!("{}_archive", a.display()));
                    files.push(a);
                    if include_archives && arch.exists() {
                        files.push(arch);
                    }
                }
            }
            Some("org-agenda-text-search-extra-files") => {
                files.extend(
                    extra
                        .iter()
                        .filter_map(Sexp::str)
                        .filter(|f| *f != "agenda-archives")
                        .map(super::options::expand),
                );
            }
            Some(s) => {
                let p = super::options::expand(s);
                if p.is_dir() {
                    let mut v: Vec<PathBuf> = std::fs::read_dir(&p)
                        .map(|d| {
                            d.flatten()
                                .map(|e| e.path())
                                .filter(|x| x.extension().is_some_and(|e| e == "org"))
                                .collect()
                        })
                        .unwrap_or_default();
                    v.sort();
                    files.extend(v);
                } else if p.exists() {
                    files.push(p);
                }
            }
            None => {}
        }
    }
    let exclude = super::options::string("org-mobile-files-exclude-regexp", "");
    let exclude = (!exclude.is_empty())
        .then(|| super::re::compile(&exclude, false).ok())
        .flatten();
    let orgdir = super::options::directory();
    let orgdir = std::fs::canonicalize(&orgdir).unwrap_or(orgdir);
    let mut seen: Vec<PathBuf> = vec![];
    let mut out = vec![];
    for f in files {
        if exclude
            .as_ref()
            .is_some_and(|r| r.is_match(&f.display().to_string()))
        {
            continue;
        }
        let f = if f.is_relative() {
            super::options::directory().join(f)
        } else {
            f
        };
        let uname = std::fs::canonicalize(&f).unwrap_or_else(|_| f.clone());
        if seen.contains(&uname) {
            continue;
        }
        seen.push(uname.clone());
        let link = match uname.strip_prefix(&orgdir) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => uname
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        out.push((f, link));
    }
    out
}

/// org-mobile-check-setup.
fn check_setup() -> Result<(), String> {
    cleanup_tempfile();
    let blank = |name: &str| super::options::string(name, "").trim().is_empty();
    if !super::options::directory().is_dir() {
        return Err("Please set `org-directory' to the directory where your org files live".into());
    }
    if blank("org-mobile-directory") || !mobile_dir().is_dir() {
        return Err("Variable `org-mobile-directory' must point to an existing directory".into());
    }
    let inbox = inbox();
    let parent = inbox
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if inbox.as_os_str().is_empty() || !parent.exists() {
        return Err(
            "Variable `org-mobile-inbox-for-pull' must point to a file in an existing directory"
                .into(),
        );
    }
    if checksum_binary().is_none_or(|b| b.trim().is_empty()) {
        return Err("No executable found to compute checksums".into());
    }
    if use_encryption() {
        if password().is_none() {
            return Err("To use encryption, you must set `org-mobile-encryption-password'".into());
        }
        let t = tempfile();
        if !t.parent().is_some_and(Path::is_dir) {
            return Err(format!(
                "Cannot write to encryption tempfile {}",
                t.display()
            ));
        }
        if which("openssl").is_none() {
            return Err("OpenSSL is needed to encrypt files".into());
        }
    }
    Ok(())
}

// ---- encryption ----

fn openssl(decrypt: bool, infile: &Path, outfile: &Path) -> Result<(), String> {
    let pass = format!("pass:{}", password().unwrap_or_default());
    let mut c = std::process::Command::new("openssl");
    c.args(["enc", "-md", "md5"]);
    if decrypt {
        c.arg("-d");
    }
    c.args(["-aes-256-cbc", "-salt", "-pass", &pass, "-in"])
        .arg(infile)
        .arg("-out")
        .arg(outfile);
    match c.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "openssl: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("openssl: {e}")),
    }
}

/// org-mobile-encrypt-and-move.
fn encrypt_and_move(infile: &Path, outfile: &Path) -> Result<(), String> {
    let enc = PathBuf::from(format!("{}_enc", infile.display()));
    openssl(false, infile, &enc)?;
    std::fs::copy(&enc, outfile).map_err(|e| format!("{}: {e}", outfile.display()))?;
    let _ = std::fs::remove_file(&enc);
    Ok(())
}

/// org-mobile-cleanup-encryption-tempfile.
fn cleanup_tempfile() {
    let t = tempfile();
    if t.exists() {
        let _ = std::fs::remove_file(t);
    }
}

/// Write `text` to `target`, through the tempfile and openssl when encrypting.
fn write_staged(target: &Path, text: &[u8]) -> Result<(), String> {
    if use_encryption() {
        let t = tempfile();
        std::fs::write(&t, text).map_err(|e| format!("{}: {e}", t.display()))?;
        let r = encrypt_and_move(&t, target);
        cleanup_tempfile();
        r
    } else {
        std::fs::write(target, text).map_err(|e| format!("{}: {e}", target.display()))
    }
}

// ---- push ----

/// org-mobile-push.
fn push(s: &mut Session) -> Result<(), String> {
    save_buffers(s, &super::agenda_files());
    run_hooks("org-mobile-pre-push-hook")?;
    check_setup()?;
    let files = files_alist();
    let mut sums: Vec<(String, String)> = vec![];
    s.ed.set_msg("Creating agendas...");
    let saved = super::options::get("org-agenda-files");
    let list = files
        .iter()
        .map(|(f, _)| toml::Value::String(f.display().to_string()))
        .collect();
    super::options::put("org-agenda-files", toml::Value::Array(list));
    let sumo = create_sumo_agenda(s);
    super::options::put(
        "org-agenda-files",
        saved.unwrap_or(toml::Value::Boolean(false)),
    );
    sums.extend(sumo?);
    // org-save-all-org-buffers: IDs created for the agendas.
    let all: Vec<PathBuf> = (0..s.org_buffer_count())
        .filter_map(|i| s.org_buffer_path(i))
        .collect();
    save_buffers(s, &all);
    copy_agenda_files(&files, &mut sums)?;
    sums.push(create_index_file(s, &files)?);
    write_checksums(&sums)?;
    run_hooks("org-mobile-post-push-hook")?;
    s.ed.set_msg("Files for mobile viewer staged");
    Ok(())
}

/// Save the modified Org buffers visiting `files`.
fn save_buffers(s: &mut Session, files: &[PathBuf]) {
    let same = |a: &Path, b: &Path| {
        std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(x, y)| x == y)
    };
    for i in 0..s.org_buffer_count() {
        let Some(p) = s.org_buffer_path(i) else {
            continue;
        };
        let modified = s.org_with_buffer(i, |ed| ed.buf.modified && ed.org.is_some());
        if modified && files.iter().any(|f| same(f, &p)) {
            let _ = s.org_save(i);
        }
    }
}

/// org-mobile-sumo-agenda-command: one block agenda of all the views.
pub fn sumo_agenda_command() -> Option<Sexp> {
    let nth = |e: &Sexp, n: usize| {
        e.list()
            .and_then(|l| l.get(n))
            .cloned()
            .unwrap_or(Sexp::Nil)
    };
    let sym = |s: &str| Sexp::Sym(s.into());
    let str_ = |s: &str| Sexp::Str(s.into());
    let custom: Vec<Sexp> = super::sexp::option("org-agenda-custom-commands")
        .and_then(|v| v.list().map(<[Sexp]>::to_vec))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|x| {
            let key = x.car()?.clone();
            match (x.cdr(), nth(&x, 1)) {
                (Sexp::Str(_), _) => None,
                (_, Sexp::Str(_)) => Some(x),
                (rest, d) => {
                    let mut v = vec![key, str_("")];
                    let rest = rest.list().map(<[Sexp]>::to_vec).unwrap_or_default();
                    v.extend(if d.is_nil() { rest[1..].to_vec() } else { rest });
                    Some(Sexp::List(v))
                }
            }
        })
        .collect();
    let default = vec![
        Sexp::List(vec![str_("a"), str_("Agenda"), sym("agenda")]),
        Sexp::List(vec![str_("t"), str_("All TODO"), sym("alltodo")]),
    ];
    let key_is = |e: &Sexp, k: &str| e.car().and_then(Sexp::str) == Some(k);
    let mode = super::sexp::option("org-mobile-agendas").unwrap_or(sym("all"));
    let thelist: Vec<Sexp> = match mode.sym() {
        Some("custom") => custom,
        Some("default") => default,
        Some("all") => {
            let mut l = custom;
            if !l.iter().any(|e| key_is(e, "t")) {
                l.insert(
                    0,
                    Sexp::List(vec![str_("t"), str_("ALL TODO"), sym("alltodo")]),
                );
            }
            if !l.iter().any(|e| key_is(e, "a")) {
                l.insert(
                    0,
                    Sexp::List(vec![str_("a"), str_("Agenda"), sym("agenda")]),
                );
            }
            l
        }
        _ => {
            let all: Vec<Sexp> = custom.into_iter().chain(default).collect();
            mode.list()
                .unwrap_or(&[])
                .iter()
                .filter_map(|k| {
                    all.iter()
                        .find(|e| k.str().is_some_and(|k| key_is(e, k)))
                        .cloned()
                })
                .collect()
        }
    };
    let title = |s: String| Sexp::List(vec![sym("org-agenda-title-append"), Sexp::Str(s)]);
    let with = |head: Sexp, settings: Sexp| {
        let mut v = vec![head];
        v.extend(settings.list().map(<[Sexp]>::to_vec).unwrap_or_default());
        Sexp::List(v)
    };
    let mut new = vec![];
    for e in thelist {
        let ty = nth(&e, 2);
        let t = ty.sym().unwrap_or("");
        let matchs = nth(&e, 3);
        let blank_match = matchs.str().is_none_or(|m| m.trim().is_empty());
        if matches!(e.cdr(), Sexp::Str(_))
            || t == "search"
            || matches!(t, "todo-tree" | "tags-tree" | "occur-tree")
            || (matches!(t, "todo" | "tags" | "tags-todo") && blank_match)
        {
            continue;
        }
        if matches!(t, "agenda" | "alltodo" | "todo" | "tags" | "tags-todo") {
            let key = e.car().and_then(Sexp::str).unwrap_or("").to_owned();
            let desc = nth(&e, 1);
            let desc = desc.str().filter(|d| !d.is_empty()).unwrap_or(t).to_owned();
            let settings = with(
                title(format!("<after>KEYS={key} TITLE: {desc}</after>")),
                nth(&e, 4),
            );
            new.push(Sexp::List(vec![ty.clone(), matchs, settings]));
        } else if ty.sym().is_some() {
            // A function or another symbol: not exportable.
        } else {
            let gkey = e.car().and_then(Sexp::str).unwrap_or("").to_owned();
            let gdesc = nth(&e, 1).str().unwrap_or("").to_owned();
            let gsettings = nth(&e, 3);
            for (cnt, c) in ty.list().unwrap_or(&[]).iter().enumerate() {
                let m = nth(c, 1);
                let atitle = if gdesc.is_empty() {
                    m.str().unwrap_or("").to_owned()
                } else {
                    gdesc.clone()
                };
                let mut settings = gsettings.list().map(<[Sexp]>::to_vec).unwrap_or_default();
                settings.extend(nth(c, 2).list().map(<[Sexp]>::to_vec).unwrap_or_default());
                let settings = with(
                    title(format!(
                        "<after>KEYS={gkey}#{} TITLE: {atitle}</after>",
                        cnt + 1
                    )),
                    Sexp::List(settings),
                );
                new.push(Sexp::List(vec![nth(c, 0), m, settings]));
            }
        }
    }
    (!new.is_empty()).then(|| {
        Sexp::List(vec![
            str_("X"),
            str_("SUMO"),
            Sexp::List(new),
            Sexp::List(vec![Sexp::List(vec![
                sym("org-agenda-compact-blocks"),
                Sexp::Nil,
            ])]),
        ])
    })
}

/// org-mobile-create-sumo-agenda: agendas.org through org-store-agenda-views
/// with the SUMO command; its (name, md5) for checksums.dat.
fn create_sumo_agenda(s: &mut Session) -> Result<Option<(String, String)>, String> {
    let file = mobile_dir().join("agendas.org");
    let file1 = if use_encryption() {
        tempfile()
    } else {
        file.clone()
    };
    if !file1.parent().is_some_and(Path::is_dir) {
        return Err(format!("Cannot write to file {}", file1.display()));
    }
    let Some(sumo) = sumo_agenda_command() else {
        return Ok(None);
    };
    let mut cmd = sumo.list().unwrap_or(&[]).to_vec();
    cmd.push(Sexp::List(vec![Sexp::Str(file1.display().to_string())]));
    let saved = super::options::get("org-agenda-custom-commands");
    super::options::put(
        "org-agenda-custom-commands",
        toml::Value::String(format!("'({})", Sexp::List(cmd))),
    );
    let r = super::call(&mut s.ed, "org-store-agenda-views", Prefix::None);
    super::options::put(
        "org-agenda-custom-commands",
        saved.unwrap_or(toml::Value::Boolean(false)),
    );
    if let Some(eff) = s.ed.pending_effect.take() {
        s.perform(eff);
    }
    match r {
        Ok(()) => {}
        // The agenda is not ported yet: stage a placeholder view.
        Err(e) if e.contains("not yet ported") => {
            let note = "#+READONLY\n* Agenda Views\n  Fred's Org agenda is not ported yet: no agenda views were exported.\n";
            std::fs::write(&file1, note).map_err(|e| format!("{}: {e}", file1.display()))?;
        }
        Err(e) => return Err(e),
    }
    let text = std::fs::read(&file1).unwrap_or_default();
    if use_encryption() {
        encrypt_and_move(&file1, &file)?;
        cleanup_tempfile();
    }
    Ok(Some(("agendas.org".into(), md5_hex(&text))))
}

/// org-mobile-copy-agenda-files (and the empty capture file).
fn copy_agenda_files(
    files: &[(PathBuf, String)],
    sums: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let dir = mobile_dir();
    let bin = checksum_binary().unwrap_or_default();
    let hex = Regex::new("[[:xdigit:]]{30,40}").expect("regex");
    for (file, link) in files {
        if !file.exists() {
            continue;
        }
        let target = dir.join(link);
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        if use_encryption() {
            encrypt_and_move(file, &target)?;
        } else {
            std::fs::copy(file, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        let abs = std::path::absolute(file).unwrap_or(file.clone());
        let out = std::process::Command::new(&bin)
            .arg(&abs)
            .output()
            .map_err(|e| format!("{bin}: {e}"))?;
        if let Some(m) = hex.find(&String::from_utf8_lossy(&out.stdout)) {
            sums.push((link.clone(), m.as_str().to_owned()));
        }
    }
    let capture = dir.join(CAPTURE_FILE);
    let mut text = std::fs::read(&capture).unwrap_or_default();
    if text.is_empty() {
        text = b"\n".to_vec();
        write_staged(&capture, &text)?;
    }
    sums.push((CAPTURE_FILE.into(), md5_hex(&text)));
    cleanup_tempfile();
    Ok(())
}

/// The keywords of one org-todo-keywords entry, selection keys removed.
fn raw_todo_entries() -> Vec<Vec<String>> {
    let strip = |w: &str| w.split('(').next().unwrap_or(w).to_owned();
    match super::sexp::option("org-todo-keywords") {
        None => vec![vec!["TODO".into(), "DONE".into()]],
        Some(v) => {
            let l = v.list().unwrap_or(&[]);
            if l.first().is_some_and(|x| matches!(x, Sexp::Str(_))) {
                return vec![l.iter().filter_map(Sexp::str).map(strip).collect()];
            }
            l.iter()
                .map(|e| {
                    e.list()
                        .unwrap_or(&[])
                        .iter()
                        .skip(1)
                        .filter_map(Sexp::str)
                        .map(strip)
                        .collect()
                })
                .collect()
        }
    }
}

fn tag_alist_to_string(tags: &[syntax::TagEntry]) -> String {
    use syntax::TagEntry::*;
    tags.iter()
        .map(|t| match t {
            Tag(n, _) => n.as_str(),
            StartGroup => "{",
            EndGroup => "}",
            StartGroupTag => "[",
            EndGroupTag => "]",
            GroupTags => ":",
            Newline => "\\n",
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn push_new(v: &mut Vec<String>, x: &str) {
    if !v.iter().any(|y| y == x) {
        v.push(x.to_owned());
    }
}

/// The text of org-mobile-create-index-file.
fn index_text(s: &Session, files: &[(PathBuf, String)], with_agendas: bool) -> String {
    let mut files = files.to_vec();
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let (mut todo, mut done, mut tags): (Vec<String>, Vec<String>, Vec<String>) =
        (vec![], vec![], vec![]);
    for (f, _) in &files {
        let Ok(text) = s.org_text(f) else { continue };
        let st = super::settings_of(&text, f.parent());
        for k in st.not_done_names() {
            push_new(&mut todo, k);
        }
        for k in st.done_names() {
            push_new(&mut done, k);
        }
        for l in text.lines() {
            for t in syntax::headline(l, &st).map(|h| h.tags).unwrap_or_default() {
                push_new(&mut tags, &t);
            }
        }
        for t in st.file_tags.iter().chain(&st.tag_names()) {
            push_new(&mut tags, t);
        }
    }
    todo.retain(|k| !done.contains(k));
    let mut out = String::from("#+READONLY\n");
    for kwds in raw_todo_entries() {
        out.push_str(&format!("#+TODO: {}\n", kwds.join(" ")));
        let split = kwds
            .iter()
            .position(|k| k == "|")
            .unwrap_or(kwds.len().saturating_sub(1));
        let (twds, dwds) = kwds.split_at(split);
        todo.retain(|k| !twds.contains(k));
        done.retain(|k| !dwds.contains(k));
    }
    if !todo.is_empty() || !done.is_empty() {
        out.push_str(&format!(
            "#+TODO: {} | {}\n",
            todo.join(" "),
            done.join(" ")
        ));
    }
    let def: Vec<String> = tag_alist_to_string(&super::settings_of("", None).tags)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    tags.retain(|t| !def.contains(t));
    tags.sort_by_key(|t| t.to_lowercase());
    let all: Vec<String> = def.into_iter().chain(tags).collect();
    out.push_str(&format!("#+TAGS: {}\n", all.join(" ")));
    out.push_str(&format!(
        "#+ALLPRIORITIES: {}\n",
        super::options::string("org-mobile-allpriorities", "A B C")
    ));
    if with_agendas {
        out.push_str("* [[file:agendas.org][Agenda Views]]\n");
    }
    for (_, link) in &files {
        out.push_str(&format!("* [[file:{link}][{link}]]\n"));
    }
    out
}

/// org-mobile-create-index-file: its (name, md5).
fn create_index_file(s: &Session, files: &[(PathBuf, String)]) -> Result<(String, String), String> {
    let dir = mobile_dir();
    let text = index_text(s, files, dir.join("agendas.org").exists());
    write_staged(&dir.join(index_file()), text.as_bytes())?;
    Ok((index_file(), md5_hex(text.as_bytes())))
}

/// org-mobile-write-checksums: `checksums.dat`, most recent entry first.
fn write_checksums(sums: &[(String, String)]) -> Result<(), String> {
    let text: String = sums
        .iter()
        .rev()
        .map(|(f, s)| format!("{s}  {f}\n"))
        .collect();
    let p = mobile_dir().join("checksums.dat");
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
}

/// org-mobile-escape-olp: org-link-encode for `:` and `/`.
pub fn escape_olp(s: &str) -> String {
    s.replace('%', "%25")
        .replace(':', "%3A")
        .replace('/', "%2F")
}

/// org-link-decode.
fn link_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// org-mobile-get-outline-path-link: `olp:FILE:A/B/HEADING` for heading `h`
/// (for the agenda export, when an entry has no ID).
pub fn outline_path_link(ed: &Editor, h: usize) -> String {
    let st = super::settings(ed);
    let file = ed
        .path
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut path = super::refile::outline_path(&ed.buf, &st, h, false);
    path.push(
        syntax::headline(&ed.buf.line(h), &st)
            .map(|x| x.title(&ed.buf.line(h)).to_owned())
            .unwrap_or_default(),
    );
    let path: Vec<String> = path.iter().map(|p| escape_olp(p)).collect();
    format!("olp:{}:{}", escape_olp(&file), path.join("/"))
}

// ---- pull ----

/// org-mobile-pull.
fn pull(s: &mut Session) -> Result<(), String> {
    check_setup()?;
    run_hooks("org-mobile-pre-pull-hook")?;
    let Some((buf, at)) = move_capture(s)? else {
        s.ed.set_msg("No new items");
        return Ok(());
    };
    let flagged = apply(s, buf, at)?;
    run_hooks("org-mobile-post-pull-hook")?;
    if !flagged.is_empty() {
        // The FLAGGED agenda ("?"), restricted to the flagged files.
        let saved = super::options::get("org-agenda-files");
        let list = flagged
            .iter()
            .map(|f| toml::Value::String(f.display().to_string()))
            .collect();
        super::options::put("org-agenda-files", toml::Value::Array(list));
        let r = super::call_with(&mut s.ed, "org-agenda", Prefix::None, "?".into());
        super::options::put(
            "org-agenda-files",
            saved.unwrap_or(toml::Value::Boolean(false)),
        );
        match r {
            Ok(()) => {
                if let Some(eff) = s.ed.pending_effect.take() {
                    s.perform(eff);
                }
            }
            Err(_) => {
                let names: Vec<String> = flagged.iter().map(|f| f.display().to_string()).collect();
                s.ed.set_msg(format!("Flagged entries in: {}", names.join(", ")));
            }
        }
    }
    Ok(())
}

/// org-mobile-move-capture: append the capture file to the inbox and empty
/// it. Returns the inbox buffer and the first new line.
fn move_capture(s: &mut Session) -> Result<Option<(usize, usize)>, String> {
    let capture = mobile_dir().join(CAPTURE_FILE);
    let content = if use_encryption() {
        cleanup_tempfile();
        let t = tempfile();
        let enc = PathBuf::from(format!("{}_enc", t.display()));
        std::fs::copy(&capture, &enc).map_err(|e| format!("{}: {e}", capture.display()))?;
        let r = openssl(true, &enc, &t);
        let _ = std::fs::remove_file(&enc);
        r?;
        std::fs::read(&t).unwrap_or_default()
    } else {
        std::fs::read(&capture).unwrap_or_default()
    };
    let content = String::from_utf8_lossy(&content).into_owned();
    if content.trim().is_empty() {
        cleanup_tempfile();
        return Ok(None);
    }
    let inbox = inbox();
    let i = s.org_buffer(&inbox)?;
    let lines: Vec<String> = content
        .strip_suffix('\n')
        .unwrap_or(&content)
        .split('\n')
        .map(str::to_owned)
        .collect();
    let at = s.org_with_buffer(i, |ed| {
        if ed.line_count() == 1 && ed.buf.len_bytes() == 0 {
            super::splice(ed, 0, 1, &lines);
            0
        } else {
            let n = ed.line_count();
            super::insert_lines(ed, n, &lines);
            n
        }
    });
    s.org_save(i)?;
    write_staged(&capture, b"")?;
    update_checksum_for_capture_file("")?;
    Ok(Some((i, at)))
}

/// org-mobile-update-checksum-for-capture-file.
fn update_checksum_for_capture_file(text: &str) -> Result<(), String> {
    let p = mobile_dir().join("checksums.dat");
    let Ok(old) = std::fs::read_to_string(&p) else {
        return Ok(());
    };
    let re = Regex::new(&format!(
        r"(?m)([[:xdigit:]]{{30,}}).*?{}[ \t]*$",
        regex::escape(CAPTURE_FILE)
    ))
    .expect("regex");
    if let Some(m) = re.captures(&old).and_then(|c| c.get(1)) {
        let new = format!(
            "{}{}{}",
            &old[..m.start()],
            md5_hex(text.as_bytes()),
            &old[m.end()..]
        );
        std::fs::write(&p, new).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}

fn buffer_lines(s: &mut Session, buf: usize) -> Vec<String> {
    s.org_with_buffer(buf, |ed| {
        (0..ed.line_count()).map(|l| ed.buf.line(l)).collect()
    })
}

/// Insert `text` after the stars of line `l` (an error marker).
fn mark_error(s: &mut Session, buf: usize, l: usize, text: &str) {
    s.org_with_buffer(buf, |ed| {
        let line = ed.buf.line(l);
        let at = line.char_indices().nth(2).map_or(line.len(), |(i, _)| i);
        super::set_line(ed, l, &format!("{}{text}{}", &line[..at], &line[at..]));
    });
}

/// The text between a `** Old value`/`** New value` line and the next heading.
fn value_after(lines: &[String], from: usize, end: usize, what: &str) -> Option<(usize, String)> {
    let re = Regex::new(&format!(r"^\** {what} value[ \t]*$")).expect("regex");
    let i = (from..end).find(|&i| re.is_match(&lines[i]))?;
    let stop = syntax::next_heading(lines, i, usize::MAX).unwrap_or(lines.len());
    let mut body: Vec<&String> = lines[i + 1..stop].iter().collect();
    if stop == lines.len() {
        // org-back-over-empty-lines at the end of the buffer.
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
    }
    Some((stop, body.iter().map(|l| format!("{l}\n")).collect()))
}

/// org-mobile-locate-entry: (file, line) of an `id:` or `olp:` link.
fn locate_entry(s: &mut Session, link: &str) -> Result<Option<(PathBuf, usize)>, String> {
    if let Some(id) = link.strip_prefix("id:") {
        return Ok(super::find_id_file(s, id));
    }
    let Some(rest) = link.strip_prefix("olp:") else {
        return Ok(None);
    };
    let (file, path) = match rest.split_once(':') {
        Some((f, p)) => (f, Some(p)),
        None => (rest, None),
    };
    let file = super::options::directory().join(super::options::expand(&link_decode(file)));
    let Some(path) = path else {
        // The top level of the file: a new line at its end.
        let at = s.org_with_file(&file, |ed| {
            let n = ed.line_count();
            super::insert_lines(ed, n, &[String::new()]);
            ed.line_count() - 1
        })?;
        return Ok(Some((file, at)));
    };
    let path: Vec<String> = split_string(path, &Regex::new("/").expect("regex"))
        .iter()
        .map(|p| link_decode(p))
        .collect();
    let text = s
        .org_text(&file)
        .map_err(|_| format!("File not found :{}", file.display()))?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    find_olp(&lines, &super::settings_of(&text, file.parent()), &path).map(|h| Some((file, h)))
}

/// org-find-olp within one file's lines.
fn find_olp(lines: &[String], st: &syntax::Settings, path: &[String]) -> Result<usize, String> {
    let odd = st.opt_bool("org-odd-levels-only", false);
    let (mut lmin, mut lmax, mut start, mut end) = (1, 1, 0, lines.len());
    let mut found = None;
    for heading in path {
        let hits: Vec<(usize, usize)> = (start..end)
            .filter_map(|i| {
                syntax::headline(&lines[i], st)
                    .filter(|h| h.title(&lines[i]) == heading)
                    .map(|h| (i, h.level))
            })
            .filter(|&(_, lv)| lv >= lmin && lv <= lmax)
            .collect();
        match hits.as_slice() {
            [] => return Err(format!("Heading not found on level {lmax}: {heading}")),
            [(i, lv)] => {
                found = Some(*i);
                start = *i;
                lmin = lv + 1;
                lmax = lmin + usize::from(odd);
                end = syntax::subtree_end(lines, *i);
            }
            _ => return Err(format!("Heading not unique on level {lmax}: {heading}")),
        }
    }
    found.ok_or_else(|| "Empty outline path".into())
}

/// org-mobile-timestamp-buffer: whether a line was inserted at `0` or `1`.
fn timestamp_buffer(s: &mut Session, path: &Path) -> Result<Option<usize>, String> {
    let now = super::now();
    let (y, m, d, hh, mm, _) = super::localtime(now);
    let stamp = format!(
        "#+LAST_MOBILE_CHANGE: {y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{:02}",
        now.rem_euclid(60)
    );
    s.org_with_file(path, |ed| {
        let re = Regex::new(r"^([ \t]*)#\+LAST_MOBILE_CHANGE:").expect("regex");
        for l in 0..ed.line_count() {
            let line = ed.buf.line(l);
            if let Some(c) = re.captures(&line) {
                let indent = c[1].to_owned();
                super::set_line(ed, l, &format!("{indent}{stamp}"));
                return None;
            }
        }
        let first = ed.buf.line(0);
        let at = usize::from(Regex::new(r"-\*-.*-\*-").expect("regex").is_match(&first));
        super::insert_lines(ed, at, std::slice::from_ref(&stamp));
        Some(at)
    })
}

/// org-mobile-apply on buffer `buf` from line `beg`: the files with
/// FLAGGED entries.
fn apply(s: &mut Session, buf: usize, beg: usize) -> Result<Vec<PathBuf>, String> {
    let note_re = Regex::new(r"^\*\* Note ID: [-0-9A-F]+[ \t]*$").expect("regex");
    s.org_with_buffer(buf, |ed| {
        for l in (beg..ed.line_count()).rev() {
            if note_re.is_match(&ed.buf.line(l)) {
                super::delete_lines(ed, l, 1);
            }
        }
    });
    let lines = buffer_lines(s, buf);
    let cnt_new = lines[beg.min(lines.len())..]
        .iter()
        .filter_map(|l| l.strip_prefix("* "))
        .filter(|t| {
            t.chars().count() >= 2
                && !t
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .eq_ignore_ascii_case("f(")
        })
        .count();
    let flag_re =
        Regex::new(r"^\*+[ \t]+F\(([^():\n]*)(:([^()\n]*))?\)[ \t]+\[\[((id|olp):([^\]\n]+))")
            .expect("regex");
    let (mut cnt_edit, mut cnt_flag, mut cnt_error) = (0, 0, 0);
    let mut stamped: Vec<PathBuf> = vec![];
    let mut flagged: Vec<PathBuf> = vec![];
    let mut from = beg;
    loop {
        let lines = buffer_lines(s, buf);
        let Some((i, c)) =
            (from..lines.len()).find_map(|i| flag_re.captures(&lines[i]).map(|c| (i, c)))
        else {
            break;
        };
        let action = c[1].to_owned();
        let data = c.get(3).map(|m| m.as_str().to_owned());
        let link = c[4].to_owned();
        let eos = syntax::subtree_end(&lines, i);
        let note: String = lines[i + 1..eos].iter().map(|l| format!("{l}\n")).collect();
        let is_flag = action.is_empty();
        if !is_flag {
            cnt_edit += 1;
        }
        from = i + 1;
        let (path, mut line) = match locate_entry(s, &link) {
            Ok(Some(x)) => x,
            Ok(None) => {
                mark_error(s, buf, i, "BAD REFERENCE ");
                cnt_error += 1;
                continue;
            }
            Err(e) => {
                mark_error(s, buf, i, &format!("{e} "));
                cnt_error += 1;
                continue;
            }
        };
        if !stamped.contains(&path) {
            if let Some(at) = timestamp_buffer(s, &path)?
                && line >= at
            {
                line += 1;
            }
            stamped.push(path.clone());
        }
        if !is_flag && action != "edit" {
            mark_error(s, buf, i, "BAD FLAG ");
            cnt_error += 1;
            continue;
        }
        let lines = buffer_lines(s, buf);
        let trim = |v: Option<String>| {
            let v = v.filter(|x| !x.trim().is_empty());
            if data.as_deref() == Some("body") {
                v
            } else {
                v.map(|x| x.trim().to_owned())
            }
        };
        let old = value_after(&lines, i + 1, eos, "Old");
        let new = value_after(&lines, old.as_ref().map_or(i + 1, |o| o.0), eos, "New");
        let (old, new) = (trim(old.map(|o| o.1)), trim(new.map(|n| n.1)));
        let what = data.clone().unwrap_or_default();
        let r = if is_flag {
            cnt_flag += 1;
            on_target(s, &path, line, |ed| flag(ed, &note))
        } else if what == "refile" {
            refile(s, &path, line, new.as_deref().unwrap_or(""))
        } else {
            on_target(s, &path, line, |ed| {
                edit(ed, &what, old.as_deref(), new.as_deref())
            })
        };
        match r {
            Ok(()) => {
                if !matches!(
                    what.as_str(),
                    "delete" | "archive" | "archive-sibling" | "addheading" | "refile"
                ) && s.org_text(&path).is_ok_and(|t| {
                    let lines: Vec<String> = t.lines().map(str::to_owned).collect();
                    let st = super::settings_of(&t, path.parent());
                    syntax::heading_at_or_before(&lines, line)
                        .and_then(|h| syntax::headline(&lines[h], &st))
                        .is_some_and(|h| h.tags.iter().any(|t| t == "FLAGGED"))
                }) && !flagged.contains(&path)
                {
                    flagged.push(path.clone());
                }
                s.org_with_buffer(buf, |ed| {
                    let end = syntax::subtree_end(&ed.buf, i);
                    super::delete_lines(ed, i, end - i);
                });
                from = i;
            }
            Err(e) => {
                mark_error(s, buf, i, &format!("{e} "));
                cnt_error += 1;
            }
        }
    }
    s.org_save(buf)?;
    s.ed.set_msg(format!(
        "{cnt_new} new, {cnt_edit} edits, {cnt_flag} flags, {cnt_error} errors"
    ));
    Ok(flagged)
}

/// Run `f` with the target buffer current, the cursor at `line` (as
/// org-with-point-at), then perform its Session work and come back.
fn on_target(
    s: &mut Session,
    path: &Path,
    line: usize,
    f: impl FnOnce(&mut Editor) -> Result<(), String>,
) -> Result<(), String> {
    let back = s.org_current();
    let i = s.org_buffer(path)?;
    let saved = s.org_with_buffer(i, |ed| ed.cur);
    s.org_show(i);
    s.ed.set_cursor(line.min(s.ed.line_count() - 1), 0);
    s.ed.undo.begin(s.ed.cur.pos());
    let r = f(&mut s.ed);
    s.ed.undo.end(s.ed.cur.pos());
    if let Some(eff) = s.ed.pending_effect.take() {
        s.perform(eff);
    }
    if s.org_current() == i {
        s.ed.cur = saved;
        s.ed.clamp_cursor();
    }
    if s.org_current() != back {
        s.org_show(back);
    }
    r
}

/// The flag action: FLAGGED tag and the note in THEFLAGGINGNOTE.
fn flag(ed: &mut Editor, note: &str) -> Result<(), String> {
    let h = super::fold::back_to_heading(ed, ed.cur.line).ok_or("Before first headline")?;
    super::tags::toggle_tag(ed, h, "FLAGGED", Some(true));
    super::props::put(ed, Some(h), "THEFLAGGINGNOTE", &note.replace('\n', "\\n"))
}

/// The refile edit: copy the subtree under the entry `new` names, then cut it.
fn refile(s: &mut Session, path: &Path, line: usize, new: &str) -> Result<(), String> {
    let tree: String = s.org_with_file(path, |ed| {
        let end = syntax::subtree_end(&ed.buf, line);
        (line..end)
            .map(|l| format!("{}\n", ed.buf.line(l)))
            .collect()
    })?;
    let (dest, dl) =
        locate_entry(s, new)?.ok_or_else(|| format!("Cannot find refile target {new}"))?;
    let (at, n) = s.org_with_file(&dest, |ed| {
        let odd = super::settings(ed).opt_bool("org-odd-levels-only", false);
        let (at, level) = match syntax::level(&ed.buf.line(dl)) {
            Some(lv) => (
                syntax::subtree_end(&ed.buf, dl),
                super::structure::valid_level(odd, lv, 1),
            ),
            None => (dl, 1),
        };
        let before = ed.line_count();
        super::refile::insert_tree(ed, at, level, &tree);
        (at, ed.line_count() - before)
    })?;
    let same = std::fs::canonicalize(&dest).ok() == std::fs::canonicalize(path).ok();
    let line = if same && at <= line { line + n } else { line };
    s.org_with_file(path, |ed| {
        let end = syntax::subtree_end(&ed.buf, line);
        super::delete_lines(ed, line, end - line);
    })
}

/// org-split-string: split at `re`, without empty ends.
fn split_string(s: &str, re: &Regex) -> Vec<String> {
    let mut v: Vec<String> = re.split(s).map(str::to_owned).collect();
    if v.first().is_some_and(String::is_empty) {
        v.remove(0);
    }
    if v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    v
}

/// org-mobile-tags-same-p.
pub fn tags_same_p(a: &[String], b: &[String]) -> bool {
    a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}

/// org-mobile-bodies-same-p.
pub fn bodies_same_p(a: Option<&str>, b: Option<&str>) -> bool {
    let re = Regex::new("[ \t]*\n[ \t]*").expect("regex");
    let norm = |s: &str| split_string(s.trim(), &re).join("\n");
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => norm(a) == norm(b),
        _ => false,
    }
}

/// org-mobile-edit: edit WHAT of the entry at the cursor from OLD to NEW.
pub fn edit(
    ed: &mut Editor,
    what: &str,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<(), String> {
    let l = ed.cur.line;
    let st = super::settings(ed);
    let line = ed.buf.line(l);
    let hl = syntax::headline(&line, &st);
    let nil = |v: Option<&str>| v.unwrap_or("nil").to_owned();
    match what {
        "todo" | "todostate" => {
            let h = super::fold::back_to_heading(ed, l).ok_or("Before first headline")?;
            let current = syntax::headline(&ed.buf.line(h), &st).and_then(|x| x.todo);
            if new == Some("DONEARCHIVE") {
                let done = current
                    .as_deref()
                    .and_then(|c| st.seq_of(c))
                    .unwrap_or(&st.seqs[0])
                    .done
                    .first()
                    .map(|k| k.name.clone());
                super::todo::todo_to_inhibit(ed, done, Some(super::todo::How::Note))?;
                super::call(ed, "org-archive-subtree-default", Prefix::None)
            } else if new == current.as_deref() {
                Ok(())
            } else if current.as_deref() == old || force_change("todo") {
                super::todo::todo_to_inhibit(
                    ed,
                    new.map(str::to_owned),
                    Some(super::todo::How::Note),
                )
            } else {
                Err(format!(
                    "State before change was expected as \"{}\", but is \"{}\"",
                    nil(old),
                    nil(current.as_deref())
                ))
            }
        }
        "tags" => {
            let h = super::fold::back_to_heading(ed, l).ok_or("Before first headline")?;
            let current = syntax::headline(&ed.buf.line(h), &st)
                .map(|x| x.tags)
                .unwrap_or_default();
            let colons = Regex::new(":+").expect("regex");
            let new1 = new.map(|n| split_string(n, &colons)).unwrap_or_default();
            let old1 = old.map(|n| split_string(n, &colons)).unwrap_or_default();
            if tags_same_p(&current, &new1) {
                Ok(())
            } else if tags_same_p(&current, &old1) || force_change("tags") {
                super::tags::set_tags(ed, h, &new1);
                Ok(())
            } else {
                let cur = if current.is_empty() {
                    String::new()
                } else {
                    format!("({})", current.join(" "))
                };
                Err(format!(
                    "Tags before change were expected as \"{}\", but are \"{cur}\"",
                    old.unwrap_or("")
                ))
            }
        }
        "priority" => {
            let Some(hl) = hl else { return Ok(()) };
            let current = hl
                .priority_range
                .map(|r| line[r.start + 2..r.end - 1].to_owned());
            if current.as_deref() == new {
                Ok(())
            } else if current.as_deref() == old || force_change("tags") {
                super::todo::set_priority(ed, new.and_then(syntax::priority_value))
            } else {
                Err(format!(
                    "Priority was expected to be {}, but is {}",
                    nil(old),
                    nil(current.as_deref())
                ))
            }
        }
        "heading" => {
            let Some(hl) = hl else { return Ok(()) };
            let current = hl.title(&line);
            if Some(current) == new {
                Ok(())
            } else if Some(current) == old || force_change("heading") {
                let text = format!(
                    "{}{}{}",
                    &line[..hl.title.start],
                    new.unwrap_or(""),
                    &line[hl.title.end..]
                );
                super::set_line(ed, l, &text);
                if super::options::bool("org-auto-align-tags", true) && !hl.tags.is_empty() {
                    super::tags::align(ed, l);
                }
                Ok(())
            } else {
                Err("Heading changed in the mobile device and on the computer".into())
            }
        }
        "addheading" => {
            let new = new.unwrap_or("");
            match hl {
                Some(hl) => {
                    super::fold::show_subtree(ed, l);
                    let odd = st.opt_bool("org-odd-levels-only", false);
                    let level = super::structure::valid_level(odd, hl.level, 1);
                    let at = syntax::subtree_end(&ed.buf, l);
                    let mut lines: Vec<String> = new.split('\n').map(str::to_owned).collect();
                    lines[0] = format!("{} {}", "*".repeat(level), lines[0]);
                    super::insert_lines(ed, at, &lines);
                    ed.set_cursor(at, 0);
                }
                None => super::set_line(ed, l, &format!("* {new}{line}")),
            }
            Ok(())
        }
        "delete" => super::structure::copy_subtree(ed, 1, true),
        "archive" => super::call(ed, "org-archive-subtree", Prefix::None),
        "archive-sibling" => super::call(ed, "org-archive-to-archive-sibling", Prefix::None),
        "body" => {
            let end = syntax::entry_end(&ed.buf, l);
            let current: String = (l + 1..end)
                .map(|i| format!("{}\n", ed.buf.line(i)))
                .collect();
            let current = (!current.trim().is_empty()).then_some(current);
            if bodies_same_p(current.as_deref(), new) {
                Ok(())
            } else if bodies_same_p(current.as_deref(), old) || force_change("body") {
                let new = new.unwrap_or("");
                let body: Vec<String> = if new.is_empty() {
                    vec![]
                } else {
                    new.strip_suffix('\n')
                        .unwrap_or(new)
                        .split('\n')
                        .map(str::to_owned)
                        .collect()
                };
                super::splice(ed, l + 1, end - l - 1, &body);
                Ok(())
            } else {
                Err("Body was changed in the mobile device and on the computer".into())
            }
        }
        _ => Ok(()),
    }
}

// ---- md5 (Emacs `md5', for checksums.dat) ----

/// The MD5 digest of `data` in hex.
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter()
        .flat_map(|x| x.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(escape_olp("a/b:c%"), "a%2Fb%3Ac%25");
        assert!(tags_same_p(
            &["a".into(), "b".into()],
            &["b".into(), "a".into()]
        ));
    }

    #[test]
    fn edit_todo_and_heading() {
        let mut e = super::super::tests::org("* TODO Task\nbody", "");
        edit(&mut e, "todo", Some("TODO"), Some("DONE")).unwrap();
        assert!(e.buf.line(0).starts_with("* DONE Task"), "{}", e.buf.text());
        // A stale old value is refused unless forced.
        assert!(edit(&mut e, "todo", Some("TODO"), Some("WAIT")).is_err());
        edit(&mut e, "heading", Some("Task"), Some("Renamed")).unwrap();
        assert!(
            e.buf.line(0).starts_with("* DONE Renamed"),
            "{}",
            e.buf.text()
        );
    }
}
