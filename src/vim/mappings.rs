//! Key mappings (`:map` and family, `:h mapping`): typed keys are matched
//! against each mode's table before they reach the editor. Fred has no
//! 'timeout': keys that start a longer mapping wait for the next key.
//! `<leader>` is Fred's leader, Space.

use crate::editor::{Editor, Mode};
use crate::key::{Key, parse_keys};

/// The modes a mapping applies in, as `:h map-modes` names them.
pub const MODES: &str = "nxsoic";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    pub lhs: Vec<Key>,
    pub rhs: Vec<Key>,
    /// What was typed for each, to list them.
    pub lhs_text: String,
    pub rhs_text: String,
    pub noremap: bool,
    /// One of `MODES`.
    pub mode: char,
}

#[derive(Debug, Default)]
pub struct Maps {
    pub(crate) list: Vec<Map>,
    /// Typed keys waiting to be matched.
    pending: Vec<Key>,
}

/// The mapping mode the editor is in.
fn mode(ed: &Editor) -> char {
    match &ed.mode {
        Mode::Insert => 'i',
        Mode::Command(_) => 'c',
        Mode::Visual { .. } | Mode::VisualLine { .. } if ed.vim.select => 's',
        Mode::Visual { .. } | Mode::VisualLine { .. } => 'x',
        Mode::Normal if super::operator_pending(ed) => 'o',
        Mode::Normal => 'n',
        Mode::Pick(_) => '-',
    }
}

/// Mapping notation as keys: `<leader>` is Space, `<Nop>` nothing.
pub fn keys(text: &str) -> Vec<Key> {
    let mut t = text.to_string();
    for (from, to) in [
        ("<leader>", "<Space>"),
        ("<Leader>", "<Space>"),
        ("<localleader>", "\\"),
        ("<LocalLeader>", "\\"),
    ] {
        t = t.replace(from, to);
    }
    if t.eq_ignore_ascii_case("<nop>") {
        return vec![];
    }
    parse_keys(&t)
}

/// The mappings (the buffer's own first) for `mode` whose lhs starts with
/// `keys`.
fn candidates<'a>(ed: &'a Editor, mode: char, keys: &[Key]) -> impl Iterator<Item = &'a Map> {
    let k = keys.to_vec();
    ed.buf_maps
        .list
        .iter()
        .chain(ed.maps.list.iter())
        .filter(move |m| m.mode == mode && m.lhs.starts_with(&k))
}

/// Keys still held for a mapping go through as they are (end of `:normal`).
pub(crate) fn flush(ed: &mut Editor) {
    for k in std::mem::take(&mut ed.maps.pending) {
        ed.dispatch_key(k);
    }
}

/// A typed key: map it, or hold it while it may start a mapping.
pub(crate) fn typed(ed: &mut Editor, k: Key) {
    ed.maps.pending.push(k);
    resolve(ed, 0);
}

fn resolve(ed: &mut Editor, depth: usize) {
    while !ed.maps.pending.is_empty() {
        let m = mode(ed);
        let pending = ed.maps.pending.clone();
        let mut exact = None;
        let mut longer = false;
        for map in candidates(ed, m, &pending) {
            if map.lhs.len() == pending.len() {
                exact.get_or_insert_with(|| map.clone());
            } else {
                longer = true;
            }
        }
        if longer && m != '-' {
            return;
        }
        if let Some(map) = exact {
            ed.maps.pending.clear();
            run(ed, &map, depth);
            continue;
        }
        // The longest mapping that starts the keys, else the first key as it is.
        let prefix = (1..pending.len()).rev().find_map(|n| {
            candidates(ed, m, &pending[..n])
                .find(|map| map.lhs.len() == n)
                .cloned()
        });
        match prefix {
            Some(map) => {
                ed.maps.pending.drain(..map.lhs.len());
                let rest = std::mem::take(&mut ed.maps.pending);
                run(ed, &map, depth);
                ed.maps.pending.extend(rest);
            }
            None => {
                let k = ed.maps.pending.remove(0);
                let rest = std::mem::take(&mut ed.maps.pending);
                ed.dispatch_key(k);
                ed.maps.pending.extend(rest);
            }
        }
    }
}

/// A mapping's keys: remapped unless `noremap` (and not where they start
/// with its own lhs).
fn run(ed: &mut Editor, map: &Map, depth: usize) {
    if depth > 1000 {
        ed.set_err("E223: Recursive mapping");
        return;
    }
    let skip = if map.rhs.starts_with(&map.lhs) {
        map.lhs.len()
    } else {
        0
    };
    for (i, &k) in map.rhs.iter().enumerate() {
        if map.noremap || i < skip {
            ed.dispatch_key(k);
        } else {
            let saved = std::mem::take(&mut ed.maps.pending);
            ed.maps.pending.push(k);
            resolve(ed, depth + 1);
            let left = std::mem::take(&mut ed.maps.pending);
            for k in left {
                ed.dispatch_key(k);
            }
            ed.maps.pending = saved;
        }
    }
}

