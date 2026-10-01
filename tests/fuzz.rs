//! Deterministic randomized testing ("fuzzing") of fred's terminal-free core.
//!
//! Each case is a generated initial buffer plus a generated sequence of keys
//! (and bracketed pastes) fed to `Editor::handle_key` / `Editor::paste`.
//!
//! Checked after every step:
//!   1. no panic (each case runs under `catch_unwind`; seed and step are kept),
//!   2. cursor line in range, byte <= line length, byte on a grapheme boundary,
//!      and outside Insert mode never past the last grapheme; the Visual-line
//!      anchor is a valid line; an undo group is open exactly while in Insert
//!      mode; no completion popup outside Insert mode; the rope's line count
//!      agrees with the `\n`-only line model the rest of the editor uses,
//!   3. the file format round trip is stable:
//!      `from_text(to_bytes()).to_bytes() == to_bytes()`,
//!   4. a `:`/`/`/`?` line that ends in an error leaves the text unchanged, and
//!      so does any other key that reports an error (except `.`).
//!
//! Checked after the whole sequence:
//!   5. two `<Esc>` get back to Normal mode,
//!   6. `u` until "already at oldest change" restores the initial text exactly
//!      and clears the modified flag,
//!   7. `<C-r>` until "already at newest change" restores the final text.
//!
//! A bug found but not yet fixed can be listed in `KNOWN` (with a minimal
//! repro in `tests/fuzz_regressions.rs`) so the harness steers around it and
//! keeps finding new ones; `FRED_FUZZ_KNOWN=1` turns those workarounds off.
//! The list is empty: every bug found so far is fixed.
//!
//! Environment:
//!   FRED_FUZZ_ITERS=n    sequences to run (default 1500)
//!   FRED_FUZZ_SEED=n     first seed (default 0)
//!   FRED_FUZZ_THREADS=n  worker threads (default 1)
//!   FRED_FUZZ_KNOWN=1    no known-bug workarounds (=name,name: not those)
//!   FRED_FUZZ_SHRINK=0   report failures without shrinking them
//!   FRED_FUZZ_CASE=n     print the case for seed n, run (and shrink) only it
//!   FRED_FUZZ_PROGRESS=1 print progress every 1000 sequences
//!   FRED_FUZZ_TRACE=1    with FRED_FUZZ_CASE: print every step as it is fed
//!   FRED_FUZZ_KEYS=k     run (and shrink) just these keys on FRED_FUZZ_TEXT
//!   FRED_FUZZ_ODD=1      generate CR/VT/FF/NEL/LS/PS even while their bug is
//!                        worked around

use fred::buffer::Buffer;
use fred::editor::{Editor, Mode};
use fred::key::{Key, KeyCode, parse_keys};
use fred::text::{floor_grapheme, prev_grapheme};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_ITERS: u64 = 1500;
/// Stop feeding a case's keys once the buffer grows past these.
const MAX_BYTES: usize = 256 * 1024;
const MAX_LINES: usize = 5000;
/// Cap on `u` / `<C-r>` presses when unwinding history.
const HISTORY_CAP: usize = 10_000;
/// Shrinking budget (candidate runs) per failure.
const SHRINK_RUNS: usize = 6000;
/// A single run taking longer than this is reported as a hang.
const HANG_SECS: u64 = 120;

/// Known open bugs, each with a failing test of the same name in
/// `tests/fuzz_regressions.rs`. By default the harness works around them
/// (`Runner::steer` changes the step that would trigger one, and
/// `Runner::known_failure` excuses the invariant failure one causes) so it
/// keeps finding new bugs. `FRED_FUZZ_KNOWN=1` turns every workaround off;
/// `FRED_FUZZ_KNOWN=name,name` turns off just those.
const KNOWN: &[(&str, &str)] = &[];

fn workaround(name: &str) -> bool {
    static OFF: OnceLock<Option<Vec<String>>> = OnceLock::new();
    // Bugs that are fixed are no longer listed: no workaround for them.
    if !KNOWN.iter().any(|(n, _)| *n == name) {
        return false;
    }
    let off = OFF.get_or_init(|| match std::env::var("FRED_FUZZ_KNOWN") {
        Err(_) => Some(vec![]),
        Ok(v) if v.is_empty() || v == "0" => Some(vec![]),
        Ok(v) if v == "1" || v == "all" => None,
        Ok(v) => Some(v.split(',').map(|s| s.trim().to_string()).collect()),
    });
    match off {
        None => false,
        Some(list) => !list.iter().any(|n| n == name),
    }
}

// ---------------------------------------------------------------------------
// PRNG (xorshift64, seeded through splitmix64)

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(z | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }

    fn weighted(&mut self, ws: &[usize]) -> usize {
        let mut x = self.below(ws.iter().sum());
        for (i, &w) in ws.iter().enumerate() {
            if x < w {
                return i;
            }
            x -= w;
        }
        ws.len() - 1
    }
}

// ---------------------------------------------------------------------------
// Cases

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Key(Key),
    /// Bracketed paste (`Editor::paste`).
    Paste(String),
}

#[derive(Clone, Debug)]
struct Case {
    /// File contents handed to `Buffer::from_text`.
    text: String,
    steps: Vec<Step>,
}

const WORDS: &[&str] = &[
    "foo", "bar", "baz", "a", "b", "x", "foo_bar", "x1", "Foo", "BAR", "hello", "help", "world",
    "if", "fn", "the", "abc", "aaa", "_", "42",
];
const PUNCT: &[&str] = &[
    ".", ",", ";", ":", "(", ")", "{", "}", "[", "]", "->", "!", "=", "#", "/", "\\", "'", "\"",
    "-", "+", "*", "$", "^", "&", "|", "<", ">", "?", "@", "~", "%", "`",
];
/// Combining marks, wide chars, emoji with modifiers / ZWJ / flags, controls,
/// odd whitespace, case-folding oddities.
const UNI: &[&str] = &[
    "e\u{301}",
    "漢字",
    "漢",
    "👍🏽",
    "👍",
    "👨\u{200d}👩\u{200d}👧",
    "\u{1}",
    "\u{7f}",
    "é",
    "ñ",
    "ß",
    "🇺🇸",
    "🇺",
    "\u{200d}",
    "\u{301}",
    "Ω",
    "→",
    "€",
    "—",
    "\u{a0}",
    "\u{3000}",
    "ｘ",
    "İ",
    "ﬀ",
    "a\u{308}\u{301}",
    "\u{feff}",
    "ä",
    "\u{1b}",
];
/// Characters other than `\n` that ropey (with its default `unicode_lines`
/// feature) also treats as line breaks.
const ODD_BREAKS: &[&str] = &["\r", "\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"];
const INDENT: &[&str] = &["  ", "    ", "\t", "\t\t", " \t", "        ", " "];
const RAW_CHARS: &[char] = &[
    'a', 'b', 'd', 'c', 'y', 'g', 'G', 'x', 'p', 'u', 'i', 'o', 'v', 'V', 'q', 'z', 'Z', 'r', 'm',
    'f', 't', 'n', 'N', '0', '1', '9', '.', ':', '/', '?', '\'', '"', '~', '&', '@', '<', '>', '=',
    '!', '*', '#', '%', '|', '\\', '[', ']', '{', '}', ' ', '漢', 'é', '\u{301}', '👍',
];

fn gen_line(r: &mut Rng, odd: bool) -> String {
    if r.one_in(6) {
        return String::new();
    }
    let mut s = String::new();
    if r.one_in(3) {
        s.push_str(r.pick(INDENT));
    }
    let n = if r.one_in(8) {
        r.range(8, 25)
    } else {
        r.range(1, 7)
    };
    for _ in 0..n {
        match r.weighted(&[40, 15, 18, 3, 12, 2, 1]) {
            0 => s.push_str(r.pick(WORDS)),
            1 => s.push_str(r.pick(PUNCT)),
            2 => s.push(' '),
            3 => s.push('\t'),
            4 => s.push_str(r.pick(UNI)),
            5 if odd => s.push_str(r.pick(ODD_BREAKS)),
            5 => s.push('\t'),
            _ => s.push_str(&"x".repeat(r.range(40, 200))),
        }
    }
    s
}

