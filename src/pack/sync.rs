//! `fred pack sync`: install what config.toml lists, at the lock file's
//! commits, and compile grammars. The only code that runs git, a compiler
//! or the network.

use super::manifest::{Grammar, Manifest, PackageEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `~/.config/fred/packages.lock`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    #[serde(default, rename = "package")]
    pub packages: Vec<Locked>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locked {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// None for a local package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

impl Lock {
    /// An unreadable or missing lock file is an empty one.
    pub fn load(path: &Path) -> Lock {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

pub enum Mode {
    /// Install at the locked commits; lock new entries at their head.
    Sync,
    /// Move the lock of every package (None) or the named one to its head.
    Update(Option<String>),
}

/// What sync did for one package, or why it couldn't.
#[derive(Debug)]
pub struct Report {
    pub name: String,
    pub result: Result<String, String>,
}

/// The grammar library's file name in a package.
pub fn library_name() -> &'static str {
    if cfg!(target_os = "macos") { "grammar.dylib" } else { "grammar.so" }
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let name = cmd.get_program().to_string_lossy().to_string();
    let out = cmd.output().map_err(|e| format!("{name}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // The first line that says what is wrong.
    let line = err.lines().find(|l| l.contains("error") || l.contains("fatal")).or(err.lines().next());
    Err(format!("{name}: {}", line.unwrap_or("failed").trim()))
}

fn git(repo: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(repo);
    c
}

/// A clone of `url` under `root/.repos`, made if missing.
fn repo(root: &Path, url: &str) -> Result<PathBuf, String> {
    let name: String = url.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let dir = root.join(".repos").join(name);
    if !dir.exists() {
        std::fs::create_dir_all(dir.parent().unwrap()).map_err(|e| e.to_string())?;
        run(Command::new("git").args(["clone", "--quiet", "--", url]).arg(&dir))?;
    }
    Ok(dir)
}

/// `rev` as a commit of `repo`, fetching once if it isn't there yet.
fn resolve(repo: &Path, rev: &str) -> Result<String, String> {
    let parse = || run(git(repo).args(["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]));
    parse().or_else(|_| {
        run(git(repo).args(["fetch", "--quiet", "--tags", "origin"]))?;
        parse().map_err(|_| format!("no commit {rev} in the repository"))
    })
}

/// The files of `sub` (or everything) at `commit`, written into `to`.
fn export(repo: &Path, commit: &str, sub: Option<&str>, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let tar = to.join(".export.tar");
    let mut archive = git(repo);
    archive.args(["archive", "--format=tar", "-o"]).arg(&tar).arg(commit);
    if let Some(sub) = sub {
        archive.arg(sub);
    }
    run(&mut archive)?;
    let strip = sub.map_or(0, |s| Path::new(s).components().count());
    run(Command::new("tar").arg("-xf").arg(&tar).arg("-C").arg(to).arg(format!("--strip-components={strip}")))?;
    std::fs::remove_file(tar).map_err(|e| e.to_string())
}

/// Compile `src_dir`'s `parser.c` (and `scanner.c`) into `lib`.
pub fn compile(src_dir: &Path, lib: &Path) -> Result<(), String> {
    if src_dir.join("scanner.cc").exists() {
        return Err("a C++ scanner (scanner.cc) is not supported".into());
    }
    let cc = std::env::var("CC").ok().filter(|c| !c.is_empty()).unwrap_or_else(|| "cc".into());
    let mut cmd = Command::new(cc);
    cmd.args(["-shared", "-fPIC", "-O2", "-I"]).arg(src_dir).arg(src_dir.join("parser.c"));
    if src_dir.join("scanner.c").exists() {
        cmd.arg(src_dir.join("scanner.c"));
    }
    run(cmd.arg("-o").arg(lib)).map(|_| ())
}

/// Fetch and compile `pkg`'s grammar, unless it is already built from this rev.
fn build_grammar(root: &Path, pkg: &Path, g: &Grammar) -> Result<bool, String> {
    let (lib, stamp) = (pkg.join(library_name()), pkg.join(".grammar-rev"));
    let want = format!("{} {} {}", g.source, g.rev, g.dir);
    if lib.exists() && std::fs::read_to_string(&stamp).is_ok_and(|s| s == want) {
        return Ok(false);
    }
    let source = repo(root, &g.source)?;
    let commit = resolve(&source, &g.rev)?;
    let src = pkg.join(".grammar");
    let _ = std::fs::remove_dir_all(&src);
    export(&source, &commit, Some(&g.dir), &src)?;
    compile(&src, &lib)?;
    std::fs::write(stamp, want).map_err(|e| e.to_string())?;
    Ok(true)
}

fn remove(path: &Path) {
    // A local package is a symlink: remove the link, not what it points to.
    if path.is_symlink() {
        let _ = std::fs::remove_file(path);
    } else {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn install(e: &PackageEntry, lock: &Lock, root: &Path, mode: &Mode) -> Result<(Locked, String), (String, String)> {
    let label = e.label();
    let fail = |name: &str| { let name = name.to_string(); move |err: String| (name, err) };
    e.check().map_err(fail(&label))?;
    if let Some(local) = &e.local {
        let dir = e.path.as_ref().map_or(local.clone(), |p| local.join(p));
        let m = Manifest::load(&dir).map_err(fail(&label))?;
        let to = root.join(&m.name);
        remove(&to);
        std::os::unix::fs::symlink(&dir, &to).map_err(|e| e.to_string()).map_err(fail(&m.name))?;
        let built = m.grammar.as_ref().map_or(Ok(false), |g| build_grammar(root, &dir, g)).map_err(fail(&m.name))?;
        let locked = Locked { name: m.name, git: None, local: Some(local.clone()), path: e.path.clone(), commit: None };
        return Ok((locked, if built { "linked, grammar compiled" } else { "linked" }.into()));
    }
    let url = e.git.as_deref().unwrap();
    let source = repo(root, url).map_err(fail(&label))?;
    let was = lock.packages.iter().find(|l| l.git.as_deref() == Some(url) && l.path == e.path);
    let update = match mode {
        Mode::Sync => false,
        Mode::Update(None) => true,
        Mode::Update(Some(n)) => was.is_some_and(|l| &l.name == n),
    };
    let name = was.map_or(label.clone(), |l| l.name.clone());
    let commit = match was.and_then(|l| l.commit.clone()).filter(|_| !update) {
        Some(c) => resolve(&source, &c),
        None => run(git(&source).args(["fetch", "--quiet", "origin"]))
            .and_then(|_| run(git(&source).args(["rev-parse", "origin/HEAD"]))),
    }
    .map_err(fail(&name))?;
    let file = e.path.as_ref().map_or("package.toml".to_string(), |p| format!("{p}/package.toml"));
    let m = run(git(&source).args(["show", &format!("{commit}:{file}")]))
        .map_err(|_| format!("no {file} at {}", &commit[..commit.len().min(8)]))
        .and_then(|t| Manifest::parse(&t))
        .map_err(fail(&name))?;
    let to = root.join(&m.name);
    let locked = Locked { name: m.name.clone(), git: Some(url.into()), local: None, path: e.path.clone(), commit: Some(commit.clone()) };
    let installed = !to.is_symlink() && std::fs::read_to_string(to.join(".commit")).is_ok_and(|c| c == commit);
    if installed {
        let built = m.grammar.as_ref().map_or(Ok(false), |g| build_grammar(root, &to, g)).map_err(fail(&m.name))?;
        return Ok((locked, if built { "grammar compiled" } else { "up to date" }.into()));
    }
    // Build beside the installed copy, so a failure leaves that one alone.
    let tmp = root.join(format!(".new-{}", m.name));
    let _ = std::fs::remove_dir_all(&tmp);
    let built = export(&source, &commit, e.path.as_deref(), &tmp)
        .and_then(|_| m.grammar.as_ref().map_or(Ok(false), |g| build_grammar(root, &tmp, g)))
        .and_then(|_| std::fs::write(tmp.join(".commit"), &commit).map_err(|e| e.to_string()));
    if let Err(err) = built {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err((m.name, err));
    }
    remove(&to);
    std::fs::rename(&tmp, &to).map_err(|e| e.to_string()).map_err(fail(&m.name))?;
    Ok((locked, format!("installed {}", &commit[..commit.len().min(8)])))
}

/// Bring `root` in line with `entries`, updating `lock`. A package that
/// fails keeps its lock entry and whatever was installed.
pub fn sync(entries: &[PackageEntry], lock: &mut Lock, root: &Path, mode: Mode) -> Vec<Report> {
    if let Err(e) = std::fs::create_dir_all(root) {
        return vec![Report { name: root.display().to_string(), result: Err(e.to_string()) }];
    }
    let mut next = Lock::default();
    let mut reports = vec![];
    for e in entries {
        match install(e, lock, root, &mode) {
            Ok((locked, did)) => {
                reports.push(Report { name: locked.name.clone(), result: Ok(did) });
                next.packages.push(locked);
            }
            Err((name, err)) => {
                let same = |l: &&Locked| l.git == e.git && l.local == e.local && l.path == e.path;
                next.packages.extend(lock.packages.iter().find(same).cloned());
                reports.push(Report { name, result: Err(err) });
            }
        }
    }
    // Only what an earlier sync installed and the config no longer lists.
    for old in &lock.packages {
        if !next.packages.iter().any(|l| l.name == old.name) {
            remove(&root.join(&old.name));
            reports.push(Report { name: old.name.clone(), result: Ok("removed".into()) });
        }
    }
    *lock = next;
    reports
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/grammar-json");

    fn sh(dir: &Path, script: &str) {
        let ok = Command::new("sh").arg("-c").arg(script).current_dir(dir).output().unwrap();
        assert!(ok.status.success(), "{script}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    fn commit(dir: &Path) -> String {
        sh(dir, "git add -A && git -c user.name=t -c user.email=t@t commit -qm x");
        run(git(dir).args(["rev-parse", "HEAD"])).unwrap()
    }

    /// A grammar source repository and a package repository naming it.
    pub(crate) struct World {
        pub dir: tempfile::TempDir,
        pub root: PathBuf,
        pub pkgs: PathBuf,
        pub entry: PackageEntry,
    }

    pub(crate) fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let (grammar, pkgs) = (dir.path().join("grammar"), dir.path().join("pkgs"));
        std::fs::create_dir_all(pkgs.join("lang/json/queries")).unwrap();
        sh(dir.path(), &format!("git init -q grammar && cp -R {FIXTURE}/src grammar/ && git init -q pkgs"));
        let rev = commit(&grammar);
        let manifest = format!(
            "name = \"lang-json\"\n[grammar]\nname = \"json\"\nsource = \"{}\"\nrev = \"{rev}\"\nstatus_name = \"JSON\"\nextensions = [\"jsonx\"]\n",
            grammar.display()
        );
        std::fs::write(pkgs.join("lang/json/package.toml"), manifest).unwrap();
        std::fs::write(pkgs.join("lang/json/queries/highlights.scm"), "(string) @string\n(number) @number\n").unwrap();
        std::fs::write(pkgs.join("lang/json/queries/folds.scm"), "[(object) (array)] @fold\n").unwrap();
        commit(&pkgs);
        let entry = PackageEntry { git: Some(pkgs.display().to_string()), local: None, path: Some("lang/json".into()) };
        World { root: dir.path().join("root"), pkgs, entry, dir }
    }

    fn done(r: &[Report]) -> Vec<(String, String)> {
        r.iter().map(|r| (r.name.clone(), r.result.clone().unwrap_or_else(|e| format!("ERR {e}")))).collect()
    }

    #[test]
    fn sync_installs_locks_and_then_does_nothing() {
        let w = world();
        let mut lock = Lock::default();
        let r = done(&sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync));
        assert!(r[0].0 == "lang-json" && r[0].1.starts_with("installed "), "{r:?}");
        let pkg = w.root.join("lang-json");
        assert!(pkg.join("package.toml").exists() && pkg.join("queries/folds.scm").exists());
        assert_eq!(lock.packages[0].commit.as_deref().map(str::len), Some(40));
        // The library exports the grammar's symbol.
        let lib = unsafe { libloading::Library::new(pkg.join(library_name())).unwrap() };
        assert!(unsafe { lib.get::<unsafe extern "C" fn()>(b"tree_sitter_json").is_ok() });
        let r = done(&sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync));
        assert_eq!(r, [("lang-json".to_string(), "up to date".to_string())]);
        // The lock file round-trips.
        let file = w.dir.path().join("packages.lock");
        lock.save(&file).unwrap();
        assert_eq!(Lock::load(&file), lock);
    }

    #[test]
    fn sync_stays_at_the_lock_and_update_moves_it() {
        let w = world();
        let mut lock = Lock::default();
        sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync);
        let first = lock.packages[0].commit.clone();
        std::fs::write(w.pkgs.join("lang/json/queries/folds.scm"), "(object) @fold\n").unwrap();
        let head = commit(&w.pkgs);
        sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync);
        assert_eq!(lock.packages[0].commit, first);
        sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Update(Some("other".into())));
        assert_eq!(lock.packages[0].commit, first, "another package's update");
        let r = done(&sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Update(None)));
        assert_eq!(lock.packages[0].commit, Some(head));
        assert!(r[0].1.starts_with("installed "), "{r:?}");
        let folds = std::fs::read_to_string(w.root.join("lang-json/queries/folds.scm")).unwrap();
        assert_eq!(folds, "(object) @fold\n");
    }

    #[test]
    fn missing_commit_keeps_the_installed_copy() {
        let w = world();
        let mut lock = Lock::default();
        sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync);
        lock.packages[0].commit = Some("0123456789012345678901234567890123456789".into());
        let r = done(&sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync));
        assert!(r[0].1.starts_with("ERR no commit 01234567"), "{r:?}");
        assert!(w.root.join("lang-json/package.toml").exists());
        assert_eq!(lock.packages.len(), 1, "the lock entry stays");
    }

    #[test]
    fn sync_removes_only_unlisted_locked_packages() {
        let w = world();
        let mut lock = Lock::default();
        sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync);
        std::fs::create_dir_all(w.root.join("mine")).unwrap();
        let r = done(&sync(&[], &mut lock, &w.root, Mode::Sync));
        assert_eq!(r, [("lang-json".to_string(), "removed".to_string())]);
        assert!(!w.root.join("lang-json").exists() && w.root.join("mine").exists());
        assert!(lock.packages.is_empty());
    }

    #[test]
    fn a_compiler_failure_is_that_packages_error() {
        let w = world();
        std::fs::write(w.dir.path().join("grammar/src/parser.c"), "this is not C\n").unwrap();
        let rev = commit(&w.dir.path().join("grammar"));
        let file = w.pkgs.join("lang/json/package.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        let old = text.lines().find(|l| l.starts_with("rev")).unwrap().to_string();
        std::fs::write(&file, text.replace(&old, &format!("rev = \"{rev}\""))).unwrap();
        commit(&w.pkgs);
        let mut lock = Lock::default();
        let r = done(&sync(std::slice::from_ref(&w.entry), &mut lock, &w.root, Mode::Sync));
        assert!(r[0].0 == "lang-json" && r[0].1.starts_with("ERR cc: ") && r[0].1.contains("error"), "{r:?}");
        assert!(!w.root.join("lang-json").exists() && !w.root.join(".new-lang-json").exists());
    }

    #[test]
    fn a_local_package_is_linked_and_unlinked() {
        let w = world();
        let local = PackageEntry { git: None, local: Some(w.pkgs.clone()), path: Some("lang/json".into()) };
        let mut lock = Lock::default();
        let r = done(&sync(std::slice::from_ref(&local), &mut lock, &w.root, Mode::Sync));
        assert_eq!(r, [("lang-json".to_string(), "linked, grammar compiled".to_string())]);
        assert!(w.root.join("lang-json").join(library_name()).exists());
        sync(&[], &mut lock, &w.root, Mode::Sync);
        assert!(!w.root.join("lang-json").exists());
        assert!(w.pkgs.join("lang/json/package.toml").exists(), "the user's directory stays");
    }
}
