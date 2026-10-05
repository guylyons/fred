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
        ("s", "Status"),
        ("pu", "PushUpstream"),
        ("Pu", "PullUpstream"),
        ("fa", "FetchAll"),
        ("cc", "Commit"),
        ("ll", "Log"),
        ("bb", "Branches"),
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
fn snapshots_save_each_side_without_touching_index_or_worktree() {
    use super::workflows::Operation;
    for (operation, base, tree) in [
        (Operation::SnapshotBoth, "base\n", "worktree\n"),
        (Operation::SnapshotIndex, "base\n", "staged\n"),
        (Operation::SnapshotWorktree, "staged\n", "worktree\n"),
    ] {
        let (d, r) = setup();
        committed(d.path(), b"base\n");
        git(d.path(), &["config", "user.name", "Fred"]);
        git(d.path(), &["config", "user.email", "fred@example.test"]);
        fs::write(d.path().join("f"), "staged\n").unwrap();
        git(d.path(), &["add", "f"]);
        fs::write(d.path().join("f"), "worktree\n").unwrap();
        fs::write(d.path().join("[raw]*\n"), "untracked\n").unwrap();
        let status = r.read(&["status", "--porcelain=v2", "-z"]).unwrap();
        r.save_stash(operation, "", &["--include-untracked".into()])
            .unwrap();
        assert_eq!(r.read(&["status", "--porcelain=v2", "-z"]).unwrap(), status);
        assert_eq!(r.read(&["show", "stash@{0}^1:f"]).unwrap(), base.as_bytes());
        assert_eq!(r.read(&["show", "stash@{0}:f"]).unwrap(), tree.as_bytes());
        assert_eq!(r.read(&["show", "stash@{0}^2:f"]).unwrap(), b"staged\n");
        if operation != Operation::SnapshotIndex {
            assert_eq!(
                r.read(&["show", "stash@{0}^3:[raw]*\n"]).unwrap(),
                b"untracked\n"
            );
        } else {
            assert!(r.read(&["rev-parse", "--verify", "stash@{0}^3"]).is_err());
        }
    }
}

#[test]
fn worktree_stash_keeps_staged_changes_and_restores_only_unstaged_side() {
    use super::workflows::{Operation, StashAction};
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["config", "user.name", "Fred"]);
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    fs::write(d.path().join("f"), "staged\n").unwrap();
    git(d.path(), &["add", "f"]);
    fs::write(d.path().join("f"), "worktree\n").unwrap();
    fs::write(d.path().join("new"), "untracked\n").unwrap();
    r.save_stash(
        Operation::StashWorktree,
        "only unstaged",
        &["--include-untracked".into()],
    )
    .unwrap();
    assert_eq!(r.read(&["show", ":f"]).unwrap(), b"staged\n");
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"staged\n");
    assert!(!d.path().join("new").exists());
    let stash = r.stashes().unwrap().remove(0);
    r.stash_action(&stash, StashAction::Pop).unwrap();
    assert_eq!(r.read(&["show", ":f"]).unwrap(), b"staged\n");
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"worktree\n");
    assert_eq!(fs::read(d.path().join("new")).unwrap(), b"untracked\n");
}

#[test]
fn snapshots_preserve_deleted_binary_raw_paths_and_worktree_local_index() {
    use super::workflows::Operation;
    use std::os::unix::ffi::OsStringExt;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["config", "user.name", "Fred"]);
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    let linked = d.path().join("linked");
    git(
        d.path(),
        &["worktree", "add", "-qb", "other", linked.to_str().unwrap()],
    );
    let other = Repo::discover(&linked).unwrap();
    // APFS rejects invalid UTF-8 names; exercise those bytes on Linux and
    // literal wildcard/newline names on both platforms.
    let raw = if cfg!(target_os = "macos") {
        std::ffi::OsString::from("-raw*\n")
    } else {
        std::ffi::OsString::from_vec(b"-raw\xff\n".to_vec())
    };
    fs::write(linked.join(&raw), b"binary\0\xff").unwrap();
    other.stage_file(Path::new(&raw)).unwrap();
    fs::remove_file(linked.join("f")).unwrap();
    other.save_stash(Operation::SnapshotBoth, "", &[]).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let object = std::ffi::OsString::from_vec([b"stash@{0}:".as_slice(), raw.as_bytes()].concat());
    assert_eq!(
        other.run(&["show".into(), object], None).unwrap(),
        b"binary\0\xff"
    );
    assert!(other.read(&["show", "stash@{0}:f"]).is_err());
    assert_eq!(other.read(&["show", "stash@{0}^2:f"]).unwrap(), b"base\n");
    assert!(
        other
            .read(&["diff", "--name-only"])
            .unwrap()
            .starts_with(b"f")
    );
    assert_eq!(r.read(&["show", ":f"]).unwrap(), b"base\n");
    assert!(
        r.read(&["diff", "--cached", "--name-only"])
            .unwrap()
            .is_empty()
    );
    assert!(linked.join(&raw).exists());
}

#[test]
fn snapshots_reject_empty_unborn_and_conflicted_indexes_without_cleanup() {
    use super::workflows::Operation;
    let (d, r) = setup();
    fs::write(d.path().join("f"), "base\n").unwrap();
    git(d.path(), &["add", "f"]);
    assert!(r.save_stash(Operation::SnapshotBoth, "", &[]).is_err());
    git(d.path(), &["commit", "-qm", "base"]);
    assert!(r.save_stash(Operation::SnapshotBoth, "", &[]).is_err());
    assert!(r.stashes().unwrap().is_empty());
    git(d.path(), &["checkout", "-qb", "topic"]);
    fs::write(d.path().join("f"), "topic\n").unwrap();
    git(d.path(), &["commit", "-qam", "topic"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("f"), "main\n").unwrap();
    git(d.path(), &["commit", "-qam", "main"]);
    assert!(r.read(&["merge", "topic"]).is_err());
    let before = fs::read(d.path().join("f")).unwrap();
    assert!(r.save_stash(Operation::StashWorktree, "", &[]).is_err());
    assert_eq!(fs::read(d.path().join("f")).unwrap(), before);
    assert!(r.stashes().unwrap().is_empty());
}

#[test]
fn snapshot_both_keeps_unstaged_reversal_of_a_staged_change() {
    use super::workflows::Operation;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    git(d.path(), &["config", "user.name", "Fred"]);
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    fs::write(d.path().join("f"), "staged\n").unwrap();
    git(d.path(), &["add", "f"]);
    fs::write(d.path().join("f"), "base\n").unwrap();
    r.save_stash(Operation::SnapshotBoth, "", &[]).unwrap();
    assert_eq!(r.read(&["show", "stash@{0}:f"]).unwrap(), b"base\n");
    assert_eq!(r.read(&["show", "stash@{0}^2:f"]).unwrap(), b"staged\n");
    assert_eq!(r.read(&["show", ":f"]).unwrap(), b"staged\n");
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"base\n");
}

#[test]
fn worktree_stash_all_captures_ignored_files_and_keeps_published_stash_on_cleanup_failure() {
    use super::workflows::Operation;
    for fail_cleanup in [false, true] {
        let (d, r) = setup();
        committed(d.path(), b"base\n");
        git(d.path(), &["config", "user.name", "Fred"]);
        git(d.path(), &["config", "user.email", "fred@example.test"]);
        fs::write(d.path().join(".gitignore"), "ignored\n").unwrap();
        fs::write(d.path().join(".gitattributes"), "f filter=blocked\n").unwrap();
        git(d.path(), &["add", ".gitignore", ".gitattributes"]);
        git(d.path(), &["commit", "-qm", "attributes"]);
        if fail_cleanup {
            git(d.path(), &["config", "filter.blocked.clean", "cat"]);
            git(d.path(), &["config", "filter.blocked.smudge", "false"]);
            git(d.path(), &["config", "filter.blocked.required", "true"]);
        }
        fs::write(d.path().join("f"), "worktree\n").unwrap();
        fs::write(d.path().join("ignored"), "saved ignored\n").unwrap();
        let result = r.save_stash(Operation::StashWorktree, "all", &["--all".into()]);
        assert_eq!(result.is_err(), fail_cleanup, "{result:?}");
        assert_eq!(r.stashes().unwrap().len(), 1);
        assert_eq!(r.read(&["show", "stash@{0}:f"]).unwrap(), b"worktree\n");
        assert_eq!(
            r.read(&["show", "stash@{0}^3:ignored"]).unwrap(),
            b"saved ignored\n"
        );
        assert_eq!(r.read(&["show", ":f"]).unwrap(), b"base\n");
        assert_eq!(d.path().join("ignored").exists(), fail_cleanup);
    }
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
        (" mbc", "CreateCheckout"),
        (" mmm", "Merge"),
        (" mRr", "Rebase"),
        (" mca", "Amend"),
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
        e.magit_prompt = Some(super::Prompt::Workflow(
            Repo {
                root: "/tmp".into(),
            },
            super::workflows::Operation::CreateBranch,
            vec![],
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
fn user_leader_bindings_open_branch_commit_and_revert_menus() {
    use crate::{
        buffer::Buffer,
        editor::{Editor, Mode},
        key::parse_keys,
    };
    for (keys, menu) in [
        (" mb", 'b'),
        (" mB", 'B'),
        (" mc", 'C'),
        (" mr", 'v'),
        (" ml", 'l'),
    ] {
        let mut e = Editor::new(Buffer::from_text("source"));
        for key in parse_keys(keys) {
            e.handle_key(key);
        }
        assert!(matches!(e.mode, Mode::Pick(_)), "{keys}: {:?}", e.mode);
        assert_eq!(e.magit_menu, Some(menu));
        assert!(
            e.pending_effect.is_none(),
            "a prefix must open its menu, not run a command"
        );
    }
}

#[test]
fn stash_view_k_moves_up_without_requesting_a_drop() {
    use crate::{buffer::Buffer, editor::Editor, key::parse_keys};
    let (_d, repo) = setup();
    let mut e = Editor::new(Buffer::from_text("heading\nstash row\n"));
    let mut view = super::View::status(repo, Snapshot::default());
    view.kind = super::Kind::Stashes;
    e.magit = Some(Box::new(view));
    e.set_cursor(1, 0);
    for key in parse_keys("k") {
        e.handle_key(key);
    }
    assert_eq!(e.cur.line, 0);
    assert!(e.pending_effect.is_none());
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

#[test]
fn stash_pop_removes_only_selected_entry_and_restores_untracked() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "first\n").unwrap();
    fs::write(d.path().join("untracked"), "saved\n").unwrap();
    git(d.path(), &["stash", "push", "-qu", "-m", "first"]);
    fs::write(d.path().join("f"), "second\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "second"]);
    let list = r.stashes().unwrap();
    assert_eq!(list.len(), 2);
    r.stash_action(&list[1], StashAction::Pop).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "first\n");
    assert_eq!(
        fs::read_to_string(d.path().join("untracked")).unwrap(),
        "saved\n"
    );
    assert_eq!(r.stashes().unwrap()[0].id, list[0].id);
}
#[test]
fn stash_stale_selection_cannot_drop_renumbered_entry() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "first\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "first"]);
    let selected = r.stashes().unwrap()[0].clone();
    fs::write(d.path().join("f"), "second\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "second"]);
    let before = r.stashes().unwrap();
    assert!(r.stash_action(&selected, StashAction::Drop).is_err());
    assert!(r.stash_action(&selected, StashAction::Pop).is_err());
    assert_eq!(r.stashes().unwrap(), before);
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "base\n");
    r.stash_action(&selected, StashAction::Apply).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "first\n");
    assert_eq!(r.stashes().unwrap(), before);
}
#[test]
fn stash_conflicted_pop_keeps_stash_and_patch_includes_saved_changes() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "stashed\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "saved"]);
    let selected = r.stashes().unwrap()[0].clone();
    assert!(String::from_utf8_lossy(&r.stash_patch(&selected).unwrap()).contains("+stashed"));
    fs::write(d.path().join("f"), "committed\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "conflicting change"]);
    assert!(r.stash_action(&selected, StashAction::Pop).is_err());
    assert_eq!(r.stashes().unwrap(), vec![selected]);
    assert!(r.status().unwrap().entries.iter().any(|e| e.conflict));
}

#[test]
fn stash_pop_restores_staged_and_unstaged_sides() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("index-file"), "staged\n").unwrap();
    git(d.path(), &["add", "index-file"]);
    fs::write(d.path().join("f"), "unstaged\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "both"]);
    let stash = r.stashes().unwrap()[0].clone();
    r.stash_action(&stash, StashAction::Pop).unwrap();
    let status = r.status().unwrap();
    assert!(
        status
            .entries
            .iter()
            .any(|e| e.path == Path::new("index-file") && e.staged && !e.unstaged)
    );
    assert!(
        status
            .entries
            .iter()
            .any(|e| e.path == Path::new("f") && e.unstaged && !e.staged)
    );
    let patch = String::from_utf8(r.stash_patch(&stash).unwrap()).unwrap();
    assert!(patch.contains("Staged\n") && patch.contains("Unstaged\n"));
}
#[test]
fn stash_index_failure_falls_back_and_retains_stash() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "saved staged\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["stash", "push", "-qm", "index"]);
    let stash = r.stashes().unwrap()[0].clone();
    fs::write(d.path().join("f"), "other committed\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "other"]);
    assert!(r.stash_action(&stash, StashAction::Pop).is_err());
    assert!(r.status().unwrap().entries.iter().any(|e| e.conflict));
    assert_eq!(r.stashes().unwrap(), vec![stash]);
}

#[test]
fn workflow_menu_is_visible_navigable_and_cancelled() {
    use crate::{
        buffer::Buffer,
        editor::{Editor, Mode},
        key::parse_keys,
    };
    let mut ed = Editor::new(Buffer::from_text("source"));
    for key in parse_keys(" mz") {
        ed.handle_key(key);
    }
    let Mode::Pick(picker) = &ed.mode else {
        panic!("expected visible menu");
    };
    assert!(picker.rows.iter().any(|r| r.text.contains("Apply")));
    for key in parse_keys("<C-g>") {
        ed.handle_key(key);
    }
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.pending_effect.is_none());
    for key in parse_keys(" mb<Down><Down><Enter>") {
        ed.handle_key(key);
    }
    assert!(format!("{:?}", ed.pending_effect).contains("CreateCheckout"));
    assert_eq!(ed.buf.line(0), "source");
}

#[test]
fn stash_drop_uses_ordinal_identity_with_configured_log_dates() {
    use super::workflows::StashAction;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    fs::write(d.path().join("f"), "changed\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "saved"]);
    git(d.path(), &["config", "log.date", "iso"]);
    let stash = r.stashes().unwrap()[0].clone();
    r.stash_action(&stash, StashAction::Drop).unwrap();
    assert!(r.stashes().unwrap().is_empty());
}

#[test]
fn file_history_filters_literal_paths_and_follows_renames() {
    let (d, repo) = setup();
    let old = ":(glob)* old";
    let new = "renamed file";
    fs::write(d.path().join(old), "original\n").unwrap();
    git(d.path(), &["add", "--", old]);
    git(d.path(), &["commit", "-qm", "original file"]);
    fs::write(d.path().join("unrelated"), "other\n").unwrap();
    git(d.path(), &["add", "unrelated"]);
    git(d.path(), &["commit", "-qm", "unrelated commit"]);
    git(d.path(), &["mv", "--", old, new]);
    git(d.path(), &["commit", "-qm", "rename file"]);
    for (file, follow, want) in [
        (old, false, vec!["rename file", "original file"]),
        (new, false, vec!["rename file"]),
        (new, true, vec!["rename file", "original file"]),
    ] {
        let commits = repo.file_history(Path::new(file), follow).unwrap();
        assert_eq!(
            commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            want
        );
    }
    assert!(repo.file_history(Path::new("../outside"), false).is_err());
    assert!(repo.file_history(d.path(), false).is_err());
}

fn with_remote() -> (tempfile::TempDir, Repo, tempfile::TempDir) {
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "--bare", "-q", "-b", "main"]);
    git(
        d.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    (d, r, bare)
}
fn run_net(r: &Repo, op: super::network::Op, answers: &[&str], args: &[&str]) -> Vec<String> {
    let answers: Vec<String> = answers.iter().map(|s| s.to_string()).collect();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let inv = r.network(op, &answers, &args).unwrap();
    r.run(&inv.args, None).unwrap();
    inv.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}