fn gen_text(r: &mut Rng, odd: bool) -> String {
    match r.below(40) {
        0 => return String::new(),
        1 => return "\n".into(),
        2 => return "\r\n".into(),
        3 => return "\u{feff}".into(),
        4 => return "\n\n\n".into(),
        _ => {}
    }
    let nlines = if r.one_in(10) {
        r.range(15, 60)
    } else {
        r.range(1, 10)
    };
    let crlf = r.one_in(4);
    let mixed = r.one_in(12);
    let final_nl = !r.one_in(3);
    let mut s = String::new();
    if r.one_in(10) {
        s.push('\u{feff}');
    }
    for i in 0..nlines {
        s.push_str(&gen_line(r, odd));
        if i + 1 < nlines || final_nl {
            let use_crlf = if mixed { r.one_in(2) } else { crlf };
            s.push_str(if use_crlf { "\r\n" } else { "\n" });
        }
    }
    s
}

struct Gen {
    r: Rng,
    steps: Vec<Step>,
    /// Characters of the initial text (targets for f/t/r and regex atoms).
    chars: Vec<char>,
    /// Lines in the initial text (for addresses).
    lines: usize,
    /// Generate line breaks other than `\n` (see `non_lf_line_breaks_split_lines`).
    odd: bool,
}

fn regex_escape(c: char) -> String {
    if "\\.+*?()|[]{}^$#&-~".contains(c) {
        format!("\\{c}")
    } else {
        c.to_string()
    }
}

impl Gen {
    fn k(&mut self, notation: &str) {
        for k in parse_keys(notation) {
            self.steps.push(Step::Key(k));
        }
    }

    fn typed(&mut self, s: &str) {
        for c in s.chars() {
            self.steps.push(Step::Key(Key::ch(c)));
        }
    }

    /// A count prefix: usually none or small, rarely huge (saturates at 10^6).
    fn count(&mut self) -> String {
        match self.r.below(96) {
            0..=59 => String::new(),
            60..=81 => self.r.range(1, 4).to_string(),
            82..=93 => self.r.range(5, 40).to_string(),
            _ => self
                .r
                .pick(&["99999", "999999", "1000000", "18446744073709551617"])
                .to_string(),
        }
    }

    /// A count for commands whose cost grows with it (p, P, .): at most 40.
    fn small_count(&mut self) -> String {
        match self.r.below(4) {
            0 | 1 => String::new(),
            2 => self.r.range(1, 4).to_string(),
            _ => self.r.range(5, 40).to_string(),
        }
    }

    fn target_char(&mut self) -> char {
        if !self.chars.is_empty() && self.r.below(3) != 0 {
            return self.r.pick(&self.chars);
        }
        self.r.pick(&[
            ' ', 'a', 'x', '(', ')', ',', '.', '漢', 'é', '\u{301}', '👍', '→', '€', '\u{1}', '<',
            '\t', '\u{200d}', 'Z', 'o',
        ])
    }

    /// A motion's keys, and whether a huge count on it is cheap. (Counted
    /// word motions and n/N cost time per step even when they no longer
    /// move; in a debug build a count of 10^6 takes minutes.)
    fn motion_keys(&mut self) -> (Vec<Step>, bool) {
        const SIMPLE: &[&str] = &[
            "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "0", "^", "$", "gg", "G", "{", "}",
            ";", ",", "n", "N", "+", "-", "<Left>", "<Right>", "<Up>", "<Down>", "<Home>", "<End>",
            "<Enter>", "<BS>", " ", "<C-n>", "<C-p>", "j", "k", "w", "l", "h", "e", "b",
        ];
        match self.r.below(10) {
            0 | 1 => {
                let f = self.r.pick(&['f', 'F', 't', 'T']);
                let c = self.target_char();
                (vec![Step::Key(Key::ch(f)), Step::Key(Key::ch(c))], true)
            }
            2 if self.r.one_in(2) => {
                let m = self.r.pick(&['a', 'a', 'b', 'z', '<', '>', 'A']);
                (vec![Step::Key(Key::ch('\'')), Step::Key(Key::ch(m))], true)
            }
            _ => {
                let m = self.r.pick(SIMPLE);
                let cheap = !matches!(m, "w" | "b" | "e" | "W" | "B" | "E" | "n" | "N");
                (parse_keys(m).into_iter().map(Step::Key).collect(), cheap)
            }
        }
    }

    /// Count for a motion: possibly huge when that is cheap, else at most 40.
    fn count_for(&mut self, cheap: bool) -> String {
        if cheap {
            self.count()
        } else {
            self.small_count()
        }
    }

    fn motion(&mut self) {
        let (keys, cheap) = self.motion_keys();
        let c = self.count_for(cheap);
        self.typed(&c);
        self.steps.extend(keys);
    }

    fn normal(&mut self) {
        let w = self.r.weighted(&[
            30, // 0 motion
            3,  // 1 scroll
            3,  // 2 {n}G / {n}gg
            14, // 3 operator + motion
            6,  // 4 dd cc yy
            9,  // 5 x X D Y J p P <Del>
            4,  // 6 s S C
            3,  // 7 r{c}
            10, // 8 i a I A o O
            7,  // 9 u <C-r>
            5,  // 10 .
            2,  // 11 m{c}
            6,  // 12 Visual-line
            10, // 13 :ex
            4,  // 14 / ?
            2,  // 15 paste
            3,  // 16 raw keys
            2,  // 17 <Esc>
        ]);
        match w {
            0 => self.motion(),
            1 => {
                let c = self.count();
                self.typed(&c);
                let k =
                    self.r
                        .pick(&["<PageUp>", "<PageDown>", "<C-d>", "<C-u>", "<C-f>", "<C-b>"]);
                self.k(k);
            }
            2 => {
                let n = self.r.range(0, self.lines + 3);
                self.typed(&n.to_string());
                let k = self.r.pick(&["G", "gg"]);
                self.k(k);
            }
            3 => {
                let (keys, cheap) = self.motion_keys();
                let c = self.count_for(cheap);
                self.typed(&c);
                let op = self.r.pick(&['d', 'c', 'y', 'd']);
                self.steps.push(Step::Key(Key::ch(op)));
                if self.r.one_in(6) {
                    let c2 = self.count_for(cheap);
                    self.typed(&c2);
                }
                self.steps.extend(keys);
                if op == 'c' {
                    self.insert_session();
                }
            }
            4 => {
                let c = self.count();
                self.typed(&c);
                let op = self.r.pick(&["dd", "cc", "yy"]);
                self.k(op);
                if op == "cc" {
                    self.insert_session();
                }
            }
            5 => {
                let k = self
                    .r
                    .pick(&["x", "X", "D", "Y", "J", "p", "P", "<Del>", "x", "p"]);
                let c = if matches!(k, "p" | "P") {
                    self.small_count()
                } else {
                    self.count()
                };
                self.typed(&c);
                self.k(k);
            }
            6 => {
                let c = self.count();
                self.typed(&c);
                let k = self.r.pick(&["s", "S", "C"]);
                self.k(k);
                self.insert_session();
            }
            7 => {
                let c = self.count();
                self.typed(&c);
                let ch = self.target_char();
                self.steps.push(Step::Key(Key::ch('r')));
                self.steps.push(Step::Key(Key::ch(ch)));
            }
            8 => {
                if self.r.one_in(6) {
                    let c = self.count();
                    self.typed(&c);
                }
                let k = self.r.pick(&["i", "a", "I", "A", "o", "O"]);
                self.k(k);
                self.insert_session();
            }
            9 => {
                if self.r.one_in(5) {
                    let c = self.count();
                    self.typed(&c);
                }
                let k = self.r.pick(&["u", "<C-r>", "u"]);
                self.k(k);
            }
            10 => {
                if self.r.one_in(4) {
                    let c = self.small_count();
                    self.typed(&c);
                }
                self.k(".");
            }
            11 => {
                let m = self.r.pick(&['a', 'b', 'c', 'z', '<', 'A', '1']);
                self.steps.push(Step::Key(Key::ch('m')));
                self.steps.push(Step::Key(Key::ch(m)));
            }
            12 => self.visual(),
            13 => self.ex(),
            14 => self.search(),
            15 => self.paste(),
            16 => self.raw(),
            _ => self.k("<Esc>"),
        }
    }

