//! org-tempo.el (`<s` then TAB expands to a block, `<L` to `#+latex: `,
//! `<I` asks for an #+include file) and org-insert-structure-template
//! (org.el, C-c C-, and its org-mks menu).
//!
//! Org Tempo is active unless org-modules is set without `org-tempo`
//! (upstream loads it only through org-modules).

use super::Prefix;
use super::buf::EBuf;
#[cfg(test)]
use super::options;
use super::sexp::Sexp;
use crate::editor::{Editor, Mode};

/// org-structure-template-alist.
pub fn templates() -> Vec<(String, String)> {
    alist("org-structure-template-alist").unwrap_or_else(|| {
        [
            ("a", "export ascii"),
            ("c", "center"),
            ("C", "comment"),
            ("e", "example"),
            ("E", "export"),
            ("h", "export html"),
            ("l", "export latex"),
            ("q", "quote"),
            ("s", "src"),
            ("v", "verse"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    })
}

/// org-tempo-keywords-alist.
pub fn keywords() -> Vec<(String, String)> {
    alist("org-tempo-keywords-alist").unwrap_or_else(|| {
        [
            ("L", "latex"),
            ("H", "html"),
            ("A", "ascii"),
            ("i", "index"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    })
}

/// An alist option of (KEY . STRING).
fn alist(name: &str) -> Option<Vec<(String, String)>> {
    let v = super::sexp::option(name)?;
    Some(
        v.list()?
            .iter()
            .filter_map(|e| Some((e.car()?.str()?.to_owned(), e.cdr().str()?.to_owned())))
            .collect(),
    )
}

/// Org Tempo is loaded.
pub fn active() -> bool {
    match super::sexp::option("org-modules") {
        None => true,
        Some(v) => v.list().unwrap_or(&[]).iter().any(|m| {
            m.str().or_else(|| m.sym()) == Some("org-tempo")
                || matches!(m, Sexp::Sym(s) if s == "org-tempo")
        }),
    }
}

/// The first word is all upper case: `#+BEGIN_`/`#+END_`.
fn upcase(name: &str) -> bool {
    let w = name.split_whitespace().next().unwrap_or("");
    w == w.to_uppercase()
}

/// The expansion of tempo tag `<KEY`: (text before point, text after).
fn expansion(key: &str) -> Option<(String, String)> {
    if let Some((_, name)) = templates().into_iter().find(|(k, _)| k == key) {
        let special = name == "src" || name == "export";
        let (b, e) = if upcase(&name) {
            ("BEGIN", "END")
        } else {
            ("begin", "end")
        };
        let first = name.split(' ').next().unwrap_or("").to_owned();
        let begin = format!("#+{b}_{name}{}", if special { " " } else { "" });
        let end = format!("#+{e}_{first}");
        return Some(if special {
            (begin, format!("\n\n{end}"))
        } else {
            (format!("{begin}\n"), format!("\n{end}"))
        });
    }
    keywords()
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, name)| (format!("#+{name}: "), String::new()))
}

/// org-tempo-complete-tag on TAB: true when `<KEY` before point expanded.
pub fn complete_tag(ed: &mut Editor) -> Result<bool, String> {
    if !active() {
        return Ok(false);
    }
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let pt = if ed.mode == Mode::Insert {
        ed.cur.byte
    } else {
        // Evil: point is after the character under the cursor here.
        crate::text::next_grapheme(&line, ed.cur.byte)
    }
    .min(line.len());
    let Some(c) = crate::org_re!(r"^( *)(<\w+)$").captures(&line[..pt]) else {
        return Ok(false);
    };
    let indent = c[1].to_owned();
    let tag = c[2].to_owned();
    let key = &tag[1..];
    if key == "I" {
        let rest = line[pt..].to_owned();
        super::read(ed, "Include file: ", "", move |ed, f| {
            let dir = ed
                .path
                .as_deref()
                .and_then(|p| std::path::absolute(p).ok())
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                .unwrap_or_default();
            let path = std::path::Path::new(&f);
            let rel = path
                .strip_prefix(&dir)
                .map_or_else(|_| f.clone(), |p| p.display().to_string());
            let text = format!("{indent}#+include: {rel:?} ");
            ed.undo.begin(ed.cur.pos());
            super::set_line(ed, l, &format!("{text}{rest}"));
            ed.set_cursor(l, text.len());
            ed.undo.end(ed.cur.pos());
        });
        return Ok(true);
    }
    let Some((before, after)) = expansion(key) else {
        return Ok(false);
    };
    let _ = indent;
    // `>` (indent-according-to-mode) on the first and last lines.
    let pad = " ".repeat(super::editing::indentation_for(ed, l, false));
    let after = match after.rfind('\n') {
        Some(i) => format!("{}{pad}{}", &after[..=i], &after[i + 1..]),
        None => after,
    };
    let new_before = format!("{pad}{before}");
    let text = format!("{new_before}{after}{}", &line[pt..]);
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    super::splice(ed, l, 1, &lines);
    let pl = l + new_before.matches('\n').count();
    let pb = new_before.len() - new_before.rfind('\n').map_or(0, |i| i + 1);
    ed.set_cursor(pl, pb);
    ed.cur.byte = pb;
    Ok(true)
}

/// org--insert-structure-template-unique-keys.
fn unique_keys(keys: &[String]) -> Result<Vec<String>, String> {
    let mut ordered: Vec<&String> = keys.iter().collect();
    ordered.sort_by_key(|k| k.chars().count());
    let mut menu: Vec<(String, String)> = vec![];
    for key in ordered {
        let chars: Vec<char> = key.chars().collect();
        let potential = match chars.len() {
            1 => Some(format!("{key}TAB")),
            2 => Some(key.clone()),
            _ => chars[1..]
                .iter()
                .map(|c| format!("{}{c}", chars[0]))
                .find(|k| !menu.iter().any(|(m, _)| m == k)),
        };
        match potential {
            Some(p) if !menu.iter().any(|(m, _)| *m == p) => menu.push((p, key.clone())),
            _ => return Err(format!("Could not make unique key for `{key}'")),
        }
    }
    Ok(keys
        .iter()
        .map(|k| {
            menu.iter()
                .find(|(_, x)| x == k)
                .map(|(m, _)| m.clone())
                .unwrap_or_default()
        })
        .collect())
}

/// The org-mks table of org--insert-structure-template-mks: (keys, label).
pub fn menu_entries() -> Result<Vec<(String, String)>, String> {
    let mut all = templates();
    all.push((
        "\t".into(),
        "Press TAB, RET or SPC to write block name".into(),
    ));
    let mut letters: Vec<String> = vec![];
    for (k, _) in &all {
        let f: String = k.chars().take(1).collect();
        if !letters.contains(&f) {
            letters.push(f);
        }
    }
    let key = |k: &str| {
        if k == "\t" {
            "TAB".to_owned()
        } else {
            k.to_owned()
        }
    };
    let mut out = vec![];
    for l in letters {
        let group: Vec<&(String, String)> = all.iter().filter(|(k, _)| k.starts_with(&l)).collect();
        if group.len() == 1 {
            out.push((key(&l), group[0].1.clone()));
            continue;
        }
        let long = group.len() > 3;
        let names: Vec<&str> = group.iter().take(3).map(|(_, v)| v.as_str()).collect();
        out.push((
            String::new(),
            format!(
                "#{} {}{}",
                key(&l),
                names.join(", "),
                if long { ", ..." } else { "" }
            ),
        ));
        let keys: Vec<String> = group.iter().map(|(k, _)| k.clone()).collect();
        for (k, (_, v)) in unique_keys(&keys)?.into_iter().zip(group) {
            out.push((k.replace('\t', "TAB"), v.clone()));
        }
    }
    Ok(out)
}

/// org-insert-structure-template TYPE.
pub fn insert_structure_template(ed: &mut Editor, ty: &str) -> Result<(), String> {
    if ty.is_empty() {
        return Err(format!("Invalid structure type: {ty:?}"));
    }
    let region = super::region(ed);
    let lower = ty.to_lowercase();
    let extended = lower == "src" || lower == "export";
    let verbatim = ["example", "export", "src", "comment"]
        .iter()
        .any(|p| lower.starts_with(p));
    let up = upcase(ty);
    let mut b = EBuf::load_emacs(ed);
    let region_end = region.map(|(lo, hi)| {
        let at = b.pos_of_line(hi + 1);
        let at = if hi + 1 >= ed.line_count() {
            b.eol_at(b.pos_of_line(hi))
        } else {
            at
        };
        b.goto(b.pos_of_line(lo));
        b.mark(at)
    });
    let column = {
        let line = b.line();
        super::buf::column(&line[..line.len() - line.trim_start().len()])
    };
    let mut c = EBuf::new(&b.s, b.pt);
    c.skip_chars_backward(" \t", None);
    if c.bolp() {
        b.beginning_of_line();
    } else {
        b.insert("\n");
    }
    let start = b.mark(b.pt);
    let pad = " ".repeat(column);
    b.insert(&format!(
        "{pad}#+{}_{ty}{}\n",
        if up { "BEGIN" } else { "begin" },
        if extended { " " } else { "" }
    ));
    if let Some(m) = region_end {
        if verbatim {
            // org-escape-code-in-region: a comma before `*` and `#+`.
            let from = b.pt;
            let to = b.marker(m);
            let at: Vec<usize> = crate::org_re!(r"(?m)^[ \t]*(,*(?:\*|#\+))")
                .captures_iter(&b.s[from..to])
                .map(|c| from + c.get(1).unwrap().start())
                .collect();
            for p in at.into_iter().rev() {
                b.insert_at(p, ",");
            }
        }
        b.goto(b.marker(m));
        b.skip_chars_backward(" \r\t\n", None);
        if !b.eolp() {
            b.insert("\n");
            b.forward_line(-1);
        }
        b.end_of_line();
    }
    if !b.bolp() {
        b.insert("\n");
    }
    let first = ty.split_whitespace().next().unwrap_or(ty);
    b.insert(&format!(
        "{pad}#+{}_{first}",
        if up { "END" } else { "end" }
    ));
    let rest = crate::org_re!(r"^[ \t]*")
        .find(&b.s[b.pt..])
        .map_or(0, |m| m.end());
    if b.pt + rest == b.eol() {
        let (p, e) = (b.pt, b.pt + rest);
        b.delete(p, e);
    } else {
        b.insert("\n");
    }
    if b.eobp() && !b.bolp() {
        b.insert("\n");
    }
    b.goto(b.marker(start));
    if extended {
        b.end_of_line();
    } else {
        b.forward_line(1);
        b.skip_chars_forward(" \t", None);
    }
    b.store_emacs(ed);
    Ok(())
}

/// The commands.
pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-insert-structure-template" | "org-insert-block-template" => {
            if let Some(t) = super::take_call_arg() {
                return Some(insert_structure_template(ed, &t));
            }
            let entries = match menu_entries() {
                Ok(e) => e,
                Err(e) => return Some(Err(e)),
            };
            let all = templates();
            let menu: Vec<(String, String)> = entries.clone();
            let region = super::region(ed);
            super::menu(ed, "Select a key\n============", menu, move |ed, k| {
                let run = move |ed: &mut Editor, t: String| {
                    ed.org_region = region;
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = insert_structure_template(ed, &t) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                    ed.org_region = None;
                };
                if k == "TAB" || k == " " {
                    super::read(ed, "Structure type: ", "", move |ed, t| {
                        if t.is_empty() {
                            ed.set_err("Empty structure type");
                        } else {
                            run(ed, t);
                        }
                    });
                    return;
                }
                let name = entries
                    .iter()
                    .find(|(x, _)| *x == k)
                    .map(|(_, v)| v.clone())
                    .or_else(|| all.iter().find(|(x, _)| *x == k).map(|(_, v)| v.clone()));
                if let Some(t) = name {
                    run(ed, t);
                }
            });
            ed.org_menu_enter = Some("TAB".into());
            ed.org_menu.push((" ".into(), String::new()));
            Ok(())
        }
        _ => return None,
    })
}

