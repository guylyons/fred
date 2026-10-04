//! Port of upstream Org Babel (ob-*.el): see docs/org-port-notes.md.

use super::Prefix;
use crate::editor::Editor;

/// Babel's interactive commands, by upstream name.
pub fn command(_ed: &mut Editor, _name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    None
}