    fn visual(&mut self) {
        self.k("V");
        for _ in 0..self.r.below(4) {
            match self.r.below(14) {
                0..=7 => self.motion(),
                8 => self.k("o"),
                9 => {
                    if self.r.one_in(3) {
                        let c = self.count();
                        self.typed(&c);
                    }
                    self.k("<C-r>");
                }
                10 => {
                    if self.r.one_in(3) {
                        let c = self.count();
                        self.typed(&c);
                    }
                    self.k("<Del>");
                }
                11 => self.k("ma"),
                12 => {
                    let k = self.r.pick(&["<C-d>", "<C-u>", "<PageDown>", "<PageUp>"]);
                    self.k(k);
                }
                _ => self.k("u"),
            }
        }
        match self.r.below(17) {
            0 => self.k("d"),
            1 => self.k("x"),
            2 => self.k("X"),
            3 => self.k("D"),
            4 => self.k("y"),
            5 => self.k("Y"),
            6..=9 => {
                let k = self.r.pick(&["c", "s", "S", "C"]);
                self.k(k);
                self.insert_session();
            }
            10 => self.k("J"),
            11 | 12 => {
                self.k(":");
                let cmd = self.ex_command();
                self.cmdline_text(&cmd);
                self.cmdline_end();
            }
            13 => self.k("V"),
            14 => self.k("<Esc>"),
            15 => self.k("o"),
            _ => {}
        }
    }

    fn insert_chunk(&mut self) {
        let s: String = match self.r.below(13) {
            0..=4 => self.r.pick(WORDS).to_string(),
            5 => " ".into(),
            6 => self.r.pick(PUNCT).to_string(),
            7 | 8 => self.r.pick(UNI).to_string(),
            9 if !self.chars.is_empty() => self.r.pick(&self.chars).to_string(),
            10 => format!("{} ", self.r.pick(WORDS)),
            11 if self.odd => self.r.pick(ODD_BREAKS).to_string(),
            _ => self
                .r
                .pick(&["he", "fo", "ba", "wo", "Fo", "x"])
                .to_string(),
        };
        self.typed(&s);
    }

    fn insert_session(&mut self) {
        let n = if self.r.one_in(8) {
            self.r.range(8, 25)
        } else {
            self.r.range(0, 7)
        };
        for _ in 0..n {
            match self
                .r
                .weighted(&[30, 7, 6, 3, 3, 3, 2, 1, 3, 2, 2, 5, 2, 2])
            {
                0 => self.insert_chunk(),
                1 => self.k("<Enter>"),
                2 => self.k("<BS>"),
                3 => self.k("<Del>"),
                4 => self.k("<Tab>"),
                5 => self.k("<C-w>"),
                6 => self.k("<C-u>"),
                7 => self.k("<C-h>"),
                8 => self.k("<C-n>"),
                9 => self.k("<C-p>"),
                10 => self.k("<S-Tab>"),
                11 => {
                    let k = self.r.pick(&["<Left>", "<Right>", "<Up>", "<Down>"]);
                    self.k(k);
                }
                12 => {
                    let k = self.r.pick(&["<Home>", "<End>"]);
                    self.k(k);
                }
                _ => self.paste(),
            }
        }
        match self.r.below(14) {
            0 => {} // leave the insert session open
            1 => self.k("<C-c>"),
            _ => self.k("<Esc>"),
        }
    }

    fn addr(&mut self) -> String {
        let mut s = match self.r.below(15) {
            0..=4 => self.r.range(0, self.lines + 2).to_string(),
            5 => ".".into(),
            6 => "$".into(),
            7 => format!("/{}/", self.regex('/')),
            8 => format!("?{}?", self.regex('?')),
            9 => format!("'{}", self.r.pick(&['a', 'b', '<', '>', 'z'])),
            10 => self
                .r
                .pick(&["99999", "18446744073709551616", "9223372036854775807"])
                .to_string(),
            _ => String::new(),
        };
        for _ in 0..self.r.below(3) {
            let sign = self.r.pick(&["+", "-", "^", " +", "+ ", "-"]);
            let n = match self.r.below(10) {
                0 => String::new(),
                1 => "9223372036854775807".into(),
                _ => self.r.range(0, 5).to_string(),
            };
            s.push_str(sign);
            s.push_str(&n);
        }
        s
    }

