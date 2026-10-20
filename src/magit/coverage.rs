//! The scenario matrix's evidence: with FRED_MAGIT_COVERAGE naming a file,
//! every Magit action dispatched and every option read or set appends
//! "test<TAB>kind<TAB>what" to it (the test is the running thread's name, as
//! cargo test names its threads). Off otherwise.
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

pub fn record(kind: &str, what: &str) {
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("FRED_MAGIT_COVERAGE")?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Mutex::new(f))
    });
    let Some(file) = file else {
        return;
    };
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("main");
    if let Ok(mut f) = file.lock() {
        // One write per line, so concurrent tests do not interleave.
        let _ = f.write_all(format!("{test}\t{kind}\t{what}\n").as_bytes());
    }
}
