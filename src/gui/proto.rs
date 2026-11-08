//! The JSON-lines frames the core writes for the native app.
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

fn is_zero(f: &u8) -> bool {
    *f == 0
}

/// Text at column `x` in one style. `fg`/`bg`: 0-255 palette, `1<<24 | rgb`.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Run {
    pub x: u16,
    pub s: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<u32>,
    /// 1 bold, 2 italic, 4 underline, 8 reversed, 16 dim, 32 crossed out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub f: u8,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Status {
    pub mode: String,
    pub name: String,
    pub modified: bool,
    pub pos: String,
    pub branch: Option<String>,
    /// For the app's status bar: the language ("Rust"; empty for plain
    /// text), indent ("Spaces: 4", "Tab Size: 8"), "LF"/"CRLF", "UTF-8".
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub indent: String,
    #[serde(default)]
    pub eol: String,
    #[serde(default)]
    pub enc: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Win {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub divider: bool,
    pub active: bool,
    pub rows: Vec<Vec<Run>>,
    /// The selection: one span of cells per text row, window-relative.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sel: Vec<Sel>,
    pub status: Status,
    /// The buffer's file, absolute; absent for no-name and special buffers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The window id, for `tab` and `scroll`.
    #[serde(default)]
    pub id: usize,
    /// Rows at the top for the tab strip; `rows` starts below them.
    #[serde(default)]
    pub header: u16,
    /// Columns at the right for the minimap, which no row covers.
    #[serde(default)]
    pub margin: u16,
    /// The first buffer line shown, and the buffer's line count.
    #[serde(default)]
    pub top: usize,
    #[serde(default)]
    pub lines: usize,
    /// In display order, exactly one `current`.
    #[serde(default)]
    pub tabs: Vec<Tab>,
    /// The last row is draw mode's toolbar, in place of the status bar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub toolbar: bool,
    /// A terminal: the wheel scrolls its shell, not lines.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub term: bool,
    /// The symbol the cursor is in and its kind, for the breadcrumbs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<(String, String)>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Tab {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub modified: bool,
    pub preview: bool,
    pub current: bool,
    /// A directory (dired): the app shows a folder icon.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
}

/// Selected cells `x..x + w` of text row `y`.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub struct Sel {
    pub y: u16,
    pub x: u16,
    pub w: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub shape: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Frame {
    pub t: String,
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub windows: Vec<Win>,
    pub bottom: Vec<Vec<Run>>,
    pub cursor: Option<Cursor>,
    /// An open picker, for the app's floating palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    /// Zap's labels on the sidebar's rows (see `In::Nav`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zap_nav: Vec<NavLabel>,
    /// Give the sidebar the keyboard.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focus_nav: bool,
    /// Ask for a project folder (then send `cd`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open_folder: bool,
    /// `Space w` is numbering the windows: the sidebar shows its 0.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub picking: bool,
    /// The working directory (the project), absolute.
    #[serde(default)]
    pub cwd: String,
}

/// A zap label on sidebar row `row` (an index into the last `nav`), on
/// the word starting at character `at` of its name, `typed` characters matched.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct NavLabel {
    pub row: usize,
    pub at: usize,
    pub typed: usize,
    pub label: String,
}

/// A sidebar row for zap: its name and path.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct NavRow {
    pub name: String,
    pub path: String,
}

/// A picker as the GUI shows it: the query and a window of its rows.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Palette {
    pub title: String,
    pub query: String,
    /// The caret, in characters of `query`.
    pub caret: usize,
    /// Cmd-A selected the whole query.
    #[serde(default)]
    pub all: bool,
    /// Rows `first..first + rows.len()` of `total`.
    pub rows: Vec<PaletteRow>,
    pub first: usize,
    pub total: usize,
    /// The selected row's index in `rows`; none while the typed text itself is.
    pub sel: Option<usize>,
    /// Counts, or why it can't search.
    pub status: String,
    /// Rows are file paths (show a name and its folder).
    pub files: bool,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct PaletteRow {
    pub text: String,
    /// Matched character ranges of `text`.
    pub hl: Vec<(usize, usize)>,
}

/// Window `win`'s buffer as minimap runs; `lines` is None for huge files.
#[derive(Serialize, Debug)]
pub struct Minimap {
    pub t: &'static str,
    pub win: usize,
    pub version: u64,
    pub lines: Option<super::minimap::Lines>,
}

/// A ratatui color as the protocol's code; `None` is the theme default.
pub fn color(c: Color) -> Option<u32> {
    Some(match c {
        Color::Reset => return None,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Indexed(i) => u32::from(i),
        Color::Rgb(r, g, b) => 0x100_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
    })
}

/// One line from the app.
#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum In {
    Resize {
        cols: u16,
        rows: u16,
        /// Columns per window for the minimap; 0 turns it off.
        #[serde(default)]
        margin: u16,
        /// Cell height / width × 100 (a terminal's is about 200); 0 = unknown.
        #[serde(default)]
        aspect: u16,
    },
    Keys {
        seq: u64,
        keys: String,
    },
    Paste {
        seq: u64,
        text: String,
    },
    Mouse {
        seq: u64,
        kind: String,
        col: u16,
        row: u16,
    },
    Mac {
        seq: u64,
        key: char,
        shift: bool,
    },
    /// Open a file in the active window, as the `Space p` picker does.
    Open {
        seq: u64,
        path: std::path::PathBuf,
        /// A sidebar single-click: a preview tab, which the next replaces.
        #[serde(default)]
        preview: bool,
        /// A drag and drop: onto window `target`'s `zone` (as a tab drop).
        #[serde(default)]
        target: Option<usize>,
        #[serde(default)]
        zone: String,
    },
    /// A tab strip action on window `win`'s tab `index`: `select`,
    /// `close`, `move` (to `to`), `keep`, or `drop` on window `target`'s
    /// `zone` (`center` as its tab `to`, or an edge to split it).
    Tab {
        seq: u64,
        win: usize,
        op: String,
        index: usize,
        #[serde(default)]
        to: usize,
        #[serde(default)]
        target: usize,
        #[serde(default)]
        zone: String,
    },
    /// The sidebar's visible rows, top to bottom, for zap to label.
    Nav {
        rows: Vec<NavRow>,
    },
    /// The terminal panel: `op` toggle (Ctrl-`) or maximize.
    Panel {
        seq: u64,
        op: String,
    },
    /// A click on palette row `index` (absolute): select it and accept.
    Pick {
        seq: u64,
        index: usize,
    },
    /// Work in folder `path` (`:cd`): the chosen project.
    Cd {
        seq: u64,
        path: String,
    },
    /// The minimap: show window `win` from line `top`.
    Scroll {
        seq: u64,
        win: usize,
        top: usize,
    },
    Quit,
}
