//! The clock table: org-clock-report, org-dblock-write:clocktable,
//! org-clocktable-write-default and org-clocktable-steps,
//! org-clocktable-shift, and org-clock-get-clocktable for the agenda.

use super::super::dblock::{self, Out, Params};
use super::super::props::{Doc, Inherit};
use super::super::sexp::{self, Sexp};
use super::super::syntax::{self, Settings};
use super::super::time::civil::{self, Tm};
use super::super::time::stamp;
use super::super::{Prefix, fold, line, links, options, tags};
use super::sum;
use crate::editor::Editor;
use std::path::{Path, PathBuf};

/// org-clocktable-defaults.
pub fn defaults() -> Params {
    if let Some(v) = sexp::option("org-clocktable-defaults")
        && v.list().is_some()
    {
        return Params::from_sexp(&v);
    }
    let lang = options::string("org-export-default-language", "en");
    Params::parse(&format!(
        ":maxlevel 2 :lang {lang:?} :scope file :block nil :wstart 1 :mstart 1 \
         :tstart nil :tend nil :step nil :stepskip0 nil :fileskip0 nil :tags nil \
         :match nil :emphasize nil :link nil :narrow 40! :indent t :filetitle nil \
         :hidefiles nil :formula nil :timestamp nil :level nil :tcolumns nil :formatter nil"
    ))
    .unwrap_or_default()
}

/// org-clock-clocktable-language-setup.
fn translate(s: &str, lang: &str) -> String {
    const SETUP: &[[&str; 10]] = &[
        [
            "en",
            "File",
            "L",
            "Timestamp",
            "Headline",
            "Time",
            "ALL",
            "Total time",
            "File time",
            "Clock summary at",
        ],
        [
            "de",
            "Datei",
            "E",
            "Zeitstempel",
            "Kopfzeile",
            "Dauer",
            "GESAMT",
            "Gesamtdauer",
            "Dateizeit",
            "Erstellt am",
        ],
        [
            "es",
            "Archivo",
            "N",
            "Fecha y hora",
            "Tarea",
            "Duración",
            "TODO",
            "Duración total",
            "Tiempo archivo",
            "Generado el",
        ],
        [
            "fr",
            "Fichier",
            "N",
            "Horodatage",
            "En-tête",
            "Durée",
            "TOUT",
            "Durée totale",
            "Durée fichier",
            "Horodatage sommaire à",
        ],
        [
            "nl",
            "Bestand",
            "N",
            "Tijdstip",
            "Rubriek",
            "Duur",
            "ALLES",
            "Totale duur",
            "Bestandstijd",
            "Klok overzicht op",
        ],
        [
            "nn",
            "Fil",
            "N",
            "Tidspunkt",
            "Overskrift",
            "Tid",
            "ALLE",
            "Total tid",
            "Filtid",
            "Tidsoversyn",
        ],
        [
            "pl",
            "Plik",
            "P",
            "Data i godzina",
            "Nagłówek",
            "Czas",
            "WSZYSTKO",
            "Czas całkowity",
            "Czas pliku",
            "Poddumowanie zegara na",
        ],
        [
            "pt-BR",
            "Arquivo",
            "N",
            "Data e hora",
            "Título",
            "Hora",
            "TODOS",
            "Hora total",
            "Hora do arquivo",
            "Resumo das horas em",
        ],
        [
            "sk",
            "Súbor",
            "L",
            "Časová značka",
            "Záhlavie",
            "Čas",
            "VŠETKO",
            "Celkový čas",
            "Čas súboru",
            "Časový súhrn pre",
        ],
    ];
    let i = match s {
        "File" => 1,
        "L" => 2,
        "Timestamp" => 3,
        "Headline" => 4,
        "Time" => 5,
        "ALL" => 6,
        "Total time" => 7,
        "File time" => 8,
        "Clock summary at" => 9,
        _ => return s.into(),
    };
    SETUP
        .iter()
        .find(|r| r[0].eq_ignore_ascii_case(lang))
        .map_or(s, |r| r[i])
        .into()
}

fn dur(m: i64) -> String {
    super::super::time::duration::from_minutes(m as f64, None, false).unwrap_or_default()
}

/// One headline of the table (org-clock-get-table-data's entries).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub level: usize,
    pub headline: String,
    pub tags: Vec<String>,
    pub timestamp: Option<String>,
    pub time: i64,
    pub props: Vec<(String, String)>,
}

/// A file's clock data: (file name, total minutes, entries).
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub file: Option<PathBuf>,
    pub total: i64,
    pub entries: Vec<Entry>,
}

