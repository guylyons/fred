//! Picker behavior through the editor's keys.

use super::*;
use crate::buffer::Buffer;
use crate::key::parse_keys;
use std::fs;

/// A project in a temp dir (with `.git`, so it is the root) and an editor.
fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    for (p, t) in files {
        let p = dir.path().join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
    let mut ed = Editor::new(Buffer::from_text("one two\nthree"));
    ed.project = Arc::new(Project::new(Some(dir.path().to_path_buf()), None));
    ed.project.files().wait();
    (dir, ed)
}

fn keys(ed: &mut Editor, k: &str) {
    for k in parse_keys(k) {
        ed.handle_key(k);
    }
}

fn picker(ed: &Editor) -> &Picker {
    match &ed.mode {
        Mode::Pick(p) => p,
        m => panic!("not picking: {m:?}"),
    }
}

/// Set the picker's query outright (typing a long temp path key by key
/// would walk big system directories along the way).
fn query(ed: &mut Editor, q: &str) {
    if let Mode::Pick(p) = &mut ed.mode {
        set_query(p, q.to_string());
    }
    tick(ed);
}

/// Tick until `done` holds (grep runs in the background).
fn settle(ed: &mut Editor, done: impl Fn(&Picker) -> bool) {
    for _ in 0..500 {
        tick(ed);
        if done(picker(ed)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("picker never settled: {:?}", picker(ed));
}

#[test]
fn space_is_the_leader() {
    let (_d, mut ed) = setup(&[]);
    keys(&mut ed, "w");
    let before = ed.cur;
    keys(&mut ed, " x");
    assert_eq!((ed.mode.clone(), ed.cur), (Mode::Normal, before));
    keys(&mut ed, " p");
    assert_eq!(picker(&ed).kind, Kind::Files);
    keys(&mut ed, "abc<Esc>");
    assert_eq!((ed.mode.clone(), ed.cur), (Mode::Normal, before));
    assert_eq!(ed.buf.text(), "one two\nthree");
    keys(&mut ed, " g");
    assert_eq!(picker(&ed).kind, Kind::Grep);
    keys(&mut ed, "<C-c>");
    assert_eq!(ed.mode, Mode::Normal);
}

#[test]
fn find_a_file() {
    let (d, mut ed) = setup(&[
        ("src/search.rs", ""),
        ("src/session.rs", ""),
        ("README.md", ""),
    ]);
    keys(&mut ed, " p");
    // Empty query: everything.
    assert_eq!(picker(&ed).rows.len(), 3);
    keys(&mut ed, "sesrs");
    let p = picker(&ed);
    assert_eq!(p.rows[0].text, "src/session.rs");
    assert!(!p.rows[0].hl.is_empty());
    assert!(
        p.status.starts_with(&format!("{}/3", p.rows.len())),
        "{}",
        p.status
    );
    keys(&mut ed, "<Enter>");
    assert_eq!(
        ed.pending_effect,
        Some(ExEffect::Open {
            path: d.path().join("src/session.rs"),
            line: 0,
            col: 0,
            pattern: None,
        })
    );
}

#[test]
fn selection_moves_and_clamps() {
    let (_d, mut ed) = setup(&[("a", ""), ("b", ""), ("c", "")]);
    keys(&mut ed, " p");
    assert_eq!(picker(&ed).sel, 0);
    keys(&mut ed, "<Up><C-p><Up><Up>");
    assert_eq!(picker(&ed).sel, 2);
    keys(&mut ed, "<Down><C-n><C-n>");
    assert_eq!(picker(&ed).sel, 0);
    keys(&mut ed, "<Up>x");
    assert_eq!(picker(&ed).sel, 0, "a new query selects the best match");
}

#[test]
fn grep_a_pattern() {
    let (d, mut ed) = setup(&[
        ("a.rs", "use x;\n\n    fn main() {}\n"),
        ("b.rs", "nothing\n"),
    ]);
    keys(&mut ed, " gfn \\w+");
    settle(&mut ed, |p| p.status == "1 matches");
    let p = picker(&ed);
    assert_eq!(p.rows[0].text, "a.rs:3: fn main() {}");
    let (a, b) = p.rows[0].hl[0];
    assert_eq!(&p.rows[0].text[a..b], "fn main");
    keys(&mut ed, "<Enter>");
    assert_eq!(
        ed.pending_effect,
        Some(ExEffect::Open {
            path: d.path().join("a.rs"),
            line: 2,
            col: 4,
            pattern: Some("fn \\w+".into()),
        })
    );
}

#[test]
fn bad_pattern_keeps_results() {
    let (_d, mut ed) = setup(&[("a", "foo(\n")]);
    keys(&mut ed, " gfoo");
    settle(&mut ed, |p| p.status == "1 matches");
    keys(&mut ed, "(");
    settle(&mut ed, |p| p.err);
    let p = picker(&ed);
    assert!(p.status.starts_with("bad pattern"), "{}", p.status);
    assert_eq!(p.rows.len(), 1);
    keys(&mut ed, "<BS>\\(");
    settle(&mut ed, |p| !p.err && p.status == "1 matches");
}

#[test]
fn enter_with_nothing_does_nothing() {
    let (_d, mut ed) = setup(&[("a", "")]);
    keys(&mut ed, " pzzz<Enter>");
    assert_eq!(picker(&ed).rows.len(), 0);
    assert_eq!(ed.pending_effect, None);
}

#[test]
fn space_k_jumps_to_a_line_in_this_file() {
    let mut ed = Editor::new(Buffer::from_text(
        "alpha\n  fn draw()\nbeta\nlet draw = fn_x;",
    ));
    keys(&mut ed, "j k");
    let p = picker(&ed);
    assert_eq!((p.kind, p.rows.len()), (Kind::Lines, 4));
    assert_eq!(p.rows[p.sel].line, 1, "starts on the cursor's line");
    keys(&mut ed, "draw fn");
    let p = picker(&ed);
    assert_eq!(p.rows.iter().map(|r| r.line).collect::<Vec<_>>(), [3, 1]);
    assert_eq!(p.status, "2/4 lines");
    // Up goes to the earlier line (higher on screen).
    keys(&mut ed, "<Up><Enter>");
    assert_eq!((ed.mode.clone(), ed.cur.pos()), (Mode::Normal, (1, 2)));
    assert_eq!(ed.last_pat.as_deref(), Some("draw"));
    assert_eq!(ed.pending_effect, None);
    // Esc leaves the cursor where it was.
    keys(&mut ed, "gg kbeta<Esc>");
    assert_eq!((ed.mode.clone(), ed.cur.line), (Mode::Normal, 0));
}

#[test]
fn browse_like_find_file() {
    let (d, mut ed) = setup(&[
        ("src/main.rs", ""),
        ("src/lib.rs", ""),
        ("README.md", ""),
        (".hidden", ""),
    ]);
    browse(&mut ed, d.path());
    let start = browse::show(d.path());
    let p = picker(&ed);
    assert_eq!(
        (p.kind, p.query.text.clone()),
        (Kind::Browse, start.clone())
    );
    let names: Vec<&str> = p.rows.iter().map(|r| r.text.as_str()).collect();
    // Directories first; dotfiles (`.git/`, `.hidden`) hidden.
    assert_eq!(names, ["src/", "README.md"]);
    // Into a directory with Enter; Tab completes a file's name.
    keys(&mut ed, "sr<Enter>");
    assert_eq!(picker(&ed).query.text, format!("{start}src/"));
    keys(&mut ed, "ma");
    settle(&mut ed, |p| !p.searching);
    keys(&mut ed, "<Tab>");
    assert_eq!(picker(&ed).query.text, format!("{start}src/main.rs"));
    keys(&mut ed, "<Enter>");
    assert_eq!(
        ed.pending_effect.take(),
        Some(ExEffect::Open {
            path: browse::resolve(&start).join("src/main.rs"),
            line: 0,
            col: 0,
            pattern: None
        })
    );
    // Backspace after a `/` goes up a directory.
    query(&mut ed, &format!("{start}src/"));
    keys(&mut ed, "<BS>");
    assert_eq!(picker(&ed).query.text, start);
    // Dotfiles once the name starts with a dot.
    keys(&mut ed, ".h");
    assert_eq!(picker(&ed).rows[0].text, ".hidden");
    // No match: Enter opens a new file of that name.
    query(&mut ed, &start);
    keys(&mut ed, "brand-new.txt");
    settle(&mut ed, |p| !p.searching);
    keys(&mut ed, "<Enter>");
    assert!(picker(&ed).status.starts_with("new file"));
    assert!(matches!(
        ed.pending_effect.take(),
        Some(ExEffect::Open { path, .. }) if path == browse::resolve(&start).join("brand-new.txt")
    ));
    // `~/` starts over at home.
    query(&mut ed, &start);
    keys(&mut ed, "~/");
    assert_eq!(picker(&ed).query.text, "~/");
}

#[test]
fn browse_searches_below_and_honors_gitignore() {
    let (d, mut ed) = setup(&[
        (".gitignore", "target/\n*.log\n"),
        ("src/pick/browse.rs", ""),
        ("src/main.rs", ""),
        ("target/debug/browse_build.rs", ""),
        ("notes.log", ""),
        ("top.txt", ""),
    ]);
    browse(&mut ed, d.path());
    // The listing itself hides ignored entries.
    let names: Vec<String> = picker(&ed).rows.iter().map(|r| r.text.clone()).collect();
    assert_eq!(names, ["src/", "top.txt"]);
    keys(&mut ed, "brows");
    settle(&mut ed, |p| !p.searching);
    let p = picker(&ed);
    let names: Vec<&str> = p.rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(names, ["src/pick/browse.rs"]);
    let root = browse::resolve(&browse::show(d.path()));
    assert_eq!(p.rows[0].path, root.join("src/pick/browse.rs"));
}

#[test]
fn enter_waits_for_the_search_before_making_a_new_file() {
    let (_d, mut ed) = setup(&[("a", "")]);
    browse(&mut ed, _d.path());
    if let Mode::Pick(p) = &mut ed.mode {
        p.searching = true;
        p.rows.clear();
        set_query(p, format!("{}zzz", p.query.text));
    }
    pick_key(&mut ed, crate::key::Key::new(KeyCode::Enter));
    assert_eq!(ed.pending_effect, None);
}

#[test]
fn space_r_lists_recent_files() {
    let (d, mut ed) = setup(&[
        ("a.rs", ""),
        ("b.rs", ""),
        ("notes/todo.md", ""),
        ("gone.txt", ""),
    ]);
    let rf = d.path().join("state/recent");
    for f in ["gone.txt", "notes/todo.md", "b.rs", "a.rs"] {
        recent::record(&rf, &d.path().join(f), None);
    }
    fs::remove_file(d.path().join("gone.txt")).unwrap();
    ed.project = Arc::new(Project::new(Some(d.path().to_path_buf()), Some(rf)));
    ed.path = Some(d.path().join("a.rs"));
    keys(&mut ed, " r");
    let p = picker(&ed);
    assert_eq!(p.kind, Kind::Recent);
    // Newest first; not the file being edited, not a deleted one.
    let paths: Vec<PathBuf> = p.rows.iter().map(|r| r.path.clone()).collect();
    assert_eq!(
        paths,
        [d.path().join("b.rs"), d.path().join("notes/todo.md")]
    );
    assert_eq!(p.rows[0].text, browse::tilde(&d.path().join("b.rs")));
    keys(&mut ed, "todo<Enter>");
    assert!(matches!(
        ed.pending_effect.take(),
        Some(ExEffect::Open { path, .. }) if path == d.path().join("notes/todo.md")
    ));
}
