//! The project's files, listed on a background thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_FILES: usize = 200_000;

/// The enclosing git repo of `dir`, else `cwd`.
pub fn project_root(dir: &Path, cwd: &Path) -> PathBuf {
    crate::complete::nearby::repo_root(dir).unwrap_or_else(|| cwd.to_path_buf())
}

/// Paths relative to `root`, in walk order, growing until `done`.
#[derive(Debug)]
pub struct Files {
    pub root: PathBuf,
    /// Ignore files (`.gitignore` and the like) are not honored.
    pub all: bool,
    list: Mutex<Vec<String>>,
    done: AtomicBool,
    truncated: AtomicBool,
}

/// The same list, not equal contents (a picker holding one is compared).
impl PartialEq for Files {
    fn eq(&self, other: &Files) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Files {}

impl Files {
    pub fn spawn(root: PathBuf, all: bool) -> Arc<Files> {
        let files = Arc::new(Files {
            root,
            all,
            list: Mutex::default(),
            done: AtomicBool::new(false),
            truncated: AtomicBool::new(false),
        });
        let f = Arc::clone(&files);
        std::thread::spawn(move || Files::walk(&f));
        files
    }

    /// Stops early once nobody else holds the list (the picker moved on).
    fn walk(self: &Arc<Self>) {
        let mut batch = vec![];
        let mut total = 0;
        let walk = ignore::WalkBuilder::new(&self.root)
            .require_git(false)
            .standard_filters(!self.all)
            .hidden(true)
            .sort_by_file_name(|a, b| a.cmp(b))
            .build();
        for e in walk.filter_map(Result::ok) {
            if Arc::strong_count(self) == 1 {
                return;
            }
            if !e.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            // Non-UTF-8 names are skipped rather than shown mangled.
            let Some(rel) = e
                .path()
                .strip_prefix(&self.root)
                .ok()
                .and_then(Path::to_str)
            else {
                continue;
            };
            if total == MAX_FILES {
                self.truncated.store(true, Ordering::Relaxed);
                break;
            }
            batch.push(rel.to_string());
            total += 1;
            if batch.len() == 1000 {
                self.list.lock().unwrap().append(&mut batch);
            }
        }
        self.list.lock().unwrap().append(&mut batch);
        self.done.store(true, Ordering::Release);
    }

    pub fn with<R>(&self, f: impl FnOnce(&[String]) -> R) -> R {
        f(&self.list.lock().unwrap())
    }

    pub fn len(&self) -> usize {
        self.list.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn truncated(&self) -> bool {
        self.truncated.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn wait(&self) {
        while !self.done() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_files_honoring_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("src")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        fs::write(r.join("src/main.rs"), "").unwrap();
        fs::write(r.join("target/out"), "").unwrap();
        fs::write(r.join("b.txt"), "").unwrap();
        let f = Files::spawn(r.to_path_buf(), false);
        f.wait();
        assert_eq!(f.with(<[String]>::to_vec), ["b.txt", "src/main.rs"]);
        assert!(!f.truncated());
        let f = Files::spawn(r.to_path_buf(), true);
        f.wait();
        assert_eq!(
            f.with(<[String]>::to_vec),
            ["b.txt", "src/main.rs", "target/out"]
        );
    }

    #[test]
    fn stops_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..2000 {
            fs::write(dir.path().join(format!("f{i}")), "").unwrap();
        }
        let f = Files::spawn(dir.path().to_path_buf(), false);
        let weak = Arc::downgrade(&f);
        drop(f);
        // The walker notices it is alone and lets the list go.
        let t = std::time::Instant::now();
        while weak.strong_count() > 0 {
            assert!(t.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn root_is_the_repo_else_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("repo/.git")).unwrap();
        fs::create_dir_all(r.join("repo/a/b")).unwrap();
        fs::create_dir_all(r.join("loose")).unwrap();
        assert_eq!(project_root(&r.join("repo/a/b"), r), r.join("repo"));
        assert_eq!(project_root(&r.join("loose"), &r.join("x")), r.join("x"));
    }
}
