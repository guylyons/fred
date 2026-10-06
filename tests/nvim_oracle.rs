//! Differential tests: the same keys typed into Fred and into a clean
//! headless Neovim (`tools/nvim-oracle.lua`) must leave the same text,
//! cursor and mode. Skipped when `nvim` isn't installed.
//!
//! A row of `docs/nvim-parity.csv` moves to `done` only once cases here
//! cover it.

use fred::buffer::Buffer;
use fred::editor::{Editor, Mode};
use fred::key::parse_keys;
use std::process::Command;

/// (text, keys) pairs, grouped by what they cover.
const CASES: &[(&str, &str)] = &[
    // Motions.
    ("abc def ghi", "w"),
    ("abc def ghi", "2w"),
    ("abc def ghi", "$b"),
    ("abc def ghi", "e"),
    ("foo.bar baz", "W"),
    ("foo.bar baz", "$B"),
    ("foo.bar baz", "E"),
    ("  indented", "$^"),
    ("abc def", "$0"),
    ("a\nb\nc", "G"),
    ("a\nb\nc", "G2G"),
    ("a\nb\nc", "Ggg"),
    ("a\n\nb\nc\n\nd", "}"),
    ("a\n\nb\nc\n\nd", "G{"),
    ("abcabc", "fc"),
    ("abcabc", "2fc"),
    ("abcabc", "tc"),
    ("abcabc", "$Fa"),
    ("abcabc", "$Ta"),
    ("abcabc", "fb;"),
    ("abcabc", "fb;,"),
    ("one\ntwo\nthree", "j+"),
    ("one\n  two\nthree", "G-"),
    ("abc\nd\nabcdef", "$jj"),
    // Operators.
    ("abc def ghi", "dw"),
    ("abc def ghi", "d2w"),
    ("abc def ghi", "2dw"),
    ("abc def ghi", "de"),
    ("abc def ghi", "wd$"),
    ("abc def ghi", "wd0"),
    ("a\nb\nc\nd", "dd"),
    ("a\nb\nc\nd", "2dd"),
    ("a\nb\nc\nd", "dj"),
    ("a\nb\nc\nd", "jdk"),
    ("a\nb\nc\nd", "dG"),
    ("abc def", "cwxyz<Esc>"),
    ("abc def", "ccnew<Esc>"),
    ("abc def", "yyp"),
    ("abc def", "ywP"),
    ("abc def", "yw$p"),
    ("abc\ndef", "ddp"),
    ("abcdef", "x"),
    ("abcdef", "3x"),
    ("abcdef", "$X"),
    ("abcdef", "lD"),
    ("abcdef", "lCxy<Esc>"),
    ("abcdef", "sX<Esc>"),
    ("abc\ndef", "SX<Esc>"),
    ("abc\ndef", "J"),
    ("abc\ndef\nghi", "3J"),
    ("abcdef", "rx"),
    ("abcdef", "3rx"),
    // Insert mode.
    ("abc", "ixy<Esc>"),
    ("abc", "axy<Esc>"),
    ("  abc", "Ixy<Esc>"),
    ("abc", "Axy<Esc>"),
    ("abc", "oxy<Esc>"),
    ("abc", "Oxy<Esc>"),
    ("abc", "Axy<BS><Esc>"),
    ("abc def", "Ax<C-w><Esc>"),
    ("abc", "ixy"),
    ("abc def", "Axy<C-w><C-w><Esc>"),
    ("abc def", "A<C-w><Esc>"),
    ("abc def", "Axy<C-u><Esc>"),
    ("abc def", "Axy<C-u><C-u><Esc>"),
    ("  abc", "A<C-u><Esc>"),
    ("  abc", "A<C-u><C-u><Esc>"),
    // Undo, redo and repeat.
    ("abc def", "dwu"),
    ("abc def", "dwu<C-r>"),
    ("a b c d", "dw.."),
    ("abc\ndef", "x.j."),
    // Visual mode.
    ("abc def", "vld"),
    ("abc def", "vey$p"),
    ("a\nb\nc", "Vjd"),
    ("a\nb\nc", "vjd"),
    ("abc def", "wvbd"),
    ("abcdef", "lvlohd"),
    ("abc def", "vl"),
    ("a\nb", "Vj"),
    // Marks.
    ("a\nb\nc", "majjd'a"),
    // Search.
    ("foo bar foo", "/foo<CR>"),
    ("foo bar foo", "/foo<CR>n"),
    ("foo bar foo", "$?bar<CR>"),
    ("foo bar foo", "/foo<CR>N"),
    // Ex commands.
    ("a\nb\nc", ":2d<CR>"),
    ("a\nb\nc", ":%s/./X/<CR>"),
    ("a\nb\nc", ":g/b/d<CR>"),
    ("a\nb\nc", ":1m$<CR>"),
    ("a\nb\nc", ":1t.<CR>"),
    // Options: 'tabstop' decides the column `j` keeps below a tab.
    ("\tx\n12345678y", "$j"),
    ("\tx\n12345678y", ":set ts=4<CR>$j"),
    ("\tx\n12345678y", ":se ts=4 ts+=2<CR>$j"),
    ("\tx\n12345678y", ":setl ts=2<CR>$j"),
    ("\tx\n12345678y", ":set ts=2<CR>:set ts&<CR>$j"),
    ("abc", ":set nosuch<CR>x"),
    ("a\nb\nc", ":1,2j<CR>"),
    ("a\n  b\nc", ":1,2j!<CR>"),
    ("a\n  b\n)c", ":%j<CR>"),
    ("a \n  b", "J"),
];