    fn regex(&mut self, delim: char) -> String {
        let mut s = String::new();
        for _ in 0..self.r.range(1, 3) {
            let atom: String = match self.r.below(25) {
                0..=6 => self.r.pick(WORDS).to_string(),
                7 => ".".into(),
                8 => "^".into(),
                9 => "$".into(),
                10 => "a*".into(),
                11 => "x?".into(),
                12 => "[ab]".into(),
                13 => "(o)".into(),
                14 => "(fo|ba)".into(),
                15 => "\\s+".into(),
                16 => "\\d".into(),
                17 => "\\b".into(),
                18 => self.r.pick(UNI).to_string(),
                19 if !self.chars.is_empty() => regex_escape(self.r.pick(&self.chars)),
                20 => " ".into(),
                21 => self
                    .r
                    .pick(&["(", "[", "*", "\\", "a{3,1}", "(?<", "\\p{Nope}", "+", ")"])
                    .to_string(),
                22 => "\\w".into(),
                23 => ".*".into(),
                _ => "(a)(b)?".into(),
            };
            s.push_str(&atom);
        }
        let mut out = String::new();
        for c in s.chars() {
            if c == delim {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    fn replacement(&mut self, delim: char) -> String {
        let mut s = String::new();
        for _ in 0..self.r.below(4) {
            let piece: String = match self.r.below(14) {
                0..=3 => self.r.pick(WORDS).to_string(),
                4 => "&".into(),
                5 => "\\1".into(),
                6 => "\\2".into(),
                7 => "\\n".into(),
                8 => "\\t".into(),
                9 => "\\\\".into(),
                10 => "\\&".into(),
                11 => self.r.pick(UNI).to_string(),
                12 => " ".into(),
                _ => "\\".into(),
            };
            s.push_str(&piece);
        }
        let mut out = String::new();
        for c in s.chars() {
            if c == delim {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    fn subst(&mut self) -> String {
        let d = self.r.pick(&['/', '/', '/', '#', '|', ',', '\\', '→']);
        let re = if self.r.one_in(8) {
            String::new()
        } else {
            self.regex(d)
        };
        let rep = self.replacement(d);
        let flags = self.r.pick(&["", "", "g", "g", "gg", "x", " g", "G"]);
        match self.r.below(12) {
            0 => format!("s{d}{re}{d}{rep}"),
            1 => format!("s{d}{re}"),
            _ => format!("s{d}{re}{d}{rep}{d}{flags}"),
        }
    }

    fn global_cmd(&mut self) -> String {
        if self.r.one_in(4) {
            // Commands that work on some marked lines and fail on others.
            let w = self.r.pick(WORDS);
            let a = self.r.range(1, 3);
            return match self.r.below(5) {
                0 => format!("s/{w}/x/"),
                1 => format!(".,+{a}d"),
                2 => format!(".,+{a}j"),
                3 => format!("m+{a}"),
                _ => format!("s/{w}/\\n/g"),
            };
        }
        match self.r.below(16) {
            0..=2 => "d".into(),
            3..=5 => self.subst(),
            6 => "j".into(),
            7 => "m0".into(),
            8 => "m$".into(),
            9 => "t.".into(),
            10 => "t$".into(),
            11 => self
                .r
                .pick(&[".,+1d", "-1d", "+1j", ".,+2j", "s//x/", "5", "$", "1m$"])
                .to_string(),
            12 => self
                .r
                .pick(&["g/a/d", "v/a/d", "", " ", "w", "q", "zz", "p"])
                .to_string(),
            13 => {
                let a = self.addr();
                format!("{a}d")
            }
            14 => {
                let a = self.addr();
                format!("m{a}")
            }
            _ => {
                let a = self.addr();
                format!("t{a}")
            }
        }
    }

    fn ex_command(&mut self) -> String {
        match self.r.weighted(&[8, 5, 5, 5, 14, 8, 3, 3, 3, 2, 2]) {
            0 => "d".into(),
            1 => "j".into(),
            2 => {
                let a = self.addr();
                format!("m{a}")
            }
            3 => {
                let a = self.addr();
                format!("t{a}")
            }
            4 => self.subst(),
            5 => {
                let g = self.r.pick(&['g', 'v', 'g']);
                let d = self.r.pick(&['/', '/', '#', '|']);
                let re = self.regex(d);
                let cmd = self.global_cmd();
                format!("{g}{d}{re}{d}{cmd}")
            }
            6 => String::new(),
            7 => {
                let n = self.r.range(1, 6);
                (0..n).map(|_| self.r.pick(RAW_CHARS)).collect()
            }
            8 => self
                .r
                .pick(&[
                    "w", "q", "e x", "x", "wq", "q!", "w!", "e", "e! foo", "write", "edit", "xit",
                    "wq foo",
                ])
                .to_string(),
            9 => self
                .r
                .pick(&[
                    "k", "p", "a", "u", "!ls", "z", "d x", "j 2", "m", "t", "mx", "s", "g", "v",
                    "s/", "g/", "&", "~",
                ])
                .to_string(),
            _ => self
                .r
                .pick(&[" d", "d ", " s/a/b/", "j ", "  "])
                .to_string(),
        }
    }

    fn ex(&mut self) {
        self.k(":");
        let range = match self.r.below(12) {
            0..=4 => String::new(),
            5 | 6 => self.addr(),
            7 | 8 => {
                let a = self.addr();
                let b = self.addr();
                format!("{a},{b}")
            }
            9 => {
                let a = self.addr();
                let b = self.addr();
                format!("{a};{b}")
            }
            10 => self.r.pick(&[",", ";", "%", "'<,'>"]).to_string(),
            _ => {
                let a = self.addr();
                format!("{a},")
            }
        };
        let cmd = self.ex_command();
        self.cmdline_text(&format!("{range}{cmd}"));
        self.cmdline_end();
    }

    fn search(&mut self) {
        let k = self.r.pick(&['/', '?', '/']);
        self.steps.push(Step::Key(Key::ch(k)));
        let re = if self.r.one_in(8) {
            String::new()
        } else {
            self.regex('\0')
        };
        self.cmdline_text(&re);
        self.cmdline_end();
    }

    fn cmdline_text(&mut self, s: &str) {
        for c in s.chars() {
            self.steps.push(Step::Key(Key::ch(c)));
            if self.r.one_in(30) {
                self.cmdline_edit();
            }
        }
        if self.r.one_in(12) {
            self.cmdline_edit();
        }
    }

    fn cmdline_edit(&mut self) {
        match self.r.below(13) {
            0 => self.k("<Left>"),
            1 => self.k("<Right>"),
            2 => self.k("<Home>"),
            3 => self.k("<End>"),
            4 => self.k("<BS>"),
            5 => self.k("<Del>"),
            6 => self.k("<C-w>"),
            7 => self.k("<C-u>"),
            8 => self.k("<Up>"),
            9 => self.k("<Down>"),
            10 => self.k("<Tab>"),
            11 => self.paste(),
            _ => {
                let c = self.target_char();
                self.steps.push(Step::Key(Key::ch(c)));
            }
        }
    }

    fn cmdline_end(&mut self) {
        match self.r.below(24) {
            0 => self.k("<Esc>"),
            1 => self.k("<C-c>"),
            2 => {} // leave it open: what follows is typed into it
            _ => self.k("<Enter>"),
        }
    }

    fn raw(&mut self) {
        for _ in 0..self.r.range(1, 4) {
            let k = match self.r.below(10) {
                0..=4 => Key::ch(self.r.pick(RAW_CHARS)),
                5 | 6 => Key::ctrl(self.r.pick(&[
                    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'n', 'p', 'r', 'u', 'w', 'v', 'o', 'z',
                    '[', ']',
                ])),
                _ => Key::new(self.r.pick(&[
                    KeyCode::Esc,
                    KeyCode::Enter,
                    KeyCode::Backspace,
                    KeyCode::Tab,
                    KeyCode::BackTab,
                    KeyCode::Up,
                    KeyCode::Down,
                    KeyCode::Left,
                    KeyCode::Right,
                    KeyCode::Home,
                    KeyCode::End,
                    KeyCode::PageUp,
                    KeyCode::PageDown,
                    KeyCode::Delete,
                ])),
            };
            self.steps.push(Step::Key(k));
        }
    }

    fn paste_text(&mut self) -> String {
        match self.r.below(12) {
            0 => String::new(),
            1 => "\r\n".into(),
            2 => self
                .r
                .pick(&[
                    "\u{301}",
                    "\u{200d}",
                    "🇸",
                    "🇺",
                    "👨\u{200d}",
                    "\u{1}",
                    "🏽",
                    "\r",
                    "e",
                    "👍",
                ])
                .to_string(),
            3 | 4 => {
                let n = self.r.range(2, 4);
                let sep = self.r.pick(&["\n", "\r\n", "\r", "\n\n"]);
                (0..n)
                    .map(|_| gen_line(&mut self.r, self.odd))
                    .collect::<Vec<_>>()
                    .join(sep)
            }
            _ => {
                let mut s = String::new();
                for _ in 0..self.r.range(1, 4) {
                    match self.r.below(4) {
                        0 => s.push_str(self.r.pick(UNI)),
                        1 => s.push(' '),
                        _ => s.push_str(self.r.pick(WORDS)),
                    }
                }
                s
            }
        }
    }

    fn paste(&mut self) {
        let t = self.paste_text();
        self.steps.push(Step::Paste(t));
    }
}

fn gen_case(seed: u64) -> Case {
    let mut r = Rng::new(seed);
    let odd = !workaround("non_lf_line_breaks_split_lines") || env_flag("FRED_FUZZ_ODD");
    let text = gen_text(&mut r, odd);
    let mut chars: Vec<char> = text.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
    chars.sort_unstable();
    chars.dedup();
    let lines = text.matches('\n').count() + 1;
    let mut g = Gen {
        r,
        steps: vec![],
        chars,
        lines,
        odd,
    };
    let n = if g.r.one_in(10) {
        g.r.range(60, 200)
    } else {
        g.r.range(1, 40)
    };
    for _ in 0..n {
        g.normal();
    }
    Case {
        text,
        steps: g.steps,
    }
}

// ---------------------------------------------------------------------------
// Rendering (vim key notation as understood by `parse_keys`)

fn notation(k: &Key) -> String {
    let named = match k.code {
        KeyCode::Char(c) if k.ctrl => return format!("<C-{c}>"),
        KeyCode::Char('<') => "<lt>",
        KeyCode::Char(c) => return c.to_string(),
        KeyCode::Esc => "<Esc>",
        KeyCode::Enter => "<Enter>",
        KeyCode::Backspace => "<BS>",
        KeyCode::Tab => "<Tab>",
        KeyCode::BackTab => "<S-Tab>",
        KeyCode::Up => "<Up>",
        KeyCode::Down => "<Down>",
        KeyCode::Left => "<Left>",
        KeyCode::Right => "<Right>",
        KeyCode::Home => "<Home>",
        KeyCode::End => "<End>",
        KeyCode::PageUp => "<PageUp>",
        KeyCode::PageDown => "<PageDown>",
        KeyCode::Delete => "<Del>",
    };
    named.to_string()
}

/// Steps as `keys "..."` / `paste "..."` segments.
fn render(steps: &[Step]) -> String {
    let mut out: Vec<String> = vec![];
    let mut keys = String::new();
    for s in steps {
        match s {
            Step::Key(k) => keys.push_str(&notation(k)),
            Step::Paste(t) => {
                if !keys.is_empty() {
                    out.push(format!("keys {keys:?}"));
                    keys.clear();
                }
                out.push(format!("paste {t:?}"));
            }
        }
    }
    if !keys.is_empty() {
        out.push(format!("keys {keys:?}"));
    }
    if out.is_empty() {
        "(no steps)".into()
    } else {
        out.join(" ; ")
    }
}

// ---------------------------------------------------------------------------
// Running a case

#[derive(Clone, Debug)]
struct Failure {
    /// What broke: an invariant name, or `panic <location>`.
    sig: String,
    /// Step after which it was detected (`steps.len()` = end-of-sequence checks).
    step: usize,
    detail: String,
}

fn fail<T>(sig: &str, step: usize, detail: String) -> Result<T, Failure> {
    Err(Failure {
        sig: sig.to_string(),
        step,
        detail,
    })
}

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
    static PANIC: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    static STEP: Cell<usize> = const { Cell::new(0) };
    static SLOT: Cell<usize> = const { Cell::new(0) };
    static TRACE: Cell<bool> = const { Cell::new(false) };
}

/// Per worker: start of the current run (ms since `epoch()`, 0 = idle), seed, step.
static RUN_START: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static RUN_SEED: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static RUN_STEP: [AtomicUsize; 64] = [const { AtomicUsize::new(0) }; 64];

fn epoch() -> Instant {
    static E: Mutex<Option<Instant>> = Mutex::new(None);
    *E.lock().unwrap().get_or_insert_with(Instant::now)
}

fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64 + 1
}

/// First frame of this crate's `src/` in a backtrace, as `src/file.rs:line`.
fn fred_frame(bt: &str) -> Option<String> {
    for line in bt.lines() {
        let Some(path) = line.trim().strip_prefix("at ") else {
            continue;
        };
        if path.contains("/.cargo/") || path.contains("/rustc/") || path.contains("tests/") {
            continue;
        }
        let Some(i) = path.find("src/") else {
            continue;
        };
        let p = &path[i..];
        // drop the column
        let p = match p.rsplit_once(':') {
            Some((head, _)) if head.contains(':') => head,
            _ => p,
        };
        return Some(p.to_string());
    }
    None
}

fn install_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        epoch();
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                return default(info);
            }
            let p = info.payload();
            let msg = if let Some(s) = p.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = p.downcast_ref::<String>() {
                s.clone()
            } else {
                "<non-string panic payload>".to_string()
            };
            let loc = info
                .location()
                .map_or_else(String::new, |l| format!("{}:{}", l.file(), l.line()));
            let frame = if loc.starts_with("src/") {
                loc.clone()
            } else {
                let bt = std::backtrace::Backtrace::force_capture().to_string();
                fred_frame(&bt).unwrap_or_else(|| loc.clone())
            };
            PANIC.with(|c| *c.borrow_mut() = Some((frame, format!("{msg} (at {loc})"))));
        }));
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let now = now_ms();
                for i in 0..64 {
                    let start = RUN_START[i].load(Ordering::Relaxed);
                    if start != 0 && now.saturating_sub(start) > HANG_SECS * 1000 {
                        eprintln!(
                            "fuzz: HANG: seed {} stuck at step {} for over {HANG_SECS}s",
                            RUN_SEED[i].load(Ordering::Relaxed),
                            RUN_STEP[i].load(Ordering::Relaxed)
                        );
                        std::process::exit(101);
                    }
                }
            }
        });
    });
}

