use super::repo::*;
use std::{fs, path::Path, process::Command};
fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fred Test")
        .env("GIT_AUTHOR_EMAIL", "fred@example.test")
        .env("GIT_COMMITTER_NAME", "Fred Test")
        .env("GIT_COMMITTER_EMAIL", "fred@example.test")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&o.stderr)
    );
    o.stdout
}
fn setup() -> (tempfile::TempDir, Repo) {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]);
    let r = Repo::discover(d.path()).unwrap();
    (d, r)
}
#[test]
fn status_separates_index_and_worktree() {
    let (d, r) = setup();
    fs::write(d.path().join("f"), "one\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "initial"]);
    fs::write(d.path().join("f"), "two\n").unwrap();
    git(d.path(), &["add", "f"]);
    fs::write(d.path().join("f"), "three\n").unwrap();
    fs::write(d.path().join("new"), "new").unwrap();
    let s = r.status().unwrap();
    assert_eq!(s.branch, "main");
    assert!(
        s.entries
            .iter()
            .any(|e| e.path == Path::new("f") && e.staged && e.unstaged)
    );
    assert!(
        s.entries
            .iter()
            .any(|e| e.path == Path::new("new") && e.untracked)
    );
}
#[test]
fn status_handles_unusual_paths() {
    use std::os::unix::ffi::OsStringExt;
    let (d, r) = setup();
    for p in [
        std::ffi::OsString::from("- space\tline\n"),
        std::ffi::OsString::from("[literal]*"),
    ] {
        fs::write(d.path().join(&p), "data").unwrap();
        assert!(
            r.status()
                .unwrap()
                .entries
                .iter()
                .any(|e| e.path.as_os_str() == p)
        );
    }
    let raw = Repo::parse_status(b"? x\xff\0").unwrap();
    assert_eq!(
        raw.entries[0].path.as_os_str(),
        std::ffi::OsString::from_vec(vec![b'x', 255])
    );
}
#[test]
fn discover_worktree_and_unborn_status() {
    let (d, r) = setup();
    assert_eq!(r.root, d.path().canonicalize().unwrap());
    assert!(r.status().unwrap().entries.is_empty());
    fs::write(d.path().join("f"), "x").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-qm", "initial"]);
    let w = d.path().join("linked");
    git(
        d.path(),
        &["worktree", "add", "-qb", "other", w.to_str().unwrap()],
    );
    assert_eq!(Repo::discover(&w).unwrap().root, w.canonicalize().unwrap());
    git(d.path(), &["checkout", "--detach", "-q"]);
    assert!(r.status().unwrap().branch.contains("detached"));
}
fn committed(d: &Path, text: &[u8]) {
    fs::write(d.join("f"), text).unwrap();
    git(d, &["add", "f"]);
    git(d, &["commit", "-qm", "initial"]);
}
#[test]
fn file_staging_preserves_worktree() {
    let (d, r) = setup();
    fs::write(d.path().join("f"), b"new\0binary").unwrap();
    r.stage_file(Path::new("f")).unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), b"new\0binary");
    r.unstage_file(Path::new("f")).unwrap();
    assert!(r.status().unwrap().entries[0].untracked);
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"new\0binary");
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), b"changed\n").unwrap();
    r.stage_file(Path::new("f")).unwrap();
    r.unstage_file(Path::new("f")).unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), b"base\n");
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"changed\n");
}
#[test]
fn hunk_staging_changes_only_selected_hunk() {
    let (d, r) = setup();
    let base = (0..30).map(|i| format!("line{i}\n")).collect::<String>();
    committed(d.path(), base.as_bytes());
    let changed = base
        .replace("line1\n", "first\n")
        .replace("line25\n", "last\n");
    fs::write(d.path().join("f"), &changed).unwrap();
    let diff = r.diff(Path::new("f"), false).unwrap();
    assert_eq!(diff.hunks.len(), 2);
    r.apply_hunk(&diff, 0).unwrap();
    assert_eq!(
        git(d.path(), &["show", ":f"]),
        base.replace("line1\n", "first\n").as_bytes()
    );
    assert_eq!(fs::read(d.path().join("f")).unwrap(), changed.as_bytes());
    let staged = r.diff(Path::new("f"), true).unwrap();
    r.apply_hunk(&staged, 0).unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), base.as_bytes());
}
#[test]
fn stale_hunk_is_rejected() {
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "change\n").unwrap();
    let diff = r.diff(Path::new("f"), false).unwrap();
    fs::write(d.path().join("f"), "other\n").unwrap();
    assert!(r.apply_hunk(&diff, 0).unwrap_err().contains("changed"));
    assert_eq!(git(d.path(), &["show", ":f"]), b"base\n");
}
#[test]
fn hunk_deletion_and_no_final_newline() {
    for (base, changed) in [("first\nlast", "last"), ("base", "changed"), ("old\n", "")] {
        let (d, r) = setup();
        committed(d.path(), base.as_bytes());
        fs::write(d.path().join("f"), changed).unwrap();
        let diff = r.diff(Path::new("f"), false).unwrap();
        r.apply_hunk(&diff, 0).unwrap();
        assert_eq!(git(d.path(), &["show", ":f"]), changed.as_bytes());
        r.apply_hunk(&r.diff(Path::new("f"), true).unwrap(), 0)
            .unwrap();
        assert_eq!(git(d.path(), &["show", ":f"]), base.as_bytes());
    }
}
#[test]
fn hunk_added_and_deleted_files() {
    let (d, r) = setup();
    committed(d.path(), b"old\n");
    fs::remove_file(d.path().join("f")).unwrap();
    r.apply_hunk(&r.diff(Path::new("f"), false).unwrap(), 0)
        .unwrap();
    assert!(r.status().unwrap().entries.iter().any(|e| e.staged));
    r.apply_hunk(&r.diff(Path::new("f"), true).unwrap(), 0)
        .unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), b"old\n");
    fs::write(d.path().join("new"), "added\n").unwrap();
    r.stage_file(Path::new("new")).unwrap();
    r.apply_hunk(&r.diff(Path::new("new"), true).unwrap(), 0)
        .unwrap();
    assert_eq!(fs::read(d.path().join("new")).unwrap(), b"added\n");
    assert!(
        r.status()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == Path::new("new") && e.untracked)
    );
}
#[test]
fn leader_m_routes_and_cancels() {
    use crate::{buffer::Buffer, editor::Editor, key::parse_keys};
    for (suffix, action) in [
        ('s', "Status"),
        ('p', "Push"),
        ('P', "Pull"),
        ('f', "Fetch"),
        ('c', "Commit"),
        ('l', "Log"),
        ('b', "Branches"),
    ] {
        let mut e = Editor::new(Buffer::from_text("source"));
        for k in parse_keys(&format!(" m{suffix}")) {
            e.handle_key(k);
        }
        assert!(
            format!("{:?}", e.pending_effect).contains(action),
            "{suffix}: {:?}",
            e.pending_effect
        );
        assert_eq!(e.buf.line(0), "source");
    }
    for cancel in ["<Esc>", "<C-g>"] {
        let mut e = Editor::new(Buffer::from_text("source"));
        for k in parse_keys(&format!(" m{cancel}s")) {
            e.handle_key(k);
        }
        assert!(e.pending_effect.is_none());
    }
}
#[test]
fn history_and_branch_operations() {
    let (d, r) = setup();
    assert!(r.history().unwrap().is_empty());
    committed(d.path(), b"original\n");
    let log = r.history().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "initial");
    assert!(String::from_utf8_lossy(&r.commit_patch(&log[0].id).unwrap()).contains("+original"));
    git(d.path(), &["branch", "other"]);
    assert!(r.branches().unwrap().contains(&"other".into()));
}
#[test]
fn commit_invocation_uses_staged_content_and_rejects_empty() {
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), b"staged\n").unwrap();
    r.stage_file(Path::new("f")).unwrap();
    fs::write(d.path().join("f"), b"unstaged\n").unwrap();
    assert!(
        r.commit_invocation(b" \n".to_vec(), d.path().join("draft"))
            .is_err()
    );
    git(d.path(), &["config", "user.name", "Fred"]);
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    let inv = r
        .commit_invocation(b"message\n\nbody\n".to_vec(), d.path().join("draft"))
        .unwrap();
    r.run(&inv.args, inv.input.as_deref()).unwrap();
    assert_eq!(git(d.path(), &["show", "HEAD:f"]), b"staged\n");
    assert_eq!(
        git(d.path(), &["log", "-1", "--format=%B"]),
        b"message\n\nbody\n\n"
    );
}
#[test]
fn renamed_file_unstaging_restores_both_paths() {
    let (d, r) = setup();
    committed(d.path(), b"original\n");
    git(d.path(), &["mv", "f", "renamed"]);
    let s = r.status().unwrap();
    assert_eq!(s.entries[0].old_path.as_deref(), Some(Path::new("f")));
    r.unstage_file(Path::new("renamed")).unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), b"original\n");
    assert_eq!(fs::read(d.path().join("renamed")).unwrap(), b"original\n");
    assert!(r.status().unwrap().entries.iter().all(|e| !e.staged));
}
#[test]
fn whole_file_staging_uses_literal_pathspecs() {
    let (d, r) = setup();
    fs::write(d.path().join("[literal]*"), b"chosen\n").unwrap();
    fs::write(d.path().join("l-other"), b"other\n").unwrap();
    r.stage_file(Path::new("[literal]*")).unwrap();
    let s = r.status().unwrap();
    assert_eq!(s.entries.iter().filter(|e| e.staged).count(), 1);
    assert!(
        s.entries
            .iter()
            .any(|e| e.path == Path::new("l-other") && e.untracked)
    );
}
#[test]
fn status_collapse_preserves_distinct_row_actions() {
    use super::{RowAction, Section, View};
    let (d, r) = setup();
    committed(d.path(), b"old\n");
    fs::write(d.path().join("f"), b"staged\n").unwrap();
    r.stage_file(Path::new("f")).unwrap();
    fs::write(d.path().join("f"), b"unstaged\n").unwrap();
    let mut v = View::status(r.clone(), r.status().unwrap());
    assert!(
        v.rows
            .iter()
            .any(|row| row.action == Some(RowAction::File("f".into(), Section::Unstaged)))
    );
    v.closed.insert(Section::Unstaged);
    v.rebuild();
    assert!(
        !v.rows
            .iter()
            .any(|row| row.action == Some(RowAction::File("f".into(), Section::Unstaged)))
    );
    assert!(
        v.rows
            .iter()
            .any(|row| row.action == Some(RowAction::File("f".into(), Section::Staged)))
    );
}
#[test]
fn local_remote_push_fetch_and_fast_forward_pull() {
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "--bare", "-q", "-b", "main"]);
    git(
        d.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    git(d.path(), &["push", "-qu", "origin", "main"]);
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    fs::write(other.path().join("f"), b"remote\n").unwrap();
    git(other.path(), &["add", "f"]);
    git(other.path(), &["commit", "-qm", "remote"]);
    git(other.path(), &["push", "-q"]);
    r.read(&["fetch"]).unwrap();
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"base\n");
    r.read(&["pull", "--ff-only"]).unwrap();
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"remote\n");
    fs::write(d.path().join("local"), b"local").unwrap();
    git(d.path(), &["add", "local"]);
    git(d.path(), &["commit", "-qm", "local"]);
    r.read(&["push"]).unwrap();
    git(other.path(), &["pull", "-q", "--ff-only"]);
    fs::write(other.path().join("remote"), b"diverge").unwrap();
    git(other.path(), &["add", "remote"]);
    git(other.path(), &["commit", "-qm", "other"]);
    git(other.path(), &["push", "-q"]);
    fs::write(d.path().join("mine"), b"diverge").unwrap();
    git(d.path(), &["add", "mine"]);
    git(d.path(), &["commit", "-qm", "mine"]);
    let before = git(d.path(), &["rev-parse", "HEAD"]);
    assert!(r.read(&["pull", "--ff-only"]).is_err());
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), before);
    assert!(!d.path().join(".git/MERGE_HEAD").exists());
}
#[test]
fn conflict_status_and_binary_hunks() {
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["checkout", "-qb", "other"]);
    fs::write(d.path().join("f"), b"other\n").unwrap();
    git(d.path(), &["commit", "-qam", "other"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("f"), b"main\n").unwrap();
    git(d.path(), &["commit", "-qam", "main"]);
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(["merge", "other"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        r.status()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == Path::new("f") && e.conflict)
    );
    git(d.path(), &["merge", "--abort"]);
    fs::write(d.path().join("f"), b"binary\0").unwrap();
    let diff = r.diff(Path::new("f"), false).unwrap();
    assert!(diff.hunks.is_empty());
    assert!(r.apply_hunk(&diff, 0).is_err());
}

