//! The system clipboard: pbcopy/pbpaste on macOS, wl-copy/wl-paste on
//! Wayland, xclip on X11. A missing tool just means no clipboard.

use crate::editor::Register;
use std::io::Write;
use std::process::{Command, Stdio};

fn tools() -> (&'static [&'static str], &'static [&'static str]) {
    if cfg!(target_os = "macos") {
        (&["pbcopy"], &["pbpaste"])
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        (&["wl-copy"], &["wl-paste", "-n"])
    } else {
        (
            &["xclip", "-selection", "clipboard"],
            &["xclip", "-selection", "clipboard", "-o"],
        )
    }
}

/// Copy `reg` to the clipboard. Waits, so a `p` right after sees it
/// (wl-copy and xclip go to the background to serve it).
pub fn set(reg: &Register) {
    let cmd = tools().0;
    let Ok(mut child) = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(to_clip(reg).as_bytes());
    }
    let _ = child.wait();
}

/// What the clipboard holds, if it can be read.
pub fn get() -> Option<String> {
    let cmd = tools().1;
    let out = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// A register as clipboard text: whole lines end in a newline.
pub fn to_clip(reg: &Register) -> String {
    if reg.linewise {
        format!("{}\n", reg.text)
    } else {
        reg.text.clone()
    }
}

/// Clipboard text as a register: text ending in a newline is whole lines.
pub fn from_clip(text: &str) -> Register {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    match text.strip_suffix('\n') {
        Some(lines) => Register {
            text: lines.to_string(),
            linewise: true,
        },
        None => Register {
            text,
            linewise: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_round_trip() {
        for (text, linewise) in [("a\nb", true), ("", true), ("word", false), ("a\nb", false)] {
            let r = Register {
                text: text.into(),
                linewise,
            };
            assert_eq!(from_clip(&to_clip(&r)), r);
        }
        assert_eq!(from_clip("x\r\n"), from_clip("x\n"));
    }
}
