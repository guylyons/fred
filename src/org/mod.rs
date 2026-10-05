//! Org mode: a port of upstream Org (see docs/org-port-notes.md).
//!
//! Org buffers get [`Org`] state. Keys follow org-mode-map with Emacs
//! chords, the user's `Space o` leader, and `:org-NAME` runs any ported
//! command by its upstream name.

pub mod babel;
pub mod buf;
pub mod capture;
pub mod ctx;
pub mod dispatch;
pub mod editing;
pub mod element;
pub mod face;
pub mod fold;
pub mod links;
pub mod list;
pub mod table;
pub mod tags;
pub mod time;
pub mod todo;
pub mod keymap;
pub mod options;
pub mod props;
pub mod re;
pub mod refile;
pub mod sexp;
pub mod src;
pub mod structure;
pub mod syntax;

use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use syntax::Settings;

impl Org {
    /// Forget the cached settings (org-mode-restart).
    pub fn reset_settings(&mut self) {
        *self.settings.borrow_mut() = None;
    }
}

/// Column view in this buffer: C-c C-c quits it (org-colview).
pub fn colview_active(_ed: &mut Editor) -> Option<Result<(), String>> {
    None
}

/// A generated Org buffer (agenda and other views); see agenda.rs.
pub struct View {
    pub name: String,
}

/// Per-buffer Org state.
#[derive(Default)]
pub struct Org {
    settings: std::cell::RefCell<Option<(u64, Rc<Settings>)>>,
    /// Line kinds for faces, per buffer version.
    pub faces: face::Cache,
    /// The command run by the previous key (Emacs `last-command`).
    pub last_command: Option<String>,
    /// org-cycle-subtree-status and org-cycle-global-status.
    pub subtree_status: Option<&'static str>,
    pub global_status: Option<&'static str>,
    /// Folds per spec (outline, blocks, drawers).
    pub specs: fold::Specs,
    /// org-adapt-indentation bound to nil (org-cycle-level).
    pub no_adapt: bool,
    /// org-last-set-property and org-last-set-property-value.
    pub last_property: Option<String>,
    pub last_property_value: Option<String>,
    /// Lines a sparse tree matched (for highlighting and next-error).
    pub sparse_hits: Vec<usize>,
    /// org-link--search-failed.
    pub link_search_failed: bool,
    /// org-display-custom-times toggled in this buffer (None: option).
    pub custom_times: Option<bool>,
    /// The active region (Emacs transient mark) while a command runs from
    /// a Visual-line selection or a `'<,'>` range: lines `lo..=hi`.
    pub region: Option<(usize, usize)>,
    /// Plain-list state (org-list.el).
    pub list: list::State,
}

/// org-region-active-p: the region's lines.
pub fn region(ed: &Editor) -> Option<(usize, usize)> {
    ed.org_region.or_else(|| ed.org.as_ref().and_then(|o| o.region))
}

fn set_region(ed: &mut Editor, r: Option<(usize, usize)>) {
    ed.org_region = r;
    if let Some(o) = &mut ed.org {
        o.region = r;
    }
}

/// Emacs prefix argument.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prefix {
    #[default]
    None,
    /// `C-u` pressed n times: (4), (16), (64).
    U(u32),
    Num(i64),
    Minus,
}

impl Prefix {
    pub fn is_none(self) -> bool {
        self == Prefix::None
    }

    /// prefix-numeric-value.
    pub fn value(self) -> i64 {
        match self {
            Prefix::None => 1,
            Prefix::U(n) => 4i64.pow(n),
            Prefix::Num(n) => n,
            Prefix::Minus => -1,
        }
    }

    /// `(equal arg '(4))`, `'(16)`...: the number of C-u, 0 otherwise.
    pub fn universal(self) -> u32 {
        match self {
            Prefix::U(n) => n,
            _ => 0,
        }
    }
}

/// A continuation for a prompt or menu answer.
pub type Then = Box<dyn FnOnce(&mut Editor, String) + Send>;

/// Work that needs the whole session (other buffers, files, views).
#[derive(Clone)]
pub struct Effect(Arc<Mutex<Option<SessionFn>>>);

