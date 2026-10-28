//! `:draw`: ASCII drawing with the mouse. The buffer is the canvas, one
//! character a cell; a toolbar of clickable tools takes the status line.
//! Each stroke is one undo step.
// shortcut: one char is one column (tabs and wide characters misplace
// cells) and a wrapped or folded window misplaces rows; fine for ASCII art.

use crate::editor::{Editor, Mode};
use crate::key::{Key, KeyCode};
use crate::ui::View;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::BTreeMap;

type Cell = (usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Brush,
    Spray,
    Fill,
    Rect,
    Line,
    Oval,
    Eraser,
    Text,
    /// Drag over art to apply the chosen effect to it.
    Fx,
}

const TOOLS: [(Tool, &str); 10] = [
    (Tool::Pen, "Pen"),
    (Tool::Brush, "Brush"),
    (Tool::Spray, "Spray"),
    (Tool::Fill, "Fill"),
    (Tool::Rect, "Rect"),
    (Tool::Line, "Line"),
    (Tool::Oval, "Oval"),
    (Tool::Eraser, "Eraser"),
    (Tool::Text, "Text"),
    (Tool::Fx, "Fx"),
];

/// figlet-rs's own fonts: FIGlet's, then toilet's.
const FONTS: [&str; 9] = [
    "standard", "small", "big", "slant", "smblock", "mono9", "mono12", "future", "wideterm",
];

const EFFECTS: [&str; 4] = ["border", "shadow", "mirror", "flip"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Tool(Tool),
    Smaller,
    Bigger,
    Char,
    Font,
    Effect,
    Done,
}

#[derive(Debug)]
pub struct Draw {
    pub tool: Tool,
    /// Brush and spray diameter.
    pub size: usize,
    pub ch: char,
    /// A name in FONTS, or the path of a .flf or .tlf file.
    pub font: String,
    pub effect: usize,
    /// The stroke in progress: where it started, the last cell, and the
    /// undo mark that Rect and Eraser roll back to as they are dragged.
    from: Option<Cell>,
    last: Option<Cell>,
    mark: usize,
    /// Where the Text tool stamps once its prompt is answered.
    text_at: Option<Cell>,
    seed: u64,
}

impl Default for Draw {
    fn default() -> Draw {
        Draw {
            tool: Tool::Pen,
            size: 3,
            ch: '#',
            font: FONTS[0].into(),
            effect: 0,
            from: None,
            last: None,
            mark: 0,
            text_at: None,
            seed: 0x2545_f491_4f6c_dd1d,
        }
    }
}

/// `:draw` turns drawing on, or off again.
pub fn toggle(ed: &mut Editor) {
    if ed.draw.is_some() {
        leave(ed);
    } else {
        ed.draw = Some(Box::default());
        ed.set_msg("drawing: click a tool below, drag on the text; Esc leaves");
    }
}

fn leave(ed: &mut Editor) {
    end_stroke(ed);
    ed.draw = None;
    ed.set_msg("drawing off");
}

/// Esc in Normal mode leaves drawing; other keys work as usual.
pub fn key(ed: &mut Editor, k: Key) -> bool {
    if ed.draw.is_none() || ed.mode != Mode::Normal || !ed.vim.pending.is_empty() {
        return false;
    }
    end_stroke(ed);
    if k.is(KeyCode::Esc) {
        leave(ed);
        return true;
    }
    false
}

/// The toolbar, left to right: each label, its button, and whether it is on.
pub fn toolbar(d: &Draw) -> Vec<(String, Option<Button>, bool)> {
    let mut v = vec![(" DRAW ".to_string(), None, true)];
    for (t, name) in TOOLS {
        v.push((format!(" {name} "), Some(Button::Tool(t)), d.tool == t));
    }
    v.push((" │ ".into(), None, false));
    v.push((" - ".into(), Some(Button::Smaller), false));
    v.push((format!("size {}", d.size), None, false));
    v.push((" + ".into(), Some(Button::Bigger), false));
    v.push((format!(" char {} ", d.ch), Some(Button::Char), false));
    let font = std::path::Path::new(&d.font).file_stem();
    let font = font.map_or(d.font.as_str().into(), |f| f.to_string_lossy());
    v.push((format!(" font {font} "), Some(Button::Font), false));
    v.push((
        format!(" fx {} ", EFFECTS[d.effect]),
        Some(Button::Effect),
        false,
    ));
    v.push((" ✕ ".into(), Some(Button::Done), false));
    v
}

