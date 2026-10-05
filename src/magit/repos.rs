//! magit-repos.el: magit-list-repositories over magit-repository-directories.
use super::options;
use super::repo::Repo;
use std::path::{Path, PathBuf};

/// One magit-repolist-columns entry: (HEADER WIDTH FORMAT PROPS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub header: String,
    pub width: usize,
    pub format: String,
    pub right_align: bool,
}

fn col(h: &str, w: usize, f: &str, r: bool) -> Column {
    Column {
        header: h.into(),
        width: w,
        format: f.into(),
        right_align: r,
    }
}
/// magit-repolist-columns (default: name, version, B<U, B>U, path).
pub fn columns() -> Vec<Column> {
    columns_for(
        "magit-repolist-columns",
        vec![
            col("Name", 25, "magit-repolist-column-ident", false),
            col("Version", 25, "magit-repolist-column-version", false),
            col(
                "B<U",
                3,
                "magit-repolist-column-unpulled-from-upstream",
                true,
            ),
            col("B>U", 3, "magit-repolist-column-unpushed-to-upstream", true),
            col("Path", 99, "magit-repolist-column-path", false),
        ],
    )
}
/// magit-submodule-list-columns.
pub fn module_columns() -> Vec<Column> {
    columns_for(
        "magit-submodule-list-columns",
        vec![
            col("Path", 25, "magit-modulelist-column-path", false),
            col("Version", 25, "magit-repolist-column-version", false),
            col("Branch", 20, "magit-repolist-column-branch", false),
            col(
                "B<P",
                3,
                "magit-repolist-column-unpulled-from-pushremote",
                true,
            ),
            col(
                "B<U",
                3,
                "magit-repolist-column-unpulled-from-upstream",
                true,
            ),
            col(
                "B>P",
                3,
                "magit-repolist-column-unpushed-to-pushremote",
                true,
            ),
            col("B>U", 3, "magit-repolist-column-unpushed-to-upstream", true),
            col("S", 3, "magit-repolist-column-stashes", true),
            col("B", 3, "magit-repolist-column-branches", true),
        ],
    )
}
/// A columns option ([[HEADER, WIDTH, FUNCTION, PROPS]]), or DEFAULT.
fn columns_for(option: &str, default: Vec<Column>) -> Vec<Column> {
    let parsed: Option<Vec<Column>> = options::value(option)
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let c = c.as_array()?;
                    let props = c.get(3).and_then(|p| p.as_array());
                    let right = props.is_some_and(|p| {
                        p.iter().any(|kv| {
                            kv.as_array().is_some_and(|kv| {
                                kv.first().and_then(|k| k.as_str()) == Some(":right-align")
                                    && kv.get(1).and_then(|v| v.as_bool()) != Some(false)
                            })
                        })
                    });
                    Some(col(
                        c.first()?.as_str()?,
                        c.get(1)?.as_integer()?.max(1) as usize,
                        c.get(2)?.as_str()?,
                        right,
                    ))
                })
                .collect()
        });
    parsed.filter(|c| !c.is_empty()).unwrap_or(default)
}

/// magit-repository-directories: [[DIRECTORY, DEPTH], ...] (or a table).
pub fn directories() -> Vec<(PathBuf, usize)> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let expand = |d: &str| match d.strip_prefix("~/").or((d == "~").then_some("")) {
        Some(rest) => home.join(rest),
        None => PathBuf::from(d),
    };
    match options::value("magit-repository-directories") {
        Some(toml::Value::Array(a)) => a
            .iter()
            .filter_map(|e| match e {
                toml::Value::Array(p) => Some((
                    expand(p.first()?.as_str()?),
                    p.get(1).and_then(|n| n.as_integer()).unwrap_or(0).max(0) as usize,
                )),
                toml::Value::String(d) => Some((expand(d), 0)),
                _ => None,
            })
            .collect(),
        Some(toml::Value::Table(t)) => t
            .iter()
            .map(|(d, n)| (expand(d), n.as_integer().unwrap_or(0).max(0) as usize))
            .collect(),
        _ => vec![],
    }
}