fn set_step(i: usize) {
    STEP.with(|s| s.set(i));
    RUN_STEP[SLOT.with(Cell::get)].store(i, Ordering::Relaxed);
}

fn run(seed: u64, case: &Case) -> Result<(), Failure> {
    let slot = SLOT.with(Cell::get);
    RUN_SEED[slot].store(seed, Ordering::Relaxed);
    RUN_START[slot].store(now_ms(), Ordering::Relaxed);
    QUIET.with(|q| q.set(true));
    PANIC.with(|p| *p.borrow_mut() = None);
    set_step(0);
    let r = catch_unwind(AssertUnwindSafe(|| exec(case)));
    QUIET.with(|q| q.set(false));
    RUN_START[slot].store(0, Ordering::Relaxed);
    match r {
        Ok(r) => r,
        Err(_) => {
            let (frame, detail) = PANIC
                .with(|p| p.borrow_mut().take())
                .unwrap_or_else(|| ("?".into(), "?".into()));
            fail(&format!("panic {frame}"), STEP.with(Cell::get), detail)
        }
    }
}

fn check_state(ed: &Editor, step: usize) -> Result<(), Failure> {
    let n = ed.buf.len_lines();
    let c = ed.cur;
    let mode = &ed.mode;
    if c.line >= n {
        return fail(
            "cursor-line-out-of-range",
            step,
            format!("cursor {:?}, {n} lines, {mode:?}", c.pos()),
        );
    }
    let line = ed.buf.line(c.line);
    if c.byte > line.len() {
        return fail(
            "cursor-past-end-of-line",
            step,
            format!("cursor {:?} in {line:?}, {mode:?}", c.pos()),
        );
    }
    if floor_grapheme(&line, c.byte) != c.byte {
        return fail(
            "cursor-not-on-grapheme-boundary",
            step,
            format!("cursor {:?} in {line:?}, {mode:?}", c.pos()),
        );
    }
    let insert = *mode == Mode::Insert;
    if !insert && c.byte > prev_grapheme(&line, line.len()) {
        return fail(
            "cursor-past-last-grapheme",
            step,
            format!("cursor {:?} in {line:?}, {mode:?}", c.pos()),
        );
    }
    if let Mode::VisualLine { anchor } = *mode
        && anchor >= n
    {
        return fail(
            "visual-anchor-out-of-range",
            step,
            format!("anchor {anchor}, {n} lines"),
        );
    }
    if insert != ed.undo.in_group() {
        return fail(
            if insert {
                "insert-without-open-undo-group"
            } else {
                "undo-group-left-open"
            },
            step,
            format!("{mode:?}"),
        );
    }
    if ed.popup.is_some() && !insert {
        return fail("popup-outside-insert", step, format!("{mode:?}"));
    }
    Ok(())
}

