//! The export pipeline (org-export-as).  Work in progress: only the
//! timestamp formatting the helpers need is here so far.

use crate::org::element::{NodeId, Tree};
use crate::org::time::civil::{Tm, format};

/// org-format-timestamp: the start (or END) of timestamp ID with
/// format-time-string FMT.
pub fn format_timestamp(t: &Tree, id: NodeId, fmt: &str, end: bool) -> String {
    let k = |s: &str| {
        let key = format!("{s}-{}", if end { "end" } else { "start" });
        t.int(id, &key)
    };
    let (Some(y), Some(m), Some(d)) = (k("year"), k("month"), k("day")) else {
        return String::new();
    };
    let tm = match (k("hour"), k("minute")) {
        (Some(h), Some(mi)) => Tm::at(y, m, d, h, mi),
        _ => Tm::date(y, m, d),
    };
    format(fmt, tm)
}
