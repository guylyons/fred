//! org-feed.el: RSS and Atom feeds as inbox entries.
//!
//! Adaptations: feeds are fetched with `curl` (url.el's default method) or
//! `wget`; `file://` URLs work through curl. `:filter`, `:formatter`,
//! `:new-handler` and `:changed-handler` name Emacs Lisp functions, which
//! Fred cannot run: a feed using them reports an error. `:parse-feed` and
//! `:parse-entry` accept the built-in RSS and Atom parsers. The hooks are
//! shell commands (as org-mobile's). An Atom entry's change hash is taken
//! over its XML text, where Emacs hashes the printed xml.el tree.

use super::Prefix;
use super::sexp::Sexp;
use crate::editor::Editor;
use std::path::PathBuf;

/// A feed item: (field without the colon, value).
pub type Entry = Vec<(String, String)>;

fn get<'a>(e: &'a Entry, k: &str) -> Option<&'a str> {
    e.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str())
}

fn put(e: &mut Entry, k: &str, v: String) {
    match e.iter_mut().find(|(kk, _)| kk == k) {
        Some(x) => x.1 = v,
        None => e.push((k.to_owned(), v)),
    }
}

/// One `org-feed-alist` entry.
#[derive(Clone, Debug)]
pub struct Feed {
    pub name: String,
    pub url: String,
    pub file: Option<PathBuf>,
    pub headline: String,
    pub template: String,
    pub drawer: String,
    pub atom_feed: bool,
    pub atom_entry: bool,
    pub unsupported: Option<String>,
}

/// org-feed-alist.
pub fn feeds() -> Vec<Feed> {
    let Some(alist) = super::sexp::option("org-feed-alist") else {
        return vec![];
    };
    alist
        .list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| {
            let l = f.list()?;
            let s = |i: usize| l.get(i).and_then(Sexp::str).map(str::to_owned);
            let kw = |k: &str| {
                l.iter()
                    .position(|x| x.sym() == Some(k))
                    .and_then(|i| l.get(i + 1))
                    .and_then(|v| v.str().map(str::to_owned))
            };
            let unsupported = [":filter", ":formatter", ":new-handler", ":changed-handler"]
                .into_iter()
                .find(|k| kw(k).is_some_and(|v| v != "nil"))
                .map(str::to_owned);
            let parse_feed = kw(":parse-feed").unwrap_or_else(|| "org-feed-parse-rss-feed".into());
            let parse_entry =
                kw(":parse-entry").unwrap_or_else(|| "org-feed-parse-rss-entry".into());
            let unsupported = unsupported.or_else(|| {
                (!matches!(
                    parse_feed.as_str(),
                    "org-feed-parse-rss-feed" | "org-feed-parse-atom-feed"
                ) || !matches!(
                    parse_entry.as_str(),
                    "org-feed-parse-rss-entry" | "org-feed-parse-atom-entry"
                ))
                .then(|| ":parse-feed/:parse-entry".to_owned())
            });
            Some(Feed {
                name: s(0)?,
                url: s(1)?,
                file: s(2)
                    .filter(|f| f != "nil")
                    .map(|f| super::options::expand(&f)),
                headline: s(3).unwrap_or_default(),
                template: kw(":template").unwrap_or_else(|| {
                    super::options::string(
                        "org-feed-default-template",
                        "\n* %h\n  %U\n  %description\n  %a\n",
                    )
                }),
                drawer: kw(":drawer")
                    .unwrap_or_else(|| super::options::string("org-feed-drawer", "FEEDSTATUS")),
                atom_feed: parse_feed == "org-feed-parse-atom-feed",
                atom_entry: parse_entry == "org-feed-parse-atom-entry",
                unsupported,
            })
        })
        .collect()
}

// ---- retrieving and parsing ----