pub type SessionFn = Box<dyn FnOnce(&mut crate::session::Session) + Send>;

impl Effect {
    pub fn new(f: impl FnOnce(&mut crate::session::Session) + Send + 'static) -> Effect {
        Effect(Arc::new(Mutex::new(Some(Box::new(f)))))
    }

    pub fn take(&self) -> Option<SessionFn> {
        self.0.lock().ok()?.take()
    }
}

impl PartialEq for Effect {
    fn eq(&self, o: &Effect) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}
impl Eq for Effect {}
impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "org::Effect")
    }
}

/// A special buffer's finish action (see session/org.rs).
pub struct FinishSlot(Arc<Mutex<Option<crate::session::Finish>>>);

impl FinishSlot {
    pub fn new(f: crate::session::Finish) -> FinishSlot {
        FinishSlot(Arc::new(Mutex::new(Some(f))))
    }

    pub fn take(&self) -> Option<crate::session::Finish> {
        self.0.lock().ok()?.take()
    }
}

thread_local! {
    /// A fixed "now" for tests (seconds since the epoch).
    static NOW: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

/// The current time (org-current-time), in seconds since the epoch.
pub fn now() -> i64 {
    NOW.with(|n| n.get()).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    })
}

/// Fix the time (tests).
pub fn set_now(t: Option<i64>) {
    NOW.with(|n| n.set(t));
}

/// Local broken-down time: (year, month, day, hour, minute, weekday 0=Sun).
pub fn localtime(t: i64) -> (i64, i64, i64, i64, i64, i64) {
    // SAFETY: localtime_r writes into the struct we own.
    unsafe {
        let tt: libc::time_t = t as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        (
            tm.tm_year as i64 + 1900,
            tm.tm_mon as i64 + 1,
            tm.tm_mday as i64,
            tm.tm_hour as i64,
            tm.tm_min as i64,
            tm.tm_wday as i64,
        )
    }
}

/// A timestamp for time `t`: `[2026-10-04 Sun 12:30]` (org-time-stamp-format).
pub fn timestamp(t: i64, with_time: bool, inactive: bool) -> String {
    let (y, m, d, hh, mm, wd) = localtime(t);
    let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wd as usize];
    let body = if with_time { format!("{y:04}-{m:02}-{d:02} {day} {hh:02}:{mm:02}") } else { format!("{y:04}-{m:02}-{d:02} {day}") };
    if inactive { format!("[{body}]") } else { format!("<{body}>") }
}

/// Ask the session to run `f`.
pub fn effect(ed: &mut Editor, f: impl FnOnce(&mut crate::session::Session) + Send + 'static) {
    ed.pending_effect = Some(crate::ex::ExEffect::Org(Effect::new(f)));
}

/// A buffer at `path` is Org: `.org` (and `.org_archive`), or a
/// `-*- mode: org -*-` first line.
pub fn is_org(path: Option<&std::path::Path>, first_line: &str) -> bool {
    path.and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("org") || e == "org_archive")
        || (first_line.contains("-*-") && first_line.to_ascii_lowercase().contains("mode: org"))
}

/// Turn on Org mode in a buffer that visits an Org file (org-mode).
pub fn attach(ed: &mut Editor) {
    if ed.org.is_some() || ed.magit.is_some() || ed.dired.is_some() {
        return;
    }
    let first = if ed.buf.len_lines() > 0 { ed.buf.line(0) } else { String::new() };
    if !is_org(ed.path.as_deref(), &first) {
        return;
    }
    ed.org = Some(Box::default());
    fold::startup(ed);
}

/// This buffer's settings, recomputed when the text changed.
pub fn settings(ed: &Editor) -> Rc<Settings> {
    let version = ed.buf.version;
    if let Some(o) = &ed.org
        && let Some((v, st)) = &*o.settings.borrow()
        && *v == version
    {
        return Rc::clone(st);
    }
    let dir = ed
        .path
        .as_deref()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let text = ed.buf.text();
    let st = Rc::new(syntax::settings(text.lines(), dir.as_deref()));
    if let Some(o) = &ed.org {
        *o.settings.borrow_mut() = Some((version, Rc::clone(&st)));
    }
    st
}