/// magit-list-repos-1: DIRECTORY if it has a .git, else its
/// subdirectories down to DEPTH.
pub fn list(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join(".git").exists() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for d in subdirs {
        list(&d, depth - 1, out);
    }
}

/// magit-list-repos-uniquify: basenames, with parent directory names
/// appended (separated by \) where they collide.
pub fn uniquify(paths: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut paths = paths.to_vec();
    paths.dedup();
    let mut out = vec![];
    let name = |p: &Path, n: usize| -> String {
        let parts: Vec<String> = p
            .components()
            .rev()
            .take(n)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        parts.join("\\")
    };
    for p in &paths {
        let mut n = 1;
        while n < 32
            && paths
                .iter()
                .filter(|q| *q != p)
                .any(|q| name(q, n) == name(p, n))
        {
            n += 1;
        }
        out.push((name(p, n), p.clone()));
    }
    out
}

impl Repo {
    fn upstream_counts(&self, push: bool) -> Option<(usize, usize)> {
        let target = if push { "@{push}" } else { "@{upstream}" };
        let out = self
            .read(&[
                "rev-list",
                "--count",
                "--left-right",
                &format!("HEAD...{target}"),
            ])
            .ok()?;
        let s = String::from_utf8_lossy(&out);
        let mut n = s.split_whitespace().map(|x| x.parse().unwrap_or(0));
        Some((n.next()?, n.next()?))
    }
    /// The value of a magit-repolist column function for this repository.
    pub fn repolist_cell(&self, id: &str, col: &Column) -> String {
        let count = |n: usize| {
            if n > 9 && col.width == 1 {
                "+".into()
            } else {
                n.to_string()
            }
        };
        let flag = |what: &str| -> bool {
            let args: &[&str] = match what {
                "N" => &["ls-files", "--others", "--exclude-standard", "--directory"],
                "U" => &["diff", "--name-only"],
                _ => &["diff", "--cached", "--name-only"],
            };
            self.read(args).is_ok_and(|o| !o.is_empty())
        };
        let flags = || {
            options::value("magit-repolist-column-flag-alist")
                .and_then(|v| v.as_array().cloned())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| {
                            let p = p.as_array()?;
                            let f = match p.first()?.as_str()? {
                                "magit-untracked-files" => "N",
                                "magit-unstaged-files" => "U",
                                "magit-staged-files" => "S",
                                _ => return None,
                            };
                            Some((f, p.get(1)?.as_str()?.to_owned()))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![("N", "N".into()), ("U", "U".into()), ("S", "S".into())])
        };
        match col.format.as_str() {
            "magit-repolist-column-ident" | "magit-modulelist-column-path" => id.to_owned(),
            "magit-repolist-column-path" => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                match home.and_then(|h| self.root.strip_prefix(h).ok().map(Path::to_path_buf)) {
                    Some(rest) => format!("~/{}", rest.display()),
                    None => self.root.display().to_string(),
                }
            }
            "magit-repolist-column-version" => {
                let v = self
                    .read(&["describe", "--tags", "--dirty"])
                    .or_else(|_| {
                        self.read(&["log", "-1", "--format=%cd-g%h", "--date=format:%Y%m%d.%H%M"])
                    })
                    .map(|o| String::from_utf8_lossy(&o).trim().to_owned())
                    .unwrap_or_default();
                if v.starts_with(|c: char| c.is_ascii_digit()) {
                    format!(" {v}")
                } else {
                    v
                }
            }
            "magit-repolist-column-branch" => self
                .current_branch()
                .unwrap_or_else(|_| "(detached)".into()),
            "magit-repolist-column-upstream" => self
                .current_branch()
                .ok()
                .and_then(|b| self.upstream_of(&b))
                .unwrap_or_default(),
            "magit-repolist-column-unpulled-from-upstream" => self
                .upstream_counts(false)
                .map(|(_, b)| count(b))
                .unwrap_or_default(),
            "magit-repolist-column-unpushed-to-upstream" => self
                .upstream_counts(false)
                .map(|(a, _)| count(a))
                .unwrap_or_default(),
            "magit-repolist-column-unpulled-from-pushremote" => self
                .upstream_counts(true)
                .map(|(_, b)| count(b))
                .unwrap_or_default(),
            "magit-repolist-column-unpushed-to-pushremote" => self
                .upstream_counts(true)
                .map(|(a, _)| count(a))
                .unwrap_or_default(),
            "magit-repolist-column-branches" => self
                .read(&["for-each-ref", "--format=x", "refs/heads"])
                .map(|o| count(o.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count()))
                .unwrap_or_default(),
            "magit-repolist-column-stashes" => {
                self.stashes().map(|s| count(s.len())).unwrap_or_default()
            }
            "magit-repolist-column-flag" => flags()
                .into_iter()
                .find(|(k, _)| flag(k))
                .map(|(_, f)| f)
                .unwrap_or_default(),
            "magit-repolist-column-flags" => flags()
                .into_iter()
                .map(|(k, f)| if flag(k) { f } else { " ".into() })
                .collect(),
            other => format!("?{other}"),
        }
    }
}