#[test]
fn push_remote_sets_unconfigured_push_remote_then_pushes() {
    use super::network::Op::*;
    let (d, r, bare) = with_remote();
    let prompts = r.network_prompts(PushRemote).unwrap();
    assert_eq!(prompts, ["Set branch.main.pushRemote and push there: "]);
    assert!(r.network(PushRemote, &["nope".into()], &[]).is_err());
    let args = run_net(&r, PushRemote, &["origin"], &[]);
    assert_eq!(
        args,
        ["push", "-v", "origin", "refs/heads/main:refs/heads/main"]
    );
    assert_eq!(
        git(d.path(), &["config", "branch.main.pushRemote"]),
        b"origin\n"
    );
    git(bare.path(), &["rev-parse", "--verify", "main"]);
    assert!(r.network_prompts(PushRemote).unwrap().is_empty());
    assert!(r.network_prompts(FetchRemote).unwrap().is_empty());
}
#[test]
fn push_upstream_sets_upstream_or_uses_configured_one() {
    use super::network::Op::*;
    let (d, r, bare) = with_remote();
    assert_eq!(
        r.network_prompts(PushUpstream).unwrap(),
        ["Set upstream of main and push there: "]
    );
    let args = run_net(&r, PushUpstream, &["origin/trunk"], &["--dry-run"]);
    assert_eq!(
        args,
        [
            "push",
            "-v",
            "--dry-run",
            "--set-upstream",
            "origin",
            "main:refs/heads/trunk"
        ]
    );
    assert!(git(bare.path(), &["branch"]).is_empty());
    run_net(&r, PushUpstream, &["origin/trunk"], &[]);
    assert_eq!(
        git(d.path(), &["config", "branch.main.merge"]),
        b"refs/heads/trunk\n"
    );
    assert!(r.network_prompts(PushUpstream).unwrap().is_empty());
    committed(d.path(), b"next\n");
    let args = run_net(&r, PushUpstream, &[], &[]);
    assert_eq!(args, ["push", "-v", "origin", "main:refs/heads/trunk"]);
    assert_eq!(
        git(bare.path(), &["rev-parse", "trunk"]),
        git(d.path(), &["rev-parse", "HEAD"])
    );
}
#[test]
fn push_elsewhere_other_tag_and_matching_targets() {
    use super::network::Op::*;
    let (d, r, bare) = with_remote();
    let args = run_net(&r, PushElsewhere, &["origin/topic"], &[]);
    assert_eq!(args, ["push", "-v", "origin", "main:refs/heads/topic"]);
    git(d.path(), &["fetch", "-q", "origin"]);
    let args = run_net(&r, PushOther, &["HEAD", "origin/topic"], &["--force"]);
    assert_eq!(args, ["push", "-v", "--force", "origin", "HEAD:topic"]);
    git(d.path(), &["tag", "v1"]);
    assert_eq!(r.network_prompts(PushTag).unwrap(), ["Push tag: "]);
    run_net(&r, PushTag, &["v1"], &[]);
    git(bare.path(), &["rev-parse", "--verify", "refs/tags/v1"]);
    assert!(r.network(PushTag, &["missing".into()], &[]).is_err());
    assert!(r.network_prompts(PushMatching).unwrap().is_empty());
    let args = run_net(&r, PushMatching, &[], &["--dry-run"]);
    assert_eq!(args, ["push", "-v", "--dry-run", "origin", ":"]);
    let args = run_net(&r, PushRefspecs, &["origin", "main:a, main:b"], &[]);
    assert_eq!(args, ["push", "-v", "origin", "main:a", "main:b"]);
    git(bare.path(), &["rev-parse", "--verify", "b"]);
    assert!(
        r.network(PushRefspecs, &["origin".into(), "--mirror".into()], &[])
            .is_err()
    );
    assert!(r.network(PushElsewhere, &["--exec=x".into()], &[]).is_err());
    let injected = ["origin/--upload-pack=touch pwned".to_owned()];
    assert!(r.network(PullElsewhere, &injected, &[]).is_err());
    assert!(r.network(PushUpstream, &injected, &[]).is_err());
    git(d.path(), &["remote", "add", "upstream", "/nonexistent"]);
    let inv = r.network(FetchUpstream, &[], &[]).unwrap();
    assert_eq!(inv.args[1], "upstream");
}
#[test]
fn fetch_and_pull_suffixes_use_current_remote_and_upstream() {
    use super::network::Op::*;
    let (d, r, bare) = with_remote();
    git(d.path(), &["push", "-q", "origin", "main"]);
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    fs::write(other.path().join("f"), b"remote\n").unwrap();
    git(other.path(), &["commit", "-qam", "remote"]);
    git(other.path(), &["push", "-q"]);
    let args = run_net(&r, FetchUpstream, &[], &["--prune"]);
    assert_eq!(args, ["fetch", "origin", "--prune"]);
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"base\n");
    assert_eq!(
        r.network_prompts(PullUpstream).unwrap(),
        ["Set upstream of main and pull from there: "]
    );
    let args = run_net(&r, PullUpstream, &["origin/main"], &["--ff-only"]);
    assert_eq!(args, ["pull", "--ff-only", "origin", "refs/heads/main"]);
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"remote\n");
    assert_eq!(
        git(d.path(), &["config", "branch.main.remote"]),
        b"origin\n"
    );
    let args = run_net(&r, PullElsewhere, &["origin/main"], &[]);
    assert_eq!(args, ["pull", "origin", "main"]);
    let args = run_net(&r, FetchAll, &[], &["--tags"]);
    assert_eq!(args, ["fetch", "--all", "--tags"]);
    let args = run_net(&r, FetchBranch, &["origin", "main"], &[]);
    assert_eq!(args, ["fetch", "origin", "main"]);
    assert!(
        r.network(FetchElsewhere, &["elsewhere".into()], &[])
            .is_err()
    );
    git(d.path(), &["checkout", "-q", "--detach"]);
    assert!(r.network_prompts(PullRemote).is_err());
}
#[test]
fn diff_targets_render_index_worktree_range_commit_and_paths() {
    use super::diff::{Op, Target};
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    committed(d.path(), b"two\n");
    fs::write(d.path().join("f"), b"three\n").unwrap();
    git(d.path(), &["add", "f"]);
    fs::write(d.path().join("f"), b"three \n").unwrap();
    let text = |t: &Target, args: &[&str]| {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        String::from_utf8(r.diff_output(t, &args).unwrap()).unwrap()
    };
    assert!(text(&Target::Staged, &[]).contains("+three\n"));
    assert!(text(&Target::Unstaged, &[]).contains("+three \n"));
    assert!(text(&Target::Unstaged, &["--ignore-all-space"]).is_empty());
    assert!(text(&Target::Range("HEAD".into()), &[]).contains("+three \n"));
    let range = r.diff_target(Op::Range, &["HEAD~1..HEAD".into()]).unwrap();
    assert!(text(&range, &["--stat"]).contains("1 file changed"));
    assert!(r.diff_target(Op::Range, &["-p".into()]).is_err());
    let bad = r.diff_target(Op::Range, &["nope..HEAD".into()]).unwrap();
    assert!(r.diff_output(&bad, &[]).is_err());
    let one = r.diff_target(Op::Range, &["HEAD^!".into()]).unwrap();
    assert!(text(&one, &[]).contains("+two\n"));
    let commit = r.diff_target(Op::ShowCommit, &["HEAD~1".into()]).unwrap();
    assert!(text(&commit, &[]).contains("+one\n"));
    assert!(r.diff_target(Op::ShowCommit, &["--all".into()]).is_err());
    fs::write(d.path().join("a"), b"left\n").unwrap();
    fs::write(d.path().join("b"), b"right\n").unwrap();
    let paths = r.diff_target(Op::Paths, &["a".into(), "b".into()]).unwrap();
    let out = text(&paths, &["--diff-merges=off"]);
    assert!(out.contains("-left") && out.contains("+right"), "{out}");
    assert!(
        r.diff_target(Op::Paths, &["a".into(), "missing".into()])
            .is_err()
    );
}
#[test]
fn blame_attributes_chunks_and_commit_info() {
    let (d, r) = setup();
    fs::write(d.path().join("f"), b"a\nb\nc\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "first"]);
    fs::write(d.path().join("f"), b"a\nB  \nc\n").unwrap();
    git(d.path(), &["commit", "-qam", "second"]);
    let (chunks, info) = r
        .blame(Path::new("f"), None, super::blame::Kind::Addition, &[])
        .unwrap();
    let lines: Vec<_> = chunks.iter().map(|c| (c.line, c.lines)).collect();
    assert_eq!(lines, [(0, 1), (1, 1), (2, 1)]);
    assert_eq!(info[&chunks[1].rev].summary, "second");
    assert_eq!(info[&chunks[0].rev].summary, "first");
    assert!(chunks[1].prev.is_some() && chunks[0].prev.is_none());
    let blame = super::blame::Blame {
        repo: r.clone(),
        file: "f".into(),
        args: vec![],
        chunks,
        info,
        style: 0,
        kind: super::blame::Kind::Addition,
        rev: None,
        version: 0,
        was_readonly: false,
    };
    let (w, heading) = blame.margin(1).unwrap();
    assert_eq!(w, super::blame::HEADING_WIDTH);
    assert!(
        heading.starts_with("Fred Test") && heading.ends_with("second"),
        "{heading}"
    );
    assert!(
        r.blame(
            Path::new("missing"),
            None,
            super::blame::Kind::Addition,
            &[]
        )
        .is_err()
    );
}
#[test]
fn blame_time_uses_the_local_zone() {
    let info = |t, tz: &str| super::blame::Info {
        committer_time: t,
        committer_tz: tz.into(),
        ..Default::default()
    };
    let (_d, r) = setup();
    let mut blame = super::blame::Blame {
        repo: r,
        file: "f".into(),
        args: vec![],
        chunks: vec![super::blame::Chunk {
            rev: "a".repeat(40),
            line: 0,
            lines: 2,
            orig_line: 1,
            orig_file: "f".into(),
            prev: None,
        }],
        info: [("a".repeat(40), info(1_700_000_000, "+0100"))].into(),
        style: 0,
        kind: super::blame::Kind::Addition,
        rev: None,
        version: 0,
        was_readonly: false,
    };
    // magit-blame-time-format in local time, as format-time-string.
    let local = |t| super::margin::strftime("%F %H:%M", t);
    assert!(
        blame
            .heading(&blame.chunks[0])
            .contains(&local(1_700_000_000))
    );
    blame.info.insert("a".repeat(40), info(0, "-0230"));
    assert!(blame.heading(&blame.chunks[0]).contains(&local(0)));
    assert_eq!(blame.margin(1).unwrap().1, "");
    blame.style = 2;
    assert_eq!(blame.margin(0).unwrap(), (1, "┌".into()));
    blame.kind = super::blame::Kind::Echo;
    assert_eq!((blame.margin(0), blame.width()), (None, 0));
    assert!(blame.message(0).is_some());
    blame.kind = super::blame::Kind::Addition;
    assert_eq!(blame.margin(1).unwrap(), (1, "│".into()));
}
#[test]
fn blob_history_walks_index_commits_and_renames() {
    use super::blob::{INDEX, WORKTREE};
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    committed(d.path(), b"two\n");
    git(d.path(), &["mv", "f", "g"]);
    git(d.path(), &["commit", "-qm", "rename"]);
    let g = Path::new("g");
    let head = r.blob_rev("HEAD").unwrap();
    assert_eq!(r.blob_ancestor(WORKTREE, g), Some((head.clone(), g.into())));
    fs::write(d.path().join("g"), b"staged\n").unwrap();
    git(d.path(), &["add", "g"]);
    assert_eq!(r.blob_ancestor(WORKTREE, g), Some((INDEX.into(), g.into())));
    assert_eq!(r.blob_bytes(INDEX, g).unwrap(), b"staged\n");
    let (older, file) = r.blob_ancestor(&head, g).unwrap();
    assert_eq!(file, Path::new("f"));
    assert_eq!(r.blob_bytes(&older, &file).unwrap(), b"two\n");
    let (oldest, _) = r.blob_ancestor(&older, &file).unwrap();
    assert_eq!(r.blob_bytes(&oldest, Path::new("f")).unwrap(), b"one\n");
    assert_eq!(r.blob_ancestor(&oldest, Path::new("f")), None);
    assert_eq!(r.blob_successor(&oldest, g).unwrap().0, older);
    assert_eq!(r.blob_successor(&head, g), Some((INDEX.into(), g.into())));
    assert_eq!(
        r.blob_successor(INDEX, g),
        Some((WORKTREE.into(), g.into()))
    );
    assert_eq!(r.blob_successor(WORKTREE, g), None);
    assert!(r.blob_rev("--all").is_err());
    assert!(r.blob_bytes("HEAD", Path::new("missing")).is_err());
}
#[test]
fn blame_removal_and_reverse_on_revisions() {
    use super::blame::Kind;
    let (d, r) = setup();
    fs::write(d.path().join("f"), b"a\nb\nc\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "first"]);
    let first = r.blob_rev("HEAD").unwrap();
    fs::write(d.path().join("f"), b"a\nc\n").unwrap();
    git(d.path(), &["commit", "-qam", "drop b"]);
    let (chunks, info) = r
        .blame(Path::new("f"), Some(&first), Kind::Removal, &[])
        .unwrap();
    let b = chunks.iter().find(|c| c.line == 1).unwrap();
    assert_eq!(info[&b.rev].summary, "drop b", "{chunks:?}");
    let (chunks, info) = r
        .blame(Path::new("f"), Some(&first), Kind::Reverse, &[])
        .unwrap();
    let b = chunks.iter().find(|c| c.line == 1).unwrap();
    assert_eq!(info[&b.rev].summary, "first");
    assert!(
        r.blame(Path::new("f"), Some("-x"), Kind::Addition, &[])
            .is_err()
    );
}
#[test]
fn blob_history_and_blame_handle_quoted_and_stage_like_names() {
    use super::blame::{Kind, unquote};
    assert_eq!(
        unquote(r#""caf\303\251 \"x\"\\.txt""#),
        Path::new("café \"x\"\\.txt")
    );
    assert_eq!(unquote("plain name"), Path::new("plain name"));
    let (d, r) = setup();
    let name = Path::new("café \"x\".txt");
    fs::write(d.path().join(name), b"one\n").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-qm", "one"]);
    fs::write(d.path().join(name), b"two\n").unwrap();
    git(d.path(), &["commit", "-qam", "two"]);
    let head = r.blob_rev("HEAD").unwrap();
    let (older, file) = r.blob_ancestor(&head, name).unwrap();
    assert_eq!(file, name);
    assert_eq!(r.blob_bytes(&older, &file).unwrap(), b"one\n");
    assert_eq!(r.blob_successor(&older, name).unwrap().0, head);
    let (chunks, _) = r.blame(name, Some(&head), Kind::Addition, &[]).unwrap();
    assert_eq!(chunks[0].orig_file, name);
    assert_eq!(chunks[0].prev.as_ref().unwrap().1, name);
    fs::write(d.path().join("1:f"), b"stage-like\n").unwrap();
    git(d.path(), &["add", "1:f"]);
    assert_eq!(
        r.blob_bytes(super::blob::INDEX, Path::new("1:f")).unwrap(),
        b"stage-like\n"
    );
    assert!(!r.conflicted(Path::new("1:f")));
}
#[test]
fn exact_paths_reject_symlinks_case_aliases_and_existing_targets() {
    use super::blob::exact;
    let (d, r) = setup();
    let root = r.root.clone();
    fs::create_dir(d.path().join("sub")).unwrap();
    fs::write(d.path().join("sub/f.txt"), b"x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("x"), b"precious").unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join("out")).unwrap();
    assert_eq!(
        exact(&root, Path::new("sub/f.txt"), true).unwrap(),
        root.join("sub/f.txt")
    );
    assert!(
        exact(&root, Path::new("out/x"), true)
            .unwrap_err()
            .contains("symbolic link")
    );
    assert!(exact(&root, Path::new("out/new"), false).is_err());
    assert!(exact(&root, Path::new("SUB/f.txt"), true).is_err());
    assert!(exact(&root, Path::new("sub/F.TXT"), true).is_err());
    assert!(exact(&root, Path::new("sub/new"), false).is_ok());
    // A destination never replaces an existing file.
    assert!(
        exact(&root, Path::new("sub/f.txt"), false)
            .unwrap_err()
            .contains("already exists")
    );
    assert!(super::blob::relative(".git").is_err());
    assert!(super::blob::relative("sub/.GIT/hooks").is_err());
    assert!(exact(&root, Path::new("missing/new"), false).is_err());
    // The link itself (last component) may be removed; its target is untouched.
    assert!(exact(&root, Path::new("out"), true).is_ok());
    assert_eq!(fs::read(outside.path().join("x")).unwrap(), b"precious");
}
#[test]
fn status_headers_and_log_sections_follow_upstream_and_push_remote() {
    use super::Section;
    let (d, r, bare) = with_remote();
    git(d.path(), &["tag", "v1"]);
    git(d.path(), &["push", "-qu", "origin", "main"]);
    committed(d.path(), b"local\n");
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    fs::write(other.path().join("remote"), b"r").unwrap();
    git(other.path(), &["add", "remote"]);
    git(other.path(), &["commit", "-qm", "remote work"]);
    git(other.path(), &["push", "-q"]);
    git(d.path(), &["fetch", "-q"]);
    fs::write(d.path().join("f"), b"stash me\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "parked"]);
    let x = r.status_extra();
    assert_eq!(x.headers[0], "Head:     main initial");
    assert_eq!(x.headers[1], "Merge:    origin/main remote work");
    assert_eq!(x.headers[2], "Tag:      v1 (1)");
    assert_eq!(x.stashes.len(), 1);
    let names: Vec<_> = x
        .logs
        .iter()
        .map(|(s, h, c)| (*s, h.as_str(), c.len()))
        .collect();
    assert_eq!(
        names,
        [
            (Section::UnpushedUpstream, "Unmerged into origin/main", 1),
            (Section::UnpulledUpstream, "Unpulled from origin/main", 1),
        ]
    );
    git(d.path(), &["config", "branch.main.pushRemote", "origin"]);
    git(d.path(), &["config", "branch.main.rebase", "true"]);
    let x = r.status_extra();
    assert_eq!(x.headers[1], "Rebase:   origin/main remote work");
    assert_eq!(x.headers[2], "Push:     origin/main remote work");
    // The push target equals the upstream, so no duplicate push sections.
    assert_eq!(x.logs.len(), 2);
    git(d.path(), &["config", "branch.main.pushRemote", "nowhere"]);
    let x = r.status_extra();
    assert_eq!(x.headers[2], "Push:     nowhere remote does not exist");
    git(
        d.path(),
        &["config", "branch.main.merge", "refs/heads/gone"],
    );
    let x = r.status_extra();
    assert_eq!(
        x.headers[1],
        "Rebase:   refs/heads/gone does not exist on origin"
    );
    git(
        d.path(),
        &["config", "branch.main.merge", "refs/heads/a\nb"],
    );
    assert!(!r.status_extra().headers[1].contains('\n'));
    git(d.path(), &["config", "branch.main.rebase", "merges"]);
    git(d.path(), &["config", "pull.rebase", "no"]);
    assert!(r.status_extra().headers[1].starts_with("Merge:"));
    git(d.path(), &["config", "branch.main.remote", "deleted"]);
    assert_eq!(
        r.status_extra().headers[1],
        "Merge:    invalid upstream configuration"
    );
    assert_eq!(x.logs[0].1, "Recent commits");
}
#[test]
fn status_of_unborn_and_detached_heads() {
    let (d, r) = setup();
    assert_eq!(
        r.status_extra().headers,
        ["Head:     main (no commits yet)"]
    );
    committed(d.path(), b"one\n");
    git(d.path(), &["checkout", "-q", "--detach"]);
    let x = r.status_extra();
    assert!(
        x.headers[0].starts_with("Head:     ") && x.headers[0].ends_with(" initial"),
        "{:?}",
        x.headers
    );
    assert_eq!(x.logs[0].1, "Recent commits");
}
#[test]
fn branch_suffixes_follow_magit_branch() {
    use super::Question as Q;
    use super::branch::{Next, Op};
    let (d, r, bare) = with_remote();
    git(d.path(), &["push", "-qu", "origin", "main"]);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let done = |n: Next| match n {
        Next::Done(r) => r,
        Next::Ask(op, p, _) => Err(format!("asked {op:?}: {p:?}")),
        Next::Git(a) => Err(format!("git {a:?}")),
        other => Err(format!("{other:?}")),
    };
    let head = |b: &str| git(d.path(), &["rev-parse", b]);
    // n: create without checkout; c: create and checkout.
    done(r.branch_step(Op::Create, &s(&["topic", "main"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "main");
    assert!(
        done(r.branch_step(Op::Create, &s(&["topic", "main"]), &[]))
            .unwrap_err()
            .contains("exists")
    );
    assert!(done(r.branch_step(Op::Create, &s(&["bad..name", "main"]), &[])).is_err());
    assert!(done(r.branch_step(Op::Create, &s(&["x", "-p"]), &[])).is_err());
    done(r.branch_step(Op::CreateCheckout, &s(&["work", "main"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "work");
    // l: local, remote-tracking (pushRemote set), or new name with a start point.
    done(r.branch_step(Op::CheckoutLocal, &s(&["main"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "main");
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    git(other.path(), &["checkout", "-qb", "feature"]);
    git(other.path(), &["push", "-qu", "origin", "feature"]);
    git(d.path(), &["fetch", "-q"]);
    git(d.path(), &["config", "remote.pushDefault", "elsewhere"]);
    done(r.branch_step(Op::CheckoutLocal, &s(&["origin/feature"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "feature");
    assert_eq!(
        git(d.path(), &["config", "branch.feature.pushRemote"]),
        b"origin\n"
    );
    match r.branch_step(Op::CheckoutLocal, &s(&["brand-new"]), &[]) {
        Next::Ask(Q::Branch(Op::CheckoutNew(n)), _, defaults) => {
            assert_eq!((n.as_str(), defaults[0].as_str()), ("brand-new", "feature"))
        }
        _ => panic!("expected a start-point question"),
    }
    done(r.branch_step(Op::CheckoutNew("brand-new".into()), &s(&["main"]), &[])).unwrap();
    assert_eq!(head("brand-new"), head("main"));
    // m: rename keeps the push target.
    git(
        d.path(),
        &["config", "branch.brand-new.pushRemote", "origin"],
    );
    done(r.branch_step(Op::Rename, &s(&["brand-new", "renamed"]), &[])).unwrap();
    assert_eq!(
        git(d.path(), &["config", "branch.renamed.pushRemote"]),
        b"origin\n"
    );
    // s: spin off unpushed commits; main returns to its upstream.
    done(r.branch_step(Op::CheckoutLocal, &s(&["main"]), &[])).unwrap();
    let pushed = head("main");
    committed(d.path(), b"unpushed\n");
    let unpushed = head("main");
    done(r.branch_step(Op::Spinoff, &s(&["spun"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "spun");
    assert_eq!(
        (head("spun"), head("main")),
        (unpushed.clone(), pushed.clone())
    );
    // S: spin out stays on the branch and hard-resets it (clean tree only).
    done(r.branch_step(Op::CheckoutLocal, &s(&["main"]), &[])).unwrap();
    committed(d.path(), b"more\n");
    done(r.branch_step(Op::Spinout, &s(&["out"]), &[])).unwrap();
    assert_eq!(r.current_branch().unwrap(), "main");
    assert_eq!(head("main"), pushed);
    // x: reset another branch by ref update; the current one asks when dirty.
    // The target's default is the chosen branch's own upstream.
    match r.branch_step(Op::Reset, &s(&["out"]), &[]) {
        Next::Ask(Q::Branch(Op::ResetTo(b)), _, defaults) => {
            assert_eq!((b.as_str(), defaults), ("out", vec![String::new()]))
        }
        _ => panic!("reset asks for the target next"),
    }
    match r.branch_step(Op::Reset, &s(&["main"]), &[]) {
        Next::Ask(_, _, defaults) => assert_eq!(defaults, ["origin/main"]),
        _ => panic!("reset asks for the target next"),
    }
    done(r.branch_step(Op::ResetTo("out".into()), &s(&["main"]), &[])).unwrap();
    assert_eq!(head("out"), pushed);
    fs::write(d.path().join("f"), b"dirty\n").unwrap();
    match r.branch_step(Op::ResetTo("main".into()), &s(&["spun"]), &[]) {
        Next::Ask(Q::Branch(op @ Op::ResetConfirmed(..)), _, _) => {
            assert!(done(r.branch_step(op.clone(), &s(&["no"]), &[])).is_err());
            assert_eq!(fs::read(d.path().join("f")).unwrap(), b"dirty\n");
            done(r.branch_step(op, &s(&["yes"]), &[])).unwrap();
        }
        _ => panic!("expected confirmation"),
    }
    assert_eq!(head("main"), unpushed);
    // k: merged deletes; unmerged asks; current offers detach.
    done(r.branch_step(Op::Delete, &s(&["out"]), &[])).unwrap();
    match r.branch_step(Op::Delete, &s(&["topic"]), &[]) {
        Next::Done(Ok(_)) => (),
        _ => panic!("topic is merged into HEAD"),
    }
    match r.branch_step(Op::Delete, &s(&["renamed"]), &[]) {
        Next::Done(Ok(_)) => (),
        other => panic!("{:?}", matches!(other, Next::Ask(..))),
    }
    git(d.path(), &["checkout", "-qb", "doomed"]);
    committed(d.path(), b"doomed\n");
    git(d.path(), &["checkout", "-q", "main"]);
    let op = match r.branch_step(Op::Delete, &s(&["doomed"]), &[]) {
        Next::Ask(Q::Branch(op @ Op::DeleteUnmerged(_)), _, _) => op,
        _ => panic!("unmerged branches need confirmation"),
    };
    assert!(done(r.branch_step(op.clone(), &s(&["n"]), &[])).is_err());
    done(r.branch_step(op, &s(&["y"]), &[])).unwrap();
    assert!(!r.branch_choices().contains(&"doomed".to_string()));
    match r.branch_step(Op::Delete, &s(&["main"]), &[]) {
        Next::Ask(Q::Branch(op @ Op::DeleteCurrent(_)), _, defaults) => {
            assert_eq!(defaults, ["origin/main"], "indirect upstream");
            assert!(done(r.branch_step(op.clone(), &s(&["a"]), &defaults)).is_err());
        }
        _ => panic!("current branch offers detach"),
    }
    // Detaching from an unmerged current branch asks first and stays put on "n".
    git(d.path(), &["checkout", "-qb", "lonely"]);
    committed(d.path(), b"lonely\n");
    let op = match r.branch_step(Op::Delete, &s(&["lonely"]), &[]) {
        Next::Ask(Q::Branch(op @ Op::DeleteCurrent(_)), _, defaults) => {
            assert_eq!(
                defaults,
                ["main"],
                "magit-main-branch is the checkout target"
            );
            match r.branch_step(op, &s(&["d"]), &defaults) {
                Next::Ask(Q::Branch(op @ Op::DeleteCurrentUnmerged(..)), _, _) => op,
                _ => panic!("unmerged current branch must be confirmed before detaching"),
            }
        }
        _ => panic!("current branch offers detach"),
    };
    assert!(done(r.branch_step(op.clone(), &s(&["n"]), &[])).is_err());
    assert_eq!(r.current_branch().unwrap(), "lonely");
    done(r.branch_step(op, &s(&["y"]), &[])).unwrap();
    assert!(r.current_branch().is_err());
    git(d.path(), &["checkout", "-q", "main"]);
    // Validation happens after stripping heads/.
    assert!(done(r.branch_step(Op::Rename, &s(&["heads/-M", "x"]), &[])).is_err());
    // Spin-out refuses before touching anything when the hard reset would
    // replace an untracked file that the base commit tracks.
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("precious"), b"base\n").unwrap();
    git(d.path(), &["add", "precious"]);
    git(d.path(), &["commit", "-qm", "adds precious"]);
    git(d.path(), &["checkout", "-qb", "clobber"]);
    git(d.path(), &["branch", "-q", "--set-upstream-to=main"]);
    git(d.path(), &["rm", "-q", "--cached", "precious"]);
    git(d.path(), &["commit", "-qm", "untracks precious"]);
    fs::write(d.path().join("precious"), b"mine\n").unwrap();
    let tip = head("clobber");
    let err = done(r.branch_step(Op::Spinout, &s(&["clobbered"]), &[])).unwrap_err();
    assert!(err.contains("would be overwritten"), "{err}");
    assert_eq!(fs::read(d.path().join("precious")).unwrap(), b"mine\n");
    assert_eq!(head("clobber"), tip);
    assert!(!r.branch_choices().contains(&"clobbered".to_string()));
    fs::remove_file(d.path().join("precious")).unwrap();
    git(d.path(), &["checkout", "-q", "main"]);
    match r.branch_step(Op::Delete, &s(&["origin/feature"]), &[]) {
        Next::Ask(Q::Branch(op @ Op::DeleteRemote(_)), _, _) => {
            assert!(
                matches!(r.branch_step(op.clone(), &s(&["y"]), &[]), Next::Git(a) if a == ["push", "--delete", "origin", "refs/heads/feature"])
            );
            done(r.branch_step(op, &s(&["n"]), &[])).unwrap();
            assert!(!r.branch_choices().contains(&"origin/feature".to_string()));
        }
        _ => panic!("remote branches ask about the remote"),
    }
}
#[test]
fn tag_versions_sort_like_version_to_list() {
    use super::tag::version_list;
    assert_eq!(version_list("1.2.3"), Some(vec![1, 2, 3]));
    assert_eq!(version_list("1.0-rc.2"), Some(vec![1, 0, -1, 2]));
    assert_eq!(version_list("2.0alpha"), Some(vec![2, 0, -3]));
    assert_eq!(version_list("1.0-bogus"), None);
    // A lone separator ranks below a release (magit-tag-version-regexp-alist).
    assert_eq!(version_list("1.0-1"), Some(vec![1, 0, -4, 1]));
    assert_eq!(version_list("1-2"), Some(vec![1, -4, 2]));
    assert_eq!(version_list("1.2-a"), Some(vec![1, 2, 1]));
}
#[test]
fn tag_suffixes_create_release_delete_and_prune() {
    use super::Question as Q;
    use super::branch::Next;
    use super::tag::Op;
    let (d, r, bare) = with_remote();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let run = |n: Next| match n {
        Next::Git(args) => {
            let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            r.run(&args, None).map(|_| ())
        }
        Next::Done(res) => res.map(|_| ()),
        Next::Ask(q, p, _) => Err(format!("asked {q:?} {p:?}")),
        other => Err(format!("{other:?}")),
    };
    run(r
        .tag_step(Op::Create, &s(&["v1.0.0", "HEAD"]), &[])
        .unwrap())
    .unwrap();
    assert!(r.tag_step(Op::Create, &s(&["-x", "HEAD"]), &[]).is_err());
    assert!(r.tag_step(Op::Create, &s(&["ok", "--all"]), &[]).is_err());
    let annotate = s(&["--annotate", "--edit"]);
    assert!(
        r.tag_step(Op::Create, &s(&["v0", "HEAD", ""]), &annotate)
            .is_err()
    );
    run(r
        .tag_step(Op::Create, &s(&["v0.9", "HEAD", "Nine"]), &annotate)
        .unwrap())
    .unwrap();
    assert_eq!(git(d.path(), &["cat-file", "-t", "v0.9"]), b"tag\n");
    // Releases: highest version first; a "Release version" commit names the tag.
    assert_eq!(r.releases()[0].1, "v1.0.0");
    committed(d.path(), b"next\n");
    git(
        d.path(),
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Release version 1.1.0",
        ],
    );
    let (prompts, defaults) = r.tag_prompts(&Op::Release, &[], None).unwrap();
    assert!(prompts.is_empty());
    assert_eq!(defaults, ["v1.1.0"]);
    run(r.tag_step(Op::Release, &defaults, &[]).unwrap()).unwrap();
    assert_eq!(r.releases()[0].1, "v1.1.0");
    // Annotated release messages derive from the previous one.
    git(d.path(), &["tag", "-d", "v1.1.0"]);
    git(d.path(), &["tag", "-d", "v1.0.0"]);
    git(
        d.path(),
        &["tag", "-a", "-m", "Project 1.0.0", "v1.0.0", "HEAD~1"],
    );
    match r
        .tag_step(Op::Release, &s(&["v1.1.0"]), &s(&["--annotate"]))
        .unwrap()
    {
        Next::Ask(Q::Tag(Op::ReleaseMessage(t)), _, defaults) => {
            assert_eq!(
                (t.as_str(), defaults[0].as_str()),
                ("v1.1.0", "Project 1.1.0")
            )
        }
        _ => panic!("expected a message question"),
    }
    // -e alone still asks for a message on a later release.
    assert!(matches!(
        r.tag_step(Op::Release, &s(&["v1.2.0"]), &s(&["--edit"]))
            .unwrap(),
        Next::Ask(Q::Tag(Op::ReleaseMessage(_)), ..)
    ));
    // Option-like remote names are never offered.
    git(
        d.path(),
        &["config", "remote.--upload-pack=touch.url", "/nowhere"],
    );
    assert!(git(d.path(), &["remote"]).starts_with(b"--upload-pack"));
    assert!(!r.remotes().unwrap().iter().any(|x| x.starts_with('-')));
    assert!(
        r.tag_step(Op::Prune, &s(&["--upload-pack=touch"]), &[])
            .is_err()
    );
    git(
        d.path(),
        &["config", "--remove-section", "remote.--upload-pack=touch"],
    );
    // Delete and prune.
    run(r.tag_step(Op::Delete, &s(&["v0.9"]), &[]).unwrap()).unwrap();
    git(d.path(), &["push", "-q", "origin", "main", "v1.0.0"]);
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    git(other.path(), &["tag", "remote-only"]);
    git(other.path(), &["push", "-q", "origin", "remote-only"]);
    git(d.path(), &["tag", "local-only"]);
    let next = r.tag_step(Op::Prune, &s(&["origin"]), &[]).unwrap();
    let Next::Ask(Q::Tag(op @ Op::PruneLocal(..)), prompts, _) = next else {
        panic!("expected local prune question");
    };
    assert_eq!(prompts, ["Delete local-only locally? (y or n) "]);
    let next = r.tag_step(op, &s(&["y"]), &[]).unwrap();
    assert!(
        !git(d.path(), &["tag"])
            .windows(10)
            .any(|w| w == b"local-only")
    );
    let Next::Ask(Q::Tag(op @ Op::PruneRemote(..)), _, _) = next else {
        panic!("expected remote prune question");
    };
    run(r.tag_step(op, &s(&["y"]), &[]).unwrap()).unwrap();
    assert!(
        git(bare.path(), &["tag"])
            .windows(11)
            .all(|w| w != b"remote-only")
    );
}
#[test]
fn merge_suffixes_follow_magit_merge() {
    use super::Question as Q;
    use super::branch::Next;
    use super::merge::Op;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let run = |n: Next| match n {
        Next::Git(args) => {
            let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            r.run(&args, None).map(|_| ())
        }
        Next::Done(res) => res.map(|_| ()),
        _ => Err("unexpected step".into()),
    };
    let branch = |name: &str, file: &str| {
        git(d.path(), &["checkout", "-qb", name, "main"]);
        fs::write(d.path().join(file), name.as_bytes()).unwrap();
        git(d.path(), &["add", file]);
        git(d.path(), &["commit", "-qm", name]);
        git(d.path(), &["checkout", "-q", "main"]);
    };
    branch("a", "a");
    branch("b", "b");
    // Plain merge argv, octopus via commas, option-like answers rejected.
    match r
        .merge_step(Op::Plain, &s(&["a,b"]), &s(&["--no-ff"]))
        .unwrap()
    {
        Next::Git(argv) => assert_eq!(argv, ["merge", "--no-edit", "--no-ff", "--", "a", "b"]),
        _ => panic!("plain merge runs git"),
    }
    assert!(
        r.merge_step(Op::Plain, &s(&["--upload-pack=x"]), &[])
            .is_err()
    );
    // n drops --ff-only and adds --no-ff.
    match r
        .merge_step(Op::NoCommit, &s(&["a"]), &s(&["--ff-only"]))
        .unwrap()
    {
        Next::Git(argv) => assert_eq!(argv, ["merge", "--no-commit", "--no-ff", "--", "a"]),
        _ => panic!("no-commit merge runs git"),
    }
    // Dirty worktrees ask first (magit-merge-assert), and proceed on y.
    fs::write(d.path().join("f"), b"dirty\n").unwrap();
    let op = match r.merge_step(Op::Plain, &s(&["a"]), &[]).unwrap() {
        Next::Ask(Q::Merge(op @ Op::Dirty(..)), _, _) => op,
        other => panic!("{other:?}"),
    };
    assert!(r.merge_step(op.clone(), &s(&["n"]), &[]).is_err());
    assert!(matches!(
        r.merge_step(op, &s(&["y"]), &[]).unwrap(),
        Next::Git(_)
    ));
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // Preview needs one revision; dissolve and absorb need another local branch.
    assert!(r.merge_step(Op::Preview, &s(&["a,b"]), &[]).is_err());
    git(d.path(), &["tag", "v1"]);
    assert!(r.merge_step(Op::Dissolve, &s(&["v1"]), &[]).is_err());
    assert!(r.merge_step(Op::Dissolve, &s(&["main"]), &[]).is_err());
    assert!(r.merge_step(Op::Absorb, &s(&["main"]), &[]).is_err());
    assert_eq!(r.current_branch().unwrap(), "main");
    // Preview shows the merge result without touching the worktree.
    match r.merge_step(Op::Preview, &s(&["a"]), &[]).unwrap() {
        Next::Show(super::diff::Target::Range(range)) => {
            let out = r
                .diff_output(&super::diff::Target::Range(range), &[])
                .unwrap();
            assert!(String::from_utf8_lossy(&out).contains("+a"));
        }
        _ => panic!("preview shows a diff"),
    }
    assert!(!d.path().join("a").exists());
    // e merges without committing and opens a draft with MERGE_MSG.
    assert!(matches!(
        r.merge_step(Op::EditMsg, &s(&["main"]), &[]).unwrap(),
        Next::Done(Ok(m)) if m == "Already up to date"
    ));
    match r.merge_step(Op::EditMsg, &s(&["a"]), &[]).unwrap() {
        Next::Draft(msg) => assert!(String::from_utf8_lossy(&msg).contains("Merge branch 'a'")),
        _ => panic!("edit-message merge opens a draft"),
    }
    assert!(r.merge_in_progress());
    // Abort asks first.
    let (prompts, _) = r.merge_prompts(&Op::Abort).unwrap();
    assert_eq!(prompts, ["Abort merge? (y or n) "]);
    assert!(r.merge_step(Op::Abort, &s(&["n"]), &[]).is_err());
    run(r.merge_step(Op::Abort, &s(&["y"]), &[]).unwrap()).unwrap();
    assert!(!r.merge_in_progress());
    // Absorb merges and deletes the branch; squash leaves changes staged.
    run(r.merge_step(Op::Absorb, &s(&["a"]), &[]).unwrap()).unwrap();
    assert!(d.path().join("a").exists());
    assert!(!r.branch_choices().contains(&"a".to_string()));
    run(r.merge_step(Op::Squash, &s(&["b"]), &[]).unwrap()).unwrap();
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).starts_with(b"b"));
    git(d.path(), &["commit", "-qm", "squashed b"]);
    // Absorbing the main branch must be confirmed with yes.
    git(d.path(), &["checkout", "-qb", "side"]);
    match r.merge_step(Op::Absorb, &s(&["main"]), &[]).unwrap() {
        Next::Ask(Q::Merge(op @ Op::AbsorbMain(_)), _, _) => {
            assert!(r.merge_step(op, &s(&["y"]), &[]).is_err());
        }
        _ => panic!("absorbing main asks"),
    }
    // Dissolve merges the current branch into another and removes it.
    fs::write(d.path().join("side"), b"side").unwrap();
    git(d.path(), &["add", "side"]);
    git(d.path(), &["commit", "-qm", "side"]);
    run(r.merge_step(Op::Dissolve, &s(&["main"]), &[]).unwrap()).unwrap();
    assert_eq!(r.current_branch().unwrap(), "main");
    assert!(d.path().join("side").exists());
    assert!(!r.branch_choices().contains(&"side".to_string()));
}
#[test]
fn reset_suffixes_move_head_index_and_worktree_as_named() {
    use super::reset::Op;
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    committed(d.path(), b"two\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let head = || git(d.path(), &["rev-parse", "HEAD"]);
    let first = git(d.path(), &["rev-parse", "HEAD~1"]);
    let staged = || git(d.path(), &["show", ":f"]);
    let disk = || fs::read(d.path().join("f")).unwrap();
    assert!(r.reset_step(Op::Mixed, &s(&["--hard"])).is_err());
    assert!(r.reset_step(Op::Mixed, &s(&["nope"])).is_err());
    // soft: HEAD only.
    r.reset_step(Op::Soft, &s(&["HEAD~1"])).unwrap();
    assert_eq!(
        (head(), staged(), disk()),
        (first.clone(), b"two\n".to_vec(), b"two\n".to_vec())
    );
    git(d.path(), &["commit", "-qm", "again"]);
    // mixed: HEAD and index.
    r.reset_step(Op::Mixed, &s(&["HEAD~1"])).unwrap();
    assert_eq!(
        (head(), staged(), disk()),
        (first.clone(), b"one\n".to_vec(), b"two\n".to_vec())
    );
    git(d.path(), &["commit", "-qam", "again"]);
    // index only; then worktree only.
    r.reset_step(Op::Index, &s(&["HEAD~1"])).unwrap();
    assert_eq!((staged(), disk()), (b"one\n".to_vec(), b"two\n".to_vec()));
    git(d.path(), &["reset", "-q"]);
    r.reset_step(Op::Worktree, &s(&["HEAD~1"])).unwrap();
    assert_eq!((staged(), disk()), (b"two\n".to_vec(), b"one\n".to_vec()));
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // keep refuses to lose local changes to files it would touch.
    fs::write(d.path().join("f"), b"local\n").unwrap();
    assert!(r.reset_step(Op::Keep, &s(&["HEAD~1"])).is_err());
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // hard refuses to replace an untracked file the target tracks.
    git(d.path(), &["rm", "-q", "--cached", "f"]);
    git(d.path(), &["commit", "-qm", "untrack f"]);
    fs::write(d.path().join("f"), b"precious\n").unwrap();
    assert!(
        r.reset_step(Op::Hard, &s(&["HEAD~1"]))
            .unwrap_err()
            .contains("overwritten")
    );
    assert_eq!(disk(), b"precious\n");
    fs::remove_file(d.path().join("f")).unwrap();
    r.reset_step(Op::Hard, &s(&["HEAD~1"])).unwrap();
    assert_eq!(disk(), b"two\n");
}
#[test]
fn remote_suffixes_add_rename_remove_and_prune_refspecs() {
    use super::Question as Q;
    use super::branch::Next;
    use super::remote::Op;
    let (d, r, bare) = with_remote();
    git(d.path(), &["push", "-q", "origin", "main"]);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let ok = |n: Next| match n {
        Next::Done(res) => res,
        other => Err(format!("{other:?}")),
    };
    git(
        d.path(),
        &[
            "config",
            "remote.origin.url",
            "https://example.test/owner/repo.git",
        ],
    );
    assert_eq!(
        r.suggested_url("fork"),
        "https://example.test/fork/repo.git"
    );
    git(
        d.path(),
        &["config", "remote.origin.url", bare.path().to_str().unwrap()],
    );
    // Add asks for the url with the suggestion as a visible default, then about
    // remote.pushDefault when unset; -f fetches through the terminal.
    let url = bare.path().to_str().unwrap();
    match r.remote_step(Op::Add, &s(&["fork"]), &[]).unwrap() {
        Next::Ask(Q::Remote(Op::AddUrl(n)), p, d) => {
            assert_eq!(n, "fork");
            assert!(p[0].contains(&d[0]), "{p:?} {d:?}");
        }
        other => panic!("{other:?}"),
    }
    let op = match r
        .remote_step(Op::AddUrl("fork".into()), &s(&[url]), &s(&["-f"]))
        .unwrap()
    {
        Next::Ask(Q::Remote(op @ Op::AddPushDefault(..)), _, _) => op,
        other => panic!("{other:?}"),
    };
    match r.remote_step(op, &s(&["y"]), &s(&["-f"])).unwrap() {
        Next::Git(argv) => assert_eq!(argv, ["fetch", "fork"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(git(d.path(), &["config", "remote.pushDefault"]), b"fork\n");
    assert!(r.remote_step(Op::Add, &s(&["--x"]), &[]).is_err());
    assert!(r.remote_step(Op::Add, &s(&["fork"]), &[]).is_err());
    assert!(
        r.remote_step(Op::AddUrl("x".into()), &s(&["--upload-pack=y"]), &[])
            .is_err()
    );
    // Rename and remove carry or clean the push variables.
    git(d.path(), &["config", "branch.main.pushRemote", "fork"]);
    ok(r.remote_step(Op::Rename, &s(&["fork", "mine"]), &[])
        .unwrap())
    .unwrap();
    assert_eq!(git(d.path(), &["config", "remote.pushDefault"]), b"mine\n");
    assert_eq!(
        git(d.path(), &["config", "branch.main.pushRemote"]),
        b"mine\n"
    );
    ok(r.remote_step(Op::Remove, &s(&["mine"]), &[]).unwrap()).unwrap();
    assert!(r.read(&["config", "remote.pushDefault"]).is_err());
    assert!(r.read(&["config", "branch.main.pushRemote"]).is_err());
    assert!(r.remote_step(Op::Remove, &s(&["nope"]), &[]).is_err());
    // A refspec for a branch the remote no longer has is stale.
    git(
        d.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/gone:refs/remotes/origin/gone",
        ],
    );
    git(
        d.path(),
        &["update-ref", "refs/remotes/origin/gone", "HEAD"],
    );
    let op = match r
        .remote_step(Op::PruneRefspecs, &s(&["origin"]), &[])
        .unwrap()
    {
        Next::Ask(Q::Remote(op @ Op::PruneStale(..)), p, _) => {
            assert!(p[0].contains("refs/heads/gone"), "{p:?}");
            op
        }
        other => panic!("{other:?}"),
    };
    ok(r.remote_step(op, &s(&["y"]), &[]).unwrap()).unwrap();
    assert!(
        r.read(&["rev-parse", "--verify", "-q", "refs/remotes/origin/gone"])
            .is_err()
    );
    let fetch = git(d.path(), &["config", "--get-all", "remote.origin.fetch"]);
    assert_eq!(fetch, b"+refs/heads/*:refs/remotes/origin/*\n");
    assert!(matches!(
        r.remote_step(Op::PruneRefspecs, &s(&["origin"]), &[]).unwrap(),
        Next::Done(Ok(m)) if m.contains("No stale refspecs")
    ));
}
#[test]
fn reset_guards_directory_file_conflicts_and_defaults_to_current_branch() {
    use super::reset::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // Target tracks a file `a`; worktree has an untracked directory a/ with content.
    fs::write(d.path().join("a"), b"file\n").unwrap();
    git(d.path(), &["add", "a"]);
    git(d.path(), &["commit", "-qm", "file a"]);
    git(d.path(), &["rm", "-q", "a"]);
    git(d.path(), &["commit", "-qm", "drop a"]);
    fs::create_dir(d.path().join("a")).unwrap();
    fs::write(d.path().join("a/precious"), b"keep\n").unwrap();
    assert!(r.reset_step(Op::Hard, &s(&["HEAD~1"])).is_err());
    assert!(r.reset_step(Op::Worktree, &s(&["HEAD~1"])).is_err());
    assert_eq!(fs::read(d.path().join("a/precious")).unwrap(), b"keep\n");
    fs::remove_dir_all(d.path().join("a")).unwrap();
    // Target tracks b/f; worktree has an untracked file `b`.
    fs::create_dir(d.path().join("b")).unwrap();
    fs::write(d.path().join("b/f"), b"tracked\n").unwrap();
    git(d.path(), &["add", "b/f"]);
    git(d.path(), &["commit", "-qm", "b/f"]);
    git(d.path(), &["rm", "-qr", "b"]);
    git(d.path(), &["commit", "-qm", "drop b"]);
    fs::write(d.path().join("b"), b"mine\n").unwrap();
    assert!(r.reset_step(Op::Hard, &s(&["HEAD~1"])).is_err());
    assert_eq!(fs::read(d.path().join("b")).unwrap(), b"mine\n");
    let (_, defaults) = r.reset_prompt(Op::Hard, None);
    assert_eq!(defaults, ["main"]);
}
#[test]
fn cherry_pick_and_revert_suffixes_follow_magit_sequence() {
    use super::Question as Q;
    use super::branch::Next;
    use super::sequence::Op;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let run = |n: Next| match n {
        Next::Git(args) => {
            let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            r.run(&args, None).map(|_| ())
        }
        Next::Done(res) => res.map(|_| ()),
        other => Err(format!("{other:?}")),
    };
    let commit = |file: &str| {
        fs::write(d.path().join(file), file.as_bytes()).unwrap();
        git(d.path(), &["add", file]);
        git(d.path(), &["commit", "-qm", file]);
        String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned()
    };
    git(d.path(), &["checkout", "-qb", "topic"]);
    let a = commit("a");
    let b = commit("b");
    git(d.path(), &["checkout", "-q", "main"]);
    // A: pick with -x; option-like answers are rejected.
    match r.sequence_step(Op::Pick, &s(&[&a]), &s(&["-x"])).unwrap() {
        Next::Git(argv) => {
            assert_eq!(argv[..3], ["cherry-pick", "-x", "--end-of-options"]);
            run(Next::Git(argv)).unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert!(d.path().join("a").exists());
    assert!(r.sequence_step(Op::Pick, &s(&["--all"]), &[]).is_err());
    // a: apply without committing drops --ff.
    match r
        .sequence_step(Op::Apply, &s(&[&b]), &s(&["--ff"]))
        .unwrap()
    {
        Next::Git(argv) => assert_eq!(
            argv[..3],
            ["cherry-pick", "--no-commit", "--end-of-options"]
        ),
        other => panic!("{other:?}"),
    }
    // V with --edit lets Git run Fred as its editor (keeps authorship).
    match r
        .sequence_step(Op::Revert, &s(&["HEAD"]), &s(&["--edit"]))
        .unwrap()
    {
        Next::GitEditor(argv) => assert_eq!(argv[..2], ["revert", "--edit"]),
        other => panic!("{other:?}"),
    }
    // Reverting a range goes newest first, like git revert A..B.
    match r
        .sequence_step(Op::Revert, &s(&["main~1..main"]), &s(&["--no-edit"]))
        .unwrap()
    {
        Next::Git(argv) => assert_eq!(argv.len(), 4, "{argv:?}"),
        other => panic!("{other:?}"),
    }
    // Abort needs a running sequence and asks first.
    assert!(r.sequence_step(Op::Abort, &s(&["y"]), &[]).is_err());
    // Merge commits ask for a mainline.
    git(
        d.path(),
        &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"],
    );
    match r
        .sequence_step(Op::Revert, &s(&["HEAD"]), &s(&["--no-edit"]))
        .unwrap()
    {
        Next::Ask(Q::Sequence(op @ Op::Mainline(..)), _, defaults) => {
            assert_eq!(defaults, ["1"]);
            assert!(r.sequence_step(op.clone(), &s(&["0"]), &[]).is_err());
            match r.sequence_step(op, &s(&["1"]), &s(&["--no-edit"])).unwrap() {
                Next::Git(argv) => assert!(argv.contains(&"--mainline=1".to_string())),
                other => panic!("{other:?}"),
            }
        }
        other => panic!("{other:?}"),
    }
    git(d.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    // d: donate the tip commit to another branch; main drops it.
    let c = commit("c");
    git(d.path(), &["branch", "dest", "HEAD~1"]);
    match r.sequence_step(Op::Donate, &s(&[&c]), &[]).unwrap() {
        Next::Ask(Q::Sequence(op @ Op::DonateTo(_)), _, _) => {
            run(r.sequence_step(op, &s(&["dest"]), &[]).unwrap()).unwrap()
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(r.current_branch().unwrap(), "main");
    assert!(!d.path().join("c").exists());
    assert!(git(d.path(), &["show", "dest:c"]) == b"c");
    // n: spin out a commit that is not the tip; later commits are kept.
    let e = commit("e");
    let _f = commit("f");
    match r.sequence_step(Op::Spinout, &s(&[&e]), &[]).unwrap() {
        Next::Ask(Q::Sequence(op @ Op::NewBranch(..)), _, _) => {
            run(r.sequence_step(op, &s(&["side", "HEAD~2"]), &[]).unwrap()).unwrap()
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(r.current_branch().unwrap(), "main");
    assert!(!d.path().join("e").exists() && d.path().join("f").exists());
    assert!(git(d.path(), &["show", "side:e"]) == b"e");
    // Moving cherries away requires them to be reachable from HEAD.
    assert!(r.sequence_step(Op::Spinoff, &s(&[&b]), &[]).is_err());
    // Harvesting names a branch that contains the cherries.
    git(d.path(), &["branch", "unrelated", "main"]);
    assert!(
        r.sequence_step(Op::HarvestFrom(b.clone()), &s(&["unrelated"]), &[])
            .unwrap_err()
            .contains("does not contain")
    );
    // Moving cherries refuses a dirty worktree before changing anything.
    fs::write(d.path().join("f"), b"dirty\n").unwrap();
    git(d.path(), &["branch", "dest2", "HEAD~1"]);
    let tip = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"])).unwrap();
    assert!(
        r.sequence_step(Op::DonateTo(tip.trim().to_owned()), &s(&["dest2"]), &[])
            .is_err()
    );
    assert_eq!(r.current_branch().unwrap(), "main");
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // h: harvest from a branch onto the current one.
    match r.sequence_step(Op::Harvest, &s(&[&b]), &[]).unwrap() {
        Next::Done(Ok(_)) => (),
        other => panic!("{other:?}"),
    }
    assert!(d.path().join("b").exists());
    assert_eq!(r.current_branch().unwrap(), "main");
}
#[test]
fn rebase_captures_and_replays_todo_lists() {
    use super::Question as Q;
    use super::branch::Next;
    use super::rebase::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let commit = |file: &str| {
        fs::write(d.path().join(file), file.as_bytes()).unwrap();
        git(d.path(), &["add", file]);
        git(d.path(), &["commit", "-qm", file]);
        String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned()
    };
    let one = commit("one");
    let two = commit("two");
    let _three = commit("three");
    let run = |inv: super::repo::GitInvocation| {
        inv.validate().unwrap();
        r.run(&inv.args, None).unwrap();
    };
    // i: capture Git's todo without starting a rebase; autostash survives.
    fs::write(d.path().join("one"), b"dirty").unwrap();
    let plan = match r
        .rebase_step(Op::Interactive, &s(&[&two]), &s(&["--autostash"]))
        .unwrap()
    {
        Next::Todo(plan) => plan,
        other => panic!("{other:?}"),
    };
    assert!(!r.rebase_in_progress());
    assert_eq!(fs::read(d.path().join("one")).unwrap(), b"dirty");
    let todo = fs::read_to_string(&plan.todo).unwrap();

    // A staged change survives the capture (autostash restores only the worktree).
    fs::write(d.path().join("three"), b"staged").unwrap();
    git(d.path(), &["add", "three"]);
    let again = match r
        .rebase_step(Op::Interactive, &s(&[&two]), &s(&["--autostash"]))
        .unwrap()
    {
        Next::Todo(p) => p,
        other => panic!("{other:?}"),
    };
    assert_ne!(again.todo, plan.todo, "each capture gets its own file");
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).starts_with(b"three"));
    git(d.path(), &["reset", "-q", "--hard"]);
    assert!(
        todo.starts_with("pick ") && todo.contains("# two") && todo.contains("# three"),
        "{todo}"
    );
    git(d.path(), &["checkout", "-q", "--", "one"]);
    // Replay with "two" dropped.
    let edited = todo.replacen("pick", "drop", 1);
    fs::write(&plan.todo, edited).unwrap();
    run(plan.replay().unwrap());
    assert!(!d.path().join("two").exists() && d.path().join("three").exists());
    // A plan whose HEAD moved refuses to replay.
    let plan = match r.rebase_step(Op::Interactive, &s(&[&one]), &[]).unwrap() {
        Next::Todo(plan) => plan,
        other => panic!("{other:?}"),
    };
    commit("four");
    assert!(plan.replay().is_err());
    // k: remove a commit; the root commit uses --root.
    match r
        .rebase_step(Op::RemoveCommit, &s(&["HEAD~1"]), &[])
        .unwrap()
    {
        Next::Replay(plan) => run(plan.replay().unwrap()),
        other => panic!("{other:?}"),
    }
    assert!(!d.path().join("three").exists() && d.path().join("four").exists());
    match r.rebase_step(Op::Interactive, &s(&[&one]), &[]).unwrap() {
        Next::Todo(plan) => assert_eq!(plan.base, ["--root"]),
        other => panic!("{other:?}"),
    }
    // m: stop at a commit; continue goes through Git with Fred as editor.
    match r.rebase_step(Op::EditCommit, &s(&["HEAD~1"]), &[]).unwrap() {
        Next::Replay(plan) => run(plan.replay().unwrap()),
        other => panic!("{other:?}"),
    }
    assert!(r.rebase_in_progress());
    assert!(matches!(
        r.rebase_step(Op::Continue, &[], &[]).unwrap(),
        Next::GitEditor(a) if a == ["rebase", "--continue"]
    ));
    assert!(r.rebase_step(Op::Abort, &s(&["n"]), &[]).is_err());
    match r.rebase_step(Op::Abort, &s(&["y"]), &[]).unwrap() {
        Next::Git(argv) => r
            .run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
            .map(|_| ())
            .unwrap(),
        other => panic!("{other:?}"),
    }
    assert!(!r.rebase_in_progress());
    // Published commits ask first.
    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "--bare", "-q", "-b", "main"]);
    git(
        d.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    // magit-published-branches defaults to origin/master only.
    git(d.path(), &["push", "-qu", "origin", "main"]);
    git(d.path(), &["push", "-q", "origin", "main:master"]);
    git(d.path(), &["fetch", "-q", "origin"]);
    match r.rebase_step(Op::RewordCommit, &s(&["HEAD"]), &[]).unwrap() {
        Next::Ask(Q::Rebase(op @ Op::Published(..)), p, _) => {
            assert!(p[0].contains("origin/master"));
            assert!(r.rebase_step(op, &s(&["n"]), &[]).is_err());
        }
        other => panic!("{other:?}"),
    }
    // Non-interactive rebases run in the terminal with their arguments.
    match r
        .rebase_step(Op::Elsewhere, &s(&["HEAD~1"]), &s(&["--autostash"]))
        .unwrap()
    {
        Next::GitEditor(argv) => assert_eq!(
            argv,
            ["rebase", "--autostash", "--end-of-options", "HEAD~1"]
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        r.rebase_step(Op::Elsewhere, &s(&["--exec=x"]), &[])
            .is_err()
    );
}
#[test]
fn commit_fixup_family_follows_magit_commit() {
    use super::Question as Q;
    use super::branch::Next;
    use super::commit::Op;
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    let target = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    committed(d.path(), b"two\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // Nothing staged at all.
    assert!(r.commit_step(Op::Fixup, &s(&[&target]), &[]).is_err());
    // Unstaged only: ask to commit everything.
    fs::write(d.path().join("f"), b"three\n").unwrap();
    let op = match r.commit_step(Op::Fixup, &s(&[&target]), &[]).unwrap() {
        Next::Ask(Q::Commit(op @ Op::StageAll(..)), _, _) => op,
        other => panic!("{other:?}"),
    };
    match r.commit_step(op, &s(&["y"]), &[]).unwrap() {
        Next::GitEditor(argv) => {
            assert_eq!(
                argv,
                ["commit", "--all", &format!("--fixup={target}"), "--no-edit"]
            );
            r.run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
                .unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert!(
        String::from_utf8(git(d.path(), &["log", "-1", "--format=%s"]))
            .unwrap()
            .starts_with("fixup! initial")
    );
    // Revise needs no patch and edits the message.
    match r.commit_step(Op::Revise, &s(&[&target]), &[]).unwrap() {
        Next::GitEditor(argv) => assert_eq!(
            argv[1..],
            [format!("--fixup=reword:{target}"), "--edit".into()]
        ),
        other => panic!("{other:?}"),
    }
    // Instant fixup commits and then autosquashes into the target.
    fs::write(d.path().join("g"), b"g1\n").unwrap();
    git(d.path(), &["add", "g"]);
    git(d.path(), &["commit", "-qm", "adds g"]);
    let target = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    fs::write(d.path().join("g"), b"g2\n").unwrap();
    git(d.path(), &["add", "g"]);
    let before = git(d.path(), &["rev-list", "--count", "HEAD"]);
    match r
        .commit_step(Op::InstantFixup, &s(&[&target]), &[])
        .unwrap()
    {
        Next::GitEditor(argv) => {
            assert!(argv.contains(&"--autosquash".to_string()));
            r.run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
                .unwrap();
        }
        other => panic!("{other:?}"),
    }
    // The new fixup was folded in, so the commit count did not grow.
    assert_eq!(git(d.path(), &["rev-list", "--count", "HEAD"]), before);
    assert!(r.commit_step(Op::Squash, &s(&["--all"]), &[]).is_err());
    // Instant variants refuse during another operation and ask about merges.
    let target = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    git(d.path(), &["checkout", "-qb", "side"]);
    fs::write(d.path().join("side"), b"s").unwrap();
    git(d.path(), &["add", "side"]);
    git(d.path(), &["commit", "-qm", "side"]);
    git(d.path(), &["checkout", "-q", "main"]);
    git(
        d.path(),
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
    );
    fs::write(d.path().join("g"), b"g3\n").unwrap();
    git(d.path(), &["add", "g"]);
    match r
        .commit_step(Op::InstantFixup, &s(&[&target]), &[])
        .unwrap()
    {
        Next::Ask(Q::Commit(op @ Op::Merges(..)), _, _) => {
            assert!(r.commit_step(op, &s(&["n"]), &[]).is_err());
        }
        other => panic!("{other:?}"),
    }
    let merge_parents =
        String::from_utf8(git(d.path(), &["rev-list", "--parents", "-1", "HEAD"])).unwrap();
    assert_eq!(merge_parents.split_whitespace().count(), 3, "merge intact");
}
#[test]
fn stash_transforms_branch_patch_and_clear() {
    use super::branch::Next;
    use super::stash::Op;
    let (d, r) = setup();
    committed(d.path(), b"base\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let ok = |n: Next| match n {
        Next::Done(res) => res,
        other => Err(format!("{other:?}")),
    };
    fs::write(d.path().join("f"), b"stashed\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "parked"]);
    committed(d.path(), b"moved on\n");
    // b: branch from where the stash was made, dropping it when clean.
    ok(r.stash_step(Op::Branch, &s(&["stash@{0}", "from-stash"]))
        .unwrap())
    .unwrap();
    assert_eq!(r.current_branch().unwrap(), "from-stash");
    assert_eq!(fs::read(d.path().join("f")).unwrap(), b"stashed\n");
    assert!(r.stashes().unwrap().is_empty());
    git(d.path(), &["checkout", "-q", "--", "f"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("f"), b"again\n").unwrap();
    git(d.path(), &["stash", "push", "-qm", "again"]);
    // B: branch here and apply, keeping the stash.
    assert!(
        r.stash_step(Op::BranchHere, &s(&["stash@{0}", "-x"]))
            .is_err()
    );
    ok(r.stash_step(Op::BranchHere, &s(&["stash@{0}", "here"]))
        .unwrap())
    .unwrap();
    assert_eq!(r.current_branch().unwrap(), "here");
    assert_eq!(r.stashes().unwrap().len(), 1);
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // f: patch file named like format-patch.
    ok(r.stash_step(Op::FormatPatch, &s(&["stash@{0}"])).unwrap()).unwrap();
    let patch = fs::read_dir(d.path())
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().ends_with(".patch"))
        .expect("patch file");
    assert!(fs::read_to_string(patch.path()).unwrap().contains("+again"));
    // Clear asks first, then drops every stash.
    assert!(r.stash_step(Op::Clear, &s(&["n"])).is_err());
    ok(r.stash_step(Op::Clear, &s(&["y"])).unwrap()).unwrap();
    assert!(r.stashes().unwrap().is_empty());
    assert!(r.stash_step(Op::FormatPatch, &s(&["stash@{0}"])).is_err());
}
#[test]
fn worktree_suffixes_create_move_delete_and_visit() {
    use super::Question as Q;
    use super::branch::Next;
    use super::worktree::Op;
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("proj");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    committed(&root, b"base\n");
    let r = Repo::discover(&root).unwrap();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // b: sibling default "<prefix>_<commit>".
    let dir = match r.worktree_step(Op::Checkout, &s(&["HEAD", ""])) {
        Ok(Next::Status(dir)) => dir,
        other => panic!("{other:?}"),
    };
    assert_eq!(dir.file_name().unwrap(), "proj_HEAD");
    assert!(dir.join("f").exists());
    // c: new branch in a named directory.
    let named = parent.path().join("feature-dir");
    match r.worktree_step(
        Op::Branch,
        &s(&["feature", "main", named.to_str().unwrap()]),
    ) {
        Ok(Next::Status(d)) => assert_eq!(d, named),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.worktrees().unwrap().len(), 3);
    assert!(
        r.worktree_step(Op::Branch, &s(&["--x", "main", ""]))
            .is_err()
    );
    // The main worktree can be neither moved nor deleted.
    assert!(
        r.worktree_step(Op::Move, &s(&[root.to_str().unwrap(), "/tmp/x"]))
            .is_err()
    );
    assert!(
        r.worktree_step(Op::Delete, &s(&[root.to_str().unwrap()]))
            .is_err()
    );
    // m: move a linked worktree.
    let moved = parent.path().join("moved");
    r.worktree_step(
        Op::Move,
        &s(&[named.to_str().unwrap(), moved.to_str().unwrap()]),
    )
    .unwrap();
    assert!(moved.join("f").exists() && !named.exists());
    // k: a dirty worktree needs a typed yes; a clean one takes y.
    fs::write(moved.join("f"), b"dirty\n").unwrap();
    let op = match r
        .worktree_step(Op::Delete, &s(&[moved.to_str().unwrap()]))
        .unwrap()
    {
        Next::Ask(Q::Worktree(op), p, _) => {
            assert!(p[0].contains("despite uncommitted changes"), "{p:?}");
            op
        }
        other => panic!("{other:?}"),
    };
    assert!(r.worktree_step(op.clone(), &s(&["y"])).is_err());
    assert!(moved.exists());
    r.worktree_step(op, &s(&["yes"])).unwrap();
    assert!(!moved.exists());
    assert_eq!(r.worktrees().unwrap().len(), 2);
    // Delete has no default; deleting the worktree you are in reopens the primary.
    let (prompts, defaults) = r.worktree_prompts(&Op::Delete, None);
    assert_eq!(
        (prompts[0].as_str(), defaults[0].as_str()),
        ("Delete worktree: ", "")
    );
    let inside = Repo::discover(&dir).unwrap();
    let op = match inside
        .worktree_step(Op::Delete, &s(&[dir.to_str().unwrap()]))
        .unwrap()
    {
        Next::Ask(Q::Worktree(op), _, _) => op,
        other => panic!("{other:?}"),
    };
    match inside.worktree_step(op, &s(&["y"])).unwrap() {
        Next::Status(p) => assert_eq!(
            fs::canonicalize(p).unwrap(),
            fs::canonicalize(&root).unwrap()
        ),
        other => panic!("{other:?}"),
    }
    assert!(!dir.exists());
    let dir = match r.worktree_step(Op::Checkout, &s(&["HEAD", ""])) {
        Ok(Next::Status(dir)) => dir,
        other => panic!("{other:?}"),
    };
    // g: visit an existing worktree only.
    assert!(matches!(
        r.worktree_step(Op::Visit, &s(&[dir.to_str().unwrap()])),
        Ok(Next::Status(_))
    ));
    assert!(r.worktree_step(Op::Visit, &s(&["/nowhere"])).is_err());
}
#[test]
fn notes_suffixes_configure_edit_remove_merge_and_prune() {
    use super::branch::Next;
    use super::notes::Op;
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // Local config: short names live under refs/notes/; empty unsets.
    r.notes_step(Op::NotesRef(false), &s(&["review"]), &[])
        .unwrap();
    assert_eq!(
        git(d.path(), &["config", "core.notesRef"]),
        b"refs/notes/review\n"
    );
    r.notes_step(Op::DisplayRef(false), &s(&["a, refs/notes/b"]), &[])
        .unwrap();
    assert_eq!(
        git(d.path(), &["config", "--get-all", "notes.displayRef"]),
        b"refs/notes/a\nrefs/notes/b\n"
    );
    r.notes_step(Op::NotesRef(false), &s(&[""]), &[]).unwrap();
    assert!(r.read(&["config", "core.notesRef"]).is_err());
    assert!(r.notes_step(Op::NotesRef(false), &s(&["-x"]), &[]).is_err());
    // Edit runs Git with Fred as the editor; remove/prune/merge build argv.
    match r.notes_step(Op::Edit, &s(&["HEAD"]), &[]).unwrap() {
        Next::GitEditor(argv) => assert_eq!(argv, ["notes", "edit", "--end-of-options", "HEAD"]),
        other => panic!("{other:?}"),
    }
    git(d.path(), &["notes", "add", "-m", "hello", "HEAD"]);
    match r.notes_step(Op::Remove, &s(&["HEAD"]), &[]).unwrap() {
        Next::Git(argv) => {
            r.run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
                .unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert!(r.read(&["notes", "show", "HEAD"]).is_err());
    match r
        .notes_step(Op::Merge, &s(&["other"]), &s(&["--strategy=union"]))
        .unwrap()
    {
        Next::GitEditor(argv) => assert_eq!(
            argv,
            ["notes", "merge", "--strategy=union", "refs/notes/other"]
        ),
        other => panic!("{other:?}"),
    }
    match r.notes_step(Op::Prune, &[], &s(&["--dry-run"])).unwrap() {
        Next::Git(argv) => assert_eq!(argv, ["notes", "prune", "--dry-run"]),
        other => panic!("{other:?}"),
    }
    assert!(!r.notes_merging());
}
#[test]
fn bisect_finds_the_bad_commit_and_runs_scripts() {
    use super::bisect::Op;
    use super::branch::Next;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let run = |n: Next| match n {
        Next::Git(argv) => r
            .run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
            .map(|_| ()),
        Next::Done(res) => res.map(|_| ()),
        other => Err(format!("{other:?}")),
    };
    for i in 0..6 {
        let text = if i >= 4 { "bug\n" } else { "fine\n" };
        fs::write(d.path().join("f"), text).unwrap();
        fs::write(d.path().join("n"), format!("{i}")).unwrap();
        git(d.path(), &["add", "f", "n"]);
        git(d.path(), &["commit", "-qm", &format!("c{i}")]);
    }
    let first_bad = String::from_utf8(git(d.path(), &["rev-parse", "HEAD~1"])).unwrap();
    assert!(r.bisect_step(Op::Good, &[], &[]).is_err());
    fs::write(d.path().join("f"), b"dirty").unwrap();
    assert!(
        r.bisect_step(Op::Start, &s(&["HEAD", "HEAD~5"]), &[])
            .is_err()
    );
    git(d.path(), &["checkout", "-q", "--", "f"]);
    run(r
        .bisect_step(Op::Start, &s(&["HEAD", "HEAD~5"]), &[])
        .unwrap())
    .unwrap();
    assert!(r.bisecting());
    assert_eq!(r.bisect_terms().unwrap(), ("bad".into(), "good".into()));
    // Mark by inspecting the file at each step.
    for _ in 0..5 {
        let bug = fs::read(d.path().join("f")).unwrap() == b"bug\n";
        let op = if bug { Op::Bad } else { Op::Good };
        let step = run(r.bisect_step(op, &[], &[]).unwrap());
        if step.is_err() {
            break;
        }
        let log = String::from_utf8(git(d.path(), &["bisect", "log"])).unwrap();
        if log.contains("first 'bad' commit") {
            break;
        }
    }
    let log = String::from_utf8(git(d.path(), &["bisect", "log"])).unwrap();
    assert!(
        log.contains(&format!("first 'bad' commit: [{}", first_bad.trim())),
        "{log}"
    );
    assert!(r.bisect_step(Op::Reset, &s(&["n"]), &[]).is_err());
    run(r.bisect_step(Op::Reset, &s(&["y"]), &[]).unwrap()).unwrap();
    assert!(!r.bisecting());
    // s: start and run a script in one go.
    match r
        .bisect_step(Op::Run, &s(&["! grep -q bug f", "HEAD", "HEAD~5"]), &[])
        .unwrap()
    {
        Next::Git(argv) => {
            assert_eq!(argv[..4], ["bisect", "run", "sh", "-c"]);
            let out = r
                .run(&argv.into_iter().map(Into::into).collect::<Vec<_>>(), None)
                .unwrap();
            assert!(String::from_utf8_lossy(&out).contains(first_bad.trim()));
        }
        other => panic!("{other:?}"),
    }
    run(r.bisect_step(Op::Reset, &s(&["y"]), &[]).unwrap()).unwrap();
}

#[test]
fn log_variants_arguments_and_merged() {
    use super::branch::Next;
    use super::log::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let commit = |name: &str| {
        fs::write(d.path().join(name), name).unwrap();
        git(d.path(), &["add", name]);
        git(d.path(), &["commit", "-qm", name]);
    };
    commit("base");
    let main = String::from_utf8(git(d.path(), &["symbolic-ref", "--short", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    git(d.path(), &["checkout", "-qb", "topic"]);
    commit("feature");
    let feature = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    git(d.path(), &["checkout", "-q", &main]);
    commit("mainline");
    git(
        d.path(),
        &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"],
    );
    commit("after");
    git(d.path(), &["tag", "v1", "HEAD~1"]);
    let lines = |n: Next| match n {
        Next::View(super::Kind::Log(revs, args)) => (
            revs.clone(),
            r.log_lines(&revs, &args, &[])
                .unwrap()
                .into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        other => panic!("{other:?}"),
    };
    let args = s(&["-n256", "--graph", "--decorate"]);
    let (revs, text) = lines(r.log_step(Op::Current, &[], &args).unwrap());
    assert_eq!(revs, vec![main.clone()]);
    assert!(text.contains("* ") && text.contains("tag: v1"), "{text}");
    // Limits, message search and revisions read from the minibuffer.
    let (_, text) = lines(r.log_step(Op::Head, &[], &s(&["-n1"])).unwrap());
    assert_eq!(text.lines().count(), 1);
    let (_, text) = lines(
        r.log_step(Op::Other, &s(&["topic"]), &s(&["--grep=feat"]))
            .unwrap(),
    );
    assert!(text.contains("feature") && !text.contains("base"), "{text}");
    assert!(r.log_step(Op::Other, &s(&["--all"]), &[]).is_err());
    assert!(r.log_lines(&s(&["HEAD"]), &s(&["-nx"]), &[]).is_err());
    // --reverse drops --graph (git refuses both).
    assert!(
        r.log_lines(&s(&["HEAD"]), &s(&["--graph", "--reverse"]), &[])
            .is_ok()
    );
    // Patch text containing the record separator is not a commit line.
    fs::write(d.path().join("sep"), "\x1ea\u{e9}\u{e9}\u{e9}\u{e9}\n").unwrap();
    git(d.path(), &["add", "sep"]);
    git(d.path(), &["commit", "-qm", "sep"]);
    let patched = r
        .log_lines(&s(&["HEAD"]), &s(&["-n1", "--patch"]), &[])
        .unwrap();
    assert_eq!(patched.iter().filter(|l| l.commit.is_some()).count(), 1);
    git(d.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    let (revs, _) = lines(r.log_step(Op::MatchingTags, &s(&["v*"]), &[]).unwrap());
    assert_eq!(revs, s(&["HEAD", "--tags=v*"]));
    let (revs, _) = lines(r.log_step(Op::LocalBranches, &[], &[]).unwrap());
    assert_eq!(revs, s(&["--branches"]));
    // magit-log-merged: the merge that brought the commit in, as M^1..M.
    let (revs, text) = lines(
        r.log_step(Op::Merged, &[feature.clone(), main.clone()], &[])
            .unwrap(),
    );
    let merge = String::from_utf8(git(d.path(), &["rev-parse", "HEAD~1"]))
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(revs, vec![format!("{merge}^1..{merge}")]);
    assert!(
        text.contains("feature") && text.contains("merge topic"),
        "{text}"
    );
    assert!(!text.contains("mainline"), "{text}");
    // A commit directly on the branch shows its first-parent neighborhood.
    let (_, text) = lines(r.log_step(Op::Merged, &s(&["HEAD~2", &main]), &[]).unwrap());
    assert!(
        text.contains("mainline") && !text.contains("feature"),
        "{text}"
    );
    // magit-shortlog-since: "REV.." with the menu's arguments.
    let Next::View(super::Kind::Shortlog(rev, args)) = r
        .log_step(
            Op::ShortlogSince,
            &s(&["v1"]),
            &s(&["--numbered", "--summary"]),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(rev, "v1..");
    let out = String::from_utf8(r.shortlog(&rev, &args).unwrap()).unwrap();
    assert!(out.trim().starts_with("1\t"), "{out:?}");
    assert!(r.log_step(Op::ShortlogRange, &s(&["--all"]), &[]).is_err());
    // magit-cherry: topic's commit is merged, so cherry from v1's parent.
    git(d.path(), &["checkout", "-qb", "pick", "HEAD~3"]);
    // "own" first: a pick onto feature's own parent in the same second
    // would recreate the identical commit.
    commit("own");
    git(d.path(), &["cherry-pick", &feature]);
    let Next::View(super::Kind::Cherry(head, upstream)) =
        r.log_step(Op::Cherry, &s(&["pick", "topic"]), &[]).unwrap()
    else {
        panic!()
    };
    let cherries = r.cherry(&head, &upstream).unwrap();
    let signs: Vec<_> = cherries.iter().map(|c| (c.0, c.2.as_str())).collect();
    assert_eq!(signs, vec![('-', "feature"), ('+', "own")]);
    assert!(r.log_step(Op::Cherry, &s(&["pick", "-x"]), &[]).is_err());
}

#[test]
fn submodule_add_populate_list_and_remove() {
    use super::branch::Next;
    use super::submodule::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let (sub, _) = setup();
    fs::write(sub.path().join("x"), "x").unwrap();
    git(sub.path(), &["add", "x"]);
    git(sub.path(), &["commit", "-qm", "sub"]);
    git(sub.path(), &["tag", "v9"]);
    fs::write(d.path().join("f"), "f").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "base"]);
    // Terminal Git commands run here with local file transport allowed.
    let run = |n: Next| match n {
        Next::Git(argv) => {
            let mut all = s(&["-c", "protocol.file.allow=always"]);
            all.extend(argv);
            git(
                d.path(),
                &all.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
        other => panic!("{other:?}"),
    };
    let url = sub.path().to_string_lossy().into_owned();
    assert!(r.submodule_step(Op::Add, &s(&["-x"]), &[]).is_err());
    assert!(
        r.submodule_step(Op::Add, &s(&[&url, "../out"]), &[])
            .is_err()
    );
    run(r
        .submodule_step(
            Op::Add,
            &s(&[&url, "lib", ""]),
            &s(&["--force", "--rebase"]),
        )
        .unwrap());
    git(d.path(), &["commit", "-qm", "add lib"]);
    assert_eq!(r.module_paths().unwrap(), s(&["lib"]));
    let rows = r.module_rows().unwrap();
    assert!(rows[0].0.contains("v9"), "{rows:?}");
    // Populate only applies to unpopulated modules; unpopulate, then populate.
    assert!(r.submodule_step(Op::Populate, &s(&["lib"]), &[]).is_err());
    assert!(r.submodule_step(Op::Update, &s(&["nope"]), &[]).is_err());
    run(r.submodule_step(Op::Unpopulate, &s(&["lib"]), &[]).unwrap());
    assert!(r.module_rows().unwrap()[0].0.contains("(unpopulated)"));
    run(r.submodule_step(Op::Populate, &s(&["lib"]), &[]).unwrap());
    assert!(d.path().join("lib/x").exists());
    // A dirty module is omitted without --force, and confirmed with it.
    fs::write(d.path().join("lib/x"), "dirty").unwrap();
    assert!(r.submodule_step(Op::Remove, &s(&["lib"]), &[]).is_err());
    let Next::Ask(super::Question::Submodule(op), ..) = r
        .submodule_step(Op::Remove, &s(&["lib"]), &s(&["--force"]))
        .unwrap()
    else {
        panic!()
    };
    assert!(
        r.submodule_step(op.clone(), &s(&["n"]), &s(&["--force"]))
            .is_err()
    );
    assert!(d.path().join("lib/x").exists());
    // Untracked files count as dirty and are stashed with the tracked change.
    fs::write(d.path().join("lib/untracked"), "keep").unwrap();
    git(
        d.path().join("lib").as_path(),
        &["config", "status.showUntrackedFiles", "no"],
    );
    r.submodule_step(op, &s(&["y"]), &s(&["--force"])).unwrap();
    assert!(r.module_paths().unwrap().is_empty());
    assert!(!d.path().join("lib").exists());
    let gitdir = d.path().join(".git/modules/lib");
    let tracked = git(&gitdir, &["diff", "--name-only", "stash^1", "stash"]);
    let untracked = git(&gitdir, &["ls-tree", "-r", "--name-only", "stash^3"]);
    assert_eq!(String::from_utf8(tracked).unwrap().trim(), "x");
    assert_eq!(String::from_utf8(untracked).unwrap().trim(), "untracked");
    // Paths naming .git in any case are refused.
    for bad in [".GIT/foo", "a/.Git", "git~1", ".git."] {
        assert!(
            r.submodule_step(Op::Add, &s(&[&url, bad]), &[]).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn subtree_commands_take_prefix_from_arguments_or_answers() {
    use super::branch::Next;
    use super::subtree::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let (prompts, _) = r.subtree_prompts(&Op::Add, &[]);
    assert_eq!(prompts.len(), 3);
    let (prompts, _) = r.subtree_prompts(&Op::Split, &s(&["--prefix=lib"]));
    assert_eq!(prompts, s(&["Commit: "]));
    let Next::Git(argv) = r
        .subtree_step(Op::Add, &s(&["lib/", "origin", "main"]), &s(&["--squash"]))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        argv,
        s(&[
            "subtree",
            "add",
            "--prefix=lib",
            "--squash",
            "origin",
            "main"
        ])
    );
    let abs = d.path().join("vendor").to_string_lossy().into_owned();
    let Next::Git(argv) = r
        .subtree_step(
            Op::Split,
            &s(&["HEAD"]),
            &s(&[&format!("--prefix={abs}"), "--rejoin"]),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        argv,
        s(&["subtree", "split", "--prefix=vendor", "--rejoin", "HEAD"])
    );
    for bad in ["../x", ".git", "-x", "/elsewhere"] {
        assert!(
            r.subtree_step(Op::Merge, &s(&[bad, "HEAD"]), &[]).is_err(),
            "{bad}"
        );
    }
    assert!(
        r.subtree_step(Op::Push, &s(&["lib", "--upload-pack=x", "main"]), &[])
            .is_err()
    );
}

#[test]
fn patch_create_am_apply_save_and_request_pull() {
    use super::branch::Next;
    use super::patch::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let commit = |name: &str| {
        fs::write(d.path().join(name), name).unwrap();
        git(d.path(), &["add", name]);
        git(d.path(), &["commit", "-qm", name]);
    };
    commit("base");
    commit("one");
    commit("two");
    // A single commit means just that commit; the cover letter is visited.
    let out = d.path().join("out");
    fs::create_dir(&out).unwrap();
    let Next::Visit(cover) = r
        .patch_step(
            Op::Create,
            &s(&["HEAD~1"]),
            &s(&[
                "--cover-letter",
                "--output-directory=out",
                "--reroll-count=2",
            ]),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(cover, r.root.join("out/v2-0000-cover-letter.patch"));
    assert!(out.join("v2-0001-one.patch").exists());
    assert!(!out.join("v2-0002-two.patch").exists());
    assert!(r.patch_step(Op::Create, &s(&["--all"]), &[]).is_err());
    // magit-am applies them elsewhere; continue/skip/abort need a session.
    git(d.path(), &["checkout", "-qb", "other", "HEAD~2"]);
    assert!(r.patch_resolve(Op::AmSkip).is_err());
    assert_eq!(r.patch_resolve(Op::AmApply).unwrap(), Op::Apply);
    let Next::GitEditor(argv) = r
        .patch_step(
            Op::AmPatches,
            &s(&["out/v2-0001-one.patch"]),
            &s(&["--3way"]),
        )
        .unwrap()
    else {
        panic!()
    };
    git(
        d.path(),
        &argv.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(d.path().join("one").exists());
    // A plain patch applies to the worktree only.
    git(d.path(), &["diff", "HEAD~1", "HEAD", "--output=plain.diff"]);
    git(d.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    r.patch_step(Op::Apply, &s(&["plain.diff"]), &[]).unwrap();
    assert!(d.path().join("one").exists());
    // Save the worktree diff; never over an existing file.
    git(d.path(), &["add", "-N", "one"]);
    let target = super::diff::Target::Unstaged;
    r.patch_step(
        Op::SaveDiff(target.clone(), vec![]),
        &s(&["saved.patch"]),
        &[],
    )
    .unwrap();
    assert!(
        fs::read_to_string(d.path().join("saved.patch"))
            .unwrap()
            .contains("+one")
    );
    assert!(
        r.patch_step(Op::SaveDiff(target, vec![]), &s(&["saved.patch"]), &[])
            .is_err()
    );
    // request-pull reads the remote's url.
    git(
        d.path(),
        &["remote", "add", "up", &d.path().to_string_lossy()],
    );
    let Next::View(super::Kind::Output(_, argv)) = r
        .patch_step(Op::RequestPull, &s(&["up", "main~2", "main"]), &[])
        .unwrap()
    else {
        panic!()
    };
    let text = String::from_utf8(
        r.read_network(&argv.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap(),
    )
    .unwrap();
    assert!(
        text.contains("are available in the Git repository"),
        "{text}"
    );
    assert!(
        r.patch_step(Op::RequestPull, &s(&["nope", "a", "b"]), &[])
            .is_err()
    );
    // Defaults follow the action: commit for create, file for apply.
    let (_, def) = r.patch_prompts(&Op::Create, None, Some("f".into()));
    assert_eq!(def, s(&["other"]));
    let (_, def) = r.patch_prompts(&Op::Apply, Some("abc".into()), None);
    assert_eq!(def, s(&[""]));
    // Saved patches leave out the buffer's --stat.
    r.patch_step(
        Op::SaveDiff(super::diff::Target::Commit("HEAD".into()), s(&["--stat"])),
        &s(&["nostat.patch"]),
        &[],
    )
    .unwrap();
    let saved = fs::read_to_string(d.path().join("nostat.patch")).unwrap();
    assert!(!saved.contains(" | "), "{saved}");
}

#[test]
fn bundle_create_tracked_update_verify_and_heads() {
    use super::branch::Next;
    use super::bundle::{Op, Tracked};
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let commit = |name: &str| {
        fs::write(d.path().join(name), name).unwrap();
        git(d.path(), &["add", name]);
        git(d.path(), &["commit", "-qm", name]);
    };
    commit("one");
    r.bundle_step(Op::Create, &s(&["all.bundle", ""]), &s(&["--all"]))
        .unwrap();
    r.bundle_step(Op::Verify, &s(&["all.bundle"]), &[]).unwrap();
    let Next::View(super::Kind::Output(_, argv)) = r
        .bundle_step(Op::ListHeads, &s(&["all.bundle"]), &[])
        .unwrap()
    else {
        panic!()
    };
    let heads = r
        .read_network(&argv.iter().map(String::as_str).collect::<Vec<_>>())
        .unwrap();
    assert!(
        String::from_utf8(heads)
            .unwrap()
            .contains("refs/heads/main")
    );
    assert!(
        r.bundle_step(Op::Create, &s(&["x.bundle", "--all"]), &[])
            .is_err()
    );
    // Tracked: the tag records the bundle; an update bundles only what's new.
    r.bundle_step(Op::CreateTracked, &s(&["snap", "main", "", ""]), &[])
        .unwrap();
    let msg = String::from_utf8(git(
        d.path(),
        &["for-each-ref", "--format=%(contents)", "refs/tags/snap"],
    ))
    .unwrap();
    let t = Tracked::parse(&msg).unwrap();
    assert_eq!((t.branch.as_str(), t.refs.clone()), ("main", s(&["HEAD"])));
    assert!(t.file.ends_with("snap.bundle"));
    commit("two");
    r.bundle_step(Op::UpdateTracked, &s(&["snap"]), &[])
        .unwrap();
    let heads = String::from_utf8(git(d.path(), &["bundle", "list-heads", &t.file])).unwrap();
    let two = String::from_utf8(git(d.path(), &["rev-parse", "HEAD"])).unwrap();
    assert!(heads.contains(two.trim()), "{heads}");
    // A tag naming some other file is refused (tags can be fetched).
    let victim = d.path().join("precious.txt");
    fs::write(&victim, "precious").unwrap();
    let evil = format!(
        ";; git-bundle tracking\n((file . \"{}\")\n (branch . \"main\")\n (refs)\n (args))\n",
        victim.display()
    );
    git(d.path(), &["tag", "-f", "evil", "-m", &evil]);
    assert!(
        r.bundle_step(Op::UpdateTracked, &s(&["evil"]), &[])
            .is_err()
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
    fs::write(d.path().join("fake.bundle"), "not a bundle").unwrap();
    let evil = evil.replace("precious.txt", "fake.bundle");
    git(d.path(), &["tag", "-f", "evil", "-m", &evil]);
    assert!(
        r.bundle_step(Op::UpdateTracked, &s(&["evil"]), &[])
            .is_err()
    );
    assert!(r.bundle_step(Op::UpdateTracked, &s(&["sn*"]), &[]).is_err());
    // Upstream's own pp-to-string output parses too.
    let upstream = ";; git-bundle tracking\n((file . \"/tmp/a \\\"b\\\".bundle\")\n (branch . \"main\")\n (refs)\n (args \"--all\"))\n";
    let t = Tracked::parse(upstream).unwrap();
    assert_eq!(t.file, "/tmp/a \"b\".bundle");
    assert_eq!((t.refs.len(), t.args.clone()), (0, s(&["--all"])));
    assert_eq!(Tracked::parse(&t.message()), Some(t));
}

#[test]
fn clone_regular_sparse_and_into_non_empty_directory() {
    use super::branch::Next;
    use super::clone::Op;
    let (src, _) = setup();
    fs::write(src.path().join("f"), "f").unwrap();
    git(src.path(), &["add", "f"]);
    git(src.path(), &["commit", "-qm", "f"]);
    let base = tempfile::tempdir().unwrap();
    let r = Repo {
        root: base.path().to_path_buf(),
    };
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let url = src.path().to_string_lossy().into_owned();
    let run = |n: Next| match n {
        Next::Invoke(inv) => {
            inv.repo.run(&inv.args, None).unwrap();
            let Some(After::Clone(after)) = inv.after else {
                panic!()
            };
            after.finish().unwrap()
        }
        other => panic!("{other:?}"),
    };
    let Next::Status(dir) = run(r.clone_step(Op::Regular, &s(&[&url, "copy"]), &[]).unwrap())
    else {
        panic!()
    };
    assert!(dir.join("f").exists());
    // An existing non-empty directory gets the repository's name inside it.
    let name = super::clone::url_to_name(&url).unwrap();
    let Next::Status(dir) = run(r
        .clone_step(Op::Sparse, &s(&[&url, "copy"]), &s(&["--origin=up"]))
        .unwrap())
    else {
        panic!()
    };
    assert_eq!(dir, base.path().join("copy").join(&name));
    let cone = String::from_utf8(git(&dir, &["config", "core.sparseCheckoutCone"])).unwrap();
    assert_eq!(cone.trim(), "true");
    assert!(git(&dir, &["remote"]).starts_with(b"up"));
    assert!(
        r.clone_step(Op::Regular, &s(&["--upload-pack=x", "y"]), &[])
            .is_err()
    );
    assert!(
        r.clone_step(Op::ShallowSince, &s(&[&url, "z", ""]), &[])
            .is_err()
    );
}

#[test]
fn refs_list_branches_remotes_tags_with_counts_and_filters() {
    use super::refs::{Count, Op};
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let commit = |name: &str| {
        fs::write(d.path().join(name), name).unwrap();
        git(d.path(), &["add", name]);
        git(d.path(), &["commit", "-qm", name]);
    };
    commit("one");
    git(d.path(), &["tag", "-a", "v1", "-m", "release one"]);
    git(
        d.path(),
        &["remote", "add", "origin", "https://example.test/x.git"],
    );
    git(
        d.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(d.path(), &["branch", "--set-upstream-to=origin/main"]);
    commit("two");
    git(d.path(), &["branch", "side", "HEAD~1"]);
    git(
        d.path(),
        &["config", "branch.main.description", "Main line"],
    );
    let text = |rows: Vec<(String, Option<String>)>| {
        rows.into_iter().map(|r| r.0).collect::<Vec<_>>().join("\n")
    };
    let all = text(r.refs_rows("HEAD", &[], Count::Nothing).unwrap());
    assert!(all.starts_with("main: Main line"), "{all}");
    assert!(
        all.contains("@ main 1>") && all.contains("origin/main two"),
        "{all}"
    );
    assert!(
        all.contains("Remote origin (https://example.test/x.git):"),
        "{all}"
    );
    assert!(
        all.contains("Tags (1)") && all.contains("v1") && all.contains("release one"),
        "{all}"
    );
    // Commit counts relative to the focus; tags only with All.
    let counted = text(r.refs_rows("main", &[], Count::All).unwrap());
    assert!(counted.contains("   1> side"), "{counted}");
    // --no-merged=side hides branches merged into side.
    let filtered = text(
        r.refs_rows("HEAD", &s(&["--no-merged=side"]), Count::Nothing)
            .unwrap(),
    );
    assert!(
        filtered.contains("main") && !filtered.contains("  side "),
        "{filtered}"
    );
    assert!(r.refs_focus(&Op::Other, Some("-x")).is_err());
    assert_eq!(r.refs_focus(&Op::Current, None).unwrap(), "main");
}

#[test]
fn gitignore_rules_skip_worktree_and_sparse_checkout() {
    use super::ignore::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    fs::create_dir_all(d.path().join("sub/deep")).unwrap();
    fs::write(d.path().join("sub/deep/f"), "f").unwrap();
    fs::write(d.path().join("tracked"), "t").unwrap();
    fs::create_dir(d.path().join("other")).unwrap();
    fs::write(d.path().join("other/o"), "o").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-qm", "base"]);
    fs::write(d.path().join("build.log"), "x").unwrap();
    fs::write(d.path().join(".gitignore"), "existing").unwrap();
    // Defaults: the untracked file itself.
    let (_, def) = r.ignore_prompts(&Op::Topdir, Some("build.log".into()));
    assert_eq!(def, s(&["/build.log"]));
    r.ignore_step(Op::Topdir, &s(&["/build.log"]), &[]).unwrap();
    let text = fs::read_to_string(d.path().join(".gitignore")).unwrap();
    assert_eq!(text, "existing\n/build.log\n");
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).starts_with(b".gitignore"));
    r.ignore_step(Op::Subdir, &s(&["sub", "a\\b"]), &[])
        .unwrap();
    assert_eq!(
        fs::read_to_string(d.path().join("sub/.gitignore")).unwrap(),
        "a\\\\b\n"
    );
    r.ignore_step(Op::Gitdir, &s(&["*.tmp"]), &[]).unwrap();
    assert!(
        fs::read_to_string(d.path().join(".git/info/exclude"))
            .unwrap()
            .ends_with("*.tmp\n")
    );
    for bad in ["../x", ".git", ".GIT/x", "/tmp", "missing"] {
        assert!(
            r.ignore_step(Op::Subdir, &s(&[bad, "p"]), &[]).is_err(),
            "{bad}"
        );
    }
    // A symlinked directory leading outside the worktree is refused.
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
    assert!(r.ignore_step(Op::Subdir, &s(&["link", "p"]), &[]).is_err());
    assert!(!outside.path().join(".gitignore").exists());
    fs::remove_file(d.path().join("link")).unwrap();
    // An absolute directory inside the worktree is accepted.
    let abs = d.path().join("sub").to_string_lossy().into_owned();
    r.ignore_step(Op::Subdir, &s(&[&abs, "q"]), &[]).unwrap();
    // Skip worktree only for tracked files.
    r.ignore_step(Op::SkipWorktree, &s(&["tracked"]), &[])
        .unwrap();
    assert!(git(d.path(), &["ls-files", "-v", "tracked"]).starts_with(b"S "));
    assert!(
        r.ignore_step(Op::AssumeUnchanged, &s(&["build.log"]), &[])
            .is_err()
    );
    r.ignore_step(Op::NoSkipWorktree, &s(&["tracked"]), &[])
        .unwrap();
    // Sparse: set auto-enables cone mode; disable/reapply need it enabled.
    assert!(r.ignore_step(Op::SparseReapply, &[], &[]).is_err());
    git(d.path(), &["commit", "-qm", "ignore"]);
    r.ignore_step(Op::SparseSet, &s(&["sub"]), &[]).unwrap();
    assert!(r.sparse_enabled());
    // Cone mode keeps top-level files.
    assert!(d.path().join("sub/deep/f").exists() && !d.path().join("other/o").exists());
    // Re-enabling is Git's business, as upstream (converts to cone mode).
    r.ignore_step(Op::SparseEnable, &[], &[]).unwrap();
    assert!(r.ignore_step(Op::SparseAdd, &s(&["--x"]), &[]).is_err());
    r.ignore_step(Op::SparseDisable, &[], &[]).unwrap();
    assert!(d.path().join("other/o").exists());
}

#[test]
fn commit_reshelve_and_absorb_modules() {
    use super::branch::Next;
    use super::commit::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    git(d.path(), &["config", "user.email", "fred@example.test"]);
    git(d.path(), &["config", "user.name", "Fred Test"]);
    fs::write(d.path().join("f"), "f").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "f"]);
    let (prompts, def) = r.commit_prompts(&Op::Reshelve, None);
    assert!(prompts[0].starts_with("Change author and committer dates"));
    r.commit_step(Op::Reshelve, &s(&["2020-01-02 03:04:05 +0000"]), &[])
        .unwrap();
    let dates = String::from_utf8(git(
        d.path(),
        &["log", "-1", "--format=%ad|%cd", "--date=iso"],
    ))
    .unwrap();
    assert_eq!(
        dates.trim(),
        "2020-01-02 03:04:05 +0000|2020-01-02 03:04:05 +0000"
    );
    r.commit_step(Op::Reshelve, &def, &[]).unwrap();
    let year = String::from_utf8(git(
        d.path(),
        &["log", "-1", "--format=%cd", "--date=format:%Y"],
    ))
    .unwrap();
    assert_ne!(year.trim(), "2020");
    assert!(r.commit_step(Op::Reshelve, &s(&["-x"]), &[]).is_err());
    // Absorb modules: a fixup commit for a module whose gitlink moved.
    let (sub, _) = setup();
    fs::write(sub.path().join("x"), "1").unwrap();
    git(sub.path(), &["add", "x"]);
    git(sub.path(), &["commit", "-qm", "one"]);
    git(
        d.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &sub.path().to_string_lossy(),
            "lib",
        ],
    );
    git(d.path(), &["commit", "-qm", "add lib"]);
    assert!(
        r.commit_step(Op::AbsorbModules, &s(&["HEAD~1"]), &[])
            .is_err()
    );
    fs::write(d.path().join("lib/x"), "2").unwrap();
    git(&d.path().join("lib"), &["commit", "-qam", "two"]);
    let Next::Done(Ok(_)) = r
        .commit_step(Op::AbsorbModules, &s(&["HEAD~1"]), &[])
        .unwrap()
    else {
        panic!()
    };
    let subject = String::from_utf8(git(d.path(), &["log", "-1", "--format=%s"])).unwrap();
    assert_eq!(subject.trim(), "fixup! add lib");
}

#[test]
fn diff_location_maps_lines_to_both_sides() {
    use super::diff::location;
    use std::path::PathBuf;
    let patch = "diff --git a/old b/f\nsimilarity index 90%\nrename from old\nrename to f\n--- a/old\n+++ b/f\n@@ -3,4 +3,4 @@ ctx\n 3\n 4\n--- five\n\\ No newline at end of file\n+5\n 6\ndiff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\ndiff --git a/sp ace b/sp ace\n--- a/sp ace\\t\n+++ b/sp ace\\t\n@@ -1 +1 @@\n-a\n+b";
    let lines: Vec<&str> = patch.lines().collect();
    let at = |l| location(&lines, l).unwrap();
    // Context and added lines: the new side; removed: the old side and name.
    assert_eq!((at(8).line, at(8).removed), (4, false));
    let removed = at(9);
    assert!(removed.removed);
    assert_eq!(
        (removed.old_line, removed.old_file.clone()),
        (5, PathBuf::from("old"))
    );
    // "\ No newline" lines are not counted.
    assert_eq!((at(11).line, at(11).file.clone()), (5, PathBuf::from("f")));
    // A hunk header visits its first change.
    assert_eq!((at(6).old_line, at(6).removed), (5, true));
    assert_eq!(at(18).old_file, PathBuf::from("gone"));
    assert_eq!(at(24).file, PathBuf::from("sp ace"));
    assert!(location(&lines, 0).is_some_and(|l| l.line == 1));
    assert_eq!(at(13).file, PathBuf::from("gone"));
    // Combined diffs: @@@ headers with two marker columns.
    let cc = "diff --cc f\n--- a/f\n+++ b/f\n@@@ -1,2 -1,2 +1,3 @@@\n  a\n+ b\n +c";
    let lines: Vec<&str> = cc.lines().collect();
    let l = location(&lines, 6).unwrap();
    assert_eq!((l.line, l.removed), (3, false));
}

#[test]
fn menu_keys_are_unique_within_each_menu() {
    for menu in "*OzFBbdpflMrxtCGNYXvSoukyJjWKawIEg>DceQP!hUVR".chars() {
        let entries = super::menu_entries(menu);
        assert!(!entries.is_empty(), "menu {menu} is empty");
        let mut seen = std::collections::HashSet::new();
        for (key, ..) in &entries {
            assert!(seen.insert(*key), "menu {menu} repeats key {key}");
        }
    }
    let has = |m: char, k: &str| super::menu_entries(m).iter().any(|e| e.0 == k);
    assert!(has('r', "-f") && has('r', "-x") && has('r', "+s"));
    assert!(has('x', "-m") && has('v', "-m") && has('v', "-S"));
    assert!(!has('p', "-x"));
}

#[test]
fn configure_variables_orphan_shelve_and_unshallow() {
    use super::branch::Next;
    use super::configure::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let cfg = |k: &str| r.config(k);
    fs::write(d.path().join("f"), "f").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "f"]);
    git(
        d.path(),
        &["remote", "add", "origin", "https://example.test/a.git"],
    );
    git(
        d.path(),
        &["remote", "add", "fork", "https://example.test/b.git"],
    );
    git(d.path(), &["config", "branch.main.remote", "origin"]);
    // Choices cycle, then unset.
    for want in ["true", "merges", "interactive", "false"] {
        r.configure_step(Op::BranchRebase, &[]).unwrap();
        assert_eq!(cfg("branch.main.rebase").as_deref(), Some(want));
    }
    r.configure_step(Op::BranchRebase, &[]).unwrap();
    assert_eq!(cfg("branch.main.rebase"), None);
    r.configure_step(Op::PushDefault, &[]).unwrap();
    r.configure_step(Op::PushDefault, &[]).unwrap();
    assert_eq!(cfg("remote.pushDefault").as_deref(), Some("origin"));
    r.configure_step(Op::RemoteTagopt, &[]).unwrap();
    assert_eq!(cfg("remote.origin.tagOpt").as_deref(), Some("--no-tags"));
    r.configure_step(Op::RemotePushurl, &s(&["git@example.test:a.git"]))
        .unwrap();
    assert_eq!(
        cfg("remote.origin.pushurl").as_deref(),
        Some("git@example.test:a.git")
    );
    r.configure_step(Op::RemotePushurl, &s(&[""])).unwrap();
    assert_eq!(cfg("remote.origin.pushurl"), None);
    assert!(r.configure_step(Op::Upstream, &s(&["-x"])).is_err());
    // Shelve keeps the reflog and the commit; unshelve drops the date.
    git(d.path(), &["branch", "side"]);
    assert!(r.configure_step(Op::Shelve, &s(&["main"])).is_err());
    r.configure_step(Op::Shelve, &s(&["side"])).unwrap();
    let shelved = String::from_utf8(git(
        d.path(),
        &["for-each-ref", "--format=%(refname)", "refs/shelved"],
    ))
    .unwrap();
    let name = shelved
        .trim()
        .strip_prefix("refs/shelved/")
        .unwrap()
        .to_owned();
    assert!(name.ends_with("-side") && name.len() == 15, "{name}");
    assert!(d.path().join(".git/logs/refs/shelved").join(&name).exists());
    r.configure_step(Op::Unshelve, &s(&[&name])).unwrap();
    assert!(git(d.path(), &["branch", "--list", "side"]).starts_with(b"  side"));
    // Delete a shelved branch; push a notes ref to a remote.
    git(
        d.path(),
        &["update-ref", "refs/shelved/2020-01-01-old", "HEAD"],
    );
    r.configure_step(Op::DeleteShelved, &s(&["2020-01-01-old"]))
        .unwrap();
    assert!(
        r.read(&["show-ref", "--verify", "-q", "refs/shelved/2020-01-01-old"])
            .is_err()
    );
    let Next::Git(argv) = r
        .configure_step(Op::PushNotesRef, &s(&["commits", "origin"]))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(argv, s(&["push", "origin", "refs/notes/commits"]));
    assert!(
        r.configure_step(Op::PushNotesRef, &s(&["commits", "nope"]))
            .is_err()
    );
    // Orphan branch from HEAD keeps the tree, without history.
    r.configure_step(Op::Orphan, &s(&["fresh", "HEAD"]))
        .unwrap();
    assert_eq!(r.current_branch().unwrap(), "fresh");
    assert!(r.read(&["rev-parse", "--verify", "-q", "HEAD"]).is_err());
    // Unshallow asks about a single non-wildcard refspec first.
    git(d.path(), &["checkout", "-q", "main"]);
    git(
        d.path(),
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );
    let Next::Ask(super::Question::Configure(op), ..) =
        r.configure_step(Op::Unshallow, &[]).unwrap()
    else {
        panic!()
    };
    let Next::Git(argv) = r.configure_step(op, &s(&["yes"])).unwrap() else {
        panic!()
    };
    assert_eq!(argv, s(&["fetch", "--unshallow", "origin"]));
    assert_eq!(
        cfg("remote.origin.fetch").as_deref(),
        Some("+refs/heads/*:refs/remotes/origin/*")
    );
}

#[test]
fn discard_keeps_unrelated_work_by_status() {
    use super::Section;
    use super::apply::{Kind, Op, Thing};
    let (d, r) = setup();
    let w = |f: &str, t: &str| fs::write(d.path().join(f), t).unwrap();
    let read = |f: &str| fs::read_to_string(d.path().join(f)).ok();
    for f in ["m", "r", "keep", "au", "x"] {
        w(f, "base\n");
    }
    fs::create_dir(d.path().join("src")).unwrap();
    w("src/x", "one\n");
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-qm", "base"]);
    let xy = |p: &str| {
        r.status()
            .unwrap()
            .entries
            .into_iter()
            .find(|e| e.path == std::path::Path::new(p))
            .map(|e| e.xy)
            .unwrap()
    };
    let discard = |thing: Thing| {
        r.apply_step(
            Op {
                kind: Kind::Discard,
                thing: Some(thing),
            },
            "y",
        )
    };
    // MM: staged discard keeps the unstaged work.
    w("m", "staged\n");
    git(d.path(), &["add", "m"]);
    w("m", "unstaged-work\n");
    discard(Thing::File("m".into(), Section::Staged, xy("m"))).unwrap();
    assert_eq!(read("m").as_deref(), Some("unstaged-work\n"));
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).is_empty());
    // AM: a new file with unstaged edits becomes untracked, content kept.
    w("new", "first\n");
    git(d.path(), &["add", "new"]);
    w("new", "edited\n");
    discard(Thing::File("new".into(), Section::Staged, xy("new"))).unwrap();
    assert_eq!(read("new").as_deref(), Some("edited\n"));
    assert_eq!(xy("new"), "??");
    // R: a staged rename is undone, edits to the renamed file kept.
    git(d.path(), &["mv", "r", "r2"]);
    w("r2", "wip\n");
    discard(Thing::File("r2".into(), Section::Staged, xy("r2"))).unwrap();
    assert_eq!(read("r").as_deref(), Some("wip\n"));
    assert!(read("r2").is_none());
    // Intent-to-add is refused, not truncated.
    w("ita", "precious\n");
    git(d.path(), &["add", "-N", "ita"]);
    assert!(discard(Thing::File("ita".into(), Section::Unstaged, xy("ita"))).is_err());
    assert_eq!(read("ita").as_deref(), Some("precious\n"));
    // The Unstaged section only touches the files it listed.
    git(d.path(), &["update-index", "--assume-unchanged", "keep"]);
    w("keep", "hidden edit\n");
    w("au", "listed edit\n");
    w("x", "unlisted edit\n");
    discard(Thing::Section(Section::Unstaged, vec!["au".into()])).unwrap();
    assert_eq!(read("au").as_deref(), Some("base\n"));
    assert_eq!(read("x").as_deref(), Some("unlisted edit\n"));
    assert_eq!(read("keep").as_deref(), Some("hidden edit\n"));
    assert_eq!(read("ita").as_deref(), Some("precious\n"));
    // Reverse uses fixed prefixes: diff.noprefix cannot redirect it.
    git(d.path(), &["config", "diff.noprefix", "true"]);
    w("src/x", "two\n");
    git(d.path(), &["add", "src/x"]);
    w("x", "two\n");
    r.apply_step(
        Op {
            kind: Kind::Reverse,
            thing: Some(Thing::File("src/x".into(), Section::Staged, xy("src/x"))),
        },
        "y",
    )
    .unwrap();
    assert_eq!(read("src/x").as_deref(), Some("one\n"));
    assert_eq!(read("x").as_deref(), Some("two\n"));
    // A file whose status changed since the buffer was drawn is refused.
    assert!(discard(Thing::File("src/x".into(), Section::Staged, "A ".into())).is_err());
}

#[test]
fn misc_git_command_reset_quickly_and_checkout_stage() {
    use super::Section;
    use super::apply::{Kind, Op as A, Thing};
    use super::branch::Next;
    use super::misc::{Op, split_words};
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        split_words(r#"git log -1 --format="%s by %an" 'a b' c\ d"#).unwrap(),
        s(&["git", "log", "-1", "--format=%s by %an", "a b", "c d"])
    );
    assert!(split_words("log 'oops").is_err());
    let Next::GitEditor(argv) = r
        .misc_step(Op::GitCommand { topdir: true }, &s(&["git status -s"]))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(argv, s(&["status", "-s"]));
    fs::write(d.path().join("f"), "base\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "base"]);
    fs::write(d.path().join("f"), "two\n").unwrap();
    git(d.path(), &["commit", "-qam", "two"]);
    // reset --mixed keeps the worktree.
    r.misc_step(Op::ResetQuickly, &s(&["HEAD~1"])).unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "two\n");
    assert!(r.misc_step(Op::ResetQuickly, &s(&["--hard"])).is_err());
    // A conflict: x on the file takes theirs (and stages it).
    git(d.path(), &["checkout", "-q", "--", "f"]);
    git(d.path(), &["checkout", "-qb", "side"]);
    fs::write(d.path().join("f"), "side\n").unwrap();
    git(d.path(), &["commit", "-qam", "side"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(d.path().join("f"), "main\n").unwrap();
    git(d.path(), &["commit", "-qam", "main"]);
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["merge", "-q", "side"])
        .output();
    let xy = r
        .status()
        .unwrap()
        .entries
        .into_iter()
        .find(|e| e.conflict)
        .unwrap()
        .xy;
    let op = A {
        kind: Kind::Discard,
        thing: Some(Thing::File("f".into(), Section::Conflicts, xy)),
    };
    assert!(op.question().unwrap().unwrap().contains("[t]heirs"));
    r.apply_step(op, "t").unwrap();
    assert_eq!(fs::read_to_string(d.path().join("f")).unwrap(), "side\n");
    assert!(r.status().unwrap().entries.iter().all(|e| !e.conflict));
}

#[test]
fn wip_commit_logs_and_purge() {
    use super::branch::Next;
    use super::wip::Op;
    let (d, r) = setup();
    fs::write(d.path().join("f"), "base\n").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "base"]);
    fs::write(d.path().join("f"), "staged\n").unwrap();
    git(d.path(), &["add", "f"]);
    fs::write(d.path().join("f"), "worktree\n").unwrap();
    r.wip_step(Op::Commit, &[]).unwrap();
    let show = |r: &str| String::from_utf8(git(d.path(), &["show", &format!("{r}:f")])).unwrap();
    assert_eq!(show("refs/wip/index/refs/heads/main"), "staged\n");
    assert_eq!(show("refs/wip/wtree/refs/heads/main"), "worktree\n");
    // Index and worktree untouched.
    assert_eq!(
        fs::read_to_string(d.path().join("f")).unwrap(),
        "worktree\n"
    );
    assert_eq!(
        String::from_utf8(git(d.path(), &["diff", "--cached", "--name-only"])).unwrap(),
        "f\n"
    );
    let Next::Done(Ok(m)) = r.wip_step(Op::Commit, &[]).unwrap() else {
        panic!()
    };
    assert!(m.contains("No changes"), "{m}");
    let Next::View(super::Kind::Log(revs, _)) = r.wip_step(Op::LogCurrent, &[]).unwrap() else {
        panic!()
    };
    assert_eq!(revs.len(), 3);
    // A deleted branch leaves dangling wip refs to purge.
    git(d.path(), &["checkout", "-qb", "gone"]);
    r.wip_step(Op::Commit, &[]).unwrap();
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["branch", "-D", "gone"]);
    let Next::Ask(super::Question::Wip(op), ..) = r.wip_step(Op::Purge, &[]).unwrap() else {
        panic!()
    };
    r.wip_step(op, &["y".into()]).unwrap();
    let refs = String::from_utf8(git(
        d.path(),
        &["for-each-ref", "--format=%(refname)", "refs/wip/"],
    ))
    .unwrap();
    assert!(!refs.contains("gone") && refs.contains("main"), "{refs}");
}

#[test]
fn smerge_keeps_a_side_of_the_conflict_at_point() {
    use super::smerge::{Keep, keep};
    use crate::{buffer::Buffer, editor::Editor};
    let text = "a\n<<<<<<< HEAD\nours\n||||||| base\nold\n=======\ntheirs\n>>>>>>> side\nz\n";
    let run = |line: usize, how: Keep| {
        let mut e = Editor::new(Buffer::from_text(text));
        e.set_cursor(line, 0);
        assert!(keep(&mut e, how));
        e.buf.text()
    };
    assert_eq!(run(2, Keep::Upper), "a\nours\nz");
    assert_eq!(run(2, Keep::Lower), "a\ntheirs\nz");
    assert_eq!(run(2, Keep::Base), "a\nold\nz");
    assert_eq!(run(2, Keep::All), "a\nours\ntheirs\nz");
    assert_eq!(run(6, Keep::Current), "a\ntheirs\nz");
    let mut e = Editor::new(Buffer::from_text(text));
    e.set_cursor(0, 0);
    assert!(!keep(&mut e, Keep::Upper));
}

#[test]
fn clean_config_file_and_notes_ref() {
    use super::branch::Next;
    use super::misc::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    fs::write(d.path().join(".gitignore"), "ign\n").unwrap();
    fs::write(d.path().join("f"), "f").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-qm", "base"]);
    fs::write(d.path().join("loose"), "x").unwrap();
    fs::write(d.path().join("ign"), "x").unwrap();
    assert!(r.misc_step(Op::Clean(0), &s(&["y"])).is_err());
    r.misc_step(Op::Clean(0), &s(&["yes"])).unwrap();
    assert!(!d.path().join("loose").exists() && d.path().join("ign").exists());
    r.misc_step(Op::Clean(2), &s(&["yes"])).unwrap();
    assert!(!d.path().join("ign").exists());
    let Next::Visit(p) = r.misc_step(Op::GitConfigFile, &[]).unwrap() else {
        panic!()
    };
    assert!(p.ends_with(".git/config"));
    // notes --ref goes before the subcommand.
    let Next::Git(argv) = r
        .notes_step(
            super::notes::Op::Remove,
            &s(&["HEAD"]),
            &s(&["--ref=review"]),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        &argv[..3],
        &s(&["notes", "--ref=refs/notes/review", "remove"])
    );
}

#[test]
fn diffstat_and_diff_jump() {
    use super::diff::stat_or_diff;
    let text = "header\n a | 2 +-\n b | 1 +\n 2 files changed\n\ndiff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/b b/b\n--- /dev/null\n+++ b/b\n@@ -0,0 +1 @@\n+z";
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(stat_or_diff(&lines, 2), Some(11));
    assert_eq!(stat_or_diff(&lines, 1), Some(5));
    assert_eq!(stat_or_diff(&lines, 15), Some(2));
    assert_eq!(stat_or_diff(&lines, 0), Some(1));
}

#[test]
fn ediff_runs_difftool_and_mergetool() {
    use super::branch::Next;
    use super::ediff::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    fs::write(d.path().join("f"), "f").unwrap();
    git(d.path(), &["add", "f"]);
    git(d.path(), &["commit", "-qm", "f"]);
    let argv = |op, a: &[&str], args: &[&str]| match r.ediff_step(op, &s(a), &s(args)).unwrap() {
        Next::Git(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        argv(Op::ShowStaged, &["f"], &[]),
        s(&["difftool", "-y", "--cached", "--", "f"])
    );
    assert_eq!(
        argv(Op::ShowCommit, &["HEAD"], &["--tool=vimdiff"]),
        s(&["difftool", "-y", "--tool=vimdiff", "HEAD^", "HEAD", "--"])
    );
    assert!(
        r.ediff_step(Op::Resolve, &s(&["f"]), &[]).is_err(),
        "no conflict"
    );
    assert!(r.ediff_step(Op::ShowUnstaged, &s(&["-x"]), &[]).is_err());
    let (_, def) = r.ediff_prompts(&Op::ShowCommit, None, None);
    assert_eq!(def, s(&["HEAD"]));
}

#[test]
fn hunk_patch_keeps_raw_bytes_and_drops_renames_for_hunks() {
    use super::diff::hunk_patch;
    let raw = b"diff --git a/old b/new\nsimilarity index 90%\nrename from old\nrename to new\n--- a/old\n+++ b/new\n@@ -1 +1 @@\n-x\n+\ty\r\n";
    let lines: Vec<&[u8]> = raw.split(|b| *b == b'\n').collect();
    let hunk = hunk_patch(&lines, 7).unwrap();
    let text = String::from_utf8(hunk).unwrap();
    assert!(
        text.starts_with("diff --git a/new b/new\n--- a/new\n+++ b/new\n@@"),
        "{text}"
    );
    assert!(text.contains("+\ty\r\n"), "{text:?}");
    let whole = String::from_utf8(hunk_patch(&lines, 0).unwrap()).unwrap();
    assert!(whole.contains("rename from old"));
    let cc: Vec<&[u8]> = b"diff --cc f\n@@@ -1 -1 +1 @@@\n+ x"
        .split(|b| *b == b'\n')
        .collect();
    assert!(hunk_patch(&cc, 2).is_err());
}
#[test]
fn branch_or_checkout_remote_ref_and_default_branch() {
    use super::Question as Q;
    use super::branch::{Next, Op};
    let (d, r, bare) = with_remote();
    git(d.path(), &["push", "-qu", "origin", "main"]);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // An existing revision is checked out; a new name asks for a start point.
    git(d.path(), &["branch", "side"]);
    let n = r.branch_step(Op::OrCheckout, &s(&["side"]), &[]);
    assert!(matches!(n, Next::Done(Ok(_))), "{n:?}");
    assert_eq!(r.current_branch().unwrap(), "side");
    let Next::Ask(Q::Branch(Op::OrCheckoutNew(new)), _, _) =
        r.branch_step(Op::OrCheckout, &s(&["fresh"]), &[])
    else {
        panic!()
    };
    // Uncommitted changes block it, unless --merge carries them over.
    fs::write(d.path().join("f"), "dirty\n").unwrap();
    assert!(matches!(
        r.branch_step(Op::OrCheckoutNew(new.clone()), &s(&["main"]), &[]),
        Next::Done(Err(_))
    ));
    let n = r.branch_step_args(Op::OrCheckoutNew(new), &s(&["main"]), &[], &s(&["--merge"]));
    assert!(matches!(n, Next::Done(Ok(_))), "{n:?}");
    assert_eq!(r.current_branch().unwrap(), "fresh");
    git(d.path(), &["checkout", "-q", "--", "f"]);
    // checkout-remote-ref: fetch the ref, then check out FETCH_HEAD.
    let Next::Ask(Q::Branch(op), _, _) = r.branch_step(Op::RemoteRef, &s(&["origin"]), &[]) else {
        panic!()
    };
    let Next::Invoke(inv) = r.branch_step(op, &s(&["main"]), &[]) else {
        panic!()
    };
    assert_eq!(inv.args, ["fetch", "origin", "main"]);
    assert!(matches!(&inv.after, Some(After::Git(a)) if a == &s(&["checkout", "FETCH_HEAD"])));
    assert!(matches!(
        r.branch_step(Op::RemoteRef, &s(&["nope"]), &[]),
        Next::Done(Err(_))
    ));
    // The remote renames main to trunk: rename locally and fix upstreams.
    git(d.path(), &["checkout", "-q", "main"]);
    git(d.path(), &["remote", "set-head", "origin", "main"]);
    git(bare.path(), &["branch", "-m", "main", "trunk"]);
    git(bare.path(), &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
    git(
        d.path(),
        &["branch", "-q", "--set-upstream-to=origin/main", "side"],
    );
    let Next::Ask(Q::Branch(op), p, _) = r.branch_step(Op::UpdateDefault, &[], &[]) else {
        panic!()
    };
    assert!(p[0].contains("from `main' to `trunk'"), "{p:?}");
    assert!(matches!(
        r.branch_step(op.clone(), &s(&["n"]), &[]),
        Next::Done(Err(_))
    ));
    assert!(matches!(
        r.branch_step(op, &s(&["y"]), &[]),
        Next::Done(Ok(_))
    ));
    assert_eq!(r.current_branch().unwrap(), "trunk");
    assert_eq!(r.upstream_of("trunk").as_deref(), Some("origin/trunk"));
    assert_eq!(r.upstream_of("side").as_deref(), Some("origin/trunk"));
}
#[test]
fn pull_into_upstream_and_push_to_remote() {
    use super::network::Op::*;
    let (d, r, bare) = with_remote();
    git(d.path(), &["push", "-qu", "origin", "main"]);
    // main tracks origin/main; work tracks main.
    git(d.path(), &["checkout", "-qb", "work", "--track", "main"]);
    let other = tempfile::tempdir().unwrap();
    git(
        other.path(),
        &["clone", "-q", bare.path().to_str().unwrap(), "."],
    );
    fs::write(other.path().join("g"), "remote\n").unwrap();
    git(other.path(), &["add", "g"]);
    git(other.path(), &["commit", "-qm", "remote"]);
    git(other.path(), &["push", "-q", "origin", "main"]);
    let argv = run_net(&r, PullIntoUpstream, &[], &[]);
    assert_eq!(argv[..2], ["fetch", "origin"]);
    assert_eq!(
        git(d.path(), &["rev-parse", "main"]),
        git(other.path(), &["rev-parse", "HEAD"])
    );
    assert_eq!(r.current_branch().unwrap(), "work");
    // Not possible when the upstream is remote.
    git(d.path(), &["checkout", "-q", "main"]);
    assert!(r.network(PullIntoUpstream, &[], &[]).is_err());
    // push-to-remote: no refspec, the menu's arguments.
    let inv = r
        .network(PushToRemote, &["origin".into()], &["--dry-run".into()])
        .unwrap();
    assert_eq!(inv.args, ["push", "-v", "--dry-run", "origin"]);
    assert!(r.network(PushToRemote, &["nope".into()], &[]).is_err());
}
#[test]
fn changelog_entries_and_message_ring() {
    use super::message::*;
    let diff = b"diff --git a/src/x.rs b/src/x.rs\n--- a/src/x.rs\n+++ b/src/x.rs\n@@ -3 +3 @@ pub fn alpha(x: u8) -> u8 {\n-a\n+b\n@@ -9 +9 @@ (defun magit-foo (x)\n-c\n+d\n@@ -12 +12 @@ pub fn alpha(x: u8) -> u8 {\n-e\n+f\ndiff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
    let defs = modified_defuns(diff);
    assert_eq!(
        defs,
        vec![
            ("src/x.rs".into(), vec!["alpha".into(), "magit-foo".into()]),
            ("gone".into(), vec![]),
        ]
    );
    assert_eq!(
        changelog(&defs, true),
        ["* src/x.rs (alpha):", "(magit-foo):", "* gone:"]
    );
    assert_eq!(
        changelog(&defs, false),
        ["src/x.rs:", "  `alpha'", "  `magit-foo'", "gone:"]
    );
    assert_eq!(defun_name("class Foo:").as_deref(), Some("class Foo"));
    // magit-commit-add-log-insert: a new entry after the summary, a defun
    // added to an existing entry, and trailers and comments kept below.
    let (t, line) = add_log_insert(
        "Summary\n\nSigned-off-by: A <a@b>\n# comment\n",
        "f",
        Some("g"),
    );
    assert_eq!(
        t,
        "Summary\n\n* f (g): \n\nSigned-off-by: A <a@b>\n# comment\n"
    );
    assert_eq!(line, 2);
    let (t, line) = add_log_insert(&t, "f", Some("h"));
    assert_eq!(
        t,
        "Summary\n\n* f (g): \n(h): \n\nSigned-off-by: A <a@b>\n# comment\n"
    );
    assert_eq!(line, 3);
    let (t2, _) = add_log_insert(&t, "f", Some("h"));
    assert_eq!(t2, t);
    let (t, _) = add_log_insert(&t, "other", None);
    assert!(t.contains("(h): \n* other: \n"), "{t:?}");
    let (t, line) = add_log_insert("", "f", None);
    assert_eq!((t.as_str(), line), ("* f: \n", 0));
    // ChangeLog files: today's heading per author, items under it.
    let h = "2026-10-04  Fred  <f@x>";
    let log = change_log_add("", h, "a.c", Some("main"));
    assert_eq!(log, format!("{h}\n\n\t* a.c (main): \n"));
    let log = change_log_add(&log, h, "a.c", Some("util"));
    assert_eq!(log, format!("{h}\n\n\t* a.c (main): \n\t(util): \n"));
    let log = change_log_add(&log, "2026-10-05  Fred  <f@x>", "b.c", None);
    assert!(
        log.starts_with("2026-10-05  Fred  <f@x>\n\n\t* b.c: \n\n2026-10-04"),
        "{log:?}"
    );
    // git-commit-buffer-message drops comments and the scissors section.
    assert_eq!(
        buffer_message(
            "\n\nmsg\n# c\n\n# ------------------------ >8 ------------------------\ndiff"
        )
        .as_deref(),
        Some("msg\n")
    );
    assert_eq!(buffer_message("# only\n \n"), None);
}
#[test]
fn stage_and_unstage_files_and_absorb_needs_its_tool() {
    use super::branch::Next;
    use super::misc::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    fs::write(d.path().join("a"), "a").unwrap();
    fs::write(d.path().join("*"), "star").unwrap();
    // Before the first commit, unstaging is git rm --cached.
    r.misc_step(Op::StageFiles(false), &s(&["a, *"])).unwrap();
    let staged = || {
        String::from_utf8_lossy(&git(d.path(), &["diff", "--cached", "--name-only"])).into_owned()
    };
    assert_eq!(staged(), "*\na\n");
    r.misc_step(Op::UnstageFiles, &s(&["*"])).unwrap();
    assert_eq!(staged(), "a\n");
    git(d.path(), &["commit", "-qm", "base"]);
    fs::write(d.path().join("a"), "b").unwrap();
    r.misc_step(Op::StageFiles(false), &s(&["a"])).unwrap();
    assert_eq!(staged(), "a\n");
    r.misc_step(Op::UnstageFiles, &s(&["a"])).unwrap();
    assert_eq!(staged(), "");
    assert!(r.misc_step(Op::StageFiles(false), &s(&["../x"])).is_err());
    assert!(r.misc_step(Op::StageFiles(false), &s(&[" , "])).is_err());
    // Ignored files need the force variant.
    fs::write(d.path().join(".gitignore"), "ign\n").unwrap();
    fs::write(d.path().join("ign"), "i").unwrap();
    assert!(r.misc_step(Op::StageFiles(false), &s(&["ign"])).is_err());
    r.misc_step(Op::StageFiles(true), &s(&["ign"])).unwrap();
    assert!(staged().contains("ign"));
    let has = |t: &str| {
        Command::new("sh")
            .args(["-c", &format!("command -v {t}")])
            .status()
            .is_ok_and(|s| s.success())
    };
    if !has("git-absorb") {
        let e = r
            .commit_step(super::commit::Op::Absorb, &s(&["HEAD"]), &[])
            .unwrap_err();
        assert!(e.contains("git-absorb"), "{e}");
    }
    let _ = Next::Done(Ok(String::new()));
}
#[test]
fn margin_ages_widths_stamps_and_refinement() {
    use super::margin::*;
    assert_eq!(age(1, false), (1, "second".into()));
    assert_eq!(age(90, false), (2, "minutes".into()));
    assert_eq!(age(3 * 86_400, true), (3, "d".into()));
    assert_eq!(age(400 * 86_400, false), (1, "year".into()));
    let mut m = Margin::for_kind(&super::Kind::Log(vec![], vec![])).unwrap();
    assert!(m.shown && m.details);
    assert!(!Margin::for_kind(&super::Kind::Status).unwrap().shown);
    assert!(Margin::for_kind(&super::Kind::Modules).is_none());
    // magit-log-margin-width: 18 + 1 author, 2 + 1 + 1 + 7 ("minutes").
    assert_eq!(m.width(), 30);
    let st = Stamp {
        author: "A very long author name indeed".into(),
        time: 1000,
        stat: Some("   3+    1-   2".into()),
    };
    let t = m.text(&st, 1000 + 2 * 3600);
    assert_eq!(t.chars().count(), m.width());
    assert!(t.starts_with("A very long autho… "), "{t:?}");
    assert!(t.ends_with(" 2 hours   "), "{t:?}");
    m.cycle_style();
    assert!(
        m.text(&st, 1000 + 7200).ends_with(" 2h "),
        "{:?}",
        m.text(&st, 8200)
    );
    m.cycle_style();
    assert_eq!(m.style, Style::Format("%Y-%m-%d %H:%M ".into()));
    m.cycle_style();
    assert_eq!(m.style, Style::Age);
    m.shortstat = true;
    assert_eq!(m.width(), 16);
    assert_eq!(m.text(&st, 0), "   3+    1-   2");
    assert_eq!(
        shortstat("2 files changed, 3 insertions(+), 1 deletion(-)"),
        "   3+    1-   2"
    );
    assert_eq!(
        shortstat("1 file changed, 4 deletions(-)"),
        "         4-   1"
    );
    // Author and date of commits in one call, with shortstats on request.
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    let id = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let stamps = r.stamps(std::slice::from_ref(&id), false, true).unwrap();
    assert_eq!(stamps.len(), 1);
    assert_eq!(stamps[0].0, id);
    assert_eq!(stamps[0].1.author, "Fred Test");
    assert!(stamps[0].1.time > 0);
    assert!(
        stamps[0]
            .1
            .stat
            .as_deref()
            .is_some_and(|s| s.contains("1+"))
    );
    // Hunk refinement: changed words, and each line's partner in its run.
    let (a, b) = super::diff::refine("let x = 1;", "let y = 1;");
    assert_eq!(
        (a.len(), b.len(), a[0].clone(), b[0].clone()),
        (1, 1, 4..5, 4..5)
    );
    let lines = [" ctx", "-a", "-b", "+A", "+B", "+C", " ctx"];
    let p = |l| super::diff::refine_partner(&lines, l);
    assert_eq!(
        (p(1), p(2), p(3), p(4), p(5), p(0)),
        (Some(3), Some(4), Some(1), Some(2), None, None)
    );
}
#[test]
fn removing_file_and_fixup_target() {
    use super::branch::Next;
    use super::misc::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    committed(d.path(), b"one\n");
    fs::write(d.path().join("gone"), "x").unwrap();
    git(d.path(), &["add", "gone"]);
    git(d.path(), &["commit", "-qm", "add gone"]);
    git(d.path(), &["rm", "-q", "gone"]);
    git(d.path(), &["commit", "-qm", "remove gone"]);
    let removed = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let Next::Show(super::diff::Target::Commit(id)) =
        r.misc_step(Op::RemovingFile, &s(&["gone"])).unwrap()
    else {
        panic!()
    };
    assert_eq!(id, removed);
    assert!(
        r.misc_step(Op::RemovingFile, &s(&["f"]))
            .unwrap_err()
            .contains("not been removed")
    );
    git(
        d.path(),
        &["commit", "-q", "--allow-empty", "-m", "fixup! add gone"],
    );
    let target = r.fixup_target("HEAD").unwrap();
    assert!(
        removed.starts_with(&target) || {
            let add = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "--short", "HEAD~2"]))
                .trim()
                .to_owned();
            target == add
        },
        "{target}"
    );
    assert_eq!(r.fixup_target("HEAD~1").as_deref(), Some("HEAD~1"));
}
#[test]
fn shell_commands_wip_file_and_recorded_calls() {
    use super::branch::Next;
    use super::misc::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    committed(d.path(), b"one\n");
    // magit-shell-command runs in the root (quoted); & substitutes the file.
    let Next::Shell(cmd) = r
        .misc_step(Op::ShellCommand { topdir: true }, &s(&["ls"]))
        .unwrap()
    else {
        panic!()
    };
    assert!(cmd.starts_with("cd '") && cmd.ends_with("' && ls"), "{cmd}");
    let Next::Shell(cmd) = r
        .misc_step(Op::AsyncShell("it's.txt".into()), &s(&["wc -l * | sort"]))
        .unwrap()
    else {
        panic!()
    };
    assert!(cmd.ends_with("wc -l 'it'\\''s.txt' | sort"), "{cmd}");
    assert_eq!(super::misc::shell_quote("a'b"), "'a'\\''b'");
    assert!(
        r.misc_step(Op::ShellCommand { topdir: false }, &s(&[""]))
            .is_err()
    );
    let Next::Done(Ok(m)) = r.misc_step(Op::DebugGit, &[]).unwrap() else {
        panic!()
    };
    assert!(m.contains("git version"), "{m}");
    // magit-wip-commit-buffer-file: only that file's state.
    fs::write(d.path().join("f"), "two\n").unwrap();
    fs::write(d.path().join("g"), "untracked\n").unwrap();
    let m = r.wip_commit_file(Path::new("f")).unwrap();
    assert!(m.contains("refs/wip/wtree/refs/heads/main"), "{m}");
    let blob = git(d.path(), &["show", "refs/wip/wtree/refs/heads/main:f"]);
    assert_eq!(blob, b"two\n");
    assert!(
        r.wip_commit_file(Path::new("f"))
            .unwrap()
            .contains("No changes")
    );
    // magit-toggle-subprocess-record logs background calls.
    use std::sync::atomic::Ordering;
    super::repo::RECORD.store(true, Ordering::Relaxed);
    r.read(&["rev-parse", "HEAD"]).unwrap();
    super::repo::RECORD.store(false, Ordering::Relaxed);
    let calls = super::repo::take_calls();
    assert!(
        calls
            .iter()
            .any(|(root, line, res)| *root == r.root && line == "rev-parse HEAD" && res.is_ok())
    );
}
#[test]
fn branch_name_prompts_turn_spaces_into_dashes() {
    use super::{Prompt, Question, reads_branch_name};
    let r = Repo { root: "/r".into() };
    let ask = |q, answers: Vec<String>| {
        Prompt::Ask(
            r.clone(),
            q,
            vec![],
            vec!["a: ".into(), "b: ".into()],
            answers,
        )
    };
    use super::branch::Op as B;
    assert!(reads_branch_name(&ask(Question::Branch(B::Create), vec![])));
    assert!(!reads_branch_name(&ask(
        Question::Branch(B::Create),
        vec!["x".into()]
    )));
    assert!(reads_branch_name(&ask(
        Question::Branch(B::Rename),
        vec!["old".into()]
    )));
    assert!(!reads_branch_name(&ask(
        Question::Branch(B::Delete),
        vec![]
    )));
}
#[test]
fn pop_revision_stack_inserts_references() {
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    let id = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let mut ed =
        crate::editor::Editor::new(crate::buffer::Buffer::from_text("Fix it \n\n# comment"));
    ed.set_cursor(0, 7);
    ed.revision_stack.push((id.clone(), r.root.clone()));
    ed.revision_stack.push((id.clone(), r.root.clone()));
    super::message::pop_revision_stack(&mut ed);
    let short = &id[..7];
    let text = ed.buf.text();
    assert!(text.starts_with(&format!("Fix it [1: {short}")), "{text:?}");
    assert!(text.contains("\n\n1: ") && text.contains(&id), "{text:?}");
    assert!(text.ends_with("# comment"), "{text:?}");
    // The next one is numbered after the last index before point.
    super::message::pop_revision_stack(&mut ed);
    let text = ed.buf.text();
    assert!(text.contains("[2: ") && text.contains("\n2: "), "{text:?}");
    assert!(!text.contains("\n\n2: "), "entries stay together: {text:?}");
    super::message::pop_revision_stack(&mut ed);
    assert!(ed.msg.as_ref().is_some_and(|m| m.0.contains("empty")));
}
#[test]
fn reshelve_since_rewrites_dates_a_minute_apart() {
    use super::branch::Next;
    use super::commit::Op;
    let (d, r) = setup();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    committed(d.path(), b"one\n");
    for m in ["two", "three"] {
        fs::write(d.path().join(m), m).unwrap();
        git(d.path(), &["add", m]);
        git(d.path(), &["commit", "-qm", m]);
    }
    let tree = git(d.path(), &["rev-parse", "HEAD^{tree}"]);
    let Next::Ask(super::Question::Commit(op), p, defaults) = r
        .commit_step(Op::ReshelveSince, &s(&["HEAD~1"]), &[])
        .unwrap()
    else {
        panic!()
    };
    assert!(p[0].starts_with("Date for first commit"), "{p:?}");
    assert!(!defaults[0].is_empty());
    r.commit_step(op, &s(&["2001-02-03 04:05:06 +0000"]), &[])
        .unwrap();
    let dates =
        String::from_utf8_lossy(&git(d.path(), &["log", "-3", "--format=%at %ct %s"])).into_owned();
    let lines: Vec<&str> = dates.lines().collect();
    assert_eq!(lines[0], "981173166 981173166 three", "{dates}");
    assert_eq!(lines[1], "981173106 981173106 two", "{dates}");
    assert!(
        !lines[2].starts_with("981"),
        "the base is untouched: {dates}"
    );
    assert_eq!(git(d.path(), &["rev-parse", "HEAD^{tree}"]), tree);
    assert_eq!(r.current_branch().unwrap(), "main");
    assert!(
        r.commit_step(Op::ReshelveSinceDate("HEAD".into()), &s(&["-x"]), &[])
            .is_err()
    );
}
#[test]
fn process_kill_interrupts_background_git() {
    let (_d, r) = setup();
    let worker = {
        let r = r.clone();
        std::thread::spawn(move || r.read(&["-c", "alias.zz=!sleep 30", "zz"]))
    };
    let start = std::time::Instant::now();
    let mut killed = 0;
    while killed == 0 && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        killed = super::repo::kill_running(&r.root);
    }
    assert!(killed >= 1);
    assert!(worker.join().unwrap().is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn confirmation_actions_keep_delete_and_remote_configuration_separate() {
    let repo = Repo {
        root: std::path::PathBuf::from("/unused"),
    };
    let ask =
        |q, text: &str| super::Prompt::Ask(repo.clone(), q, vec![], vec![text.into()], vec![]);
    assert_eq!(
        super::confirm_action(&ask(
            super::Question::Apply(super::apply::Op {
                kind: super::apply::Kind::Discard,
                thing: Some(super::apply::Thing::Section(
                    super::Section::Untracked,
                    vec![]
                )),
            }),
            "Delete files? (y or n) "
        )),
        Some("delete")
    );
    assert_eq!(
        super::confirm_action(&ask(
            super::Question::Remote(super::remote::Op::AddPushDefault(
                "origin".into(),
                "url".into()
            )),
            "Set default? (y or n) "
        )),
        None
    );
    assert_eq!(
        super::confirm_action(&ask(
            super::Question::Sequence(super::sequence::Op::Abort),
            "Abort revert? (y or n) "
        )),
        Some("abort-revert")
    );
}
#[test]
fn repository_list_finds_names_and_columns() {
    use super::repos::*;
    let top = tempfile::tempdir().unwrap();
    for p in ["a/proj", "b/proj", "c", "deep/x/y/z"] {
        let d = top.path().join(p);
        fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
    }
    fs::create_dir_all(top.path().join("plain")).unwrap();
    let mut found = vec![];
    list(top.path(), 2, &mut found);
    let names: Vec<String> = uniquify(&found).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["proj\\a", "proj\\b", "c"], "{found:?}");
    let mut deep = vec![];
    list(top.path(), 4, &mut deep);
    assert_eq!(deep.len(), 4);
    let mut none = vec![];
    list(&top.path().join("plain"), 0, &mut none);
    assert!(none.is_empty());
    // Columns: version from the date when untagged, counts against upstream.
    let c = top.path().join("c");
    committed(&c, b"x\n");
    let r = Repo { root: c.clone() };
    let cols = columns();
    assert_eq!(
        cols.iter().map(|c| c.header.as_str()).collect::<Vec<_>>(),
        ["Name", "Version", "B<U", "B>U", "Path"]
    );
    assert_eq!(r.repolist_cell("c", &cols[0]), "c");
    assert!(
        r.repolist_cell("c", &cols[1]).starts_with(" 20"),
        "{}",
        r.repolist_cell("c", &cols[1])
    );
    assert_eq!(r.repolist_cell("c", &cols[2]), "");
    git(&c, &["tag", "v1.0"]);
    assert_eq!(r.repolist_cell("c", &cols[1]), "v1.0");
    let flag = Column {
        header: "F".into(),
        width: 1,
        format: "magit-repolist-column-flag".into(),
        right_align: false,
    };
    fs::write(c.join("new"), "n").unwrap();
    assert_eq!(r.repolist_cell("c", &flag), "N");
    assert_eq!(pad("123", &cols[2]), "123");
    assert_eq!(pad("7", &cols[2]), "  7");
    // Sorted by Path; without magit-repository-directories, upstream's error.
    let (_, rows) = table_in(&[(top.path().to_path_buf(), 2)]).unwrap();
    let paths: Vec<&std::path::PathBuf> = rows.iter().map(|r| &r.1).collect();
    assert_eq!(paths.len(), 3);
    assert!(paths.windows(2).all(|w| w[0] <= w[1]), "{paths:?}");
    assert!(
        table_in(&[])
            .unwrap_err()
            .contains("magit-repository-directories")
    );
}
#[test]
fn trash_keeps_earlier_trashed_files() {
    let d = tempfile::tempdir().unwrap();
    let (files, info) = (d.path().join("files"), d.path().join("info"));
    for content in ["one", "two"] {
        fs::write(d.path().join("f"), content).unwrap();
        super::apply::trash_into(&d.path().join("f"), &files, Some(&info)).unwrap();
        assert!(!d.path().join("f").exists());
    }
    assert_eq!(fs::read_to_string(files.join("f")).unwrap(), "one");
    assert_eq!(fs::read_to_string(files.join("f.~1~")).unwrap(), "two");
    assert!(
        fs::read_to_string(info.join("f.~1~.trashinfo"))
            .unwrap()
            .contains("Path=")
    );
}
#[test]
fn rename_offers_to_rename_the_push_target() {
    use super::Question as Q;
    use super::branch::{Next, Op};
    let (d, r, _bare) = with_remote();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    git(d.path(), &["branch", "old"]);
    git(d.path(), &["push", "-q", "origin", "old"]);
    git(d.path(), &["config", "branch.old.pushRemote", "origin"]);
    let Next::Ask(Q::Branch(op), p, _) = r.branch_step(Op::Rename, &s(&["old", "new"]), &[]) else {
        panic!()
    };
    assert!(p[0].contains("on \"origin\""), "{p:?}");
    assert_eq!(r.config("branch.new.pushRemote").as_deref(), Some("origin"));
    let Next::Git(argv) = r.branch_step(op.clone(), &s(&["y"]), &[]) else {
        panic!()
    };
    assert_eq!(
        argv,
        s(&[
            "push",
            "-v",
            "origin",
            "refs/remotes/origin/old:refs/heads/new",
            ":refs/heads/old"
        ])
    );
    r.run(&argv.iter().map(Into::into).collect::<Vec<_>>(), None)
        .unwrap();
    assert!(matches!(
        r.branch_step(op, &s(&["n"]), &[]),
        Next::Done(Ok(_))
    ));
}
#[test]
fn clone_names_become_urls() {
    use super::clone::name_to_url;
    let cfg = |k: &str| (k == "github.user").then(|| "me".to_owned());
    assert_eq!(
        name_to_url("magit/magit", cfg).unwrap(),
        "git@github.com:magit/magit.git"
    );
    assert_eq!(
        name_to_url("gh:fred", cfg).unwrap(),
        "git@github.com:me/fred.git"
    );
    assert_eq!(
        name_to_url("gl:a/b", cfg).unwrap(),
        "git@gitlab.com:a/b.git"
    );
    assert_eq!(name_to_url("sh:~x/y", cfg).unwrap(), "git@git.sr.ht:~x/y");
    assert!(
        name_to_url("gl:solo", cfg)
            .unwrap_err()
            .contains("gitlab.user")
    );
    let re = super::options::emacs_regex(r"\`\(?:a\|b\)\([^:]+\)\'");
    assert_eq!(re, r"\A(?:a|b)([^:]+)\z");
}
#[test]
fn revision_buffer_layout_follows_magit_revision_mode() {
    let (d, r) = setup();
    committed(d.path(), b"one\n");
    git(d.path(), &["tag", "v1"]);
    fs::write(d.path().join("g"), "g\n").unwrap();
    git(d.path(), &["add", "g"]);
    git(d.path(), &["commit", "-qm", "second\n\nbody line"]);
    git(d.path(), &["tag", "v2"]);
    git(d.path(), &["notes", "add", "-m", "a note", "HEAD"]);
    let id = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "HEAD~0"]))
        .trim()
        .to_owned();
    let text = String::from_utf8_lossy(&r.commit_patch(&id).unwrap()).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].ends_with(&id) && lines[0].contains("tag: v2"),
        "{text}"
    );
    assert!(lines[1].starts_with("Author:     Fred Test <"), "{text}");
    assert!(lines[2].starts_with("AuthorDate: "));
    assert!(
        lines.iter().any(|l| l.starts_with("Parent:     ")),
        "{text}"
    );
    assert!(lines.contains(&"Contained:  main"), "{text}");
    assert!(lines.contains(&"Follows:    v2 (0)"), "{text}");
    let blank = lines.iter().position(|l| l.is_empty()).unwrap();
    assert_eq!(&lines[blank + 1..blank + 4], &["second", "", "body line"]);
    assert!(text.contains("Notes:\n    a note"), "{text}");
    assert!(lines.iter().any(|l| l.starts_with(" g | 1 +")), "{text}");
    assert!(text.contains("diff --git a/g b/g"));
    // An untagged commit before v2 precedes it.
    git(d.path(), &["tag", "-d", "v1"]);
    let first = String::from_utf8_lossy(&git(d.path(), &["rev-parse", "HEAD~1"]))
        .trim()
        .to_owned();
    let text = String::from_utf8_lossy(&r.commit_patch(&first).unwrap()).into_owned();
    assert!(text.contains("Precedes:   v2 (1)"), "{text}");
}