fn button_at(d: &Draw, x: usize) -> Option<Button> {
    let mut at = 0;
    for (label, b, _) in toolbar(d) {
        let w = unicode_width::UnicodeWidthStr::width(label.as_str());
        if x < at + w {
            return b;
        }
        at += w;
    }
    None
}

fn press(ed: &mut Editor, b: Button) {
    let Some(d) = ed.draw.as_mut() else {
        return;
    };
    match b {
        Button::Tool(t) => d.tool = t,
        Button::Smaller => d.size = d.size.saturating_sub(1).max(1),
        Button::Bigger => d.size = (d.size + 1).min(15),
        Button::Font => {
            crate::pick::choose(ed, "fonts", fonts(), "");
            ed.org_require_match = true;
            ed.org_then = Some(Box::new(|ed, picked| {
                if let Some(d) = ed.draw.as_mut() {
                    d.font = picked;
                }
            }));
        }
        Button::Effect => {
            crate::pick::choose(ed, "effects", EFFECTS.map(String::from).to_vec(), "");
            ed.org_require_match = true;
            ed.org_then = Some(Box::new(|ed, picked| {
                if let Some(d) = ed.draw.as_mut() {
                    d.effect = EFFECTS.iter().position(|&e| e == picked).unwrap_or(0);
                    d.tool = Tool::Fx;
                }
            }));
        }
        Button::Char => {
            d.text_at = None;
            ask(ed, "Draw with: ");
        }
        Button::Done => leave(ed),
    }
}

fn ask(ed: &mut Editor, prompt: &str) {
    ed.open_cmdline('D', "");
    if let Mode::Command(cl) = &mut ed.mode {
        cl.prompt = prompt.into();
    }
}

/// The answer to a `D` prompt: the drawing character, or Text's words.
pub fn answer(ed: &mut Editor, text: &str) {
    let Some(d) = ed.draw.as_mut() else {
        return;
    };
    match d.text_at.take() {
        Some(at) => {
            let font = d.font.clone();
            match banner(&font, text) {
                Some(lines) => stroke(ed, at, |ed| stamp(ed, at, &lines)),
                None => ed.set_err(format!("{font} can't write that")),
            }
        }
        None => {
            if let Some(c) = text.chars().next() {
                d.ch = c;
            }
        }
    }
}

/// The fonts to pick from: figlet-rs's, then font files in the usual
/// figlet directories and ~/.config/fred/fonts.
fn fonts() -> Vec<String> {
    let mut v: Vec<String> = FONTS.map(String::from).to_vec();
    let mut dirs = vec![crate::config::config_path().with_file_name("fonts")];
    for d in [
        "/opt/homebrew/share/figlet",
        "/usr/local/share/figlet",
        "/usr/share/figlet",
    ] {
        dirs.push(d.into());
    }
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<String> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "flf" || x == "tlf"))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        files.sort();
        v.extend(files);
    }
    v
}

/// `text` in a FIGlet or toilet font, as lines.
pub fn banner(font: &str, text: &str) -> Option<Vec<String>> {
    use figlet_rs::{FIGlet, Toilet};
    let fig = match font {
        "small" => FIGlet::small().ok()?.convert(text)?.as_str(),
        "big" => FIGlet::big().ok()?.convert(text)?.as_str(),
        "slant" => FIGlet::slant().ok()?.convert(text)?.as_str(),
        "smblock" => Toilet::smblock().ok()?.convert(text)?.as_str(),
        "mono9" => Toilet::mono9().ok()?.convert(text)?.as_str(),
        "mono12" => Toilet::mono12().ok()?.convert(text)?.as_str(),
        "future" => Toilet::future().ok()?.convert(text)?.as_str(),
        "wideterm" => Toilet::wideterm().ok()?.convert(text)?.as_str(),
        f if f.ends_with(".tlf") => Toilet::from_file(f).ok()?.convert(text)?.as_str(),
        f if f.ends_with(".flf") => FIGlet::from_file(f).ok()?.convert(text)?.as_str(),
        _ => FIGlet::standard().ok()?.convert(text)?.as_str(),
    };
    let mut lines: Vec<String> = fig.lines().map(|l| l.trim_end().to_string()).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Some(lines)
}