/// Settings for buffers without Org state (agenda files read from disk).
pub fn settings_of(text: &str, dir: Option<&std::path::Path>) -> Settings {
    syntax::settings(text.lines(), dir)
}

// ---- editing helpers: every change goes through Fred's undo ----

pub fn line(ed: &Editor, l: usize) -> String {
    ed.buf.line(l)
}

pub fn lines(ed: &Editor, range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|l| ed.buf.line(l)).collect()
}

/// Replace `remove` lines at `at` with `with`.
pub fn splice(ed: &mut Editor, at: usize, remove: usize, with: &[String]) {
    crate::vim::ops::splice_lines(ed, at, remove, with);
    ed.sync_marks();
}

pub fn set_line(ed: &mut Editor, l: usize, text: &str) {
    if ed.buf.line(l) != text {
        let with = [text.to_owned()];
        splice(ed, l, 1, &with);
    }
}

pub fn insert_lines(ed: &mut Editor, at: usize, with: &[String]) {
    if at >= ed.line_count() && ed.line_count() == 1 && ed.buf.len_bytes() == 0 {
        splice(ed, 0, 1, with);
    } else {
        splice(ed, at, 0, with);
    }
}

pub fn delete_lines(ed: &mut Editor, at: usize, n: usize) {
    if n > 0 {
        splice(ed, at, n, &[]);
    }
}

// ---- prompts and menus ----

/// Read a string in the minibuffer, then `then(answer)`. Esc cancels.
pub fn read(ed: &mut Editor, prompt: &str, initial: &str, then: impl FnOnce(&mut Editor, String) + Send + 'static) {
    ed.org_then = Some(Box::new(then));
    ed.open_cmdline('o', initial);
    if let Mode::Command(cl) = &mut ed.mode {
        cl.prompt = prompt.into();
    }
}

/// completing-read: pick one of `candidates` (or type a new one unless
/// `require_match`).
pub fn complete(
    ed: &mut Editor,
    prompt: &str,
    candidates: Vec<String>,
    require_match: bool,
    then: impl FnOnce(&mut Editor, String) + Send + 'static,
) {
    ed.org_then = Some(Box::new(then));
    ed.org_require_match = require_match;
    crate::pick::org_choice(ed, prompt, candidates);
}

/// A key menu (org-mks, dispatchers): `entries` are (keys, label); a
/// label starting with `#` is a heading. `then(keys)` runs on a choice.
pub fn menu(
    ed: &mut Editor,
    title: &str,
    entries: Vec<(String, String)>,
    then: impl FnOnce(&mut Editor, String) + Send + 'static,
) {
    ed.org_then = Some(Box::new(then));
    ed.org_menu_typed.clear();
    ed.org_menu = entries;
    crate::pick::org_menu(ed, title);
}

/// A prompt's answer arrived (Enter on the `o` command line).
pub fn answer(ed: &mut Editor, text: &str) {
    if let Some(then) = ed.org_then.take() {
        then(ed, text.to_owned());
    }
}

/// y-or-n-p as a one-key menu.
pub fn yes_or_no(ed: &mut Editor, question: &str, then: impl FnOnce(&mut Editor) + Send + 'static) {
    menu(
        ed,
        question,
        vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
        move |ed, k| {
            if k == "y" {
                then(ed)
            }
        },
    );
}