/// Checks that only depend on the text (run when it changed).
fn check_text(ed: &Editor, text: &str, step: usize) -> Result<(), Failure> {
    let newlines = text.matches('\n').count();
    if ed.buf.len_lines() != newlines + 1 {
        return fail(
            "line-count-disagrees-with-newlines",
            step,
            format!(
                "{} lines but {newlines} '\\n' in {:?}",
                ed.buf.len_lines(),
                short(text)
            ),
        );
    }
    let bytes = ed.buf.to_bytes();
    let Ok(s) = String::from_utf8(bytes.clone()) else {
        return fail("to-bytes-not-utf8", step, String::new());
    };
    let again = Buffer::from_text(&s).to_bytes();
    // A line whose text ends in CR can't be written unambiguously: on disk
    // its "\r\n" reads back as a CRLF line ending (vim has the same limit).
    // Keyboard input can't produce one (Enter is a key, pastes are
    // normalized), so only check files without one.
    if again != bytes && !s.contains("\r\n") {
        return fail(
            "format-roundtrip-unstable",
            step,
            format!(
                "to_bytes {:?} reloads as {:?}",
                short(&s),
                short(&String::from_utf8_lossy(&again))
            ),
        );
    }
    Ok(())
}

fn longest_digit_run(s: &str) -> usize {
    s.split(|c: char| !c.is_ascii_digit())
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn short(s: &str) -> String {
    if s.chars().count() <= 200 {
        s.to_string()
    } else {
        let head: String = s.chars().take(200).collect();
        format!("{head}…")
    }
}

/// Fingerprint of the text at an undo-history state.
struct Snap {
    len: usize,
    hash: u64,
    /// The text itself when small (for messages).
    text: Option<String>,
}

impl Snap {
    fn of(text: &str) -> Snap {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        text.hash(&mut h);
        Snap {
            len: text.len(),
            hash: h.finish(),
            text: (text.len() <= 4096).then(|| text.to_string()),
        }
    }
}

/// What the harness knew right before a step.
struct Before {
    cmdline_enter: bool,
    dot: bool,
    msg: Option<(String, bool)>,
}

struct Runner {
    ed: Editor,
    /// Text after the last step (kept in sync via `buf.version`).
    text: String,
    version: u64,
    last_id: u64,
    /// Undo-history state id -> text seen at that state ("equal ids mean
    /// equal text"). A state must look the same whenever it is revisited, and
    /// redo must only reach states that were actually seen.
    seen: std::collections::HashMap<u64, Snap>,
    max_seen: u64,
    /// A command line failed after editing (its edits were rolled back).
    failed_cmdline_edited: bool,
    /// Harness-side guess of the count being typed in Normal/Visual mode.
    count: CountGuess,
    /// n or N was used in this case.
    searched: bool,
}

/// Tracks digits typed before a Normal-mode command (approximately: keys
/// that are arguments, like the `3` in `f3`, are miscounted).
#[derive(Default)]
struct CountGuess {
    /// Product of finished count runs (`3d2w` -> 3 when at `2`), 0 = none.
    product: u64,
    run: u64,
    op: bool,
}

impl CountGuess {
    /// Feed a Normal-mode key; returns the effective count once a command
    /// completes (1 if none was typed).
    fn key(&mut self, k: &Key) -> Option<u64> {
        if let Some(c) = k.char() {
            if let Some(d) = c.to_digit(10)
                && (d != 0 || self.run > 0)
            {
                self.run = (self.run * 10 + u64::from(d)).min(1_000_000);
                return None;
            }
            if matches!(c, 'd' | 'c' | 'y') && !self.op {
                self.product = self.product.max(1) * self.run.max(1);
                self.run = 0;
                self.op = true;
                return None;
            }
        }
        let n = (self.product.max(1) * self.run.max(1)).min(1_000_000);
        *self = CountGuess::default();
        Some(n)
    }
}

impl Runner {
    /// Known-bug steering: `Some((bug, replacement))` = don't feed `step`;
    /// feed `replacement` (if any) instead.
    fn steer(&mut self, step: &Step) -> Option<(&'static str, Option<Step>)> {
        let Step::Key(k) = step else { return None };
        let esc = Some(Step::Key(Key::new(KeyCode::Esc)));
        match &self.ed.mode {
            Mode::Command(cl) => {
                self.count = CountGuess::default();
                let bug = "cmdline_ctrl_w_after_multibyte_char_panics";
                if *k == Key::ctrl('w') && workaround(bug) {
                    // What `Editor::cmdline_key` computes for Ctrl-W.
                    let trimmed = cl.text[..cl.cursor].trim_end();
                    let word = |c: char| c.is_alphanumeric() || c == '_';
                    if let Some(i) = trimmed.rfind(|c: char| !word(c)) {
                        let start = if i + 1 == trimmed.len() { i } else { i + 1 };
                        if !cl.text.is_char_boundary(start) {
                            return Some((bug, None));
                        }
                    }
                }
                let bug = "huge_address_offset_overflows";
                if *k == Key::new(KeyCode::Enter)
                    && cl.kind == ':'
                    && workaround(bug)
                    && longest_digit_run(&cl.text) >= 19
                {
                    return Some((bug, esc));
                }
                None
            }
            Mode::Insert => {
                self.count = CountGuess::default();
                None
            }
            Mode::Normal | Mode::VisualLine { .. } => {
                let bug = "redo_in_visual_line_leaves_anchor_past_end";
                if matches!(self.ed.mode, Mode::VisualLine { .. })
                    && *k == Key::ctrl('r')
                    && workaround(bug)
                {
                    self.count = CountGuess::default();
                    return Some((bug, esc));
                }
                let n = self.count.key(k)?;
                let bug = "counted_search_freezes";
                if matches!(k.char(), Some('n' | 'N')) {
                    self.searched = true;
                    if n > 500 && workaround(bug) {
                        return Some((bug, esc));
                    }
                }
                if k.char() == Some('.') && n > 50 && self.searched && workaround(bug) {
                    return Some((bug, esc));
                }
                None
            }
        }
    }

    /// Known-bug excuses: `Some(bug)` if this failure is explained by one.
    fn known_failure(&self, f: &Failure, last: Option<&Step>) -> Option<&'static str> {
        let bug = match f.sig.as_str() {
            "line-count-disagrees-with-newlines" => "non_lf_line_breaks_split_lines",
            "redo-reached-unseen-state" if self.failed_cmdline_edited => {
                "failed_ex_command_partial_edits_are_redoable"
            }
            "cursor-not-on-grapheme-boundary"
                if matches!(last, Some(Step::Paste(_))) && self.ed.mode == Mode::Insert =>
            {
                "paste_in_insert_mode_leaves_cursor_inside_grapheme"
            }
            _ => return None,
        };
        workaround(bug).then_some(bug)
    }

    fn excuse(&self, r: Result<(), Failure>, last: Option<&Step>) -> Result<(), Failure> {
        match r {
            Err(f) if self.known_failure(&f, last).is_some() => Ok(()),
            r => r,
        }
    }

    fn check(&mut self, i: usize, step: Option<&Step>, before: &Before) -> Result<(), Failure> {
        self.excuse(check_state(&self.ed, i), step)?;
        let version = self.ed.buf.version;
        let changed = version != self.version;
        if changed {
            let text = self.ed.buf.text();
            let err = matches!(&self.ed.msg, Some((_, true)))
                && (matches!(step, Some(Step::Key(_))) || self.ed.msg != before.msg);
            if err && before.cmdline_enter {
                self.failed_cmdline_edited = true;
            }
            if err && text != self.text && (before.cmdline_enter || !before.dot) {
                let sig = if before.cmdline_enter {
                    "cmdline-error-changed-text"
                } else {
                    "error-changed-text"
                };
                let r = fail(
                    sig,
                    i,
                    format!(
                        "{:?}: {:?} -> {:?}",
                        self.ed.msg.as_ref().unwrap().0,
                        short(&self.text),
                        short(&text)
                    ),
                );
                self.excuse(r, step)?;
            }
            self.excuse(check_text(&self.ed, &text, i), step)?;
            self.text = text;
            self.version = version;
        }
        let id = self.ed.undo.state_id();
        if self.ed.undo.in_group() || (!changed && id == self.last_id) {
            return Ok(());
        }
        self.last_id = id;
        let snap = Snap::of(&self.text);
        match self.seen.get(&id) {
            Some(s) if s.len == snap.len && s.hash == snap.hash => Ok(()),
            Some(s) => {
                let r = fail(
                    "history-state-text-mismatch",
                    i,
                    format!(
                        "undo state {id} had text {:?}, now {:?}",
                        s.text.as_deref().map(short),
                        short(&self.text)
                    ),
                );
                self.excuse(r, step)
            }
            None => {
                let redo = matches!(step, Some(Step::Key(k)) if *k == Key::ctrl('r'));
                let r = if redo {
                    fail(
                        "redo-reached-unseen-state",
                        i,
                        format!("undo state {id} with text {:?}", short(&self.text)),
                    )
                } else if id <= self.max_seen {
                    fail(
                        "undo-reached-unseen-state",
                        i,
                        format!("undo state {id} with text {:?}", short(&self.text)),
                    )
                } else {
                    Ok(())
                };
                self.excuse(r, step)?;
                self.seen.insert(id, snap);
                self.max_seen = self.max_seen.max(id);
                Ok(())
            }
        }
    }

    fn feed(&mut self, i: usize, step: &Step) -> Result<(), Failure> {
        if TRACE.with(Cell::get) {
            let s = match step {
                Step::Key(k) => notation(k),
                Step::Paste(t) => format!("paste {t:?}"),
            };
            eprintln!(
                "  step {i}: {s}  [{:?}, {} lines, {} bytes, cur {:?}]",
                self.ed.mode,
                self.ed.buf.len_lines(),
                self.ed.buf.len_bytes(),
                self.ed.cur.pos()
            );
        }
        let before = Before {
            cmdline_enter: matches!(step, Step::Key(k) if *k == Key::new(KeyCode::Enter))
                && matches!(self.ed.mode, Mode::Command(_)),
            dot: matches!(step, Step::Key(k) if k.char() == Some('.'))
                && matches!(self.ed.mode, Mode::Normal),
            msg: self.ed.msg.clone(),
        };
        let t0 = Instant::now();
        match step {
            Step::Key(k) => self.ed.handle_key(*k),
            Step::Paste(s) => self.ed.paste(s),
        }
        self.ed.pending_effect = None;
        let t1 = Instant::now();
        let r = self.check(i, Some(step), &before);
        if TRACE.with(Cell::get) && t0.elapsed() > Duration::from_millis(20) {
            eprintln!(
                "    slow: step {i} took {:.1?} (+{:.1?} checking)",
                t1 - t0,
                t1.elapsed()
            );
        }
        r
    }
}