fn reduced_level(st: &Settings, l: usize) -> usize {
    if l > 0 && st.opt_bool("org-odd-levels-only", false) {
        1 + l / 2
    } else {
        l
    }
}

/// A time parameter (:tstart, :tend) as seconds: an absolute day number
/// or a time string read by org-matcher-time.
fn param_time(v: Option<&Sexp>) -> Option<f64> {
    match v? {
        Sexp::Int(n) => {
            let t = Tm::from_absolute(*n);
            Some(Tm::date(t.year, t.month, t.day).epoch() as f64)
        }
        Sexp::Nil => None,
        v => Some(stamp::matcher_time(v.str()?)),
    }
}

/// org-link-create-headline-link-for-table.
fn headline_link(h: &str, file: Option<&Path>) -> String {
    let desc = links::normalize(h, false, true);
    let search = format!("*{}", links::normalize(h, false, true));
    let link = match file {
        Some(f) => format!("file:{}::{search}", f.display()),
        None => search,
    };
    links::make_string(&link, Some(&desc)).unwrap_or(desc)
}

/// org-clock-get-table-data on lines `lo..hi`.
pub fn table_data(
    b: &[String],
    lo: usize,
    hi: usize,
    file: Option<&Path>,
    params: &Params,
    st: &Settings,
) -> Table {
    let maxlevel = params.int(":maxlevel").unwrap_or(3).max(0) as usize;
    let mut ts = param_time(params.get(":tstart"));
    let mut te = param_time(params.get(":tend"));
    if let Some(block) = params.str(":block")
        && let Ok(r) = sum::special_range(
            &block,
            civil::now(),
            params.int(":wstart"),
            params.int(":mstart"),
        )
    {
        let (s, e) = r.seconds();
        ts = r.start.map(|_| s);
        te = Some(e);
    }
    let doc = Doc::new(b, st, file);
    let matcher = params
        .str(":match")
        .map(|m| tags::Matcher::new(&m, st, civil::now_epoch()));
    let filter = |l: usize| {
        let Some(m) = &matcher else { return true };
        let line = &b[l];
        let Some(h) = syntax::headline(line, st) else {
            return false;
        };
        let tags = doc.tags(Some(l), false);
        let prop = |p: &str| doc.get(Some(l), p, Inherit::Selective, false);
        let not_done = |t: &str| !st.is_done(t);
        m.matches(&tags::EntryInfo {
            todo: h.todo.as_deref(),
            tags: &tags,
            level: h.level,
            prop: &prop,
            not_done: &not_done,
        })
    };
    let f: Option<&dyn Fn(usize) -> bool> = if matcher.is_some() {
        Some(&filter)
    } else {
        None
    };
    let s = sum::sum(b, lo, hi, ts, te, f, super::clocking_in(b, file));
    let link = params.on(":link");
    let with_tags = params.on(":tags");
    let with_ts = params.on(":timestamp");
    let inherit = if params.on(":inherit-props") {
        Inherit::Yes
    } else {
        Inherit::No
    };
    let properties: Vec<String> = params
        .get(":properties")
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    let mut entries = vec![];
    for (&l, &time) in &s.minutes {
        let Some(lvl) = syntax::level(&b[l]) else {
            continue;
        };
        let level = reduced_level(st, lvl);
        if time <= 0 || level > maxlevel {
            continue;
        }
        let title = super::title(&b[l], st);
        let headline = if link {
            headline_link(&title, file)
        } else {
            links::normalize(&title, false, true)
        };
        entries.push(Entry {
            level,
            headline,
            tags: if with_tags {
                doc.tags(Some(l), false)
            } else {
                vec![]
            },
            timestamp: if with_ts {
                ["SCHEDULED", "DEADLINE", "TIMESTAMP", "TIMESTAMP_IA"]
                    .iter()
                    .find_map(|p| doc.get(Some(l), p, Inherit::No, false))
            } else {
                None
            },
            time,
            props: properties
                .iter()
                .filter_map(|p| doc.get(Some(l), p, inherit, false).map(|v| (p.clone(), v)))
                .collect(),
        });
    }
    Table {
        file: file.map(Path::to_path_buf),
        total: s.total,
        entries,
    }
}

/// org-shorten-string.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.into();
    }
    let n = max.saturating_sub(4).max(1);
    let re = regex::Regex::new(&format!(r"^(.{{1,{n}}}[^ ])( |$)")).unwrap();
    match re.captures(s) {
        Some(c) => format!("{}...", &c[1]),
        None => format!(
            "{}...",
            s.chars().take(max.saturating_sub(3)).collect::<String>()
        ),
    }
}