#[derive(Debug, PartialEq)]
struct State {
    text: String,
    line: usize,
    col: usize,
    mode: String,
}

fn fred(text: &str, keys: &str) -> State {
    let mut ed = Editor::new(Buffer::from_text(text));
    for k in parse_keys(keys) {
        ed.handle_key(k);
    }
    let mode = match ed.mode {
        Mode::Normal => "n",
        Mode::Insert => "i",
        Mode::Visual { .. } => "v",
        Mode::VisualLine { .. } => "V",
        Mode::Command(_) => "c",
        Mode::Pick(_) => "pick",
    };
    State {
        text: ed.buf.text(),
        line: ed.cur.line,
        col: ed.cur.byte,
        mode: mode.into(),
    }
}

/// None when `nvim` isn't installed.
fn nvim(text: &str, keys: &str) -> Option<State> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/nvim-oracle.lua");
    let out = Command::new("nvim")
        .args(["--clean", "--headless", "-l", script, text, keys])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout.lines().last().unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("nvim oracle for {keys:?}: {e}: {stdout}"));
    Some(State {
        text: v["text"].as_str().unwrap_or_default().into(),
        line: v["line"].as_u64().unwrap_or_default() as usize,
        col: v["col"].as_u64().unwrap_or_default() as usize,
        mode: v["mode"].as_str().unwrap_or_default().into(),
    })
}

#[test]
fn fred_matches_neovim() {
    let results: Vec<_> = std::thread::scope(|s| {
        let jobs: Vec<_> = CASES
            .iter()
            .map(|&(text, keys)| s.spawn(move || (text, keys, nvim(text, keys))))
            .collect();
        jobs.into_iter().map(|j| j.join().unwrap()).collect()
    });
    let mut wrong = vec![];
    for (text, keys, want) in results {
        let Some(want) = want else {
            eprintln!("nvim not found; skipping the Neovim oracle");
            return;
        };
        let got = fred(text, keys);
        if got != want {
            wrong.push(format!(
                "{text:?} + {keys:?}\n  fred: {got:?}\n  nvim: {want:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ from Neovim:\n{}",
        wrong.len(),
        CASES.len(),
        wrong.join("\n")
    );
}
