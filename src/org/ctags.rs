//! org-ctags.el: `<<targets>>` as tags of an etags `TAGS` table built by
//! the `ctags` program, so plain links find targets in other files.
//!
//! Fred has no etags: the `TAGS` file is read here. The active table is
//! the one last visited or created, else `TAGS` beside the buffer's file
//! (org-ctags--visit-tags-table), else the `tags-file-name` option.
//! `org-ctags-enable` (or `org-ctags-enable = true` in `[org]`) installs
//! `org-ctags-open-link-functions` in org-open-link-functions, which
//! links.rs runs (as upstream) before searching for a fuzzy, custom-id,
//! coderef or radio link.

use super::Prefix;
use super::links::{self, Link};
use crate::editor::Editor;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

/// org-ctags-tag-regexp.
pub const TAG_REGEXP: &str = "/<<([^<>]+)>>/\\1/d,definition/";

/// org-ctags--open-link-functions-list.
pub const OPEN_LINK_FUNCTIONS: &[&str] = &[
    "org-ctags-find-tag",
    "org-ctags-ask-rebuild-tags-file-then-find-tag",
    "org-ctags-rebuild-tags-file-then-find-tag",
    "org-ctags-ask-append-topic",
    "org-ctags-append-topic",
    "org-ctags-ask-visit-buffer-or-file",
    "org-ctags-visit-buffer-or-file",
    "org-ctags-fail-silently",
];

thread_local! {
    /// tags-file-name (visit-tags-table).
    static TAGS_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// org-ctags-enable was called (None: the `org-ctags-enable` option).
    static ENABLED: Cell<Option<bool>> = const { Cell::new(None) };
    /// org-ctags-find-tag-history.
    static HISTORY: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
    /// The default search runs after the hook functions all failed.
    static INHIBIT: Cell<bool> = const { Cell::new(false) };
}

/// org-ctags-path-to-ctags.
fn ctags_program() -> String {
    let dflt = if which("ctags-exuberant") {
        "ctags-exuberant"
    } else {
        "ctags"
    };
    super::options::string("org-ctags-path-to-ctags", dflt)
}