/// Keys in an Org menu or completion picker.
fn picker_key(ed: &mut Editor, k: Key) -> bool {
    let Mode::Pick(p) = &mut ed.mode else {
        return false;
    };
    match p.kind {
        crate::pick::Kind::OrgMenu => {
            let keys: Vec<String> = ed
                .org_menu
                .iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, _)| k.clone())
                .collect();
            let abort = k.is(KeyCode::Esc)
                || k == Key::ctrl('g')
                || k == Key::ctrl('c')
                || (k == Key::ch('q') && ed.org_menu_typed.is_empty() && !keys.iter().any(|x| x == "q"));
            if abort {
                ed.mode = Mode::Normal;
                ed.org_then = None;
                ed.org_menu_enter = None;
                ed.org_menu_typed.clear();
                ed.set_msg("Abort");
                return true;
            }
            let choice = match k.code {
                KeyCode::Enter if ed.org_menu_enter.is_some() => ed.org_menu_enter.clone(),
                KeyCode::Enter if !k.alt && !k.ctrl => p
                    .rows
                    .get(p.sel)
                    .and_then(|r| ed.org_menu.get(r.line))
                    .map(|(key, _)| key.clone())
                    .filter(|key| !key.is_empty()),
                KeyCode::Up | KeyCode::Down => return false,
                KeyCode::Char('n' | 'p') if k.ctrl => return false,
                _ => {
                    let c = match k.code {
                        KeyCode::Char(c) if k.ctrl => format!("C-{c}"),
                        KeyCode::Char(c) => c.to_string(),
                        KeyCode::Tab => "TAB".into(),
                        _ => return true,
                    };
                    let typed = format!("{}{c}", ed.org_menu_typed);
                    if keys.iter().any(|x| *x == typed) {
                        Some(typed)
                    } else if keys.iter().any(|x| x.starts_with(&typed)) {
                        ed.org_menu_typed = typed;
                        return true;
                    } else {
                        ed.org_menu_typed.clear();
                        ed.set_err(format!("Invalid key: {typed}"));
                        return true;
                    }
                }
            };
            if let Some(choice) = choice {
                ed.mode = Mode::Normal;
                ed.org_menu_typed.clear();
                if let Some(then) = ed.org_then.take() {
                    then(ed, choice);
                }
            }
            true
        }
        crate::pick::Kind::OrgChoice => {
            if k.is(KeyCode::Esc) || k == Key::ctrl('g') || k == Key::ctrl('c') {
                ed.mode = Mode::Normal;
                ed.org_then = None;
                return true;
            }
            if k.is(KeyCode::Enter) || k == Key::with(KeyCode::Enter, "M") {
                let typed = p.query.text.clone();
                let row = p.rows.get(p.sel).map(|r| r.text.clone());
                // M-RET (or no match) takes the typed text.
                let ans = match row {
                    Some(r) if k.is(KeyCode::Enter) => r,
                    _ if ed.org_require_match && !typed.is_empty() => {
                        ed.set_err("[No match]");
                        return true;
                    }
                    _ => typed,
                };
                ed.mode = Mode::Normal;
                if let Some(then) = ed.org_then.take() {
                    then(ed, ans);
                }
                return true;
            }
            false
        }
        _ => false,
    }
}

// ---- keys ----

/// Parse an Emacs key description (`C-c C-x C-i`, `M-S-<left>`, `TAB`).
pub fn parse_keys(desc: &str) -> Option<Vec<Key>> {
    desc.split(' ').map(parse_key).collect()
}

fn parse_key(tok: &str) -> Option<Key> {
    let mut mods = String::new();
    let mut rest = tok;
    while rest.len() > 2 && rest.as_bytes()[1] == b'-' && matches!(rest.as_bytes()[0], b'C' | b'M' | b'S' | b's') {
        if rest.as_bytes()[0] == b's' {
            return None; // super
        }
        mods.push(rest.as_bytes()[0] as char);
        rest = &rest[2..];
    }
    let name = rest.trim_start_matches('<').trim_end_matches('>');
    let code = match name {
        "TAB" | "tab" => KeyCode::Tab,
        "RET" | "return" => KeyCode::Enter,
        "SPC" => KeyCode::Char(' '),
        "DEL" | "backspace" => KeyCode::Backspace,
        "delete" | "deletechar" => KeyCode::Delete,
        "ESC" | "escape" => KeyCode::Esc,
        "backtab" => KeyCode::BackTab,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "prior" => KeyCode::PageUp,
        "next" => KeyCode::PageDown,
        _ => {
            let mut it = rest.chars();
            let c = it.next()?;
            if it.next().is_some() {
                return None;
            }
            KeyCode::Char(c)
        }
    };
    if code == KeyCode::Tab && mods == "S" {
        return Some(Key::new(KeyCode::BackTab));
    }
    let mut k = Key::with(code, &mods);
    if let KeyCode::Char(c) = code {
        k.shift = false;
        if k.ctrl {
            k.code = KeyCode::Char(c.to_ascii_lowercase());
        }
    }
    Some(k)
}

