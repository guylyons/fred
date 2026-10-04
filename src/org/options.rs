//! Org user options: the `[org]` table of the config, keyed by the Emacs
//! variable name (`org-todo-keywords = ...`). Callers give the upstream
//! default.

use std::cell::RefCell;
use toml::Value;

thread_local! {
    static TABLE: RefCell<toml::Table> = RefCell::new(toml::Table::new());
}

/// Install the `[org]` table (at startup, or from a test).
pub fn set(table: toml::Table) {
    TABLE.with(|t| *t.borrow_mut() = table);
}

/// Set one option (`:org-set-option`, tests).
pub fn put(name: &str, v: Value) {
    TABLE.with(|t| t.borrow_mut().insert(name.to_owned(), v));
}

pub fn get(name: &str) -> Option<Value> {
    TABLE.with(|t| t.borrow().get(name).cloned())
}

pub fn bool(name: &str, default: bool) -> bool {
    match get(name) {
        Some(Value::Boolean(b)) => b,
        // Emacs non-nil symbols: any string except "nil" counts as t.
        Some(Value::String(s)) => s != "nil",
        Some(Value::Integer(i)) => i != 0,
        _ => default,
    }
}

pub fn int(name: &str, default: i64) -> i64 {
    match get(name) {
        Some(Value::Integer(i)) => i,
        Some(Value::Float(f)) => f as i64,
        Some(Value::String(s)) => s.parse().unwrap_or(default),
        _ => default,
    }
}

/// A string or symbol option; booleans read as "t"/"nil".
pub fn string(name: &str, default: &str) -> String {
    match get(name) {
        Some(Value::String(s)) => s,
        Some(Value::Boolean(b)) => if b { "t" } else { "nil" }.into(),
        Some(Value::Integer(i)) => i.to_string(),
        _ => default.into(),
    }
}

/// A list of strings (a single string is a one-element list).
pub fn strings(name: &str, default: &[&str]) -> Vec<String> {
    match get(name) {
        Some(Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => vec![s],
        _ => default.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// `~/` expanded.
pub fn expand(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/").or((path == "~").then_some("")) {
        Some(rest) => std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => std::path::PathBuf::from(path),
    }
}

/// org-directory.
pub fn directory() -> std::path::PathBuf {
    expand(&string("org-directory", "~/org"))
}

/// A file option relative to org-directory.
pub fn file(name: &str, default: &str) -> std::path::PathBuf {
    let f = expand(&string(name, default));
    if f.is_absolute() { f } else { directory().join(f) }
}