/// org-cycle's org-tab-before-tab-emulation-hook: Org Tempo.
pub fn tab_hook(ed: &mut Editor) -> Result<bool, String> {
    complete_tag(ed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::footnote::tests::{at, text};

    fn insert(t: &str, ty: &str) -> String {
        let mut e = at(t);
        insert_structure_template(&mut e, ty).unwrap();
        text(&e)
    }

    #[test]
    fn structure_templates() {
        // Ported from test-org/insert-structure-template.
        assert_eq!(insert("", "foo"), "#+begin_foo\n#+end_foo\n");
        assert_eq!(insert("", "FOO"), "#+BEGIN_FOO\n#+END_FOO\n");
        assert_eq!(insert("", "src"), "#+begin_src \n#+end_src\n");
        assert_eq!(insert("  <point>", "foo"), "  #+begin_foo\n  #+end_foo\n");
        assert_eq!(
            insert("Text<point>", "foo"),
            "Text\n#+begin_foo\n#+end_foo\n"
        );
        assert_eq!(
            insert("Before\n<point>After", "foo"),
            "Before\n#+begin_foo\n#+end_foo\nAfter"
        );
        // Wrapping the region, escaping verbatim blocks.
        let mut e = at("a\n* b\nc");
        e.org_region = Some((0, 1));
        insert_structure_template(&mut e, "example").unwrap();
        assert_eq!(text(&e), "#+begin_example\na\n,* b\n#+end_example\nc");
        let mut e = at("a\nb");
        e.org_region = Some((0, 1));
        insert_structure_template(&mut e, "quote").unwrap();
        assert_eq!(text(&e), "#+begin_quote\na\nb\n#+end_quote\n");
    }

    #[test]
    fn menu_keys() {
        let m = menu_entries().unwrap();
        assert!(m.contains(&("s".into(), "src".into())));
        assert!(m.contains(&(
            "TAB".into(),
            "Press TAB, RET or SPC to write block name".into()
        )));
        assert_eq!(
            unique_keys(&["a".into(), "aa".into(), "abc".into()]).unwrap(),
            ["aTAB", "aa", "ab"]
        );
        // C-c C-, s inserts a source block.
        let e = crate::org::tests::org("", "<C-c><C-,>s");
        assert_eq!(text(&e), "#+begin_src \n#+end_src\n");
        let e = crate::org::tests::org("", "<C-c><C-,><Tab>foo<Enter>");
        assert_eq!(text(&e), "#+begin_foo\n#+end_foo\n", "{:?}", e.msg);
    }

    #[test]
    fn tempo_tags() {
        // Ported from test-org-tempo.
        let e = crate::org::tests::org("", "i<s<Tab>");
        assert_eq!(e.buf.text(), "#+begin_src \n\n#+end_src");
        assert_eq!((e.cur.line, e.cur.byte), (0, 12));
        let e = crate::org::tests::org("", "i<L<Tab>");
        assert_eq!(e.buf.text(), "#+latex: ");
        let e = crate::org::tests::org("", "i<q<Tab>x");
        assert_eq!(e.buf.text(), "#+begin_quote\nx\n#+end_quote");
        let e = crate::org::tests::org("", "i<l<Tab>");
        assert_eq!(e.buf.text(), "#+begin_export latex\n\n#+end_export");
        assert_eq!((e.cur.line, e.cur.byte), (1, 0));
        // `>`: the expected indentation (here none) replaces the tag's.
        let e = crate::org::tests::org("", "i  <e<Tab>");
        assert_eq!(e.buf.text(), "#+begin_example\n\n#+end_example");
        let e = crate::org::tests::org("- item\n  ", "jA<s<Tab>");
        assert_eq!(e.buf.text(), "- item\n  #+begin_src \n\n  #+end_src");
        // Not a tag: TAB does its usual thing.
        let e = crate::org::tests::org("", "i<z<Tab>");
        assert_eq!(e.buf.text(), "<z");
        options::put("org-modules", toml::Value::String("'(ol-info)".into()));
        let e = crate::org::tests::org("", "i<s<Tab>");
        assert_eq!(e.buf.text(), "<s");
        options::remove("org-modules");
    }
}