/// The keymap of this buffer.
type Map = &'static [(&'static str, &'static str)];

/// The keymaps of this buffer, minor modes first.
fn active_map(ed: &Editor) -> Option<Vec<Map>> {
    if ed.org_view.is_some() {
        return Some(vec![keymap::AGENDA_MAP]);
    }
    let mut maps: Vec<Map> = vec![];
    match ed.org_buffer_name.as_deref() {
        Some(n) if n.starts_with("CAPTURE-") => maps.push(keymap::CAPTURE_MAP),
        Some(n) if n.starts_with("*Org Src") => maps.push(keymap::SRC_MAP),
        _ => {}
    }
    if ed.org.is_some() {
        maps.push(keymap::ORG_MODE_MAP);
    }
    (!maps.is_empty()).then_some(maps)
}

/// Keys Fred's Vim layer keeps in Normal mode (evil-normal-state-map wins).
fn vim_keeps_normal(k: Key) -> bool {
    match k.code {
        KeyCode::Char(c) if k.ctrl && !k.alt => "abdefjknopqrstuvwxyz6^_/l".contains(c),
        KeyCode::Char(_) => !k.ctrl && !k.alt,
        KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete => !k.ctrl && !k.alt && !k.shift,
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Esc => {
            !k.ctrl && !k.alt && !k.shift
        }
        _ => false,
    }
}

/// Keys Fred's Insert mode keeps (evil-insert-state-map).
fn vim_keeps_insert(k: Key) -> bool {
    match k.code {
        KeyCode::Char(c) if k.ctrl && !k.alt => "wuhnprovdtkeyaxz".contains(c),
        KeyCode::Char(c) => !k.ctrl && !k.alt && c != '|',
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Esc => !k.ctrl && !k.alt,
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown => {
            !k.ctrl && !k.alt && !k.shift
        }
        _ => false,
    }
}