#[test]
fn hunk_staging_ignores_user_diff_prefix_settings() {
    let (d, r) = setup();
    committed(d.path(), b"original\n");
    r.read(&["config", "diff.noprefix", "true"]).unwrap();
    fs::write(d.path().join("f"), b"changed\n").unwrap();
    let diff = r.diff(Path::new("f"), false).unwrap();
    r.apply_hunk(&diff, 0).unwrap();
    assert_eq!(git(d.path(), &["show", ":f"]), b"changed\n");
}

#[test]
fn workflow_refs_validate_and_stash_roundtrip() {
    use super::workflows::Operation;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let inv = r.operation(Operation::CreateBranch, "topic").unwrap();
    r.run(&inv.args, None).unwrap();
    assert!(r.branches().unwrap().contains(&"topic".into()));
    assert!(r.operation(Operation::CreateBranch, "--help").is_err());
    assert!(r.operation(Operation::Merge, "--help").is_err());
    fs::write(d.path().join("f"), "dirty\n").unwrap();
    let inv = r.operation(Operation::Stash, "draft").unwrap();
    r.run(&inv.args, None).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "base\n");
    let inv = r.operation(Operation::StashApply, "stash@{0}").unwrap();
    r.run(&inv.args, None).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "dirty\n");
}
#[test]
fn workflow_continuation_requires_matching_operation() {
    use super::workflows::Operation;
    let (_d, r) = setup();
    assert!(r.operation(Operation::RebaseContinue, "").is_err());
    assert!(r.operation(Operation::MergeAbort, "").is_err());
}

