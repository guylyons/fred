//! Words from files near the one being edited.

use super::index::words;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_FILES: usize = 200;
const MAX_TOTAL: usize = 2 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024;
const MAX_WORDS: usize = 50_000;
/// Directory entries the nearby-file search may look at.
const MAX_VISIT: usize = 5_000;

fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

fn read_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let mut data = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .read_to_end(&mut data)
        .ok()?;
    if data[..data.len().min(8192)].contains(&0) {
        return None;
    }
    String::from_utf8(data).ok()
}

/// Candidate files: siblings first, then same-extension files in the repo.
fn files(file: Option<&Path>) -> Vec<PathBuf> {
    files_limited(file, MAX_VISIT)
}

/// Like `files`, visiting at most `max_visit` directory entries in total
/// (a git repo at `~` would otherwise mean walking the whole home directory).
fn files_limited(file: Option<&Path>, max_visit: usize) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let abs = file.map(|f| {
        if f.is_absolute() {
            f.to_path_buf()
        } else {
            cwd.join(f)
        }
    });
    let dir = abs
        .as_deref()
        .and_then(Path::parent)
        .map_or(cwd.clone(), Path::to_path_buf);
    let me = abs.as_ref().and_then(|a| std::fs::canonicalize(a).ok());
    let not_me = |p: &Path| me.as_deref() != std::fs::canonicalize(p).ok().as_deref();
    let walk = |root: &Path, depth: Option<usize>| {
        ignore::WalkBuilder::new(root)
            .max_depth(depth)
            .require_git(false)
            .sort_by_file_name(|a, b| a.cmp(b))
            .build()
            .filter_map(Result::ok)
    };
    let mut out = vec![];
    let mut visited = 0;
    for e in walk(&dir, Some(1)) {
        visited += 1;
        if visited > max_visit {
            return out;
        }
        if e.file_type().is_some_and(|t| t.is_file()) && not_me(e.path()) {
            out.push(e.into_path());
        }
    }
    let ext = abs
        .as_deref()
        .and_then(Path::extension)
        .map(|e| e.to_os_string());
    if let (Some(root), Some(ext)) = (repo_root(&dir), ext) {
        for e in walk(&root, None) {
            visited += 1;
            if out.len() >= MAX_FILES * 4 || visited > max_visit {
                break;
            }
            let p = e.path();
            if e.file_type().is_some_and(|t| t.is_file())
                && p.extension() == Some(ext.as_os_str())
                && p.parent() != Some(dir.as_path())
                && not_me(p)
            {
                out.push(e.into_path());
            }
        }
    }
    out
}

/// Collect deduplicated words from nearby files.
pub fn collect(file: Option<&Path>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    let mut total = 0;
    for p in files(file).into_iter().take(MAX_FILES) {
        let Some(text) = read_text(&p) else { continue };
        total += text.len();
        for w in words(&text) {
            if seen.insert(w.to_string()) {
                out.push(w.to_string());
            }
        }
        if total >= MAX_TOTAL || out.len() >= MAX_WORDS {
            break;
        }
    }
    out
}

/// Collect on a background thread; the list fills in when it is done.
pub fn spawn(file: Option<PathBuf>) -> Arc<Mutex<Vec<String>>> {
    let shared = Arc::new(Mutex::new(vec![]));
    let out = Arc::clone(&shared);
    std::thread::spawn(move || {
        let words = collect(file.as_deref());
        if let Ok(mut g) = out.lock() {
            *g = words;
        }
    });
    shared
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn repo_walk_is_bounded() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir(root.join("a")).unwrap();
        for i in 0..50 {
            fs::write(root.join(format!("a/f{i:02}.txt")), "x").unwrap();
        }
        fs::create_dir(root.join("z")).unwrap();
        fs::write(root.join("z/deep.rs"), "deepword").unwrap();
        let me = root.join("main.rs");
        let found = |limit| files_limited(Some(&me), limit).iter().any(|p| p.ends_with("z/deep.rs"));
        assert!(found(MAX_VISIT), "found within the normal limit");
        assert!(!found(20), "the walk stops after the visit limit");
    }

    #[test]
    fn collects_from_siblings_and_repo() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/sibling.txt"), "siblingword").unwrap();
        fs::write(root.join("src/deep/other.rs"), "deepword").unwrap();
        fs::write(root.join("src/deep/other.py"), "pythonword").unwrap();
        fs::write(root.join("ignored.rs"), "ignoredword").unwrap();
        fs::write(root.join("src/bin.dat"), b"binaryword\0\0").unwrap();
        let w = collect(Some(&root.join("src/main.rs")));
        assert!(w.contains(&"siblingword".to_string()));
        assert!(w.contains(&"deepword".to_string()));
        assert!(
            !w.contains(&"pythonword".to_string()),
            "other extensions outside the dir are skipped"
        );
        assert!(!w.contains(&"ignoredword".to_string()));
        assert!(!w.contains(&"binaryword".to_string()));
        assert!(
            !w.contains(&"main".to_string()),
            "the file itself is not a nearby file"
        );
    }
}
