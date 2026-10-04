//! magit-bundle.el: create (regular and tracked), update, verify and list
//! heads of bundles. Tracked bundles use upstream's tag message format.
use super::branch::Next;
use super::repo::Repo;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create,
    CreateTracked,
    UpdateTracked,
    Verify,
    ListHeads,
}

/// What a tracked bundle's tag records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tracked {
    pub file: String,
    pub branch: String,
    pub refs: Vec<String>,
    pub args: Vec<String>,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Tracked {
    /// The tag message magit-bundle-create-tracked writes (pp-to-string alist).
    pub fn message(&self) -> String {
        let list = |v: &[String]| {
            v.iter()
                .map(|x| format!(" {}", quote(x)))
                .collect::<String>()
        };
        format!(
            ";; git-bundle tracking\n((file . {})\n (branch . {})\n (refs{})\n (args{}))\n",
            quote(&self.file),
            quote(&self.branch),
            list(&self.refs),
            list(&self.args)
        )
    }
    /// Read the alist back (strings and symbols; enough for upstream's output).
    pub fn parse(text: &str) -> Option<Self> {
        #[derive(Debug)]
        enum Tok {
            Open,
            Close,
            Dot,
            Str(String),
            Sym(String),
        }
        let mut toks = vec![];
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                ';' => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                '(' => toks.push(Tok::Open),
                ')' => toks.push(Tok::Close),
                '"' => {
                    let mut s = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => s.push(chars.next()?),
                            '"' => break,
                            c => s.push(c),
                        }
                    }
                    toks.push(Tok::Str(s));
                }
                c if c.is_whitespace() => {}
                c => {
                    let mut s = c.to_string();
                    while let Some(&n) = chars.peek() {
                        if n.is_whitespace() || n == '(' || n == ')' || n == '"' {
                            break;
                        }
                        s.push(n);
                        chars.next();
                    }
                    toks.push(if s == "." { Tok::Dot } else { Tok::Sym(s) });
                }
            }
        }
        // ((key . "v") (key "a" "b") ...)
        let mut it = toks.into_iter();
        if !matches!(it.next()?, Tok::Open) {
            return None;
        }
        let mut t = Tracked::default();
        loop {
            match it.next()? {
                Tok::Close => break,
                Tok::Open => {}
                _ => return None,
            }
            let Tok::Sym(key) = it.next()? else {
                return None;
            };
            let mut values = vec![];
            let mut dotted = false;
            loop {
                match it.next()? {
                    Tok::Close => break,
                    Tok::Dot => dotted = true,
                    Tok::Str(s) => values.push(s),
                    Tok::Sym(s) if s == "nil" => {}
                    _ => return None,
                }
            }
            let one = || values.first().cloned().unwrap_or_default();
            match key.as_str() {
                "file" if dotted => t.file = one(),
                "branch" if dotted => t.branch = one(),
                "refs" => t.refs = values,
                "args" => t.args = values,
                _ => {}
            }
        }
        (!t.file.is_empty() && !t.branch.is_empty()).then_some(t)
    }
}

fn rev(v: &str) -> Result<&str, String> {
    if v.is_empty() || v.starts_with('-') || v.chars().any(char::is_control) {
        return Err(format!("invalid ref {v:?}"));
    }
    Ok(v)
}

