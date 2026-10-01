//! Loading files, safe writes, and detecting changes made by others.

use crate::buffer::Buffer;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What the file looked like when we last read or wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: SystemTime,
    pub size: u64,
    pub hash: u64,
}

#[derive(Debug)]
pub struct Loaded {
    pub buf: Buffer,
    pub stamp: Option<FileStamp>,
    pub readonly: bool,
    pub notice: Option<String>,
}

fn hash(data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(data);
    h.finish()
}

pub fn err_msg(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        io::ErrorKind::NotFound => "no such file or directory".into(),
        io::ErrorKind::IsADirectory => "is a directory".into(),
        _ => {
            let s = e.to_string();
            s.split(" (os error").next().unwrap_or(&s).to_lowercase()
        }
    }
}

fn writable(path: &Path) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

fn stamp_of(path: &Path, data: &[u8]) -> Option<FileStamp> {
    let m = fs::metadata(path).ok()?;
    Some(FileStamp { mtime: m.modified().ok()?, size: m.len(), hash: hash(data) })
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    match fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut buf = Buffer::from_text("");
            buf.final_newline = true;
            return Ok(Loaded { buf, stamp: None, readonly: false, notice: Some("[new]".into()) });
        }
        Err(e) => return Err(err_msg(&e)),
        Ok(m) if m.is_dir() => return Err("is a directory".into()),
        Ok(_) => {}
    }
    let bytes = fs::read(path).map_err(|e| err_msg(&e))?;
    let stamp = stamp_of(path, &bytes);
    let (buf, readonly, mut notice) = match std::str::from_utf8(&bytes) {
        Ok(s) => (Buffer::from_text(s), !writable(path), None),
        Err(_) => (Buffer::from_text(&String::from_utf8_lossy(&bytes)), true, Some("not valid UTF-8".to_string())),
    };
    if buf.mixed_endings && notice.is_none() {
        let as_ = if buf.line_ending == crate::buffer::LineEnding::CrLf { "CRLF" } else { "LF" };
        notice = Some(format!("mixed line endings; writing as {as_}"));
    }
    Ok(Loaded { buf, stamp, readonly, notice })
}

/// Whether the file no longer matches `stamp` (content compared by hash).
pub fn changed_on_disk(path: &Path, stamp: Option<&FileStamp>) -> bool {
    let Some(st) = stamp else { return false };
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match fs::metadata(&real) {
        Err(_) => true,
        Ok(m) => differs(&real, st, &m),
    }
}

fn differs(real: &Path, st: &FileStamp, m: &fs::Metadata) -> bool {
    if m.len() == st.size && m.modified().ok() == Some(st.mtime) {
        return false;
    }
    match fs::read(real) {
        Ok(data) => hash(&data) != st.hash,
        Err(_) => true,
    }
}

/// Write `data` to `path` without ever leaving a half-written file behind.
pub fn write(path: &Path, data: &[u8], expected: Option<&FileStamp>, force: bool) -> Result<FileStamp, String> {
    let real: PathBuf = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let meta = fs::metadata(&real).ok();
    if let Some(m) = &meta {
        if m.is_dir() {
            return Err("is a directory".into());
        }
        if !force {
            match expected {
                Some(st) if differs(&real, st, m) => return Err("file changed on disk (w! to overwrite)".into()),
                None => return Err("file exists (w! to overwrite)".into()),
                _ => {}
            }
        }
        if !writable(&real) {
            return Err("permission denied".into());
        }
    }
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let replaceable = meta.as_ref().is_none_or(|m| m.nlink() <= 1 && m.uid() == uid);
    let written = if replaceable { write_replace(&real, data, meta.as_ref()) } else { Err(None) };
    match written {
        Ok(()) => {}
        Err(Some(e)) => return Err(e),
        Err(None) if meta.is_some() => write_in_place(&real, data).map_err(|e| err_msg(&e))?,
        Err(None) => {
            return Err(match real.parent().map(Path::exists) {
                Some(false) => "no such directory".into(),
                _ => "permission denied".into(),
            });
        }
    }
    stamp_of(&real, data).ok_or_else(|| "cannot stat written file".into())
}