enum Lookup {
    Exact(&'static str),
    Prefix,
    None,
}

fn lookup(maps: &[Map], seq: &[Key]) -> Lookup {
    let mut prefix = false;
    for (desc, cmd) in maps.iter().flat_map(|m| m.iter()) {
        let Some(keys) = parse_keys(desc) else { continue };
        if keys == seq {
            return Lookup::Exact(cmd);
        }
        if keys.len() > seq.len() && keys[..seq.len()] == *seq {
            prefix = true;
        }
    }
    if prefix { Lookup::Prefix } else { Lookup::None }
}

/// Describe keys the Emacs way, for messages.
pub fn describe(keys: &[Key]) -> String {
    keys.iter()
        .map(|k| {
            let mut s = String::new();
            if k.ctrl {
                s.push_str("C-");
            }
            if k.alt {
                s.push_str("M-");
            }
            if k.shift {
                s.push_str("S-");
            }
            match k.code {
                KeyCode::Char(' ') => s.push_str("SPC"),
                KeyCode::Char(c) => s.push(c),
                KeyCode::Tab => s.push_str("TAB"),
                KeyCode::BackTab => s.push_str("<backtab>"),
                KeyCode::Enter => s.push_str("RET"),
                KeyCode::Esc => s.push_str("ESC"),
                KeyCode::Backspace => s.push_str("DEL"),
                c => s.push_str(&format!("<{c:?}>").to_lowercase()),
            }
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Org's Alt keys reach Org buffers whole (see app.rs).
pub fn wants_alt(ed: &Editor) -> bool {
    ed.org.is_some() || ed.org_view.is_some()
}

/// Org keys before Vim: menus, the `Space o` leader and org-mode-map.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if time::prompt_key(ed, k) || picker_key(ed, k) {
        return true;
    }
    if ed.zap.is_some() || ed.explain.is_some() {
        return false;
    }
    let normal = ed.mode == Mode::Normal;
    // The user's `Space o` leader (gl-keys.el) and `Space u` (universal-argument).
    if normal && ed.vim.pending == [Key::ch(' ')] {
        match k.char() {
            Some('o') => {
                ed.vim.pending.push(k);
                return true;
            }
            Some('u') => {
                ed.vim.pending.clear();
                ed.org_arg = match ed.org_arg {
                    Prefix::U(n) => Prefix::U(n + 1),
                    _ => Prefix::U(1),
                };
                ed.set_msg(vec!["C-u"; ed.org_arg.universal() as usize].join(" ") + "-");
                return true;
            }
            _ => {}
        }
    }
    if normal && ed.vim.pending == [Key::ch(' '), Key::ch('o')] {
        ed.vim.pending.clear();
        let cmd = match k.char() {
            Some('a') => "org-agenda",
            Some('c') => "org-capture",
            Some('f') => "gl/org-folder",
            Some('h') => "org-insert-heading",
            Some('i') => "org-insert-link",
            Some('s') => "org-store-link",
            Some('t') => "org-insert-todo-heading",
            Some('I') => "org-clock-in",
            Some('O') => "org-clock-out",
            _ => {
                ed.set_err(format!("SPC o {} is undefined", describe(&[k])));
                return true;
            }
        };
        let arg = std::mem::take(&mut ed.org_arg);
        run(ed, cmd, arg);
        return true;
    }
    let Some(map) = active_map(ed) else {
        return false;
    };
    let insert = ed.mode == Mode::Insert;
    let visual = match ed.mode {
        Mode::VisualLine { anchor } => Some((anchor.min(ed.cur.line), anchor.max(ed.cur.line))),
        _ => None,
    };
    if !normal && !insert && visual.is_none() {
        return false;
    }
    if ed.org_keys.is_empty() {
        let kept = if insert { vim_keeps_insert(k) } else { vim_keeps_normal(k) };
        // A Vim count or operator in progress keeps its keys too.
        let digits = ed.vim.pending.iter().all(|p| p.char().is_some_and(|c| c.is_ascii_digit()));
        if kept || !digits {
            if ed.org_view.is_none() {
                if let Some(o) = &mut ed.org {
                    o.last_command = None;
                }
            }
            return false;
        }
    }
    let mut seq = ed.org_keys.clone();
    seq.push(k);
    match lookup(&map, &seq) {
        Lookup::Exact(cmd) => {
            ed.org_keys.clear();
            // A Vim count typed first is the numeric prefix.
            let digits: String = ed.vim.pending.iter().filter_map(Key::char).collect();
            ed.vim.pending.clear();
            let mut arg = std::mem::take(&mut ed.org_arg);
            if arg.is_none() && !digits.is_empty() {
                arg = Prefix::Num(digits.parse().unwrap_or(1));
            }
            if visual.is_some() {
                ed.mode = Mode::Normal;
                set_region(ed, visual);
            }
            run(ed, cmd, arg);
            set_region(ed, None);
            true
        }
        Lookup::Prefix => {
            ed.org_keys = seq;
            ed.set_msg(format!("{}-", describe(&ed.org_keys)));
            true
        }
        Lookup::None => {
            ed.org_keys.clear();
            if seq.len() == 1 {
                return false;
            }
            // Insert mode: C-c then a plain key leaves Insert, as Fred's C-c does.
            if insert && seq.len() == 2 && seq[0] == Key::ctrl('c') {
                ed.handle_key(Key::new(KeyCode::Esc));
                ed.handle_key(k);
                return true;
            }
            ed.set_err(format!("{} is undefined", describe(&seq)));
            true
        }
    }
}

/// Run an Org command by its upstream name (M-x).
pub fn run(ed: &mut Editor, name: &str, arg: Prefix) {
    let before = ed.cur.pos();
    ed.undo.begin(before);
    let r = dispatch(ed, name, arg);
    ed.undo.end(ed.cur.pos());
    ed.clamp_cursor();
    if let Err(e) = r {
        ed.set_err(e);
    }
    if let Some(o) = &mut ed.org {
        o.last_command = Some(name.to_owned());
    }
}

/// A module's interactive commands: Some(result) when it owns `name`.
type Module = fn(&mut Editor, &str, Prefix) -> Option<Result<(), String>>;

/// Every module's command table, tried in order.
const MODULES: &[Module] = &[
    fold::command,
    dispatch::command,
    structure::command,
    todo::command,
    tags::command,
    props::command,
    links::command,
    capture::command,
    refile::command,
    src::command,
    editing::command,
    table::command,
    list::command,
    time::command,
    element::command,
    babel::command,
];

thread_local! {
    /// A string argument for the next command run with [`call_with`].
    static CALL_ARG: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run a command by name with a string argument (an interactive spec's answer).
pub fn call_with(ed: &mut Editor, name: &str, arg: Prefix, s: String) -> Result<(), String> {
    CALL_ARG.with(|c| *c.borrow_mut() = Some(s));
    let r = dispatch(ed, name, arg);
    CALL_ARG.with(|c| c.borrow_mut().take());
    r
}

/// The string argument passed by [`call_with`], if any.
pub fn take_call_arg() -> Option<String> {
    CALL_ARG.with(|c| c.borrow_mut().take())
}

/// org-agenda-files, expanded (directories to their Org files).
pub fn agenda_files() -> Vec<std::path::PathBuf> {
    let re = options::string("org-agenda-file-regexp", r"\`[^.].*\.org\'");
    let re = re::compile(&re, false).ok();
    let mut out = vec![];
    let list: Vec<String> = match sexp::option("org-agenda-files") {
        Some(sexp::Sexp::Str(f)) => std::fs::read_to_string(options::expand(&f))
            .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect())
            .unwrap_or_default(),
        Some(v) => v.list().unwrap_or(&[]).iter().filter_map(|x| x.str().map(str::to_owned)).collect(),
        None => vec![],
    };
    for f in list {
        let p = options::expand(&f);
        let p = if p.is_relative() { options::directory().join(p) } else { p };
        if p.is_dir() {
            let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&p)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.path())
                        .filter(|f| f.is_file() && f.file_name().is_some_and(|n| re.as_ref().is_none_or(|r| r.is_match(&n.to_string_lossy()))))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            out.extend(files);
        } else {
            out.push(p);
        }
    }
    out
}

/// org-id-find: the file and heading line of entry ID, searching open
/// buffers, the agenda files and org-id-locations.
pub fn find_id_file(s: &mut crate::session::Session, id: &str) -> Option<(std::path::PathBuf, usize)> {
    let mut files: Vec<std::path::PathBuf> = (0..s.org_buffer_count()).filter_map(|i| s.org_buffer_path(i)).filter(|p| p.extension().is_some_and(|e| e == "org")).collect();
    files.extend(agenda_files());
    let loc = options::expand(&options::string("org-id-locations-file", "~/.emacs.d/.org-id-locations"));
    if let Ok(text) = std::fs::read_to_string(&loc)
        && let Ok(v) = sexp::read(&text)
    {
        for e in v.list().unwrap_or(&[]) {
            if let Some(items) = e.list()
                && items.iter().skip(1).any(|x| x.str() == Some(id))
                && let Some(f) = items.first().and_then(|x| x.str())
            {
                files.insert(0, options::expand(f));
            }
        }
    }
    let needle = id.to_owned();
    for f in files {
        let Ok(text) = s.org_text(&f) else { continue };
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        for (i, l) in lines.iter().enumerate() {
            if let Some((k, v)) = props::parse_property(l)
                && k.eq_ignore_ascii_case("ID")
                && v == needle
            {
                let h = syntax::heading_at_or_before(&lines, i).unwrap_or(0);
                return Some((f, h));
            }
        }
    }
    None
}

/// A timestamp for a typed date (`2026-10-04`, `2026-10-04 13:00`, `+2d`,
/// `today`), until the full org-read-date is wired in by time.rs.
pub fn read_date_timestamp(s: &str, with_time: bool, inactive: bool) -> Option<String> {
    let s = s.trim();
    let now = now();
    let (y, m, d, hh, mm, _) = localtime(now);
    let today = tags::days_from_civil(y, m, d);
    let (days, time) = if s.is_empty() || s == "." || s == "today" {
        (today, None)
    } else if let Some(n) = s.strip_prefix('+').and_then(|r| r.strip_suffix('d')).and_then(|n| n.parse::<i64>().ok()) {
        (today + n, None)
    } else {
        let (date, time) = match s.split_once(' ') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let mut it = date.split('-');
        let (yy, mo, dd) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        (tags::days_from_civil(yy, mo, dd), time.map(str::to_owned))
    };
    let (yy, mo, dd) = capture::civil_from_days(days);
    let wd = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][(days + 4).rem_euclid(7) as usize];
    let t = match (time, with_time) {
        (Some(t), _) => format!(" {t}"),
        (None, true) => format!(" {hh:02}:{mm:02}"),
        (None, false) => String::new(),
    };
    let body = format!("{yy:04}-{mo:02}-{dd:02} {wd}{t}");
    Some(if inactive { format!("[{body}]") } else { format!("<{body}>") })
}