#[test]
fn workflow_menu_dispatch_and_prompt_cancel() {
    use crate::{
        buffer::Buffer,
        editor::{Editor, Mode},
        key::parse_keys,
    };
    for (keys, expected) in [
        (" mzz", "Stash"),
        (" mBc", "CreateBranch"),
        (" mMm", "Merge"),
        (" mrr", "Rebase"),
        (" mCa", "Amend"),
    ] {
        let mut e = Editor::new(Buffer::from_text("source"));
        for k in parse_keys(keys) {
            e.handle_key(k);
        }
        assert!(
            format!("{:?}", e.pending_effect).contains(expected),
            "{keys}: {:?}",
            e.pending_effect
        );
        assert!(e.vim.pending.is_empty());
    }
    for cancel in ["<Esc>", "<C-g>", "<C-c>"] {
        let mut e = Editor::new(Buffer::from_text("source"));
        e.magit_prompt = Some((
            Repo {
                root: "/tmp".into(),
            },
            super::workflows::Operation::CreateBranch,
        ));
        e.open_cmdline('=', "topic");
        for k in parse_keys(cancel) {
            e.handle_key(k);
        }
        assert_eq!(e.mode, Mode::Normal);
        assert!(e.magit_prompt.is_none());
        assert!(e.pending_effect.is_none());
    }
}
#[test]
fn workflow_merge_conflict_and_abort_in_linked_worktree() {
    use super::workflows::Operation;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["checkout", "-qb", "topic"]);
    fs::write(d.path().join("f"), "topic\n").unwrap();
    git(d.path(), &["commit", "-qam", "topic"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("f"), "main\n").unwrap();
    git(d.path(), &["commit", "-qam", "main"]);
    let other = d.path().join("linked");
    git(
        d.path(),
        &["worktree", "add", "-qb", "linked", other.to_str().unwrap()],
    );
    let linked = Repo::discover(&other).unwrap();
    let inv = linked.operation(Operation::Merge, "topic").unwrap();
    assert!(linked.run(&inv.args, None).is_err());
    assert_eq!(linked.active_workflow().unwrap(), Some("merge"));
    assert!(linked.status().unwrap().entries.iter().any(|e| e.conflict));
    assert_eq!(r.active_workflow().unwrap(), None);
    let inv = linked.operation(Operation::MergeAbort, "").unwrap();
    linked.run(&inv.args, None).unwrap();
    assert_eq!(linked.active_workflow().unwrap(), None);
    assert_eq!(fs::read_to_string(other.join("f")).unwrap(), "main\n");
}