fn which(prog: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

/// org-ctags-enabled-p, and the functions installed by org-ctags-enable.
fn installed() -> bool {
    super::options::bool("org-ctags-enabled-p", true)
        && ENABLED
            .with(Cell::get)
            .unwrap_or_else(|| super::options::bool("org-ctags-enable", false))
}

/// org-ctags-open-link-functions.
fn open_link_functions() -> Vec<String> {
    super::options::strings(
        "org-ctags-open-link-functions",
        &[
            "org-ctags-find-tag",
            "org-ctags-ask-rebuild-tags-file-then-find-tag",
            "org-ctags-ask-append-topic",
        ],
    )
}

/// org-open-link-functions: the user's list, then (when enabled) org-ctags's.
fn hook_functions() -> Vec<String> {
    let mut v = super::options::strings("org-open-link-functions", &[]);
    if installed() {
        for f in open_link_functions() {
            if !v.contains(&f) {
                v.push(f);
            }
        }
    }
    v
}

/// The buffer's directory (default-directory).
fn buffer_dir(ed: &Editor) -> PathBuf {
    ed.path
        .as_deref()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// The active tags table.
fn tags_file(ed: &Editor) -> Option<PathBuf> {
    TAGS_FILE.with(|t| t.borrow().clone()).or_else(|| {
        let local = buffer_dir(ed).join("TAGS");
        if ed.path.is_some() && local.is_file() {
            return Some(local);
        }
        let opt = super::options::string("tags-file-name", "");
        (!opt.is_empty()).then(|| super::options::expand(&opt))
    })
}

/// visit-tags-table.
pub fn visit_tags_table(path: Option<PathBuf>) {
    TAGS_FILE.with(|t| *t.borrow_mut() = path);
}

/// The tag lines of an etags file: (file, pattern, name, line, pos).
fn tag_entries(text: &str) -> Vec<(String, String, usize, usize)> {
    let mut out = vec![];
    let mut file = String::new();
    let mut header = false;
    for l in text.lines() {
        if l == "\x0c" {
            header = true;
            continue;
        }
        if header {
            header = false;
            file = l.rsplit_once(',').map_or(l, |(f, _)| f).to_owned();
            continue;
        }
        let Some((pat, rest)) = l.split_once('\x7f') else {
            continue;
        };
        let Some((name, nums)) = rest.split_once('\x01') else {
            continue;
        };
        let Some((line, pos)) = nums.split_once(',') else {
            continue;
        };
        let (Ok(line), Ok(pos)) = (line.parse(), pos.parse()) else {
            continue;
        };
        let _ = pat;
        out.push((file.clone(), name.to_owned(), line, pos));
    }
    out
}

/// org-ctags-get-filename-for-tag on the text of a TAGS file.
pub fn filename_for_tag(text: &str, tag: &str) -> Option<(String, usize, usize)> {
    tag_entries(text)
        .into_iter()
        .find(|(_, n, _, _)| n == tag)
        .map(|(f, _, l, p)| (f, l, p))
}

/// org-ctags-all-tags-in-current-tags-table on a TAGS text (pushed, so
/// the last tag comes first).
pub fn all_tags(text: &str) -> Vec<String> {
    let mut v: Vec<String> = tag_entries(text)
        .into_iter()
        .map(|(_, n, _, _)| n)
        .collect();
    v.reverse();
    v
}

/// shell-quote-argument (POSIX).
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let mut out = String::new();
    for c in s.chars() {
        if c == '\n' {
            out.push_str("'\n'");
        } else {
            if !(c.is_ascii_alphanumeric() || "-_./".contains(c)) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

/// The command org-ctags-create-tags runs for directory `dir` (with a slash).
pub fn create_tags_command(program: &str, dir: &str) -> String {
    let d = shell_quote(dir);
    format!(
        "{program} --langdef=orgmode --langmap=orgmode:.org --regex-orgmode={} -f {d}TAGS -e -R {d}*",
        shell_quote(TAG_REGEXP)
    )
}

/// org-ctags-create-tags.
pub fn create_tags(ed: &Editor, dir: Option<&str>) -> Result<bool, String> {
    if ed.path.is_none() {
        return Err("Assertion failed: (buffer-file-name)".into());
    }
    let dir = match dir {
        Some(d) => {
            let p = super::options::expand(d);
            let p = if p.is_relative() {
                buffer_dir(ed).join(p)
            } else {
                p
            };
            let mut s = p.display().to_string();
            if !s.ends_with('/') {
                s.push('/');
            }
            s
        }
        None => format!("{}/", buffer_dir(ed).display()),
    };
    let cmd = create_tags_command(&ctags_program(), &dir);
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(buffer_dir(ed))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        let t = PathBuf::from(format!("{dir}TAGS"));
        if t.is_file() {
            visit_tags_table(Some(t));
        }
    }
    Ok(ok)
}

fn read_tags(ed: &Editor) -> Option<(PathBuf, String)> {
    let f = tags_file(ed)?;
    let text = std::fs::read_to_string(&f).ok()?;
    Some((f, text))
}

/// org-ctags-tag-list.
fn tag_list(ed: &Editor) -> Vec<String> {
    read_tags(ed).map(|(_, t)| all_tags(&t)).unwrap_or_default()
}

/// org-ctags-find-tag: visit tag NAME (xref-find-definitions with
/// etags matches the explicit name, ignoring case).
pub fn find_tag(ed: &mut Editor, name: &str) -> bool {
    let Some((f, text)) = read_tags(ed) else {
        return false;
    };
    let hit = tag_entries(&text)
        .into_iter()
        .find(|(_, n, _, _)| n.to_lowercase() == name.to_lowercase());
    let Some((file, _, line, _)) = hit else {
        return false;
    };
    let base = f.parent().map(Path::to_path_buf).unwrap_or_default();
    let path = base.join(file);
    // org--ctags-set-org-mark-before-finding-tag.
    if ed.org.is_some() && super::options::bool("org-ctags-enabled-p", true) {
        links::mark_ring_push(ed);
    }
    super::effect(ed, move |s| {
        if let Err(e) = s.org_visit(&path, line.saturating_sub(1)) {
            s.ed.set_err(e);
        }
    });
    true
}

/// Capitalize each word (Emacs `capitalize`).
fn capitalize(s: &str) -> String {
    let mut out = String::new();
    let mut start = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// org-ctags-new-topic-template with `%t` replaced.
fn topic_text(title: &str) -> String {
    super::options::string("org-ctags-new-topic-template", "* <<%t>>\n\n\n\n\n\n")
        .replace("%t", &capitalize(title))
}

/// The template's lines (its last newline ends the buffer).
fn topic_lines(title: &str) -> Vec<String> {
    let t = topic_text(title);
    let t = t.strip_suffix('\n').unwrap_or(&t);
    t.split('\n').map(str::to_owned).collect()
}

/// org-ctags-append-topic.
pub fn append_topic(ed: &mut Editor, name: &str, narrow: bool) -> bool {
    super::fold::widen(ed);
    // (goto-char (point-max)) (newline 2), then the template.
    let last = ed.line_count() - 1;
    let mut lines = vec![ed.buf.line(last).to_string(), String::new()];
    lines.extend(topic_lines(name));
    lines.push(String::new());
    super::splice(ed, last, 1, &lines);
    ed.set_msg(format!(
        "Adding topic in buffer {}",
        ed.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or("*scratch*".into(), |f| f.to_string_lossy().into_owned())
    ));
    // (backward-char 4) (end-of-line) (forward-line 2).
    let last = ed.line_count() - 1;
    ed.set_cursor(last.saturating_sub(2), 0);
    if narrow {
        let _ = super::call(ed, "org-narrow-to-subtree", Prefix::None);
    }
    true
}

/// org-ctags-open-file: visit or create FILE, add a topic, org-mode-restart.
pub fn open_file(ed: &mut Editor, file: PathBuf, title: String) {
    super::effect(ed, move |s| {
        if let Err(e) = s.org_visit(&file, 0) {
            s.ed.set_err(format!("Error {e:?} in org-ctags-open-file"));
            return;
        }
        let ed = &mut s.ed;
        let empty = ed.line_count() == 1 && ed.buf.len_bytes() == 0;
        let at = if empty { 0 } else { ed.line_count() };
        super::insert_lines(ed, at, &topic_lines(&title));
        let last = ed.line_count() - 1;
        ed.set_cursor(last, 0);
        if let Some(o) = &mut ed.org {
            o.reset_settings();
        }
        ed.set_msg("Inserted new file text OK");
    });
}

/// The file org-ctags-visit-buffer-or-file visits for NAME.
fn org_file_for(ed: &Editor, name: &str) -> PathBuf {
    let p = super::options::expand(name);
    let p = if p.is_relative() {
        buffer_dir(ed).join(p)
    } else {
        p
    };
    PathBuf::from(format!("{}.org", p.display()))
}

/// A y-or-n question whose answer goes to `k`.
fn ask(ed: &mut Editor, q: &str, k: impl FnOnce(&mut Editor, bool) + Send + 'static) {
    super::menu(
        ed,
        q,
        vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
        move |ed, a| k(ed, a == "y"),
    );
}

type Next = Box<dyn FnOnce(&mut Editor, bool) + Send>;

/// Run one org-open-link-functions member on NAME; `k` gets its result.
fn run_hook_fn(ed: &mut Editor, f: &str, name: String, k: Next) {
    match f {
        "org-ctags-find-tag" => {
            let ok = find_tag(ed, &name);
            k(ed, ok)
        }
        "org-ctags-rebuild-tags-file-then-find-tag" => {
            let ok = rebuild_then_find(ed, &name);
            k(ed, ok)
        }
        "org-ctags-ask-rebuild-tags-file-then-find-tag" => {
            let Some(p) = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()) else {
                return k(ed, false);
            };
            let dir = format!(
                "{}/",
                p.parent()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default()
            );
            ask(
                ed,
                &format!(
                    "Tag \u{2018}{name}\u{2019} not found.  Rebuild table \u{2018}{dir}/TAGS\u{2019} and look again?"
                ),
                move |ed, y| {
                    let ok = y && rebuild_then_find(ed, &name);
                    k(ed, ok)
                },
            )
        }
        "org-ctags-append-topic" => {
            let ok = append_topic(ed, &name, false);
            k(ed, ok)
        }
        "org-ctags-ask-append-topic" => ask(
            ed,
            &format!("Topic \u{2018}{name}\u{2019} not found; append to end of buffer?"),
            move |ed, y| {
                let ok = y && append_topic(ed, &name, false);
                k(ed, ok)
            },
        ),
        "org-ctags-visit-buffer-or-file" => visit_buffer_or_file(ed, &name, Create::No, k),
        "org-ctags-ask-visit-buffer-or-file" => visit_buffer_or_file(ed, &name, Create::Ask, k),
        "org-ctags-fail-silently" => k(ed, true),
        other => {
            let ok = super::call_with(ed, other, Prefix::None, name).is_ok();
            k(ed, ok)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Create {
    No,
    Yes,
    Ask,
}

/// org-ctags-visit-buffer-or-file.
pub fn visit_buffer_or_file(ed: &mut Editor, name: &str, create: Create, k: Next) {
    let file = org_file_for(ed, name);
    if file.exists() {
        ed.set_msg(format!(
            "Opening existing org file `{:?}'...",
            file.display().to_string()
        ));
        super::effect(ed, move |s| {
            if let Err(e) = s.org_visit(&file, 0) {
                s.ed.set_err(e);
            }
        });
        return k(ed, true);
    }
    let name = name.to_owned();
    match create {
        Create::Yes => {
            open_file(ed, file, name);
            k(ed, true)
        }
        Create::Ask => ask(
            ed,
            &format!("File \u{2018}{name}.org\u{2019} not found; create?"),
            move |ed, y| {
                if y {
                    open_file(ed, file, name);
                }
                k(ed, y)
            },
        ),
        Create::No => k(ed, false),
    }
}

/// org-ctags-rebuild-tags-file-then-find-tag.
fn rebuild_then_find(ed: &mut Editor, name: &str) -> bool {
    if ed.path.is_some() {
        let _ = create_tags(ed, None);
    }
    find_tag(ed, name)
}

/// run-hook-with-args-until-success over `fns`, then `fallback` if none succeeded.
fn run_chain(
    ed: &mut Editor,
    mut fns: Vec<String>,
    name: String,
    fallback: Box<dyn FnOnce(&mut Editor) + Send>,
) {
    if fns.is_empty() {
        return fallback(ed);
    }
    let f = fns.remove(0);
    let n = name.clone();
    run_hook_fn(
        ed,
        &f,
        n,
        Box::new(move |ed, ok| {
            if !ok {
                run_chain(ed, fns, name, fallback);
            }
        }),
    );
}

/// org-ctags-find-tag-interactive.
fn find_tag_interactive(ed: &mut Editor) {
    let tags = tag_list(ed);
    super::complete(ed, "Topic: ", tags.clone(), false, move |ed, tag| {
        if tag.is_empty() {
            return;
        }
        if tags.contains(&tag) {
            HISTORY.with(|h| h.borrow_mut().insert(0, tag.clone()));
            if !find_tag(ed, &tag) {
                ed.set_err(format!("No definitions found for: {tag}"));
            }
        } else {
            run_chain(ed, hook_functions(), tag, Box::new(|_| {}));
        }
    });
}

/// Prompt for a tags table when none is active (visit-tags-table).
fn with_tags_table(ed: &mut Editor, then: impl FnOnce(&mut Editor) + Send + 'static) {
    if tags_file(ed).is_some() {
        return then(ed);
    }
    let dflt = buffer_dir(ed).join("TAGS");
    super::read(ed, "Visit tags table (default TAGS): ", "", move |ed, f| {
        let p = if f.trim().is_empty() {
            dflt
        } else {
            super::options::expand(f.trim())
        };
        visit_tags_table(Some(p));
        then(ed)
    });
}

/// The org-open-link-functions hook for link `kind\npath`: Ok when a
/// function (or the default search after them) took the link.
fn open_link_hook(ed: &mut Editor, arg: Prefix, spec: &str) -> Result<(), String> {
    let fns = hook_functions();
    if fns.is_empty() || INHIBIT.with(Cell::get) {
        return Err("no org-open-link-functions".into());
    }
    let (kind, path) = spec.split_once('\n').unwrap_or(("fuzzy", spec));
    let lk = Link {
        kind: kind.to_owned(),
        path: path.to_owned(),
        search: None,
        application: None,
        desc: None,
        raw: path.to_owned(),
        range: 0..path.len(),
    };
    let here = (ed.cur.line, ed.cur.byte);
    run_chain(
        ed,
        fns,
        path.to_owned(),
        Box::new(move |ed| {
            // Org's own search, as if no function were on the hook.
            ed.set_cursor(here.0, here.1);
            INHIBIT.with(|i| i.set(true));
            let r = links::open(ed, &lk, arg);
            INHIBIT.with(|i| i.set(false));
            if let Err(e) = r {
                ed.set_err(e);
            }
        }),
    );
    Ok(())
}

type Then = Box<dyn FnOnce(&mut Editor, String) + Send>;

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let given = super::take_call_arg();
    // Commands reading one string argument (or taking it from call_with).
    let with_arg = |ed: &mut Editor, prompt: &str, f: Then| match given.clone() {
        Some(s) => f(ed, s),
        None => super::read(ed, prompt, "", f),
    };
    Some(match name {
        "org-open-link-functions" => open_link_hook(ed, arg, given.as_deref().unwrap_or("")),
        "org-ctags-enable" => {
            ENABLED.with(|e| e.set(Some(true)));
            ed.set_msg("org-ctags enabled");
            Ok(())
        }
        "org-ctags-unload-function" => {
            ENABLED.with(|e| e.set(Some(false)));
            Ok(())
        }
        "org-ctags-find-tag" => {
            with_arg(
                ed,
                "Tag: ",
                Box::new(|ed, t| {
                    find_tag(ed, &t);
                }),
            );
            Ok(())
        }
        "org-ctags-get-filename-for-tag" => {
            with_arg(
                ed,
                "Tag: ",
                Box::new(|ed, tag| {
                    with_tags_table(ed, move |ed| {
                        match read_tags(ed).and_then(|(_, t)| filename_for_tag(&t, &tag)) {
                            Some((f, l, p)) => ed.set_msg(format!("({f:?} {l} {p})")),
                            None => ed.set_msg("nil"),
                        }
                    })
                }),
            );
            Ok(())
        }
        "org-ctags-all-tags-in-current-tags-table" => {
            with_tags_table(ed, |ed| {
                let v = tag_list(ed);
                ed.set_msg(format!(
                    "({})",
                    v.iter()
                        .map(|t| format!("{t:?}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            });
            Ok(())
        }
        "org-ctags-open-file" => {
            with_arg(
                ed,
                "File name: ",
                Box::new(|ed, f| {
                    let p = super::options::expand(&f);
                    let p = if p.is_relative() {
                        buffer_dir(ed).join(p)
                    } else {
                        p
                    };
                    open_file(ed, p, f);
                }),
            );
            Ok(())
        }
        "org-ctags-visit-buffer-or-file" | "org-ctags-ask-visit-buffer-or-file" => {
            let create = if name == "org-ctags-ask-visit-buffer-or-file" || arg == Prefix::U(1) {
                Create::Ask
            } else {
                Create::No
            };
            with_arg(
                ed,
                "Visit: ",
                Box::new(move |ed, n| visit_buffer_or_file(ed, &n, create, Box::new(|_, _| {}))),
            );
            Ok(())
        }
        "org-ctags-append-topic" => {
            let narrow = !arg.is_none();
            with_arg(
                ed,
                "Topic: ",
                Box::new(move |ed, n| {
                    append_topic(ed, &n, narrow);
                }),
            );
            Ok(())
        }
        "org-ctags-ask-append-topic"
        | "org-ctags-rebuild-tags-file-then-find-tag"
        | "org-ctags-ask-rebuild-tags-file-then-find-tag"
        | "org-ctags-fail-silently" => {
            let f = name.to_owned();
            with_arg(
                ed,
                "Tag: ",
                Box::new(move |ed, n| run_hook_fn(ed, &f, n, Box::new(|_, _| {}))),
            );
            Ok(())
        }
        "org-ctags-create-tags" => match create_tags(ed, given.as_deref()) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        },
        "org-ctags-find-tag-interactive" => {
            find_tag_interactive(ed);
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAGS: &str = "\x0c\nsub/a.org,49\n<<Foo bar>> text\x7fFoo bar\x012,4\n** <<baz>>\x7fbaz\x013,21\n\x0c\nb.org,20\n<<q>>\x7fq\x011,0\n";

    #[test]
    fn reads_etags_tables() {
        assert_eq!(
            filename_for_tag(TAGS, "baz"),
            Some(("sub/a.org".into(), 3, 21))
        );
        assert_eq!(filename_for_tag(TAGS, "q"), Some(("b.org".into(), 1, 0)));
        assert_eq!(filename_for_tag(TAGS, "nope"), None);
        assert_eq!(all_tags(TAGS), vec!["q", "baz", "Foo bar"]);
    }

    #[test]
    fn quoting_and_capitalize() {
        assert_eq!(shell_quote("/a b/it's$HOME/"), "/a\\ b/it\\'s\\$HOME/");
        assert_eq!(capitalize("hello wORLD-x"), "Hello World-X");
        assert_eq!(
            topic_lines("my topic"),
            vec!["* <<My Topic>>", "", "", "", "", ""]
        );
    }

    #[test]
    fn append_topic_at_end() {
        let mut e = super::super::tests::org("* A\ntext", "");
        append_topic(&mut e, "new thing", false);
        assert_eq!(e.buf.text(), "* A\ntext\n\n* <<New Thing>>\n\n\n\n\n\n");
        assert_eq!(e.cur.line, 7);
    }

    /// test-org-ctags/create-tags-escape: the shell arguments ctags gets.
    #[test]
    fn create_tags_escapes_shell_arguments() {
        if !which("sh") {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap().display().to_string();
        let run = |sub: &str, arg: bool| -> Vec<String> {
            let dir = format!("{base}/{sub}");
            std::fs::create_dir(&dir).unwrap();
            let out = format!("{dir}/ctags.txt");
            std::fs::write(&out, "Should be overwritten by org-ctags mock script").unwrap();
            let mock = format!(
                "ctags_mock() {{ printf '%s\\n' ctags_mock \"$@\" >{} 2>&1 ; false ; }} ; ctags_mock",
                shell_quote(&out)
            );
            super::super::options::put("org-ctags-path-to-ctags", toml::Value::String(mock));
            let mut e = Editor::new(crate::buffer::Buffer::from_text(""));
            e.path = Some(PathBuf::from(&out));
            create_tags(&e, arg.then_some(dir.as_str())).unwrap();
            let text = std::fs::read_to_string(&out)
                .unwrap()
                .replace(&base, "TMPDIR");
            assert!(text.starts_with("ctags_mock\n"), "{text}");
            text.lines().skip(1).map(str::to_owned).collect()
        };
        let a = run("regexp", false);
        assert_eq!(a[2], format!("--regex-orgmode={TAG_REGEXP}"));
        assert_eq!(run("regular", true)[7], "TMPDIR/regular/ctags.txt");
        let a = run("space char", true);
        assert_eq!(
            (a[4].as_str(), a[7].as_str()),
            ("TMPDIR/space char/TAGS", "TMPDIR/space char/ctags.txt")
        );
        let a = run("apostrophe' sep '", false);
        assert_eq!(
            (a[4].as_str(), a[7].as_str()),
            (
                "TMPDIR/apostrophe' sep '/TAGS",
                "TMPDIR/apostrophe' sep '/ctags.txt"
            )
        );
        let a = run("def-dir.$HOME", false);
        assert_eq!(
            (a[4].as_str(), a[7].as_str()),
            (
                "TMPDIR/def-dir.$HOME/TAGS",
                "TMPDIR/def-dir.$HOME/ctags.txt"
            )
        );
        let a = run("arg.$HOME", true);
        assert_eq!(
            (a[4].as_str(), a[7].as_str()),
            ("TMPDIR/arg.$HOME/TAGS", "TMPDIR/arg.$HOME/ctags.txt")
        );
        super::super::options::put(
            "org-ctags-path-to-ctags",
            toml::Value::String("ctags".into()),
        );
    }

    /// With real ctags: build a table and find a target in another file.
    #[test]
    fn real_ctags_finds_targets() {
        let ctags = ctags_program();
        let supports = std::process::Command::new(&ctags)
            .arg("--version")
            .output()
            .is_ok_and(|o| {
                let v = String::from_utf8_lossy(&o.stdout);
                v.contains("Universal Ctags") || v.contains("Exuberant Ctags")
            });
        if !supports {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        std::fs::write(dir.join("a.org"), "* H\nsee [[Target here]]\n").unwrap();
        std::fs::write(dir.join("b.org"), "* B\n\n<<Target here>> body\n").unwrap();
        let mut e = Editor::new(crate::buffer::Buffer::from_text("* H\nsee [[Target here]]"));
        e.path = Some(dir.join("a.org"));
        assert!(create_tags(&e, None).unwrap());
        let text = std::fs::read_to_string(dir.join("TAGS")).unwrap();
        let (f, l, _) = filename_for_tag(&text, "Target here").unwrap();
        assert!(f.ends_with("b.org"), "{f}");
        assert_eq!(l, 3);
        assert!(find_tag(&mut e, "target here"));
        assert!(e.pending_effect.is_some());
        visit_tags_table(None);
    }
}