/// The running clock's heading as a refile target (org-refile with 2).
pub fn clock_target(_s: &crate::session::Session) -> Option<refile::Target> {
    None
}

/// Typing in a table (org-self-insert-command's table part): true when
/// the tables module handled the character.
pub fn table_self_insert(_ed: &mut Editor, _c: char) -> bool {
    false
}

/// Clocked minutes in the subtree at `h` (org-clock-sum), from CLOCK lines.
pub fn clock_minutes(ed: &Editor, h: usize) -> i64 {
    let end = syntax::subtree_end(&ed.buf, h);
    let re = crate::org_re!(r"=>\s*(-?\d+):(\d\d)");
    (h..end)
        .filter_map(|l| {
            let t = ed.buf.line(l);
            if !t.trim_start().starts_with("CLOCK:") {
                return None;
            }
            re.captures(&t).map(|c| c[1].parse::<i64>().unwrap_or(0) * 60 + c[2].parse::<i64>().unwrap_or(0))
        })
        .sum()
}

/// Run a command by name from another command (context dispatchers).
pub fn call(ed: &mut Editor, name: &str, arg: Prefix) -> Result<(), String> {
    dispatch(ed, name, arg)
}

/// Commands not yet ported say so (and are tracked in docs/org-parity.csv).
fn dispatch(ed: &mut Editor, name: &str, arg: Prefix) -> Result<(), String> {
    for m in MODULES {
        if let Some(r) = m(ed, name, arg) {
            return r;
        }
    }
    match name {
        "gl/org-folder" => {
            ed.pending_effect = Some(crate::ex::ExEffect::Open {
                path: options::directory(),
                line: 0,
                col: 0,
                pattern: None,
            });
            Ok(())
        }
        "org-mode" => {
            if ed.org.is_none() && ed.magit.is_none() {
                ed.org = Some(Box::default());
                fold::startup(ed);
            }
            Ok(())
        }
        _ => Err(format!("{name}: not yet ported to fred")),
    }
}