fn exec(case: &Case) -> Result<(), Failure> {
    let buf = Buffer::from_text(&case.text);
    let initial = buf.text();
    let ed = Editor::new(buf);
    let mut r = Runner {
        text: initial.clone(),
        version: ed.buf.version,
        last_id: ed.undo.state_id(),
        seen: std::collections::HashMap::from([(ed.undo.state_id(), Snap::of(&initial))]),
        max_seen: ed.undo.state_id(),
        ed,
        failed_cmdline_edited: false,
        count: CountGuess::default(),
        searched: false,
    };
    r.excuse(check_state(&r.ed, 0), None)?;
    r.excuse(check_text(&r.ed, &initial, 0), None)?;

    for (i, step) in case.steps.iter().enumerate() {
        set_step(i);
        match r.steer(step) {
            None => r.feed(i, step)?,
            Some((_, Some(instead))) => r.feed(i, &instead)?,
            Some((_, None)) => {}
        }
        if r.ed.buf.len_bytes() > MAX_BYTES || r.ed.buf.len_lines() > MAX_LINES {
            break;
        }
    }

    // End of sequence: back to Normal, then unwind and replay all history.
    let end = case.steps.len();
    set_step(end);
    let esc = Step::Key(Key::new(KeyCode::Esc));
    let undo = Step::Key(Key::ch('u'));
    let redo = Step::Key(Key::ctrl('r'));
    for _ in 0..2 {
        r.feed(end, &esc)?;
    }
    if r.ed.mode != Mode::Normal {
        return fail("esc-does-not-reach-normal", end, format!("{:?}", r.ed.mode));
    }
    let final_text = r.ed.buf.text();
    let final_id = r.ed.undo.state_id();
    let at = |r: &Runner, m: &str| matches!(&r.ed.msg, Some((t, true)) if t.contains(m));
    let mut presses = 0;
    loop {
        r.feed(end, &undo)?;
        if at(&r, "already at oldest change") {
            break;
        }
        presses += 1;
        if presses > HISTORY_CAP {
            return fail("undo-never-reaches-oldest", end, String::new());
        }
    }
    let undone = r.ed.buf.text();
    if undone != initial {
        return fail(
            "undo-all-does-not-restore-initial-text",
            end,
            format!(
                "initial {:?}, after undo-all {:?}",
                short(&initial),
                short(&undone)
            ),
        );
    }
    if r.ed.buf.modified {
        return fail("modified-after-undo-all", end, String::new());
    }
    let mut ids = vec![r.ed.undo.state_id()];
    presses = 0;
    loop {
        r.feed(end, &redo)?;
        if at(&r, "already at newest change") {
            break;
        }
        ids.push(r.ed.undo.state_id());
        presses += 1;
        if presses > HISTORY_CAP {
            return fail("redo-never-reaches-newest", end, String::new());
        }
    }
    if !ids.contains(&final_id) {
        return r.excuse(
            fail(
                "redo-all-skips-final-state",
                end,
                format!("final state {final_id} not among redo states {ids:?}"),
            ),
            Some(&redo),
        );
    }
    let redone = r.ed.buf.text();
    if *ids.last().unwrap() == final_id && redone != final_text {
        return fail(
            "redo-all-does-not-restore-final-text",
            end,
            format!(
                "final {:?}, after redo-all {:?}",
                short(&final_text),
                short(&redone)
            ),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shrinking

fn shrink(seed: u64, case: &Case, sig: &str) -> (Case, Failure) {
    let runs = Cell::new(0usize);
    let still = |c: &Case| -> Option<Failure> {
        runs.set(runs.get() + 1);
        if runs.get() > SHRINK_RUNS {
            return None;
        }
        match run(seed, c) {
            Err(f) if f.sig == sig => Some(f),
            _ => None,
        }
    };
    let mut best = case.clone();
    let Some(mut best_f) = still(&best) else {
        let f = Failure {
            sig: sig.into(),
            step: 0,
            detail: "(did not reproduce)".into(),
        };
        return (best, f);
    };
    if best_f.step + 1 < best.steps.len() {
        let mut c = best.clone();
        c.steps.truncate(best_f.step + 1);
        if let Some(f) = still(&c) {
            best = c;
            best_f = f;
        }
    }
    loop {
        let mut progress = false;
        // Remove chunks of steps, halving the chunk size.
        let mut chunk = (best.steps.len() / 2).max(1);
        loop {
            let mut i = 0;
            while i < best.steps.len() {
                let mut c = best.clone();
                let end = (i + chunk).min(c.steps.len());
                c.steps.drain(i..end);
                if let Some(f) = still(&c) {
                    best = c;
                    best_f = f;
                    progress = true;
                } else {
                    i += chunk;
                }
            }
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
        // Then small windows at every offset (halving skips sizes like 2).
        for w in [2, 3, 4] {
            let mut i = 0;
            while i + w <= best.steps.len() {
                let mut c = best.clone();
                c.steps.drain(i..i + w);
                if let Some(f) = still(&c) {
                    best = c;
                    best_f = f;
                    progress = true;
                } else {
                    i += 1;
                }
            }
        }
        // Remove whole lines of the initial text, then single chars.
        let mut i = 0;
        loop {
            let lines: Vec<&str> = best.text.split_inclusive('\n').collect();
            if i >= lines.len() {
                break;
            }
            let mut c = best.clone();
            c.text = lines
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, l)| *l)
                .collect();
            if let Some(f) = still(&c) {
                best = c;
                best_f = f;
                progress = true;
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        loop {
            let chars: Vec<char> = best.text.chars().collect();
            if i >= chars.len() {
                break;
            }
            let mut c = best.clone();
            c.text = chars
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, ch)| *ch)
                .collect();
            if let Some(f) = still(&c) {
                best = c;
                best_f = f;
                progress = true;
            } else {
                i += 1;
            }
        }
        // Shrink pasted text.
        for si in 0..best.steps.len() {
            let mut ci = 0;
            while let Step::Paste(t) = &best.steps[si] {
                let chars: Vec<char> = t.chars().collect();
                if ci >= chars.len() {
                    break;
                }
                let mut c = best.clone();
                c.steps[si] = Step::Paste(
                    chars
                        .iter()
                        .enumerate()
                        .filter(|&(j, _)| j != ci)
                        .map(|(_, ch)| *ch)
                        .collect(),
                );
                if let Some(f) = still(&c) {
                    best = c;
                    best_f = f;
                    progress = true;
                } else {
                    ci += 1;
                }
            }
        }
        // Simplify typed characters to 'a'.
        for si in 0..best.steps.len() {
            if let Step::Key(k) = best.steps[si]
                && let Some(ch) = k.char()
                && !ch.is_ascii_alphanumeric()
            {
                let mut c = best.clone();
                c.steps[si] = Step::Key(Key::ch('a'));
                if let Some(f) = still(&c) {
                    best = c;
                    best_f = f;
                    progress = true;
                }
            }
        }
        if !progress || runs.get() > SHRINK_RUNS {
            break;
        }
    }
    (best, best_f)
}

// ---------------------------------------------------------------------------
// Driver

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

fn describe(seed: u64, case: &Case) -> String {
    format!(
        "seed {seed}\n  text  {:?}\n  steps {}",
        case.text,
        render(&case.steps)
    )
}

#[test]
fn fuzz() {
    install_hook();
    let all_known = env_flag("FRED_FUZZ_KNOWN");
    let do_shrink = std::env::var("FRED_FUZZ_SHRINK").map_or(true, |v| v != "0");

    let explicit = std::env::var("FRED_FUZZ_KEYS").ok();
    if explicit.is_some() || std::env::var("FRED_FUZZ_CASE").is_ok() {
        let (seed, case) = match &explicit {
            Some(keys) => (
                0,
                Case {
                    text: std::env::var("FRED_FUZZ_TEXT").unwrap_or_default(),
                    steps: parse_keys(keys).into_iter().map(Step::Key).collect(),
                },
            ),
            None => {
                let s = std::env::var("FRED_FUZZ_CASE").unwrap();
                let seed: u64 = s.trim().parse().expect("FRED_FUZZ_CASE=<seed>");
                (seed, gen_case(seed))
            }
        };
        eprintln!("{}", describe(seed, &case));
        TRACE.with(|t| t.set(env_flag("FRED_FUZZ_TRACE")));
        let first = run(seed, &case);
        TRACE.with(|t| t.set(false));
        match first {
            Ok(()) => eprintln!("ok"),
            Err(f) => {
                eprintln!("FAIL {} at step {}: {}", f.sig, f.step, f.detail);
                if do_shrink {
                    let (min, mf) = shrink(seed, &case, &f.sig);
                    eprintln!("minimal: {}\n  {}", describe(seed, &min), mf.detail);
                }
                panic!("seed {seed} fails");
            }
        }
        return;
    }

    let iters = env_u64("FRED_FUZZ_ITERS", DEFAULT_ITERS);
    let first = env_u64("FRED_FUZZ_SEED", 0);
    let threads = env_u64("FRED_FUZZ_THREADS", 1).clamp(1, 64) as usize;
    let progress = env_flag("FRED_FUZZ_PROGRESS");
    let next = AtomicU64::new(first);
    let done = AtomicU64::new(0);
    let failures: Mutex<BTreeMap<String, Vec<(u64, Failure)>>> = Mutex::new(BTreeMap::new());
    let started = Instant::now();

    std::thread::scope(|sc| {
        for w in 0..threads {
            let (next, done, failures) = (&next, &done, &failures);
            sc.spawn(move || {
                SLOT.with(|s| s.set(w));
                loop {
                    let seed = next.fetch_add(1, Ordering::Relaxed);
                    if seed >= first + iters {
                        break;
                    }
                    let case = gen_case(seed);
                    let t0 = Instant::now();
                    let result = run(seed, &case);
                    if progress && t0.elapsed() > Duration::from_millis(500) {
                        eprintln!("fuzz: slow: seed {seed} took {:.1?}", t0.elapsed());
                    }
                    if let Err(f) = result {
                        failures
                            .lock()
                            .unwrap()
                            .entry(f.sig.clone())
                            .or_default()
                            .push((seed, f));
                    }
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if progress && d % 1000 == 0 {
                        let nf = failures.lock().unwrap().len();
                        eprintln!(
                            "fuzz: {d}/{iters} sequences, {nf} distinct failures, {:.0?}",
                            started.elapsed()
                        );
                    }
                }
            });
        }
    });

    let failures = failures.into_inner().unwrap();
    eprintln!(
        "fuzz: {iters} sequences (seeds {first}..{}) in {:.1?}, {} distinct failure(s){}",
        first + iters,
        started.elapsed(),
        failures.len(),
        if all_known {
            " (known-bug workarounds partly or fully off)"
        } else {
            " (known bugs worked around; FRED_FUZZ_KNOWN=1 to include them)"
        }
    );
    if failures.is_empty() {
        return;
    }
    let mut report = String::new();
    for (i, (sig, mut hits)) in failures.into_iter().enumerate() {
        hits.sort_by_key(|(s, _)| *s);
        let seeds: Vec<String> = hits.iter().take(12).map(|(s, _)| s.to_string()).collect();
        let (seed, f) = &hits[0];
        report.push_str(&format!(
            "\n[{}] {sig}: {} seed(s): {}{}\n    first: step {}: {}\n",
            i + 1,
            hits.len(),
            seeds.join(", "),
            if hits.len() > 12 { ", ..." } else { "" },
            f.step,
            f.detail
        ));
        if do_shrink {
            // Shrink the smallest of the first few hits.
            let (seed, case) = hits
                .iter()
                .take(5)
                .map(|(s, _)| (*s, gen_case(*s)))
                .min_by_key(|(_, c)| c.steps.len() + c.text.len())
                .unwrap();
            let (min, mf) = shrink(seed, &case, &sig);
            let keys_ok = min.steps.iter().all(|s| matches!(s, Step::Key(_)));
            report.push_str(&format!(
                "    minimal (from seed {seed}): text {:?} ; {}\n      -> step {}: {}\n",
                min.text,
                render(&min.steps),
                mf.step,
                mf.detail
            ));
            if keys_ok {
                let notation: String = min
                    .steps
                    .iter()
                    .map(|s| match s {
                        Step::Key(k) => notation(k),
                        Step::Paste(_) => unreachable!(),
                    })
                    .collect();
                let parsed: Vec<Step> = parse_keys(&notation).into_iter().map(Step::Key).collect();
                assert_eq!(parsed, min.steps, "notation round trip");
            }
        } else {
            let _ = seed;
        }
    }
    eprintln!("{report}");
    panic!("fuzzing found failures (see report above)");
}