impl Repo {
    fn bundle_file(&self, answer: &str) -> Result<PathBuf, String> {
        let answer = answer.trim();
        if answer.is_empty() || answer.chars().any(char::is_control) {
            return Err("A file name is required".into());
        }
        Ok(match answer.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
            None => self.root.join(answer),
        })
    }
    pub fn bundle_prompts(&self, op: &Op, at_point: Option<String>) -> (Vec<String>, Vec<String>) {
        let top = self
            .root
            .file_name()
            .map(|n| format!("{}.bundle", n.to_string_lossy()))
            .unwrap_or_default();
        let here = self.current_branch().unwrap_or_default();
        match op {
            Op::Create => (
                vec![
                    format!("Create bundle (default {top}): "),
                    "Refnames (zero or more, space separated): ".into(),
                ],
                vec![top, String::new()],
            ),
            Op::CreateTracked => (
                vec![
                    "Track bundle using tag: ".into(),
                    format!("Bundle branch (default {here}): "),
                    "Additional refnames (zero or more, space separated): ".into(),
                    "File (default TAG.bundle): ".into(),
                ],
                vec![String::new(), here, String::new(), String::new()],
            ),
            Op::UpdateTracked => (
                vec!["Update bundle tracked by tag: ".into()],
                vec![String::new()],
            ),
            Op::Verify | Op::ListHeads => {
                let d = at_point.unwrap_or_default();
                let verb = if *op == Op::Verify {
                    "Verify bundle"
                } else {
                    "List heads of bundle"
                };
                (vec![format!("{verb} (default {d}): ")], vec![d])
            }
        }
    }
    fn bundle_create(
        &self,
        file: &PathBuf,
        refs: &[String],
        args: &[String],
    ) -> Result<(), String> {
        let mut argv: Vec<std::ffi::OsString> = vec!["bundle".into(), "create".into(), file.into()];
        argv.extend(refs.iter().map(Into::into));
        argv.extend(args.iter().map(Into::into));
        self.run(&argv, None).map(|_| ())
    }
    pub fn bundle_step(&self, op: Op, a: &[String], args: &[String]) -> Result<Next, String> {
        let at = |i: usize| a.get(i).map(String::as_str).unwrap_or("").trim();
        let refs = |s: &str| -> Result<Vec<String>, String> {
            s.split_whitespace()
                .map(|r| rev(r).map(str::to_owned))
                .collect()
        };
        match op {
            Op::Create => {
                let file = self.bundle_file(at(0))?;
                self.bundle_create(&file, &refs(at(1))?, args)?;
                Ok(Next::Done(Ok(format!("Created {}", file.display()))))
            }
            Op::CreateTracked => {
                let tag = rev(at(0))?;
                self.read(&["check-ref-format", &format!("refs/tags/{tag}")])
                    .map_err(|_| format!("{tag:?} is not a valid tag name"))?;
                let branch = rev(at(1))?;
                let mut extra = refs(at(2))?;
                // magit-bundle-create-tracked: the current branch also bundles HEAD.
                if self.current_branch().ok().as_deref() == Some(branch) {
                    extra.insert(0, "HEAD".into());
                }
                let file = match at(3) {
                    "" => self.bundle_file(&format!("{tag}.bundle"))?,
                    f => self.bundle_file(f)?,
                };
                let mut all = vec![branch.to_owned()];
                all.extend(extra.iter().cloned());
                self.bundle_create(&file, &all, args)?;
                let tracked = Tracked {
                    file: file.to_string_lossy().into_owned(),
                    branch: branch.into(),
                    refs: extra,
                    args: args.to_vec(),
                };
                self.read(&["tag", "--force", tag, branch, "-m", &tracked.message()])?;
                Ok(Next::Done(Ok(format!(
                    "Created {} tracked by {tag}",
                    file.display()
                ))))
            }
            Op::UpdateTracked => {
                let tag = rev(at(0))?;
                let msg = String::from_utf8_lossy(&self.read(&[
                    "for-each-ref",
                    "--format=%(contents)",
                    &format!("refs/tags/{tag}"),
                ])?)
                .into_owned();
                let t = Tracked::parse(&msg)
                    .ok_or_else(|| format!("Tag {tag} does not appear to track a bundle"))?;
                rev(&t.branch)?;
                for r in &t.refs {
                    rev(r)?;
                }
                // Only arguments Fred's bundle menu offers.
                if let Some(bad) = t.args.iter().find(|x| {
                    ![
                        "--all",
                        "--branches",
                        "--tags",
                        "--remotes",
                        "--glob=",
                        "--exclude=",
                        "-n",
                        "--since=",
                        "--until=",
                    ]
                    .iter()
                    .any(|p| x.starts_with(p))
                }) {
                    return Err(format!("Tag {tag} records an unexpected argument {bad:?}"));
                }
                let mut all = vec![format!("{tag}..{}", t.branch)];
                all.extend(t.refs.iter().cloned());
                self.bundle_create(&PathBuf::from(&t.file), &all, &t.args)?;
                self.read(&["tag", "--force", tag, &t.branch, "-m", &msg])?;
                Ok(Next::Done(Ok(format!("Updated {}", t.file))))
            }
            Op::Verify => {
                let file = self.bundle_file(at(0))?;
                let argv: Vec<std::ffi::OsString> =
                    vec!["bundle".into(), "verify".into(), file.clone().into()];
                self.run(&argv, None)?;
                Ok(Next::Done(Ok(format!("{} is okay", file.display()))))
            }
            Op::ListHeads => {
                let file = self.bundle_file(at(0))?;
                Ok(Next::View(super::Kind::Output(
                    format!("bundle list-heads {}", file.display()),
                    vec![
                        "bundle".into(),
                        "list-heads".into(),
                        file.to_string_lossy().into_owned(),
                    ],
                )))
            }
        }
    }
}
