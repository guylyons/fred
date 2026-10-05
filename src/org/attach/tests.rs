//! Ported from testing/lisp/test-org-attach.el (temp directories).

use super::*;
use crate::buffer::Buffer;

/// testing/examples/attachments.org with its attachment directories.
const EXAMPLE: &str = "#+TITLE: Org attach testfile
Used to test and verify the functionality of org-attach.

* H1
 :PROPERTIES:
 :DIR:      att1
 :END:
A link to one attachment: [[attachment:fileA]]

** H1.1
A link to another attachment: [[attachment:fileB]]

** H1.2
  :PROPERTIES:
  :DIR:      att2
  :END:

* H2
 :PROPERTIES:
 :ID:       abcd123
 :END:

* H3
 :PROPERTIES:
 :DIR:      att1
 :ID:       abcd1234
 :END:

** H3.1
  :PROPERTIES:
  :ID:       abcd12345
  :END:
";

fn setup(text: &str) -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    for (d, files) in [
        ("att1", &["fileA", "fileB"][..]),
        ("att2", &["fileC", "fileD"]),
        ("data/ab/cd123", &["fileE"]),
    ] {
        std::fs::create_dir_all(dir.path().join(d)).unwrap();
        for f in files {
            std::fs::write(dir.path().join(d).join(f), format!("Text in {f}\n")).unwrap();
        }
    }
    let path = dir.path().join("attachments.org");
    std::fs::write(&path, text).unwrap();
    let mut e = Editor::new(Buffer::from_text(text));
    e.path = Some(path);
    super::super::attach(&mut e);
    (dir, e)
}

fn at(e: &mut Editor, heading: &str) {
    let l = (0..e.line_count())
        .find(|&l| e.buf.line(l) == heading)
        .unwrap();
    e.set_cursor(l, 0);
}

fn inheritance(v: bool) {
    super::super::options::put("org-attach-use-inheritance", toml::Value::Boolean(v));
}

#[test]
fn folder_formats() {
    assert_eq!(uuid_folder_format("abcd123").as_deref(), Some("ab/cd123"));
    assert_eq!(uuid_folder_format("ab"), None);
    assert_eq!(
        ts_folder_format("20261004T1200").as_deref(),
        Some("202610/04T1200")
    );
    assert_eq!(ts_folder_format("abc"), None);
    assert_eq!(fallback_folder_format("x1").as_deref(), Some("__/x/x1"));
}

#[test]
fn dir_resolution() {
    let (tmp, mut e) = setup(EXAMPLE);
    let p = tmp.path();
    inheritance(false);
    at(&mut e, "** H1.1");
    assert_eq!(dir(&e, false).unwrap(), None);
    inheritance(true);
    assert_eq!(dir(&e, false).unwrap(), Some(p.join("att1")));
    at(&mut e, "** H1.2");
    assert_eq!(
        file_list(&dir(&e, false).unwrap().unwrap()),
        ["fileC", "fileD"]
    );
    at(&mut e, "* H2");
    assert_eq!(file_list(&dir(&e, false).unwrap().unwrap()), ["fileE"]);
    at(&mut e, "* H3");
    assert_eq!(
        file_list(&dir(&e, false).unwrap().unwrap()),
        ["fileA", "fileB"]
    );
    // A folder not yet created is found only without the fs check.
    inheritance(false);
    at(&mut e, "** H3.1");
    assert_eq!(dir(&e, false).unwrap(), None);
    assert_eq!(dir(&e, true).unwrap(), Some(p.join("data/ab/cd12345")));
    // DIR (inherited) wins over the entry's own ID.
    inheritance(true);
    assert_eq!(
        file_list(&dir(&e, false).unwrap().unwrap()),
        ["fileA", "fileB"]
    );
    // attachment: links expand into the entry's directory.
    at(&mut e, "* H1");
    assert_eq!(expand(&e, "fileA"), p.join("att1/fileA"));
    super::super::options::put(
        "org-attach-use-inheritance",
        toml::Value::String("selective".into()),
    );
}

#[test]
fn attach_tags_stores_link_and_sync_untags() {
    let (tmp, mut e) = setup("* foo\n");
    let src = tmp.path().join("a.txt");
    std::fs::write(&src, "hello").unwrap();
    super::super::call_with(
        &mut e,
        "org-attach-attach",
        Prefix::None,
        src.display().to_string(),
    )
    .unwrap();
    assert!(
        e.buf.line(0).starts_with("* foo") && e.buf.line(0).ends_with(":ATTACH:"),
        "{}",
        e.buf.text()
    );
    let id = props::get(&e, Some(0), "ID", Inherit::No).expect("an ID was created");
    let d = dir_from_id(&e, &id, false).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "hello");
    assert!(src.exists(), "cp keeps the original");
    assert_eq!(
        links::stored_links()
            .first()
            .map(|l| l.0.clone())
            .as_deref(),
        Some("attachment:a.txt")
    );
    assert_eq!(
        e.msg.as_ref().unwrap().0,
        "File \"a.txt\" is now an attachment"
    );
    super::super::call_with(
        &mut e,
        "org-attach-delete-one",
        Prefix::None,
        "a.txt".into(),
    )
    .unwrap();
    assert!(!d.join("a.txt").exists());
    super::super::options::put(
        "org-attach-sync-delete-empty-dir",
        toml::Value::Boolean(true),
    );
    super::super::call(&mut e, "org-attach-sync", Prefix::None).unwrap();
    assert!(!e.buf.line(0).contains("ATTACH"), "{}", e.buf.text());
    assert!(!d.exists(), "empty directory removed");
}

#[test]
fn git_commit_hook_records_attachments() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let (tmp, mut e) = setup("* foo\n");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let g = |args: &[&str]| run_git(&data, args).unwrap();
    g(&["init", "-q"]);
    g(&["config", "user.email", "t@example.com"]);
    g(&["config", "user.name", "T"]);
    super::super::options::put(
        "org-attach-after-change-hook",
        toml::Value::Array(vec![toml::Value::String("org-attach-git-commit".into())]),
    );
    let src = tmp.path().join("b.txt");
    std::fs::write(&src, "x").unwrap();
    super::super::call_with(
        &mut e,
        "org-attach-attach",
        Prefix::None,
        src.display().to_string(),
    )
    .unwrap();
    super::super::options::put("org-attach-after-change-hook", toml::Value::Boolean(false));
    assert!(g(&["log", "--oneline"]).contains("Synchronized attachments"));
}
