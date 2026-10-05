//! Editing source blocks and other special elements in a dedicated buffer
//! (org-src.el), and org-edit-special (C-c ').

use super::{Prefix, call, ctx, fold, syntax};
use crate::editor::Editor;
use std::path::PathBuf;

/// org-src-lang-modes plus the file extension Fred highlights with.
pub fn lang_extension(lang: &str) -> String {
    let mapped = super::sexp::option("org-src-lang-modes").and_then(|v| {
        v.list()?
            .iter()
            .find(|e| e.car().and_then(|c| c.str()) == Some(lang))
            .and_then(|e| e.cdr().sym().map(str::to_owned))
    });
    let l = mapped.as_deref().unwrap_or(lang).to_ascii_lowercase();
    match l.as_str() {
        "python" | "python3" => "py",
        "ruby" => "rb",
        "sh" | "shell" | "bash" | "zsh" | "dash" => "sh",
        "fish" => "fish",
        "emacs-lisp" | "elisp" => "el",
        "lisp" | "common-lisp" => "lisp",
        "scheme" => "scm",
        "clojure" => "clj",
        "r" => "r",
        "sqlite" | "sql" => "sql",
        "dot" | "graphviz-dot" => "dot",
        "latex" | "tex" => "tex",
        "js" | "javascript" | "node" => "js",
        "ts" | "typescript" => "ts",
        "c" => "c",
        "c++" | "cpp" => "cpp",
        "d" => "d",
        "rust" => "rs",
        "go" => "go",
        "java" => "java",
        "kotlin" => "kt",
        "haskell" => "hs",
        "ocaml" => "ml",
        "lua" => "lua",
        "perl" => "pl",
        "php" => "php",
        "css" => "css",
        "sass" => "sass",
        "html" => "html",
        "xml" => "xml",
        "json" => "json",
        "yaml" => "yaml",
        "toml" => "toml",
        "makefile" | "make" => "mk",
        "org" => "org",
        "markdown" | "md" => "md",
        "julia" => "jl",
        "octave" | "matlab" => "m",
        "gnuplot" => "gp",
        "awk" => "awk",
        "sed" => "sed",
        "fortran" | "f90" => "f90",
        "groovy" => "groovy",
        "csharp" | "c#" => "cs",
        "swift" => "swift",
        "elixir" => "ex",
        "erlang" => "erl",
        "scala" => "scala",
        "dockerfile" => "Dockerfile",
        "diff" => "diff",
        "nix" => "nix",
        other => return other.to_owned(),
    }
    .to_owned()
}