/// Pad a cell to its column (tabulated-list-format with 1 space between).
pub fn pad(text: &str, col: &Column) -> String {
    let text: String = text.chars().take(col.width).collect();
    if col.right_align {
        format!("{text:>w$}", w = col.width)
    } else {
        format!("{text:<w$}", w = col.width)
    }
}

/// The listed repositories as (id, path, cells), sorted by
/// magit-repolist-sort-key (["Path", false] by default).
pub fn table() -> Result<Table, String> {
    table_in(&directories())
}
type Table = (Vec<Column>, Vec<(String, PathBuf, Vec<String>)>);
/// The same for these (DIRECTORY, DEPTH) pairs.
pub fn table_in(dirs: &[(PathBuf, usize)]) -> Result<Table, String> {
    if dirs.is_empty() {
        return Err(
            "You need to customize `magit-repository-directories' before you can list repositories"
                .into(),
        );
    }
    let mut found = vec![];
    for (d, depth) in dirs {
        list(d, *depth, &mut found);
    }
    let cols = columns();
    let mut rows: Vec<(String, PathBuf, Vec<String>)> = uniquify(&found)
        .into_iter()
        .map(|(id, path)| {
            let repo = Repo { root: path.clone() };
            let cells = cols.iter().map(|c| repo.repolist_cell(&id, c)).collect();
            (id, path, cells)
        })
        .collect();
    sort_rows(&cols, &mut rows, "magit-repolist-sort-key");
    Ok((cols, rows))
}
/// Sort by a sort-key option ([COLUMN, FLIP], default ["Path", false]).
pub fn sort_rows<T>(cols: &[Column], rows: &mut [(T, PathBuf, Vec<String>)], option: &str) {
    let (key, flip) = match options::value(option) {
        Some(toml::Value::Array(a)) => (
            a.first().and_then(|k| k.as_str()).map(str::to_owned),
            a.get(1).and_then(|f| f.as_bool()).unwrap_or(false),
        ),
        Some(toml::Value::Boolean(false)) => (None, false),
        Some(toml::Value::String(s)) => (Some(s), false),
        _ => (Some("Path".into()), false),
    };
    if let Some(key) = key {
        let i = cols.iter().position(|c| c.header == key).unwrap_or(0);
        let numeric = cols.get(i).is_some_and(|c| c.right_align);
        rows.sort_by(|a, b| {
            let (x, y) = (&a.2[i], &b.2[i]);
            if numeric {
                x.trim()
                    .parse::<i64>()
                    .unwrap_or(-1)
                    .cmp(&y.trim().parse().unwrap_or(-1))
            } else {
                x.cmp(y)
            }
        });
        if flip {
            rows.reverse();
        }
    }
}