/// A mouse event in drawing mode; false leaves it to the usual handling
/// (the wheel, and anything outside the text and toolbar).
pub fn mouse(ed: &mut Editor, view: &View, area: Rect, ev: MouseEvent, gutter: usize) -> bool {
    if matches!(
        ev.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) {
        return false;
    }
    let status = area.bottom().saturating_sub(2);
    if ev.row == status {
        if ev.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(b) = ed
                .draw
                .as_deref()
                .and_then(|d| button_at(d, (ev.column - area.x) as usize))
        {
            press(ed, b);
        }
        return true;
    }
    if ev.row > status {
        return false;
    }
    let Some(x) = ((ev.column - area.x) as usize).checked_sub(gutter) else {
        return true;
    };
    let at = (view.top + (ev.row - area.y) as usize, view.left + x);
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => down(ed, at),
        MouseEventKind::Drag(MouseButton::Left) => drag(ed, at),
        MouseEventKind::Up(MouseButton::Left) => end_stroke(ed),
        _ => {}
    }
    true
}

/// The button goes down on `at`.
pub fn down(ed: &mut Editor, at: Cell) {
    end_stroke(ed);
    let Some(d) = ed.draw.as_mut() else {
        return;
    };
    match d.tool {
        Tool::Fill => {
            let ch = d.ch;
            stroke(ed, at, |ed| fill(ed, at, ch));
        }
        Tool::Text => {
            d.text_at = Some(at);
            ask(ed, "Text: ");
        }
        _ => {
            d.from = Some(at);
            ed.undo.begin((ed.cur.line, ed.cur.byte));
            d.mark = ed.undo.open_len();
            drag(ed, at);
        }
    }
}

/// The button is dragged to `at`.
pub fn drag(ed: &mut Editor, at: Cell) {
    let Some(d) = ed.draw.as_mut() else {
        return;
    };
    let Some(from) = d.from else {
        return;
    };
    let last = d.last.replace(at).unwrap_or(at);
    let (ch, r) = (d.ch, d.size / 2);
    let cells: Vec<(Cell, char)> = match d.tool {
        Tool::Pen => line(last, at).into_iter().map(|c| (c, ch)).collect(),
        Tool::Brush => line(last, at)
            .into_iter()
            .flat_map(|c| disc(c, r))
            .map(|c| (c, ch))
            .collect(),
        Tool::Spray => {
            let n = (d.size * d.size / 3).max(1);
            disc(at, r)
                .into_iter()
                .filter(|_| d.rand() % (2 * r as u64 + 1).pow(2) < n as u64)
                .map(|c| (c, ch))
                .collect()
        }
        Tool::Rect | Tool::Line | Tool::Oval | Tool::Eraser => {
            ed.undo.rollback_to(&mut ed.buf, d.mark);
            if d.tool == Tool::Rect {
                outline(from, at)
            } else if d.tool == Tool::Line {
                line(from, at).into_iter().map(|c| (c, ch)).collect()
            } else if d.tool == Tool::Oval {
                oval(from, at).into_iter().map(|c| (c, ch)).collect()
            } else {
                // Only what is there to erase: no new lines for blank space.
                let n = ed.line_count();
                area(from, at)
                    .into_iter()
                    .filter(|c| c.0 < n)
                    .map(|c| (c, ' '))
                    .collect()
            }
        }
        // Applied when the button comes up.
        Tool::Fx => return,
        Tool::Fill | Tool::Text => return,
    };
    put(ed, &cells);
    ed.set_cursor(at.0, 0);
}