/// org-feed-get-feed.
pub fn get_feed(url: &str) -> Result<String, String> {
    let method = super::options::string("org-feed-retrieve-method", "curl");
    let out = if method == "wget" {
        std::process::Command::new("wget")
            .args(["-q", "-O", "-", url])
            .output()
    } else {
        std::process::Command::new("curl")
            .args(["--silent", "--location", url])
            .output()
    }
    .map_err(|e| format!("Cannot get feed: {e}"))?;
    if !out.status.success() {
        return Err(format!("Cannot get feed {url}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// xml-substitute-special.
pub fn xml_unescape(s: &str) -> String {
    let re = crate::org_re!(r"&(#x[0-9a-fA-F]+|#[0-9]+|lt|gt|amp|quot|apos);");
    re.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        let ch = match e {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ => e[1..].parse().ok().and_then(char::from_u32),
        };
        ch.map_or_else(|| c[0].to_owned(), String::from)
    })
    .into_owned()
}

/// org-feed-parse-rss-feed: entries with `guid` and `item-full-text`.
pub fn parse_rss_feed(text: &str) -> Vec<Entry> {
    let open = crate::org_re!(r"(?is)<item\b.*?>");
    let guid = crate::org_re!(r"(?is)<guid\b.*?>(.*?)</guid>");
    let mut out = vec![];
    let mut from = 0;
    while let Some(m) = open.find_at(text, from) {
        let Some(end) = text[m.end()..]
            .to_ascii_lowercase()
            .find("</item>")
            .map(|i| m.end() + i)
        else {
            break;
        };
        let item = &text[m.end()..end];
        let g = guid
            .captures(item)
            .map(|c| xml_unescape(&c[1]))
            .unwrap_or_default();
        out.push(vec![
            ("guid".into(), g),
            ("item-full-text".into(), item.to_owned()),
        ]);
        from = end;
    }
    out
}

/// org-feed-parse-rss-entry: every `<tag>value</tag>` becomes a field.
pub fn parse_rss_entry(mut e: Entry) -> Entry {
    let text = get(&e, "item-full-text").unwrap_or_default().to_owned();
    let tag = crate::org_re!(r"(?s)<([a-zA-Z]+)\b.*?>");
    let mut from = 0;
    while let Some(c) = tag.captures_at(&text, from) {
        let m = c.get(0).unwrap();
        let name = c[1].to_owned();
        let close = format!("</{name}>");
        match text[m.end()..].find(&close) {
            Some(i) => {
                put(&mut e, &name, xml_unescape(&text[m.end()..m.end() + i]));
                from = m.end() + i + close.len();
            }
            None => from = m.end(),
        }
    }
    if !crate::org_re!(r#"isPermaLink[ \t]*=[ \t]*"false""#).is_match(&text) {
        put(&mut e, "guid-permalink", "t".into());
    }
    e
}

fn element<'a>(xml: &'a str, name: &str) -> Option<(&'a str, Option<&'a str>)> {
    let re = regex::Regex::new(&format!(r"(?s)<{name}\b([^>]*?)(/>|>(.*?)</{name}>)")).ok()?;
    let c = re.captures(xml)?;
    Some((c.get(1).unwrap().as_str(), c.get(3).map(|m| m.as_str())))
}

fn attribute(attrs: &str, name: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r#"\b{name}\s*=\s*("([^"]*)"|'([^']*)')"#)).ok()?;
    let c = re.captures(attrs)?;
    Some(xml_unescape(c.get(2).or(c.get(3)).unwrap().as_str()))
}

/// org-feed-parse-atom-feed.
pub fn parse_atom_feed(text: &str) -> Vec<Entry> {
    let re = crate::org_re!(r"(?s)<entry\b[^>]*>(.*?)</entry>");
    re.captures_iter(text)
        .map(|c| {
            let item = c[1].to_owned();
            let id = element(&item, "id")
                .and_then(|(_, b)| b)
                .map(xml_unescape)
                .unwrap_or_default();
            vec![("guid".into(), id), ("item-full-text".into(), item)]
        })
        .collect()
}

/// org-feed-parse-atom-entry.
pub fn parse_atom_entry(mut e: Entry) -> Entry {
    let xml = get(&e, "item-full-text").unwrap_or_default().to_owned();
    if let Some(href) = element(&xml, "link").and_then(|(a, _)| attribute(a, "href")) {
        put(&mut e, "link", href);
    }
    if let Some(t) = element(&xml, "title").and_then(|(_, b)| b) {
        put(&mut e, "title", xml_unescape(t));
    }
    if let Some((attrs, body)) = element(&xml, "content") {
        let kind = attribute(attrs, "type");
        let body = body.unwrap_or_default();
        let d = match kind.as_deref() {
            Some("text") | Some("html") => xml_unescape(body),
            Some("xhtml") => body.to_owned(),
            k => format!("Unknown `{}' content.", k.unwrap_or("nil")),
        };
        put(&mut e, "description", d);
    }
    e
}

// ---- formatting ----

/// The time of an entry's pubDate (org-read-date with TO-TIME), or now.
fn entry_time(e: &Entry) -> i64 {
    match get(e, "pubDate") {
        Some(d) => {
            let a = super::time::read::analyze(d, super::time::civil::now());
            a.tm.epoch()
        }
        None => super::now(),
    }
}

/// Inside an unclosed `%(` form of `s`.
fn inside_sexp(s: &str) -> bool {
    let Some(i) = s.rfind("%(") else { return false };
    let mut depth = 0i64;
    for c in s[i + 1..].chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// org-feed-format-entry.
pub fn format_entry(e: &Entry, template: &str) -> String {
    let desc = get(e, "description").unwrap_or("???");
    let time = entry_time(e);
    let v_h = get(e, "title")
        .map(str::to_owned)
        .or_else(|| {
            desc.split('\n')
                .find(|l| !l.trim().is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "???".into());
    let link = if get(e, "guid-permalink").is_some() {
        get(e, "guid")
    } else {
        None
    }
    .or(get(e, "link"));
    let v_a = link.map(|l| format!("[[{l}]]\n")).unwrap_or_default();
    let re = crate::org_re!(r"%([a-zA-Z]+)");
    let mut out = String::new();
    let mut last = 0;
    for c in re.captures_iter(template) {
        let m = c.get(0).unwrap();
        out.push_str(&template[last..m.start()]);
        last = m.end();
        // org-capture-escaped-%: a backslash keeps the escape.
        if out.ends_with('\\') {
            out.pop();
            out.push_str(m.as_str());
            continue;
        }
        let rep = match &c[1] {
            "h" => v_h.clone(),
            "t" => super::timestamp(time, false, false),
            "T" => super::timestamp(time, true, false),
            "u" => super::timestamp(time, false, true),
            "U" => super::timestamp(time, true, true),
            "a" => v_a.clone(),
            name => get(e, name).unwrap_or_default().to_owned(),
        };
        if inside_sexp(&out) {
            out.push_str(&rep.replace('"', "\\\""));
        } else {
            out.push_str(&rep);
        }
    }
    out.push_str(&template[last..]);
    super::capture::eval_sexps(&out)
}

// ---- status drawer ----

/// (guid, handled, hash).
type Status = Vec<(String, bool, String)>;

fn lisp_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// pp-to-string of the status list.
fn print_status(st: &Status) -> Vec<String> {
    let items: Vec<String> = st
        .iter()
        .map(|(g, h, s)| {
            format!(
                "({} {} {})",
                lisp_string(g),
                if *h { "t" } else { "nil" },
                lisp_string(s)
            )
        })
        .collect();
    if items.is_empty() {
        return vec!["nil".into()];
    }
    let mut out = vec![];
    for (i, it) in items.iter().enumerate() {
        let pre = if i == 0 { "(" } else { " " };
        let post = if i + 1 == items.len() { ")" } else { "" };
        out.push(format!("{pre}{it}{post}"));
    }
    out
}

fn drawer_lines(ed: &Editor, h: usize, drawer: &str) -> Option<(usize, usize)> {
    let end = super::syntax::subtree_end(&ed.buf, h);
    let open = format!(":{drawer}:");
    let beg = (h + 1..end).find(|&i| ed.buf.line(i).trim() == open)?;
    let close = (beg + 1..ed.line_count()).find(|&i| ed.buf.line(i).trim() == ":END:")?;
    Some((beg, close))
}

/// org-feed-read-previous-status.
fn read_status(ed: &Editor, h: usize, drawer: &str) -> Status {
    let Some((b, e)) = drawer_lines(ed, h, drawer) else {
        return vec![];
    };
    let text = super::lines(ed, b + 1..e).join("\n");
    let Ok(v) = super::sexp::read(&text) else {
        return vec![];
    };
    v.list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| {
            let l = x.list()?;
            Some((
                l.first()?.str()?.to_owned(),
                l.get(1).is_some_and(|v| !v.is_nil()),
                l.get(2).and_then(Sexp::str).unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

/// org-feed-write-status.
fn write_status(ed: &mut Editor, h: usize, drawer: &str, st: &Status) {
    let lines = print_status(st);
    match drawer_lines(ed, h, drawer) {
        Some((b, e)) => super::splice(ed, b + 1, e - b - 1, &lines),
        None => {
            let at = super::syntax::next_heading(&ed.buf, h, usize::MAX).unwrap_or(ed.line_count());
            let mut v = vec![format!("  :{drawer}:")];
            v.extend(lines);
            v.push("  :END:".into());
            super::insert_lines(ed, at, &v);
        }
    }
}

/// org-feed-goto-inbox-internal: the inbox heading (created if needed).
fn goto_inbox(ed: &mut Editor, heading: &str) -> usize {
    super::fold::widen(ed);
    let re = regex::Regex::new(&format!(
        r"^\*+[ \t]+{}[ \t]*(:.*?:[ \t]*)?$",
        regex::escape(heading)
    ))
    .unwrap();
    if let Some(h) = (0..ed.line_count()).find(|&i| re.is_match(&ed.buf.line(i))) {
        return h;
    }
    // (insert "\n\n* " heading "\n\n") at the end.
    let last = ed.line_count() - 1;
    let tail = ed.buf.line(last).to_string();
    super::splice(
        ed,
        last,
        1,
        &[
            tail,
            String::new(),
            format!("* {heading}"),
            String::new(),
            String::new(),
        ],
    );
    last + 2
}

/// The work of org-feed-update in the inbox buffer: returns new item count.
pub fn update_in(ed: &mut Editor, feed: &Feed, raw: &str) -> usize {
    let entries = if feed.atom_feed {
        parse_atom_feed(raw)
    } else {
        parse_rss_feed(raw)
    };
    let h = goto_inbox(ed, &feed.headline);
    let old = read_status(ed, h, &feed.drawer);
    let handled = |g: &str| {
        old.iter()
            .find(|(og, _, _)| og == g)
            .is_some_and(|(_, h, _)| *h)
    };
    let mut new = vec![];
    for e in &entries {
        if !handled(get(e, "guid").unwrap_or_default()) {
            new.push(e.clone());
        }
    }
    let parse = |e: Entry| {
        if feed.atom_entry {
            parse_atom_entry(e)
        } else {
            parse_rss_entry(e)
        }
    };
    // `new` was built with push in Emacs: newest-last items come first.
    new.reverse();
    let new: Vec<Entry> = new.into_iter().map(parse).collect();
    if new.is_empty() {
        ed.set_msg(format!("No new items in feed {}", feed.name));
        return 0;
    }
    let status: Status = entries
        .iter()
        .map(|e| {
            let g = get(e, "guid").unwrap_or_default().to_owned();
            let now = new.iter().any(|n| get(n, "guid") == Some(&g)) || handled(&g);
            (
                g,
                now,
                super::babel::sha1::hex(get(e, "item-full-text").unwrap_or_default().as_bytes()),
            )
        })
        .collect();
    // org-feed-add-items: as children at level+1, after the subtree's text.
    let level = super::syntax::level(&ed.buf.line(h)).unwrap_or(1) + 1;
    let mut lines = vec![];
    for e in &new {
        let text = format_entry(e, &feed.template);
        for l in text.strip_suffix('\n').unwrap_or(&text).split('\n') {
            lines.push(match super::syntax::level(l) {
                Some(n) => format!("{}{}", "*".repeat(n + level - 1), &l[n..]),
                None => l.to_owned(),
            });
        }
    }
    let end = super::syntax::subtree_end(&ed.buf, h);
    let mut at = end;
    while at > h + 1 && ed.buf.line(at - 1).trim().is_empty() {
        at -= 1;
    }
    super::insert_lines(ed, at, &lines);
    write_status(ed, h, &feed.drawer, &status);
    super::fold::fold_subtree(ed, h, true);
    super::fold::show_children(ed, h, None);
    new.len()
}

fn run_hooks(name: &str) {
    for cmd in super::options::strings(name, &[]) {
        let _ = std::process::Command::new("sh").args(["-c", &cmd]).status();
    }
}

/// org-feed-update: fetch and add; the count reaches `done`.
fn update(
    ed: &mut Editor,
    feed: Feed,
    done: impl FnOnce(&mut crate::session::Session, Result<usize, String>) + Send + 'static,
) {
    if let Some(k) = &feed.unsupported {
        let e = format!(
            "Feed {}: {k} names an Emacs Lisp function, which Fred cannot run",
            feed.name
        );
        return super::effect(ed, move |s| done(s, Err(e)));
    }
    let raw = match get_feed(&feed.url) {
        Ok(r) => r,
        Err(e) => return super::effect(ed, move |s| done(s, Err(e))),
    };
    let file = feed.file.clone().or_else(|| ed.path.clone());
    super::effect(ed, move |s| {
        let Some(file) = file else {
            return done(s, Err("No file for the feed inbox".into()));
        };
        let i = match s.org_buffer(&file) {
            Ok(i) => i,
            Err(e) => return done(s, Err(e)),
        };
        run_hooks("org-feed-before-adding-hook");
        let n = s.org_with_buffer(i, |ed| update_in(ed, &feed, &raw));
        if n > 0 {
            if super::options::bool("org-feed-save-after-adding", true)
                && let Err(e) = s.org_save(i)
            {
                return done(s, Err(e));
            }
            s.ed.set_msg(format!(
                "Added {n} new item{} from feed {} to file {}, heading {}",
                if n > 1 { "s" } else { "" },
                feed.name,
                file.file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                feed.headline
            ));
            run_hooks("org-feed-after-adding-hook");
        }
        done(s, Ok(n))
    });
}

fn choose(ed: &mut Editor, all: bool, then: impl FnOnce(&mut Editor, Feed) + Send + 'static) {
    let fs = feeds();
    if all && fs.len() == 1 {
        return then(ed, fs[0].clone());
    }
    let names: Vec<String> = fs.iter().map(|f| f.name.clone()).collect();
    super::complete(ed, "Feed name: ", names, true, move |ed, n| {
        match fs.into_iter().find(|f| f.name == n) {
            Some(f) => then(ed, f),
            None => ed.set_err("No such feed in `org-feed-alist"),
        }
    });
}

pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-feed-update-all" => {
            let fs = feeds();
            let total = fs.len();
            let tally = std::sync::Arc::new(std::sync::Mutex::new((0usize, 0usize, 0usize)));
            for f in fs {
                let t = tally.clone();
                update(ed, f, move |s, r| {
                    let mut t = t.lock().unwrap();
                    match r {
                        Ok(n) => t.0 += n,
                        Err(_) => t.1 += 1,
                    }
                    t.2 += 1;
                    if t.2 == total {
                        let entries = match t.0 {
                            0 => "No new entries".to_owned(),
                            1 => "1 new entry".to_owned(),
                            n => format!("{n} new entries"),
                        };
                        let errs = if t.1 == 0 {
                            String::new()
                        } else {
                            format!(" (unavailable feeds: {})", t.1)
                        };
                        s.ed.set_msg(format!(
                            "{entries} from {total} {}{errs}",
                            if total == 1 { "feed" } else { "feeds" }
                        ));
                    }
                });
            }
            Ok(())
        }
        "org-feed-update" => {
            choose(ed, false, |ed, f| {
                update(ed, f, |s, r| {
                    if let Err(e) = r {
                        s.ed.set_err(e);
                    }
                })
            });
            Ok(())
        }
        "org-feed-goto-inbox" => {
            choose(ed, true, |ed, f| {
                let file = f.file.clone().or_else(|| ed.path.clone());
                super::effect(ed, move |s| {
                    let Some(file) = file else {
                        return s.ed.set_err("No file for the feed inbox");
                    };
                    match s.org_buffer(&file) {
                        Ok(i) => {
                            s.org_show(i);
                            let h = goto_inbox(&mut s.ed, &f.headline);
                            s.ed.set_cursor(h, 0);
                        }
                        Err(e) => s.ed.set_err(e),
                    }
                });
            });
            Ok(())
        }
        "org-feed-show-raw-feed" => {
            choose(ed, true, |ed, f| match get_feed(&f.url) {
                Ok(raw) => super::effect(ed, move |s| {
                    s.org_temp_buffer("*Org feed*", &raw, false);
                }),
                Err(e) => ed.set_err(e),
            });
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = "<?xml version=\"1.0\"?>\n<rss version=\"2.0\"><channel><title>T</title>\n<item>\n<title>First &amp; best</title>\n<link>http://ex.org/1</link>\n<description>Line one\nline two</description>\n<pubDate>Tue, 10 Jun 2003 04:00:00 GMT</pubDate>\n<guid>http://ex.org/1</guid>\n</item>\n<item><title>Second</title><link>http://ex.org/2</link><guid isPermaLink=\"false\">id-2</guid><description>D2</description></item>\n</channel></rss>\n";

    /// Checked against Emacs (org-feed-update-all on the same feed).
    #[test]
    fn rss_update_matches_emacs() {
        super::super::set_now(Some(1_780_000_000));
        let ts = super::super::timestamp(1_780_000_000, true, true);
        let mut e = super::super::tests::org("* Inbox\n", "");
        let feed = Feed {
            name: "F".into(),
            url: String::new(),
            file: None,
            headline: "Inbox".into(),
            template: "\n* %h\n  %U\n  %description\n  %a\n".into(),
            drawer: "FEEDSTATUS".into(),
            atom_feed: false,
            atom_entry: false,
            unsupported: None,
        };
        assert_eq!(update_in(&mut e, &feed, RSS), 2);
        assert_eq!(
            e.buf.text(),
            format!(
                "* Inbox\n\n  :FEEDSTATUS:\n((\"http://ex.org/1\" t \"f9d555ba084affca44a3322ede8f2467377e0d89\")\n (\"id-2\" t \"405ac7c5356266031c09fb133a6c67d16e6b2b08\"))\n  :END:\n** Second\n  {ts}\n  D2\n  [[http://ex.org/2]]\n\n\n** First & best\n  [2003-06-10 Tue 04:00]\n  Line one\nline two\n  [[http://ex.org/1]]\n"
            ),
            "the final newline is the buffer's flag: the file ends like Emacs's"
        );
        // A second run adds nothing.
        assert_eq!(update_in(&mut e, &feed, RSS), 0);
        super::super::set_now(None);
    }

    #[test]
    fn atom_entries() {
        let xml = "<feed><entry><id>urn:1</id><title>A &lt;b&gt;</title><link href=\"http://a/1\"/><content type=\"text\">Hi</content></entry></feed>";
        let es: Vec<Entry> = parse_atom_feed(xml)
            .into_iter()
            .map(parse_atom_entry)
            .collect();
        assert_eq!(get(&es[0], "guid"), Some("urn:1"));
        assert_eq!(get(&es[0], "title"), Some("A <b>"));
        assert_eq!(get(&es[0], "link"), Some("http://a/1"));
        assert_eq!(get(&es[0], "description"), Some("Hi"));
    }
}
