//! Swap files: unsaved text kept on disk so a crash or kill loses nothing.

use serde::{Deserialize, Serialize};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &str = "fred-swap";
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    magic: String,
    version: u32,
    pid: u32,
    host: String,
    path: Option<String>,
    saved_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInfo {
    pub pid: u32,
    pub host: String,
    pub path: Option<String>,
    /// Seconds since the Unix epoch.
    pub saved_at: u64,
    pub text: String,
}

/// `$XDG_STATE_HOME/fred/swap`, defaulting to `~/.local/state/fred/swap`.
pub fn swap_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    base.join("fred/swap")
}

/// The real absolute path a swap file is keyed on.
pub fn canonical(file: &Path) -> PathBuf {
    fs::canonicalize(file)
        .or_else(|_| std::path::absolute(file))
        .unwrap_or_else(|_| file.to_path_buf())
}

pub fn swap_name(file: Option<&Path>) -> String {
    match file {
        None => format!("unnamed-{}.swp", std::process::id()),
        Some(f) => {
            let abs = f.to_string_lossy();
            format!("{}.swp", abs.replace('%', "%25").replace('/', "%2F"))
        }
    }
}

pub fn swap_path_in(dir: &Path, file: Option<&Path>) -> PathBuf {
    dir.join(swap_name(file.map(canonical).as_deref()))
}

pub fn swap_path(file: Option<&Path>) -> PathBuf {
    swap_path_in(&swap_dir(), file)
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for buf.len() bytes; gethostname NUL-terminates on success.
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Atomically write the swap file (mode 0600).
pub fn write(swap: &Path, file: Option<&Path>, text: &str) -> Result<(), String> {
    let dir = swap.parent().ok_or("bad swap path")?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("swap: {e}"))?;
    let header = Header {
        magic: MAGIC.into(),
        version: VERSION,
        pid: std::process::id(),
        host: hostname(),
        path: file.map(|f| canonical(f).to_string_lossy().into_owned()),
        saved_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let mut data = serde_json::to_string(&header).map_err(|e| e.to_string())?;
    data.push('\n');
    data.push_str(text);
    let tmp = swap.with_extension(format!("swp.tmp{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, swap)
    })();
    res.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("swap: {e}")
    })
}

pub fn read(swap: &Path) -> Result<SwapInfo, String> {
    let data = fs::read(swap).map_err(|e| e.to_string())?;
    let data = String::from_utf8_lossy(&data);
    let (head, text) = data.split_once('\n').ok_or("not a swap file")?;
    let h: Header = serde_json::from_str(head).map_err(|_| "not a swap file")?;
    if h.magic != MAGIC || h.version != VERSION {
        return Err("not a swap file".into());
    }
    Ok(SwapInfo {
        pid: h.pid,
        host: h.host,
        path: h.path,
        saved_at: h.saved_at,
        text: text.to_string(),
    })
}

/// Another live process on this host owns the swap file.
pub fn owner_alive(info: &SwapInfo) -> bool {
    if info.pid == std::process::id() || info.host != hostname() || info.pid == 0 {
        return false;
    }
    let Ok(pid) = libc::pid_t::try_from(info.pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks for existence.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn remove(swap: &Path) {
    let _ = fs::remove_file(swap);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn name_escapes_path() {
        assert_eq!(swap_name(Some(Path::new("/a/b%c"))), "%2Fa%2Fb%25c.swp");
        assert_eq!(
            swap_name(None),
            format!("unnamed-{}.swp", std::process::id())
        );
    }

    #[test]
    fn roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let sp = swap_path_in(&d.path().join("swap"), Some(Path::new("/x/y.txt")));
        write(&sp, Some(Path::new("/x/y.txt")), "hello\nworld").unwrap();
        let info = read(&sp).unwrap();
        assert_eq!(info.text, "hello\nworld");
        assert_eq!(info.pid, std::process::id());
        assert_eq!(info.path.as_deref(), Some("/x/y.txt"));
        assert_eq!(info.host, hostname());
        assert_eq!(
            std::fs::metadata(&sp).unwrap().permissions().mode() & 0o777,
            0o600
        );
        remove(&sp);
        assert!(!sp.exists());
    }

    #[test]
    fn rejects_garbage() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bad.swp");
        std::fs::write(&p, "not a swap file").unwrap();
        assert!(read(&p).is_err());
    }

    #[test]
    fn owner_liveness() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let mut info = SwapInfo {
            pid: child.id(),
            host: hostname(),
            path: None,
            saved_at: 0,
            text: String::new(),
        };
        assert!(owner_alive(&info));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!owner_alive(&info));
        info.pid = std::process::id();
        assert!(!owner_alive(&info), "our own pid is not another owner");
        info.pid = 1;
        info.host = "some-other-host".into();
        assert!(!owner_alive(&info), "other hosts can't be checked");
    }
}
