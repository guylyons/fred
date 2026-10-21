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
    pub status: Status,
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
    Quit,
}