fn end_stroke(ed: &mut Editor) {
    let Some(d) = ed.draw.as_mut() else {
        return;
    };
    let (Some(from), last) = (d.from.take(), d.last.take()) else {
        return;
    };
    if d.tool == Tool::Fx {
        let e = EFFECTS[d.effect];
        apply(ed, from, last.unwrap_or(from), e);
    }
    ed.undo.end((ed.cur.line, ed.cur.byte));
}

/// Apply effect `e` to the art in the box with corners `a` and `b`.
fn apply(ed: &mut Editor, a: Cell, b: Cell, e: &str) {
    let (top, left, bottom, right) = corners(a, b);
    let lines: Vec<String> = (top..=bottom)
        .map(|y| (left..=right).map(|x| char_at(ed, (y, x))).collect())
        .collect();
    let (out, at) = match e {
        "border" => (
            border(&lines),
            (top.saturating_sub(1), left.saturating_sub(1)),
        ),
        "shadow" => (shadow(&lines), (top, left)),
        "mirror" => (mirror(&lines), (top, left)),
        _ => (flip(&lines), (top, left)),
    };
    let n = ed.line_count();
    let blank: Vec<(Cell, char)> = area(a, b)
        .into_iter()
        .filter(|c| c.0 < n)
        .map(|c| (c, ' '))
        .collect();
    put(ed, &blank);
    stamp(ed, at, &out);
}

/// `lines` in a `+-|` box.
fn border(lines: &[String]) -> Vec<String> {
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let edge = format!("+{}+", "-".repeat(w));
    let mut v = vec![edge.clone()];
    v.extend(lines.iter().map(|l| format!("|{l:<w$}|")));
    v.push(edge);
    v
}

/// `lines` with a shadow one cell down and right.
fn shadow(lines: &[String]) -> Vec<String> {
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) + 1;
    let mut g = vec![vec![' '; w]; lines.len() + 1];
    for (y, l) in lines.iter().enumerate() {
        for (x, c) in l.chars().enumerate() {
            if c != ' ' {
                g[y + 1][x + 1] = '░';
            }
        }
    }
    for (y, l) in lines.iter().enumerate() {
        for (x, c) in l.chars().enumerate() {
            if c != ' ' {
                g[y][x] = c;
            }
        }
    }
    g.into_iter().map(String::from_iter).collect()
}

/// `lines` left to right, as in a mirror.
fn mirror(lines: &[String]) -> Vec<String> {
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let swap = |c| match c {
        '/' => '\\',
        '\\' => '/',
        '(' => ')',
        ')' => '(',
        '<' => '>',
        '>' => '<',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        c => c,
    };
    lines
        .iter()
        .map(|l| format!("{l:<w$}").chars().rev().map(swap).collect())
        .collect()
}

/// `lines` upside down.
fn flip(lines: &[String]) -> Vec<String> {
    let swap = |c| match c {
        '/' => '\\',
        '\\' => '/',
        '^' => 'v',
        'v' => '^',
        '_' => '‾',
        '‾' => '_',
        c => c,
    };
    lines
        .iter()
        .rev()
        .map(|l| l.chars().map(swap).collect())
        .collect()
}

/// `f` as one undo step, leaving the cursor on `at`'s line.
fn stroke(ed: &mut Editor, at: Cell, f: impl FnOnce(&mut Editor)) {
    ed.undo.begin((ed.cur.line, ed.cur.byte));
    f(ed);
    ed.set_cursor(at.0, 0);
    ed.undo.end((ed.cur.line, ed.cur.byte));
}

impl Draw {
    /// xorshift64, for the spray.
    fn rand(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }
}