/// `:map`, `:nnoremap`, `:unmap`, `:mapclear`… (`name` as the table has
/// it, `force` the `!`), with `<buffer>`, `<silent>`, `<nowait>`,
/// `<unique>` and `<special>` before the lhs.
pub(crate) fn command(ed: &mut Editor, name: &str, force: bool, arg: &str) -> Result<(), String> {
    let (stem, modes) =
        parse_name(name, force).ok_or_else(|| format!("E492: Not an editor command: {name}"))?;
    let mut rest = arg.trim_start();
    let (mut buffer, mut unique) = (false, false);
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(tag) = [
            "<buffer>",
            "<silent>",
            "<nowait>",
            "<unique>",
            "<special>",
            "<script>",
            "<expr>",
        ]
        .into_iter()
        .find(|t| lower.starts_with(t)) else {
            break;
        };
        match tag {
            "<buffer>" => buffer = true,
            "<unique>" => unique = true,
            "<expr>" => {
                return Err("E1: <expr> mappings need Vimscript, which Fred doesn't have".into());
            }
            _ => {}
        }
        rest = rest[tag.len()..].trim_start();
    }
    let lhs_end = lhs_end(rest);
    let (lhs_text, rhs_text) = (&rest[..lhs_end], rest[lhs_end..].trim_start());
    match stem {
        "mapclear" => {
            let table = if buffer {
                &mut ed.buf_maps
            } else {
                &mut ed.maps
            };
            table.list.retain(|m| !modes.contains(m.mode));
            Ok(())
        }
        "unmap" => {
            if lhs_text.is_empty() {
                return Err("E474: Invalid argument".into());
            }
            let lhs = keys(lhs_text);
            let table = if buffer {
                &mut ed.buf_maps
            } else {
                &mut ed.maps
            };
            let before = table.list.len();
            table
                .list
                .retain(|m| !(modes.contains(m.mode) && m.lhs == lhs));
            if table.list.len() == before {
                return Err("E31: No such mapping".into());
            }
            Ok(())
        }
        _ if lhs_text.is_empty() || rhs_text.is_empty() => {
            list(ed, modes, lhs_text);
            Ok(())
        }
        _ => {
            let lhs = keys(lhs_text);
            if lhs.is_empty() {
                return Err("E474: Invalid argument".into());
            }
            let rhs = keys(rhs_text);
            let table = if buffer {
                &mut ed.buf_maps
            } else {
                &mut ed.maps
            };
            for mode in modes.chars() {
                if unique && table.list.iter().any(|m| m.mode == mode && m.lhs == lhs) {
                    return Err(format!("E227: Mapping already exists for {lhs_text}"));
                }
                table.list.retain(|m| !(m.mode == mode && m.lhs == lhs));
                table.list.push(Map {
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                    lhs_text: lhs_text.to_string(),
                    rhs_text: rhs_text.to_string(),
                    noremap: stem == "noremap",
                    mode,
                });
            }
            Ok(())
        }
    }
}

/// The lhs ends at the first blank (CTRL-V before one keeps it).
fn lhs_end(s: &str) -> usize {
    let mut prev = '\0';
    for (i, c) in s.char_indices() {
        if (c == ' ' || c == '\t') && prev != '\x16' {
            return i;
        }
        prev = c;
    }
    s.len()
}

/// A map command's kind (`map`, `noremap`, `unmap`, `mapclear`) and modes.
fn parse_name(name: &str, force: bool) -> Option<(&'static str, &'static str)> {
    let (prefix, stem) = ["mapclear", "noremap", "unmap", "map"]
        .iter()
        .find_map(|s| name.strip_suffix(s).map(|p| (p, *s)))?;
    let modes = match (prefix, force) {
        ("", true) => "ic",
        ("", false) => "nxso",
        ("n", _) => "n",
        ("v", _) => "xs",
        ("x", _) => "x",
        ("s", _) => "s",
        ("o", _) => "o",
        ("i", _) => "i",
        ("c", _) => "c",
        _ => return None,
    };
    Some((stem, modes))
}

/// `:map` alone: the mappings for `modes`, in a picker.
fn list(ed: &mut Editor, modes: &str, lhs: &str) {
    let lhs = keys(lhs);
    let rows: Vec<String> = ed
        .buf_maps
        .list
        .iter()
        .map(|m| (m, "@"))
        .chain(ed.maps.list.iter().map(|m| (m, " ")))
        .filter(|(m, _)| modes.contains(m.mode) && m.lhs.starts_with(&lhs))
        .map(|(m, buf)| {
            let star = if m.noremap { "*" } else { " " };
            format!("{}  {:<12} {star}{buf}{}", m.mode, m.lhs_text, m.rhs_text)
        })
        .collect();
    if rows.is_empty() {
        ed.set_msg("No mapping found");
        return;
    }
    crate::pick::choose(ed, "mappings", rows, "");
}
