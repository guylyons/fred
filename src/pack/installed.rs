//! The packages on disk.

use super::manifest::Manifest;
use std::path::{Path, PathBuf};

pub struct Installed {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

/// Every package under `root` with a manifest that parses, by name; the
/// others as (directory name, error).
pub fn installed(root: &Path) -> (Vec<Installed>, Vec<(String, String)>) {
    let (mut ok, mut bad) = (vec![], vec![]);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        // `.repos` and half-installed `.new-*` are sync's own.
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    dirs.sort();
    for dir in dirs {
        match Manifest::load(&dir) {
            Ok(manifest) => ok.push(Installed { manifest, dir }),
            Err(e) => bad.push((dir.file_name().unwrap().to_string_lossy().into(), e)),
        }
    }
    (ok, bad)
}