#[test]
fn workflow_command_owns_git_editor() {
    let (_d, r) = setup();
    let mut command = r.command();
    command.args(["var", "GIT_EDITOR"]);
    let out = command.output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "true");
}

#[test]
fn workflow_rebase_cherry_pick_revert_and_fixup() {
    use super::workflows::Operation;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["config", "user.name", "Fred"]);
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    git(d.path(), &["checkout", "-qb", "topic"]);
    fs::write(d.path().join("added"), "topic\n").unwrap();
    git(d.path(), &["add", "added"]);
    git(d.path(), &["commit", "-qm", "topic"]);
    let topic = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"])).unwrap();
    git(d.path(), &["checkout", "-q", "main"]);
    let inv = r.operation(Operation::CherryPick, topic.trim()).unwrap();
    r.run(&inv.args, None).unwrap();
    assert!(d.path().join("added").exists());
    let inv = r.operation(Operation::Revert, "HEAD").unwrap();
    r.run(&inv.args, None).unwrap();
    assert!(!d.path().join("added").exists());
    fs::write(d.path().join("f"), "fixed\n").unwrap();
    git(d.path(), &["add", "f"]);
    let inv = r.operation(Operation::Fixup, "HEAD").unwrap();
    r.run(&inv.args, None).unwrap();
    assert!(r.history().unwrap()[0].subject.starts_with("fixup! "));
    let inv = r.operation(Operation::Rebase, "HEAD~1").unwrap();
    r.run(&inv.args, None).unwrap();
    assert_eq!(r.active_workflow().unwrap(), None);
}