/// The cells from `a` to `b` (Bresenham).
fn line(a: Cell, b: Cell) -> Vec<Cell> {
    let (mut x, mut y) = (a.1 as i64, a.0 as i64);
    let (x1, y1) = (b.1 as i64, b.0 as i64);
    let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
    let (sx, sy) = ((x1 - x).signum(), (y1 - y).signum());
    let mut err = dx + dy;
    let mut v = vec![];
    loop {
        v.push((y as usize, x as usize));
        if (x, y) == (x1, y1) {
            return v;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// The cells within `r` of `c`.
fn disc(c: Cell, r: usize) -> Vec<Cell> {
    let r = r as i64;
    let mut v = vec![];
    for dy in -r..=r {
        for dx in -r..=r {
            let (y, x) = (c.0 as i64 + dy, c.1 as i64 + dx);
            if dx * dx + dy * dy <= r * r + r && y >= 0 && x >= 0 {
                v.push((y as usize, x as usize));
            }
        }
    }
    v
}

/// The ellipse inside the box with corners `a` and `b`.
fn oval(a: Cell, b: Cell) -> Vec<Cell> {
    let (top, left, bottom, right) = corners(a, b);
    let (cy, cx) = ((top + bottom) as f64 / 2.0, (left + right) as f64 / 2.0);
    let (ry, rx) = ((bottom - top) as f64 / 2.0, (right - left) as f64 / 2.0);
    let mut v = vec![];
    // Step around finely enough to touch every cell on the edge.
    let steps = (4.0 * (rx + ry) + 4.0) as usize * 2;
    for i in 0..steps {
        let t = i as f64 / steps as f64 * std::f64::consts::TAU;
        let c = (
            (cy + ry * t.sin()).round() as usize,
            (cx + rx * t.cos()).round() as usize,
        );
        if v.last() != Some(&c) && !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

fn corners(a: Cell, b: Cell) -> (usize, usize, usize, usize) {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

/// A `+-|` box with corners `a` and `b`.
fn outline(a: Cell, b: Cell) -> Vec<(Cell, char)> {
    let (top, left, bottom, right) = corners(a, b);
    let mut v = vec![];
    for y in top..=bottom {
        for x in left..=right {
            let edge_y = y == top || y == bottom;
            let edge_x = x == left || x == right;
            let c = match (edge_y, edge_x) {
                (true, true) => '+',
                (true, false) => '-',
                (false, true) => '|',
                _ => continue,
            };
            v.push(((y, x), c));
        }
    }
    v
}

fn area(a: Cell, b: Cell) -> Vec<Cell> {
    let (top, left, bottom, right) = corners(a, b);
    (top..=bottom)
        .flat_map(|y| (left..=right).map(move |x| (y, x)))
        .collect()
}

/// `lines` with their top left at `at`; their spaces let what's below show.
fn stamp(ed: &mut Editor, at: Cell, lines: &[String]) {
    let cells: Vec<(Cell, char)> = lines
        .iter()
        .enumerate()
        .flat_map(|(y, l)| {
            l.chars()
                .enumerate()
                .filter(|&(_, c)| c != ' ')
                .map(move |(x, c)| ((at.0 + y, at.1 + x), c))
        })
        .collect();
    put(ed, &cells);
}

fn char_at(ed: &Editor, (y, x): Cell) -> char {
    if y >= ed.line_count() {
        return ' ';
    }
    ed.buf.line(y).chars().nth(x).unwrap_or(' ')
}

/// Flood the run of cells like `at`'s, joined side by side, with `ch`;
/// blank space is bounded by the text's lines and its widest line.
fn fill(ed: &mut Editor, at: Cell, ch: char) {
    let target = char_at(ed, at);
    if target == ch {
        return;
    }
    let rows = ed.line_count().max(at.0 + 1);
    let lines: Vec<Vec<char>> = (0..rows)
        .map(|y| {
            if y < ed.line_count() {
                ed.buf.line(y).chars().collect()
            } else {
                vec![]
            }
        })
        .collect();
    let cols = lines.iter().map(Vec::len).max().unwrap_or(0).max(at.1 + 1);
    let get = |(y, x): Cell| lines[y].get(x).copied().unwrap_or(' ');
    let mut seen = vec![vec![false; cols]; rows];
    let mut todo = vec![at];
    let mut cells = vec![];
    while let Some((y, x)) = todo.pop() {
        if y >= rows || x >= cols || seen[y][x] || get((y, x)) != target {
            continue;
        }
        seen[y][x] = true;
        cells.push(((y, x), ch));
        todo.push((y + 1, x));
        todo.push((y, x + 1));
        if y > 0 {
            todo.push((y - 1, x));
        }
        if x > 0 {
            todo.push((y, x - 1));
        }
    }
    put(ed, &cells);
}

/// Set each cell, padding with spaces and lines as needed; changed lines
/// lose their trailing spaces.
pub fn put(ed: &mut Editor, cells: &[(Cell, char)]) {
    let mut rows: BTreeMap<usize, Vec<(usize, char)>> = BTreeMap::new();
    for &((y, x), c) in cells {
        rows.entry(y).or_default().push((x, c));
    }
    let Some(&last) = rows.keys().last() else {
        return;
    };
    let n = ed.line_count();
    if last >= n {
        crate::vim::ops::splice_lines(ed, n, 0, &vec![String::new(); last + 1 - n]);
    }
    for (y, row) in rows {
        let old = ed.buf.line(y);
        let mut l: Vec<char> = old.chars().collect();
        for (x, c) in row {
            if l.len() <= x {
                l.resize(x + 1, ' ');
            }
            l[x] = c;
        }
        let new = String::from_iter(l).trim_end().to_string();
        if new != old {
            crate::vim::ops::splice_lines(ed, y, 1, &[new]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn ed(text: &str) -> Editor {
        let mut ed = Editor::new(Buffer::from_text(text));
        toggle(&mut ed);
        ed
    }

    fn text(ed: &Editor) -> String {
        (0..ed.line_count())
            .map(|i| ed.buf.line(i) + "\n")
            .collect()
    }

    fn tool(ed: &mut Editor, t: Tool) {
        ed.draw.as_mut().unwrap().tool = t;
    }

    #[test]
    fn pen_draws_a_line_as_one_undo_step() {
        let mut ed = ed("\n");
        down(&mut ed, (0, 0));
        drag(&mut ed, (2, 4));
        end_stroke(&mut ed);
        assert_eq!(text(&ed), "#\n ##\n   ##\n");
        ed.undo.undo(&mut ed.buf);
        assert_eq!(text(&ed), "\n");
    }

    #[test]
    fn brush_is_as_wide_as_its_size() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Brush);
        down(&mut ed, (1, 1));
        end_stroke(&mut ed);
        assert_eq!(text(&ed), "###\n###\n###\n");
    }

    #[test]
    fn spray_stays_inside_its_size() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Spray);
        ed.draw.as_mut().unwrap().size = 5;
        for _ in 0..20 {
            down(&mut ed, (2, 2));
        }
        end_stroke(&mut ed);
        let t = text(&ed);
        assert!(t.contains('#'));
        assert!(
            ed.line_count() <= 5 && t.lines().all(|l| l.len() <= 5),
            "{t}"
        );
    }

    #[test]
    fn rect_follows_the_drag() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Rect);
        down(&mut ed, (0, 0));
        drag(&mut ed, (3, 5));
        drag(&mut ed, (2, 3));
        end_stroke(&mut ed);
        assert_eq!(text(&ed), "+--+\n|  |\n+--+\n");
    }

    #[test]
    fn eraser_blanks_a_rectangle() {
        let mut ed = ed("abcd\nefgh\nijkl\n");
        tool(&mut ed, Tool::Eraser);
        down(&mut ed, (0, 1));
        drag(&mut ed, (1, 9));
        end_stroke(&mut ed);
        assert_eq!(text(&ed), "a\ne\nijkl\n");
    }

    #[test]
    fn fill_floods_inside_a_box() {
        let mut ed = ed("+--+\n|  |\n+--+\n");
        ed.draw.as_mut().unwrap().ch = '.';
        tool(&mut ed, Tool::Fill);
        down(&mut ed, (1, 1));
        assert_eq!(text(&ed), "+--+\n|..|\n+--+\n");
    }

    #[test]
    fn text_stamps_a_banner() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Text);
        down(&mut ed, (0, 2));
        assert!(matches!(&ed.mode, Mode::Command(cl) if cl.kind == 'D'));
        ed.mode = Mode::Normal;
        answer(&mut ed, "Hi");
        let want = banner("standard", "Hi").unwrap();
        assert!(want.len() > 2);
        assert_eq!(ed.buf.line(1).trim(), want[1].trim());
    }

    #[test]
    fn toolbar_clicks_pick_tools() {
        let mut ed = ed("\n");
        let d = ed.draw.as_deref().unwrap();
        let x = toolbar(d)
            .iter()
            .take_while(|(_, b, _)| *b != Some(Button::Tool(Tool::Spray)))
            .map(|(l, ..)| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .sum::<usize>();
        let b = button_at(d, x).unwrap();
        press(&mut ed, b);
        assert_eq!(ed.draw.as_ref().unwrap().tool, Tool::Spray);
        press(&mut ed, Button::Done);
        assert!(ed.draw.is_none());
    }

    #[test]
    fn line_is_straight_whatever_the_drag() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Line);
        down(&mut ed, (0, 0));
        drag(&mut ed, (2, 0));
        drag(&mut ed, (0, 4));
        end_stroke(&mut ed);
        assert_eq!(text(&ed), "#####\n");
    }

    #[test]
    fn oval_fits_its_box() {
        let mut ed = ed("\n");
        tool(&mut ed, Tool::Oval);
        down(&mut ed, (0, 0));
        drag(&mut ed, (4, 8));
        end_stroke(&mut ed);
        let t = text(&ed);
        assert_eq!(ed.line_count(), 5, "{t}");
        assert!(t.lines().all(|l| l.len() <= 9), "{t}");
        assert_eq!(ed.buf.line(2), "#       #", "{t}");
        assert_eq!(ed.buf.line(0).trim(), ed.buf.line(4).trim(), "{t}");
    }

    fn fx(ed: &mut Editor, e: &str, a: Cell, b: Cell) {
        tool(ed, Tool::Fx);
        ed.draw.as_mut().unwrap().effect = EFFECTS.iter().position(|&x| x == e).unwrap();
        down(ed, a);
        drag(ed, b);
        end_stroke(ed);
    }

    #[test]
    fn effects_apply_to_the_dragged_box_as_one_undo_step() {
        let mut ed = super::tests::ed("\n ab\n");
        fx(&mut ed, "border", (1, 1), (1, 2));
        assert_eq!(text(&ed), "+--+\n|ab|\n+--+\n");
        ed.undo.undo(&mut ed.buf);
        assert_eq!(text(&ed), "\n ab\n");

        let mut ed = super::tests::ed("/a(\n");
        fx(&mut ed, "mirror", (0, 0), (0, 2));
        assert_eq!(text(&ed), ")a\\\n");

        let mut ed = super::tests::ed("/_\n^\n");
        fx(&mut ed, "flip", (0, 0), (1, 1));
        assert_eq!(text(&ed), "v\n\\‾\n");

        let mut ed = super::tests::ed("#\n");
        fx(&mut ed, "shadow", (0, 0), (0, 0));
        assert_eq!(text(&ed), "#\n ░\n");
    }

    #[test]
    fn font_button_picks_from_every_font() {
        let mut ed = ed("\n");
        press(&mut ed, Button::Font);
        let Mode::Pick(p) = &ed.mode else {
            panic!("no picker: {:?}", ed.mode)
        };
        assert_eq!(p.title, "fonts");
        ed.mode = Mode::Normal;
        let then = ed.org_then.take().unwrap();
        then(&mut ed, "mono9".into());
        assert_eq!(ed.draw.as_ref().unwrap().font, "mono9");
        for f in FONTS {
            assert!(banner(f, "Hi").is_some_and(|l| !l.is_empty()), "{f}");
        }
    }
}