/// org-escape-code-in-string.
pub fn escape(s: &str) -> String {
    s.split('\n')
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with('*')
                || t.starts_with("#+")
                || t.starts_with(",*")
                || t.starts_with(",#+")
            {
                let i = l.len() - t.len();
                format!("{},{}", &l[..i], t)
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// org-unescape-code-in-string.
pub fn unescape(s: &str) -> String {
    s.split('\n')
        .map(|l| {
            let t = l.trim_start();
            let i = l.len() - t.len();
            if t.starts_with(",*")
                || t.starts_with(",#+")
                || t.starts_with(",,*")
                || t.starts_with(",,#+")
            {
                format!("{}{}", &l[..i], &t[1..])
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The smallest indentation of non-blank lines (org-do-remove-indentation).
fn common_indent(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0)
}

/// What is being edited.
#[derive(Clone, Debug)]
struct Datum {
    kind: &'static str,
    lang: String,
    /// First and last content lines (exclusive end), in the Org buffer.
    beg: usize,
    end: usize,
    /// Indentation of the block's #+begin line.
    block_indent: usize,
    preserve: bool,
}

fn block_datum(ed: &Editor, l: usize) -> Option<Datum> {
    // A #+begin_src / example / export / comment block around `l`.
    let (begin, end_line) = {
        let t = ed.buf.line(l).trim_start().to_ascii_lowercase();
        if let Some(rest) = t.strip_prefix("#+begin_") {
            let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            let close = format!("#+end_{name}");
            let e = (l + 1..ed.line_count())
                .find(|&i| ed.buf.line(i).trim().eq_ignore_ascii_case(&close))?;
            (l, e)
        } else if let Some(rest) = t.strip_prefix("#+end_") {
            let name = rest.trim().to_owned();
            let open = format!("#+begin_{name}");
            let b = (0..l).rev().find(|&i| {
                ed.buf
                    .line(i)
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with(&open)
            })?;
            (b, l)
        } else {
            let (_, b, e) = ctx::block_at(ed, l)?;
            (b, e)
        }
    };
    let head = ed.buf.line(begin);
    let lower = head.trim_start().to_ascii_lowercase();
    let name: String = lower[8..]
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    let kind = match name.as_str() {
        "src" => "src-block",
        "example" => "example-block",
        "export" => "export-block",
        "comment" => "comment-block",
        _ => return None,
    };
    let args: Vec<&str> = head.trim_start()[8 + name.len()..]
        .split_whitespace()
        .collect();
    let lang = match kind {
        "src-block" => args.first().copied().unwrap_or("").to_owned(),
        "export-block" => args.first().copied().unwrap_or("").to_owned(),
        "example-block" => "example".into(),
        _ => "comment".into(),
    };
    let preserve = args.windows(1).any(|w| w[0] == "-i")
        || super::options::bool("org-src-preserve-indentation", false);
    Some(Datum {
        kind,
        lang,
        beg: begin + 1,
        end: end_line,
        block_indent: head.len() - head.trim_start().len(),
        preserve,
    })
}

/// Open the edit buffer for `d`.
fn edit(ed: &mut Editor, d: Datum, write_back: bool) -> Result<(), String> {
    let lines = super::lines(ed, d.beg..d.end);
    let rel = ed.cur.line.saturating_sub(d.beg);
    let col_in = ed.cur.byte;
    let (content, removed) = if d.preserve || matches!(d.kind, "fixed-width" | "latex-environment")
    {
        (lines.join("\n"), 0)
    } else {
        let ind = common_indent(&lines);
        (
            lines
                .iter()
                .map(|l| {
                    if l.len() >= ind {
                        l[ind..].to_owned()
                    } else {
                        l.trim_start().to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            ind,
        )
    };
    let content = match d.kind {
        "src-block" | "example-block" => unescape(&content),
        "fixed-width" => content
            .split('\n')
            .map(|l| {
                let t = l.trim_start();
                t.strip_prefix(": ")
                    .or(t.strip_prefix(':'))
                    .unwrap_or(t)
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => content,
    };
    let path = ed.path.clone();
    let origin_name = path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "org".into());
    let name = format!("*Org Src {origin_name}[ {} ]*", d.lang);
    let ext = lang_extension(&d.lang);
    let content_indent = if matches!(d.kind, "src-block" | "example-block") && !d.preserve {
        super::options::int("org-src-content-indentation", 2).max(0) as usize
    } else {
        0
    };
    let indent = if d.preserve {
        0
    } else {
        d.block_indent + content_indent
    };
    let heading = ed.buf.line(d.beg.saturating_sub(1));
    let (beg, end) = (d.beg, d.end);
    let kind = d.kind;
    let cursor = (
        rel.min(content.split('\n').count().saturating_sub(1)),
        col_in.saturating_sub(removed),
    );
    super::effect(ed, move |s| {
        let origin = s.org_current();
        s.org_special(
            &name,
            &content,
            cursor,
            Box::new(move |s, text, abort| {
                if abort || !write_back {
                    return;
                }
                let new = write_back_text(&text, indent, kind);
                let apply = |ed: &mut Editor| {
                    // The block may have moved: find its opening line again.
                    let b =
                        if beg >= 1 && beg - 1 < ed.line_count() && ed.buf.line(beg - 1) == heading
                        {
                            Some(beg)
                        } else {
                            (0..ed.line_count())
                                .find(|&i| ed.buf.line(i) == heading)
                                .map(|i| i + 1)
                        };
                    let Some(b) = b else {
                        return ed.set_err("Source buffer disappeared.  Aborting");
                    };
                    let e = b + (end - beg);
                    let old = super::lines(ed, b..e);
                    if old != new {
                        ed.undo.begin(ed.cur.pos());
                        super::splice(ed, b, e - b, &new);
                        ed.undo.end(ed.cur.pos());
                    }
                };
                match &path {
                    Some(p) => {
                        let _ = s.org_with_file(p, apply);
                    }
                    None => s.org_with_buffer(origin.min(s.org_buffer_count() - 1), apply),
                }
            }),
        );
        if ext == "org" {
            fold::startup(&mut s.ed);
        } else {
            s.ed.org = None;
        }
        s.ed.syntax_path = Some(PathBuf::from(format!("org-src.{ext}")));
        s.ed.set_msg("Edit, then exit with C-c ' or abort with C-c C-k");
    });
    Ok(())
}

/// org-src--contents-for-write-back: indent and escape.
fn write_back_text(text: &str, indent: usize, kind: &str) -> Vec<String> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let body = match kind {
        "src-block" | "example-block" => escape(body),
        "fixed-width" => body
            .split('\n')
            .map(|l| {
                if l.is_empty() {
                    ":".to_owned()
                } else {
                    format!(": {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => body.to_owned(),
    };
    if body.is_empty() {
        return vec![];
    }
    let pad = " ".repeat(indent);
    body.split('\n')
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect()
}

/// org-edit-src-save: write back, then keep editing at the same place.
fn save(ed: &mut Editor) -> Result<(), String> {
    if !ed
        .org_buffer_name
        .as_deref()
        .is_some_and(|n| n.starts_with("*Org Src"))
    {
        return Err("Not in a sub-editing buffer".into());
    }
    let cur = ed.cur;
    super::effect(ed, move |s| {
        s.org_finish(false);
        super::run(&mut s.ed, "org-edit-special", Prefix::None);
        if let Some(e) = s.ed.pending_effect.take() {
            s.perform(e)
        }
        s.ed.cur = cur;
        s.ed.clamp_cursor();
    });
    Ok(())
}

/// org-edit-special.
pub fn edit_special(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if ed.readonly {
        return Err("Buffer is read-only".into());
    }
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let t = line.trim_start();
    if let Some(d) = block_datum(ed, l) {
        if d.kind == "src-block" && !arg.is_none() {
            return call(ed, "org-babel-switch-to-session", arg);
        }
        return edit(ed, d, true);
    }
    if let Some((k, v)) = syntax::keyword_line(&line) {
        if matches!(k.as_str(), "INCLUDE" | "SETUPFILE" | "BIBLIOGRAPHY") {
            let file = v.trim();
            let file = if let Some(rest) = file.strip_prefix('"') {
                rest.split('"').next().unwrap_or("")
            } else {
                file.split_whitespace().next().unwrap_or("")
            };
            if file.is_empty() {
                return Err("No file to edit".into());
            }
            if file.contains("://") {
                return Err("Files located with a URL cannot be edited".into());
            }
            return super::links::open_file(ed, file, None, Prefix::None, Some("emacs"));
        }
        return Err("No special environment to edit here".into());
    }
    if ctx::at_table(ed, l) {
        return call(ed, "org-table-edit-formulas", arg);
    }
    if ctx::at_table_el(ed, l) {
        return call(ed, "org-edit-table.el", arg);
    }
    if t.starts_with(": ") || t == ":" {
        let mut b = l;
        while b > 0 && {
            let x = ed.buf.line(b - 1);
            let x = x.trim_start();
            x.starts_with(": ") || x == ":"
        } {
            b -= 1;
        }
        let mut e = l + 1;
        while e < ed.line_count() && {
            let x = ed.buf.line(e);
            let x = x.trim_start();
            x.starts_with(": ") || x == ":"
        } {
            e += 1;
        }
        let ind = {
            let h = ed.buf.line(b);
            h.len() - h.trim_start().len()
        };
        return edit(
            ed,
            Datum {
                kind: "fixed-width",
                lang: "Fixed Width".into(),
                beg: b,
                end: e,
                block_indent: ind,
                preserve: false,
            },
            true,
        );
    }
    if t.to_ascii_lowercase().starts_with("\\begin{") || ctx::in_block(ed, l) && false {
        let env: String = t[7..].chars().take_while(|&c| c != '}').collect();
        let close = format!("\\end{{{env}}}");
        let e = (l..ed.line_count())
            .find(|&i| ed.buf.line(i).trim().starts_with(&close))
            .ok_or("Not in a LaTeX environment")?;
        let ind = line.len() - t.len();
        return edit(
            ed,
            Datum {
                kind: "latex-environment",
                lang: "latex".into(),
                beg: l,
                end: e + 1,
                block_indent: ind,
                preserve: true,
            },
            true,
        );
    }
    if l > 0 && ctx::at_planning(ed, l) {
        let deadline = line.contains("DEADLINE:");
        let scheduled = line.contains("SCHEDULED:");
        if deadline {
            call(ed, "org-deadline", Prefix::None)?;
        }
        if scheduled {
            call(ed, "org-schedule", Prefix::None)?;
        }
        return Ok(());
    }
    if let Some(r) = ctx::timestamp_at(ed, l, ed.cur.byte) {
        let inactive = line[r].starts_with('[');
        return call(
            ed,
            if inactive {
                "org-timestamp-inactive"
            } else {
                "org-timestamp"
            },
            Prefix::None,
        );
    }
    if ctx::footnote_at(ed, l, ed.cur.byte).is_some() {
        return call(ed, "org-edit-footnote-reference", arg);
    }
    if let Some(lk) = super::links::link_at(ed) {
        return super::links::open(ed, &lk, Prefix::None);
    }
    if line[..ed.cur.byte.min(line.len())].contains("src_") && line.contains('{') {
        return edit_inline(ed);
    }
    Err("No special environment to edit here".into())
}

/// org-edit-inline-src-code.
fn edit_inline(ed: &mut Editor) -> Result<(), String> {
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let start = line[..ed.cur.byte.min(line.len()) + 1]
        .rfind("src_")
        .ok_or("Not on inline source code")?;
    let open = line[start..]
        .find('{')
        .map(|i| start + i)
        .ok_or("Not on inline source code")?;
    let close = line[open..]
        .find('}')
        .map(|i| open + i)
        .ok_or("Not on inline source code")?;
    let lang: String = line[start + 4..]
        .chars()
        .take_while(|c| !"[{".contains(*c))
        .collect();
    let code = line[open + 1..close].to_owned();
    let ext = lang_extension(&lang);
    let path = ed.path.clone();
    let name = format!(
        "*Org Src {}[ {lang} ]*",
        path.as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let original = line.clone();
    super::effect(ed, move |s| {
        let origin = s.org_current();
        s.org_special(
            &name,
            &code,
            (0, 0),
            Box::new(move |s, text, abort| {
                if abort {
                    return;
                }
                let new_code = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let apply = |ed: &mut Editor| {
                    if let Some(i) = (0..ed.line_count()).find(|&i| ed.buf.line(i) == original) {
                        let mut t = original.clone();
                        t.replace_range(open + 1..close, &new_code);
                        ed.undo.begin(ed.cur.pos());
                        super::set_line(ed, i, &t);
                        ed.undo.end(ed.cur.pos());
                    }
                };
                match &path {
                    Some(p) => {
                        let _ = s.org_with_file(p, apply);
                    }
                    None => s.org_with_buffer(origin.min(s.org_buffer_count() - 1), apply),
                }
            }),
        );
        s.ed.org = None;
        s.ed.syntax_path = Some(PathBuf::from(format!("org-src.{ext}")));
    });
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let in_src = ed
        .org_buffer_name
        .as_deref()
        .is_some_and(|n| n.starts_with("*Org Src"));
    Some(match name {
        "org-edit-special" => edit_special(ed, arg),
        "org-edit-src-code" | "org-edit-export-block" | "org-edit-comment-block" => {
            match block_datum(ed, ed.cur.line) {
                Some(d) => edit(ed, d, true),
                None => Err("Not in a source or example block".into()),
            }
        }
        "org-edit-inline-src-code" => edit_inline(ed),
        "org-edit-fixed-width-region" | "org-edit-latex-environment" => edit_special(ed, arg),
        "org-edit-src-exit" if in_src => {
            super::effect(ed, |s| s.org_finish(false));
            Ok(())
        }
        "org-edit-src-abort" if in_src => {
            super::effect(ed, |s| s.org_finish(true));
            Ok(())
        }
        "org-edit-src-save" => save(ed),
        "org-edit-src-exit" | "org-edit-src-abort" => Err("Not in a sub-editing buffer".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(escape("* a\n#+x\nok\n,* b"), ",* a\n,#+x\nok\n,,* b");
        assert_eq!(unescape(",* a\n,#+x\nok\n,,* b"), "* a\n#+x\nok\n,* b");
        assert_eq!(
            write_back_text("a\n\n  b\n", 2, "src-block"),
            vec!["  a", "", "    b"]
        );
        assert_eq!(lang_extension("python"), "py");
        assert_eq!(lang_extension("emacs-lisp"), "el");
    }
}