/// org-clocktable-indent-string.
fn indent_string(level: usize) -> String {
    if level <= 1 {
        String::new()
    } else {
        format!("\\_{}", " ".repeat(2 * (level - 1)))
    }
}

fn cell(fmt: &str, s: &str) -> String {
    fmt.replacen("%s", s, 1)
}

/// org-clocktable-write-default: the lines of the table; returns them and
/// the total time.
pub fn write_default(tables: &[Table], params: &Params) -> Result<(Vec<String>, i64), String> {
    let lang = params.str(":lang").unwrap_or_else(|| "en".into());
    let tr = |s: &str| translate(s, &lang);
    let multifile = params.on(":multifile");
    let block = params.str(":block");
    let link = params.on(":link");
    let maxlevel = params.int(":maxlevel").unwrap_or(3);
    let emph = params.on(":emphasize");
    let compact = params.on(":compact");
    let mut narrow = match params.get(":narrow") {
        Some(Sexp::Nil) | None if compact => Some(Sexp::Sym("40!".into())),
        v => v.cloned().filter(Sexp::truthy),
    };
    let filetitle = params.on(":filetitle");
    let level_col = !compact && params.on(":level");
    let timestamp = params.on(":timestamp");
    let tags = params.on(":tags");
    let properties: Vec<String> = params
        .get(":properties")
        .and_then(|v| {
            v.list().map(|l| {
                l.iter()
                    .filter_map(|x| x.str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    let time_columns = if compact || maxlevel < 2 {
        1
    } else {
        let levels: Vec<usize> = tables
            .iter()
            .filter(|t| t.total > 0)
            .flat_map(|t| t.entries.iter().map(|e| e.level))
            .collect();
        let deepest = levels.iter().max().copied().unwrap_or(1) as i64;
        maxlevel
            .min(params.int(":tcolumns").unwrap_or(100))
            .min(deepest)
            .max(1) as usize
    };
    let indent = compact || params.on(":indent");
    let formula = params.get(":formula").cloned().unwrap_or(Sexp::Nil);
    let percent = formula.sym() == Some("%");
    let total: i64 = tables.iter().map(|t| t.total).sum();
    let total_fmt = options::string("org-clock-total-time-cell-format", "*%s*");
    let file_fmt = options::string("org-clock-file-time-cell-format", "*%s*");
    if let Some(Sexp::Int(n)) = narrow
        && link
    {
        narrow = Some(Sexp::Sym(format!("{n}!")));
    }
    let (narrow_n, cut) = match &narrow {
        None => (None, false),
        Some(Sexp::Int(n)) => (Some(*n as usize), false),
        Some(v) => match v
            .sym()
            .and_then(|s| s.strip_suffix('!'))
            .and_then(|n| n.parse().ok())
        {
            Some(n) => (Some(n), true),
            None => {
                return Err(format!(
                    "Invalid value {v} of :narrow property in clock table"
                ));
            }
        },
    };
    let mut s = String::new();
    // The header before the table.
    match params.get(":header") {
        Some(Sexp::Str(h)) => s.push_str(h),
        _ => {
            let range = match &block {
                Some(b) => {
                    let r = sum::special_range(
                        b,
                        civil::now(),
                        params.int(":wstart"),
                        params.int(":mstart"),
                    )?;
                    format!(", for {}.", r.text)
                }
                None => String::new(),
            };
            s.push_str(&format!(
                "#+CAPTION: {} {}{range}\n",
                tr("Clock summary at"),
                civil::format("[%Y-%m-%d %a %H:%M]", civil::now())
            ));
        }
    }
    let bar = |on: bool| if on { "|" } else { "" };
    let pbars = "|".repeat(properties.len());
    let extra_cols = "|".repeat(time_columns.saturating_sub(1));
    if let (Some(n), false) = (narrow_n, cut) {
        s.push_str(&format!(
            "|{}{}{}{}{pbars}<{n}>| |\n",
            bar(multifile),
            bar(level_col),
            bar(timestamp),
            bar(tags)
        ));
    }
    s.push('|');
    if multifile {
        s.push_str(&format!("{}|", tr("File")));
    }
    if level_col {
        s.push_str(&format!("{}|", tr("L")));
    }
    if timestamp {
        s.push_str(&format!("{}|", tr("Timestamp")));
    }
    if tags {
        s.push_str("Tags |");
    }
    if !properties.is_empty() {
        s.push_str(&format!("{}|", properties.join("|")));
    }
    s.push_str(&format!("{}|{}|{extra_cols}", tr("Headline"), tr("Time")));
    s.push_str(if percent { "%|\n" } else { "\n" });
    // The total time.
    s.push_str("|-\n|");
    if multifile {
        s.push_str(&format!("| {} ", tr("ALL")));
    }
    s.push_str(&format!(
        "{}{}{}{pbars}{}| {}|{extra_cols}{}\n",
        bar(level_col),
        bar(timestamp),
        bar(tags),
        cell(&total_fmt, &tr("Total time")),
        cell(&total_fmt, &dur(total)),
        if !percent {
            ""
        } else if total == 0 {
            "0.0|"
        } else {
            "100.0|"
        }
    ));
    if total > 0 {
        for t in tables {
            if t.total <= 0 && params.on(":fileskip0") {
                continue;
            }
            s.push_str("|-\n");
            if multifile {
                let name = t
                    .file
                    .as_ref()
                    .map(|f| {
                        let base = f
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if filetitle {
                            std::fs::read_to_string(f)
                                .ok()
                                .and_then(|text| {
                                    text.lines().find_map(|l| {
                                        syntax::keyword_line(l)
                                            .filter(|(k, _)| k == "TITLE")
                                            .map(|(_, v)| v.trim().to_owned())
                                    })
                                })
                                .unwrap_or(base)
                        } else {
                            base
                        }
                    })
                    .unwrap_or_default();
                let pct = if !percent {
                    String::new()
                } else if total == 0 {
                    " 0.0 |".into()
                } else {
                    format!(" {:.1} |", 100.0 * t.total as f64 / total as f64)
                };
                s.push_str(&format!(
                    "| {name} {} | {}{}{pbars}{} | *{}*|{extra_cols}{pct}\n",
                    if level_col { "| " } else { "" },
                    if timestamp { "| " } else { "" },
                    if tags { "| " } else { "" },
                    cell(&file_fmt, &tr("File time")),
                    dur(t.total),
                ));
            }
            if maxlevel <= 0 {
                continue;
            }
            for e in &t.entries {
                let mut headline = e.headline.clone();
                if cut && let Some(n) = narrow_n {
                    let re = crate::org_re!(r"^\[\[((?:[^\]\[\\]|\\.)+)\]\[(.+?)\]\]$");
                    headline = match re.captures(&headline) {
                        Some(c) => format!("[[{}][{}]]", &c[1], shorten(&c[2], n)),
                        None => shorten(&headline, n),
                    };
                }
                let field = |f: &str| {
                    if !emph {
                        format!("{f} |")
                    } else if e.level == 1 {
                        format!("*{f}* |")
                    } else if e.level == 2 {
                        format!("/{f}/ |")
                    } else {
                        format!("{f} |")
                    }
                };
                s.push('|');
                s.push_str(bar(multifile));
                if level_col {
                    s.push_str(&format!("{}|", e.level));
                }
                if timestamp {
                    s.push_str(&format!("{}|", e.timestamp.clone().unwrap_or_default()));
                }
                if tags {
                    s.push_str(&format!("{}|", e.tags.join(", ")));
                }
                if !properties.is_empty() {
                    let vals: Vec<String> = properties
                        .iter()
                        .map(|p| {
                            e.props
                                .iter()
                                .find(|(k, _)| k == p)
                                .map_or(String::new(), |(_, v)| v.clone())
                        })
                        .collect();
                    s.push_str(&format!("{}|", vals.join("|")));
                }
                if indent {
                    s.push_str(&indent_string(e.level));
                }
                s.push_str(&field(&headline));
                s.push_str(&"|".repeat(time_columns.min(e.level).saturating_sub(1)));
                s.push_str(&field(&dur(e.time)));
                s.push_str(&"|".repeat(time_columns.saturating_sub(e.level)));
                if percent {
                    s.push_str(&format!("{:.1} |", 100.0 * e.time as f64 / total as f64));
                }
                s.push('\n');
            }
        }
    }
    s.pop();
    let mut recalc = false;
    match &formula {
        Sexp::Nil => {}
        f if f.sym() == Some("%") => {}
        Sexp::Str(f) => {
            s.push_str(&format!("\n#+TBLFM: {f}"));
            recalc = true;
        }
        _ => return Err("Invalid :formula parameter in clocktable".into()),
    }
    if matches!(formula, Sexp::Nil) || percent {
        let content = params.str(":content").unwrap_or_default();
        if let Some(l) = content
            .lines()
            .find(|l| l.trim_start().to_ascii_lowercase().starts_with("#+tblfm:"))
        {
            s.push('\n');
            s.push_str(l);
            recalc = true;
        }
    }
    let mut lines: Vec<String> = s.split('\n').map(str::to_owned).collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with('|'))
        .unwrap_or(lines.len());
    let end = (start..lines.len())
        .find(|&i| !lines[i].starts_with('|'))
        .unwrap_or(lines.len());
    let mut rows = dblock::align(&lines[start..end]);
    if let Some(sort) = params.get(":sort")
        && let Some(col) = sort.car().and_then(Sexp::int)
        && let Some(kind) = sort.cdr().int().and_then(|c| char::from_u32(c as u32))
    {
        dblock::sort_rows(&mut rows, 3, col as usize, kind);
    }
    lines.splice(start..end, rows);
    if recalc {
        let tail: Vec<String> = lines[start..].to_vec();
        let mut d = super::super::table::doc::Doc::new(&tail.join("\n"));
        if super::super::table::formula::recalculate(
            &mut d,
            super::super::table::formula::Recalc::All,
            false,
        )
        .is_ok()
        {
            lines.truncate(start);
            lines.extend(d.lines);
        }
    }
    Ok((lines, total))
}

/// The scope's files, or the current buffer.
enum Files {
    Buffer,
    List(Vec<PathBuf>, bool),
}

/// org-add-archive-files: the files with their `_archive` siblings.
fn with_archives(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = vec![];
    for f in files {
        let a = PathBuf::from(format!("{}_archive", f.display()));
        out.push(f);
        if a.is_file() {
            out.push(a);
        }
    }
    out
}

fn scope_files(ed: &Editor, scope: Option<&Sexp>) -> Result<Files, String> {
    let s = match scope {
        None | Some(Sexp::Nil) => return Ok(Files::Buffer),
        Some(v) => v,
    };
    if let Some(name) = s.sym().or(s.str().filter(|_| matches!(s, Sexp::Sym(_)))) {
        return match name {
            "agenda" => Ok(Files::List(super::super::agenda_files(), false)),
            "agenda-with-archives" => Ok(Files::List(
                with_archives(super::super::agenda_files()),
                false,
            )),
            "file-with-archives" => Ok(match super::file_of(ed) {
                Some(f) => Files::List(with_archives(vec![f]), true),
                None => Files::List(vec![], true),
            }),
            "file" | "subtree" | "tree" => Ok(Files::Buffer),
            n if n.starts_with("tree") && n[4..].parse::<usize>().is_ok() => Ok(Files::Buffer),
            n => Err(format!("Unknown scope: {n}")),
        };
    }
    let l = s.list().ok_or_else(|| format!("Unknown scope: {s}"))?;
    // (lambda () (list "file" ...)): the files of its last form.
    if l.first().and_then(Sexp::sym) == Some("lambda") {
        let body = l.last().cloned().unwrap_or(Sexp::Nil);
        let items = body.list().unwrap_or(&[]);
        let files: Vec<PathBuf> = match items.first().and_then(Sexp::sym) {
            Some("list") => items[1..]
                .iter()
                .filter_map(|x| x.str().map(PathBuf::from))
                .collect(),
            Some("quote") => items
                .get(1)
                .and_then(Sexp::list)
                .unwrap_or(&[])
                .iter()
                .filter_map(|x| x.str().map(PathBuf::from))
                .collect(),
            _ => return Err(format!("Unknown scope: {s}")),
        };
        return Ok(Files::List(files, false));
    }
    Ok(Files::List(
        l.iter()
            .filter_map(|x| x.str().map(options::expand))
            .collect(),
        false,
    ))
}

/// The lines of the current buffer the scope restricts to.
fn scope_range(ed: &Editor, scope: Option<&Sexp>, point: usize) -> Result<(usize, usize), String> {
    let name = scope.and_then(|s| s.sym().or(s.str())).unwrap_or("nil");
    let n = ed.line_count();
    let head =
        || fold::back_to_heading(ed, point.min(n.saturating_sub(1))).ok_or("Before first headline");
    let subtree = |h: usize| (h, syntax::subtree_end(&ed.buf, h));
    let st = super::super::settings(ed);
    Ok(match name {
        "nil" => fold::narrowed(ed).unwrap_or((0, n)),
        "file" => (0, n),
        "subtree" => subtree(head()?),
        "tree" => {
            let mut h = head()?;
            while let Some(p) = syntax::parent(&ed.buf, h) {
                h = p;
            }
            subtree(h)
        }
        t => {
            let lvl: usize = t[4..].parse().unwrap_or(1);
            let mut h = head()?;
            while let Some(p) = syntax::parent(&ed.buf, h) {
                h = p;
                if reduced_level(&st, syntax::level(&line(ed, h)).unwrap_or(1)) <= lvl {
                    break;
                }
            }
            subtree(h)
        }
    })
}

/// org-dblock-write:clocktable into `buf` at `pt` (the dynamic block
/// body, followed by its `#+END:`); returns the total minutes.
fn write(
    ed: &mut Editor,
    params: &Params,
    point: usize,
    buf: &mut String,
    pt: &mut usize,
) -> Result<i64, String> {
    let params = defaults().combine(params);
    let scope = params.get(":scope").cloned();
    let files = scope_files(ed, scope.as_ref())?;
    let (ws, ms) = (params.int(":wstart"), params.int(":mstart"));
    let block = params.str(":block");
    let has_range = block.is_some()
        || (params.get(":tstart").is_some_and(Sexp::truthy)
            && params.get(":tend").is_some_and(Sexp::truthy));
    if params.get(":step").is_some_and(Sexp::truthy) {
        if !has_range {
            return Err(
                "Clocktable `:step' can only be used with `:block' or `:tstart', `:tend'".into(),
            );
        }
        steps(ed, &params, point, buf, pt)?;
        return Ok(0);
    }
    if let Some(b) = &block {
        sum::special_range(b, civil::now(), ws, ms)?;
    }
    if let Some(f) = params.get(":formatter").filter(|f| f.truthy())
        && f.sym() != Some("org-clocktable-write-default")
    {
        return Err(format!("Clocktable formatter {f} cannot be called in fred"));
    }
    let me = super::file_of(ed);
    let (tables, multi) = match files {
        Files::Buffer => {
            let (lo, hi) = scope_range(ed, scope.as_ref(), point)?;
            let lines = super::super::lines(ed, 0..ed.line_count());
            let st = super::super::settings(ed);
            (
                vec![table_data(&lines, lo, hi, me.as_deref(), &params, &st)],
                false,
            )
        }
        Files::List(files, extended) => {
            let mut v = vec![];
            for f in &files {
                let abs = std::path::absolute(f).unwrap_or_else(|_| f.clone());
                let (lines, st) = if Some(&abs) == me.as_ref() {
                    (
                        super::super::lines(ed, 0..ed.line_count()),
                        (*super::super::settings(ed)).clone(),
                    )
                } else {
                    let text = std::fs::read_to_string(&abs).unwrap_or_default();
                    let st = super::super::settings_of(&text, abs.parent());
                    (text.lines().map(str::to_owned).collect::<Vec<_>>(), st)
                };
                v.push(table_data(&lines, 0, lines.len(), Some(&abs), &params, &st));
            }
            (v, !extended)
        }
    };
    let mut p = params.clone();
    p.set(
        ":multifile",
        if multi && !params.on(":hidefiles") {
            Sexp::T
        } else {
            Sexp::Nil
        },
    );
    let (lines, total) = write_default(&tables, &p)?;
    buf.insert_str(*pt, &lines.join("\n"));
    Ok(total)
}

/// org-clocktable-steps.
fn steps(
    ed: &mut Editor,
    params: &Params,
    point: usize,
    buf: &mut String,
    pt: &mut usize,
) -> Result<(), String> {
    let skip0 = params.on(":stepskip0");
    let step = params.str(":step").unwrap_or_default();
    let header = match step.as_str() {
        "day" => "Daily report: ",
        "week" => "Weekly report starting on: ",
        "semimonth" => "Semimonthly report starting on: ",
        "month" => "Monthly report starting on: ",
        "year" => "Annual report starting on: ",
        "quarter" => "Quarterly report starting on: ",
        s => return Err(format!("Unknown `:step' specification: {s}")),
    };
    let ws = params.int(":wstart").unwrap_or(1);
    let ms = params.int(":mstart").unwrap_or(1);
    let extend = options::int("org-extend-today-until", 0);
    let range = match params.str(":block") {
        Some(b) => Some(sum::special_range(&b, civil::now(), Some(ws), Some(ms))?),
        None => None,
    };
    let day_time = |v: &Sexp| -> Option<i64> {
        match v {
            Sexp::Int(n) => {
                let t = Tm::from_absolute(*n);
                Some(Tm::at(t.year, t.month, t.day, extend, 0).epoch())
            }
            Sexp::Nil => None,
            v => Some(stamp::matcher_time(v.str()?) as i64),
        }
    };
    let mut start = match &range {
        Some(r) => r.start.map(|s| s.epoch()),
        None => params.get(":tstart").and_then(day_time),
    }
    .unwrap_or_else(|| stamp::matcher_time("<2003-01-01 Thu 00:00>") as i64);
    let end = match &range {
        Some(r) => r.end.epoch(),
        None => params.get(":tend").and_then(day_time).unwrap_or(0),
    };
    let stamp_fmt = |t: i64, with_time: bool| {
        let f = if with_time {
            "[%Y-%m-%d %a %H:%M]"
        } else {
            "[%Y-%m-%d %a]"
        };
        civil::format(f, Tm::from_epoch(t))
    };
    while start < end {
        if *pt > 0 && buf.as_bytes()[*pt - 1] != b'\n' {
            buf.insert(*pt, '\n');
            *pt += 1;
        }
        let hdr = format!("\n{header}{}\n", stamp_fmt(start, false));
        buf.insert_str(*pt, &hdr);
        *pt += hdr.len();
        let table_begin = buf[..*pt - 1].rfind('\n').map_or(0, |i| i + 1);
        let t = Tm::from_epoch(start);
        let (d, m, y, dow) = (t.day, t.month, t.year, t.weekday());
        let next = match step.as_str() {
            "day" => Tm::at(y, m, d + 1, extend, 0),
            "week" => {
                let offset = if dow == ws {
                    7
                } else {
                    (ws - dow).rem_euclid(7)
                };
                Tm::at(y, m, d + offset, extend, 0)
            }
            "semimonth" => {
                if d < 16 {
                    Tm::date(y, m, 16)
                } else {
                    Tm::date(y, m + 1, 1)
                }
            }
            "month" => Tm::date(y, m + 1, ms),
            "quarter" => Tm::date(y, m + 3, ms),
            _ => Tm::at(y + 1, 1, 1, extend, 0),
        }
        .norm()
        .epoch();
        let mut sub = Params::default();
        sub.set(":header", Sexp::Str(String::new()));
        sub.set(":step", Sexp::Nil);
        sub.set(":block", Sexp::Nil);
        sub.set(":tstart", Sexp::Str(stamp_fmt(start, true)));
        sub.set(":tend", Sexp::Str(stamp_fmt(end.min(next), true)));
        let step_time = write(ed, &params.combine(&sub), point, buf, pt)?;
        let e = buf[*pt..]
            .find("#+END:")
            .map(|i| *pt + i)
            .ok_or("Dynamic block not terminated")?;
        *pt = e + "#+END:".len();
        if skip0 && step_time == 0 {
            buf.replace_range(table_begin..e, "");
            *pt = table_begin;
        }
        start = next;
        // (end-of-line 0)
        let bol = buf[..*pt].rfind('\n').map_or(0, |i| i + 1);
        *pt = if bol > 0 {
            bol - 1
        } else {
            buf.find('\n').unwrap_or(buf.len())
        };
    }
    Ok(())
}

/// The dynamic block writer for `clocktable`.
pub fn dblock_write(ed: &mut Editor, params: &Params, out: &mut Out) -> Result<(), String> {
    const END: &str = "#+END:";
    let mut buf = std::mem::take(&mut out.text);
    buf.push_str(END);
    let mut pt = out.pt;
    let r = write(ed, params, out.line, &mut buf, &mut pt);
    buf.truncate(buf.len() - END.len());
    out.pt = pt.min(buf.len());
    out.text = buf;
    r.map(drop)
}

/// org-in-clocktable-p: the #+BEGIN: line of the clocktable at line `l`.
pub fn in_clocktable(ed: &Editor, l: usize) -> Option<usize> {
    let re = crate::org_re!(r"^[ \t]*#\+BEGIN:[ \t]+clocktable");
    let start = (0..=l).rev().find(|&i| re.is_match(&line(ed, i)))?;
    let end = (start..ed.line_count()).find(|&i| dblock::end_re().is_match(&line(ed, i)))?;
    (end >= l).then_some(start)
}

/// org-clock-report.
pub fn report(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    super::remove_overlays(ed);
    if !arg.is_none()
        && let Some(b) = dblock::find(ed, "clocktable")
    {
        ed.set_cursor(b, 0);
        fold::show_entry(ed, b);
    }
    let l = ed.cur.line;
    let start = match in_clocktable(ed, l) {
        Some(s) => s,
        None => {
            let scope = if fold::before_first_heading(ed, l) {
                "file"
            } else {
                "subtree"
            };
            let mut p = Params::parse(&format!(":scope {scope}"))?;
            let defaults = sexp::option("org-clock-clocktable-default-properties")
                .map(|v| Params::from_sexp(&v))
                .unwrap_or_else(|| Params::parse(":maxlevel 2").unwrap_or_default());
            p = p.combine(&defaults);
            p.set(":name", Sexp::Str("clocktable".into()));
            dblock::create(ed, "clocktable", &p)
        }
    };
    ed.set_cursor(start, 0);
    dblock::update(ed, start)
}

/// org-clocktable-shift: move the :block of the clocktable line by N.
pub fn shift(ed: &mut Editor, dir: &str, n: Prefix) -> Result<(), String> {
    let mut n = n.value();
    if dir == "left" || dir == "down" {
        n = -n;
    }
    let l = ed.cur.line;
    let t = line(ed, l);
    let re = crate::org_re!(r"(?i)^[ \t]*#\+BEGIN:[ \t]+clocktable\b.*?:block[ \t]+(\S+)");
    let c = re
        .captures(&t)
        .ok_or("Line needs a :block definition before this command works")?;
    let g = c.get(1).unwrap();
    let mut s = g.as_str().to_owned();
    for (a, b) in [
        ("yesterday", "today-1"),
        ("lastweek", "thisweek-1"),
        ("lastmonth", "thismonth-1"),
        ("lastyear", "thisyear-1"),
        ("lastq", "thisq-1"),
    ] {
        if s == a {
            s = b.into();
        }
    }
    let rel = crate::org_re!(r"^(today|thisweek|thismonth|thisyear|thisq)([-+][0-9]+)?$");
    let abs = crate::org_re!(r"([0-9]+)(-([wWqQ]?)([0-9]{1,2})(-([0-9]{1,2}))?)?");
    let ins = if let Some(c) = rel.captures(&s) {
        let shift = c.get(2).map_or(0, |m| {
            m.as_str()
                .trim_start_matches('+')
                .parse::<i64>()
                .unwrap_or(0)
        }) + n;
        if shift == 0 {
            c[1].to_owned()
        } else {
            format!("{}{shift:+}", &c[1])
        }
    } else if let Some(c) = abs.captures(&s) {
        let num = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().unwrap_or(0));
        let y = num(1).unwrap_or(0);
        let wp = c
            .get(3)
            .map(|m| m.as_str().to_owned())
            .filter(|w| !w.is_empty());
        let mw = num(4);
        let d = num(6);
        if let (Some(d), Some(mw)) = (d, mw) {
            civil::format("%Y-%m-%d", Tm::date(y, mw, d + n).norm())
        } else if let (Some(w), Some(mw)) = (&wp, mw)
            && w.eq_ignore_ascii_case("w")
        {
            let date = Tm::from_absolute(sum::iso_to_absolute(mw + n, 1, y));
            civil::format("%G-W%V", date)
        } else if let (Some(w), Some(mut mw)) = (&wp, mw)
            && w.eq_ignore_ascii_case("q")
        {
            let mut y = y;
            if mw + n > 4 {
                mw = 0;
                y += 1;
            }
            if mw + n == 0 {
                mw = 5;
                y -= 1;
            }
            format!("{y}-Q{}", mw + n)
        } else if let Some(mw) = mw {
            civil::format("%Y-%m", Tm::date(y, mw + n, 1).norm())
        } else {
            (y + n).to_string()
        }
    } else {
        return Err("Cannot shift clocktable block".into());
    };
    super::super::set_line(ed, l, &format!("{}{ins}{}", &t[..g.start()], &t[g.end()..]));
    let b = ed.cur.byte;
    dblock::update(ed, l)?;
    ed.set_cursor(l, b);
    Ok(())
}

/// org-clock-get-clocktable: the formatted table for `params` (scope
/// `agenda` and :maxlevel 2 unless given), without the caption line.
pub fn get_clocktable(params: &Params) -> Result<String, String> {
    let mut p = Params::default();
    p.set(":name", Sexp::Str("clocktable".into()));
    let mut p = p.combine(params);
    if !p.has(":maxlevel") {
        p.set(":maxlevel", Sexp::Int(2));
    }
    if !p.has(":scope") {
        p.set(":scope", Sexp::Sym("agenda".into()));
    }
    let mut ed = Editor::new(crate::buffer::Buffer::from_text(""));
    ed.org = Some(Box::default());
    let b = dblock::create(&mut ed, "clocktable", &p);
    dblock::update(&mut ed, b)?;
    let end = (b + 1..ed.line_count())
        .find(|&i| dblock::end_re().is_match(&line(&ed, i)))
        .unwrap_or(ed.line_count());
    let body: Vec<String> = super::super::lines(&ed, (b + 2).min(end)..end);
    Ok(body.iter().map(|l| format!("{l}\n")).collect())
}