/// Temp file + rename. `Err(None)` = can't create the temp file; try in place.
fn write_replace(real: &Path, data: &[u8], meta: Option<&fs::Metadata>) -> Result<(), Option<String>> {
    let dir = match real.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = real.file_name().ok_or(None)?.to_string_lossy().into_owned();
    let tmp = dir.join(format!(".{name}.fred~{}", std::process::id()));
    let mode = meta.map_or(0o666, |m| m.permissions().mode() & 0o7777);
    let mut f = OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp).map_err(|_| None)?;
    let res = (|| -> io::Result<()> {
        f.write_all(data)?;
        if meta.is_some() {
            fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        }
        f.sync_all()?;
        fs::rename(&tmp, real)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Some(err_msg(&e)));
    }
    if let Ok(d) = File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn write_in_place(real: &Path, data: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).truncate(true).open(real)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn roundtrip_byte_identical() {
        let d = tempfile::tempdir().unwrap();
        for (i, s) in ["a\r\nb", "\u{feff}a\n", "x\ny\n", "", "\n\n"].iter().enumerate() {
            let p = d.path().join(format!("f{i}"));
            fs::write(&p, s).unwrap();
            let l = load(&p).unwrap();
            write(&p, &l.buf.to_bytes(), l.stamp.as_ref(), false).unwrap();
            assert_eq!(fs::read(&p).unwrap(), s.as_bytes(), "{s:?}");
        }
    }

    #[test]
    fn preserves_mode_and_symlink() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        let link = d.path().join("link");
        fs::write(&real, "old\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let l = load(&link).unwrap();
        write(&link, b"new\n", l.stamp.as_ref(), false).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "new\n");
        assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o640);
        let leftovers: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers.len(), 2, "temp file left behind: {leftovers:?}");
    }

    #[test]
    fn hard_link_kept() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        fs::write(&a, "old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        let l = load(&a).unwrap();
        write(&a, b"new", l.stamp.as_ref(), false).unwrap();
        assert_eq!(fs::read_to_string(&b).unwrap(), "new");
    }

    #[test]
    fn detects_external_change() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, "one\n").unwrap();
        let l = load(&p).unwrap();
        fs::write(&p, "someone else\n").unwrap();
        assert!(changed_on_disk(&p, l.stamp.as_ref()));
        let e = write(&p, b"mine\n", l.stamp.as_ref(), false).unwrap_err();
        assert_eq!(e, "file changed on disk (w! to overwrite)");
        assert_eq!(fs::read_to_string(&p).unwrap(), "someone else\n");
        let st = write(&p, b"mine\n", l.stamp.as_ref(), true).unwrap();
        assert!(!changed_on_disk(&p, Some(&st)));
    }

    #[test]
    fn touched_but_same_content_is_not_a_change() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, "one\n").unwrap();
        let l = load(&p).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&p, "one\n").unwrap();
        assert!(!changed_on_disk(&p, l.stamp.as_ref()));
    }

    #[test]
    fn deleted_file_is_written_again() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, "one\n").unwrap();
        let l = load(&p).unwrap();
        fs::remove_file(&p).unwrap();
        write(&p, b"two\n", l.stamp.as_ref(), false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two\n");
    }

    #[test]
    fn readonly_and_invalid_utf8() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ro");
        fs::write(&p, "x").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(load(&p).unwrap().readonly);
        let e = write(&p, b"y", None, true).unwrap_err();
        assert_eq!(e, "permission denied");
        let q = d.path().join("bin");
        fs::write(&q, [b'a', 0xff, b'\n']).unwrap();
        let l = load(&q).unwrap();
        assert!(l.readonly);
        assert_eq!(l.notice.as_deref(), Some("not valid UTF-8"));
    }

    #[test]
    fn missing_file_is_new() {
        let d = tempfile::tempdir().unwrap();
        let l = load(&d.path().join("nope")).unwrap();
        assert_eq!(l.notice.as_deref(), Some("[new]"));
        assert!(l.stamp.is_none());
        assert!(l.buf.final_newline);
        assert_eq!(load(d.path()).unwrap_err(), "is a directory");
    }

    #[test]
    fn write_into_missing_dir_fails() {
        let d = tempfile::tempdir().unwrap();
        assert!(write(&d.path().join("no/such/f"), b"x", None, false).is_err());
    }
}