/// Every command name `:org-…` completes and runs (ported or not).
pub fn command_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = keymap::ORG_MODE_MAP
        .iter()
        .chain(keymap::AGENDA_MAP)
        .map(|(_, c)| *c)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// `:org-NAME [args]` or `:org NAME`: true if this was an Org command.
pub fn ex(ed: &mut Editor, text: &str) -> bool {
    let mut t = text.trim();
    // A Visual-line range is the region.
    let mut range = None;
    if let Some(rest) = t.strip_prefix("'<,'>") {
        t = rest.trim_start();
        range = ed
            .marks
            .get(&'<')
            .zip(ed.marks.get(&'>'))
            .map(|(a, b)| (*a, *b));
    }
    let (name, _rest) = match t.split_once(char::is_whitespace) {
        Some((a, b)) => (a, b.trim()),
        None => (t, ""),
    };
    let name = if name == "org" {
        match _rest.split_whitespace().next() {
            Some(n) => n,
            None => return false,
        }
    } else {
        name
    };
    if !(name.starts_with("org-") || name.starts_with("orgtbl-") || name.starts_with("gl/org") || name == "org-mode") {
        return false;
    }
    let arg = std::mem::take(&mut ed.org_arg);
    set_region(ed, range);
    run(ed, name, arg);
    set_region(ed, None);
    true
}

#[cfg(test)]
mod tests;
