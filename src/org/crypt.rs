//! Encrypting entries (org-crypt.el) with the `gpg` command.
//!
//! Adaptations: EasyPG is replaced by `gpg --armor`; symmetric passphrases
//! are read in a masked prompt and passed through `--passphrase-fd 0` with
//! loopback pinentry. Emacs keeps the ciphertext of an unchanged decrypted
//! entry in text properties; Fred keeps it in a per-session table keyed by
//! the plaintext checksum. Auto-save leakage is about Fred's swap files.

use super::props::{Doc, Inherit};
use super::syntax;
use super::{Prefix, fold, tags};
use crate::editor::Editor;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

thread_local! {
    /// Plaintext checksum → (key, ciphertext) of entries decrypted here.
    static KEPT: RefCell<HashMap<u64, (String, String)>> = RefCell::new(HashMap::new());
}

fn checksum(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

const BEGIN: &str = "-----BEGIN PGP MESSAGE-----";
const END: &str = "-----END PGP MESSAGE-----";

/// org-at-encrypted-entry-p: the encrypted lines `beg..end` of the entry.
pub fn encrypted_region(ed: &Editor, h: usize) -> Option<(usize, usize)> {
    let start = super::todo::end_of_meta_data(ed, h, false);
    if start >= ed.line_count() || ed.buf.line(start).trim() != BEGIN {
        return None;
    }
    for l in start + 1..ed.line_count() {
        let t = ed.buf.line(l);
        if t.trim() == END {
            return Some((start, l + 1));
        }
        if syntax::level(&t).is_some() {
            return None;
        }
    }
    None
}

/// org-crypt-key-for-heading: the key, or None for symmetric encryption.
fn key_for(ed: &Editor, h: usize) -> Option<String> {
    let st = super::settings(ed);
    let path = ed.path.clone();
    let doc = Doc::new(&ed.buf, &st, path.as_deref());
    let global = super::options::string("org-crypt-key", "");
    let k = match doc
        .get(Some(h), "CRYPTKEY", Inherit::Selective, true)
        .as_deref()
    {
        Some("nil") => String::new(),
        Some(k) => k.to_owned(),
        None => global,
    };
    (!k.trim().is_empty()).then_some(k)
}

/// Run gpg with `input` on stdin.
fn gpg(args: &[&str], input: &str) -> Result<String, String> {
    let mut child = Command::new("gpg")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gpg: {e}"))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("gpg failed")
            .to_owned())
    }
}

/// The body lines of the entry at `h` to encrypt: after the meta data to
/// the end of the subtree, without trailing blank lines.
fn body(ed: &Editor, h: usize) -> (usize, usize) {
    let beg = super::todo::end_of_meta_data(ed, h, false);
    let mut end = syntax::subtree_end(&ed.buf, h);
    while end > beg && ed.buf.line(end - 1).trim().is_empty() {
        end -= 1;
    }
    (beg, end)
}

/// Encrypt the entry at `h` with `pass` (symmetric) when there is no key.
fn encrypt_at(ed: &mut Editor, h: usize, pass: Option<&str>) -> Result<(), String> {
    if encrypted_region(ed, h).is_some() {
        return Ok(());
    }
    let (beg, end) = body(ed, h);
    if beg >= end {
        return Ok(());
    }
    let contents: String = super::lines(ed, beg..end)
        .iter()
        .map(|l| format!("{l}\n"))
        .collect();
    let key = key_for(ed, h);
    let reuse = KEPT.with(|k| {
        k.borrow()
            .get(&checksum(&contents))
            .filter(|(kk, _)| *kk == key.clone().unwrap_or_default())
            .map(|(_, c)| c.clone())
    });
    let cipher = match reuse {
        Some(c) => c,
        None => match (&key, pass) {
            (Some(k), _) => gpg(
                &["--batch", "--yes", "--armor", "--encrypt", "--recipient", k],
                &contents,
            )?,
            (None, Some(p)) => gpg(
                &[
                    "--batch",
                    "--yes",
                    "--armor",
                    "--symmetric",
                    "--pinentry-mode",
                    "loopback",
                    "--passphrase-fd",
                    "0",
                ],
                &format!("{p}\n{contents}"),
            )?,
            (None, None) => {
                return Err(
                    "No crypt key set: a passphrase is needed for symmetric encryption".into(),
                );
            }
        },
    };
    let lines: Vec<String> = cipher.trim_end().lines().map(str::to_owned).collect();
    let folded = h + 1 < ed.line_count() && fold::hidden(ed, h + 1);
    super::splice(ed, beg, end - beg, &lines);
    if folded {
        fold::fold_subtree(ed, h, true);
    }
    Ok(())
}

/// Decrypt the entry at `h` (passphrase only for symmetric encryption).
fn decrypt_at(ed: &mut Editor, h: usize, pass: Option<&str>) -> Result<(), String> {
    let Some((beg, end)) = encrypted_region(ed, h) else {
        return Ok(());
    };
    let cipher: String = super::lines(ed, beg..end)
        .iter()
        .map(|l| format!("{}\n", l.trim_start()))
        .collect();
    let plain = match pass {
        Some(p) => gpg(
            &[
                "--batch",
                "--yes",
                "--decrypt",
                "--pinentry-mode",
                "loopback",
                "--passphrase-fd",
                "0",
            ],
            &format!("{p}\n{cipher}"),
        )?,
        None => gpg(&["--batch", "--yes", "--decrypt"], &cipher)?,
    };
    let level = syntax::level(&ed.buf.line(h)).unwrap_or(1);
    let mut lines: Vec<String> = plain
        .strip_suffix('\n')
        .unwrap_or(&plain)
        .split('\n')
        .map(str::to_owned)
        .collect();
    // Subheadings are re-levelled under the heading (org-decrypt-entry).
    let min = lines.iter().filter_map(|l| syntax::level(l)).min();
    if let Some(min) = min
        && min <= level
    {
        let shift = level + 1 - min;
        for l in lines.iter_mut() {
            if syntax::level(l).is_some() {
                *l = format!("{}{l}", "*".repeat(shift));
            }
        }
    } else {
        KEPT.with(|k| {
            k.borrow_mut().insert(
                checksum(&plain),
                (key_for(ed, h).unwrap_or_default(), cipher.clone()),
            )
        });
    }
    super::splice(ed, beg, end - beg, &lines);
    Ok(())
}

/// Entries matched by org-crypt-tag-matcher.
fn crypt_entries(ed: &Editor) -> Vec<usize> {
    let m = super::options::string("org-crypt-tag-matcher", "crypt");
    let st = super::settings(ed);
    let matcher = tags::Matcher::new(&m, &st, super::now());
    let path = ed.path.clone();
    tags::scan(&ed.buf, &st, path.as_deref(), &matcher)
}

/// Ask for a passphrase in a masked prompt, unless a key is set.
fn with_pass(
    ed: &mut Editor,
    needed: bool,
    then: impl FnOnce(&mut Editor, Option<String>) + Send + 'static,
) {
    if !needed {
        return then(ed, None);
    }
    super::read(
        ed,
        "Passphrase for symmetric encryption: ",
        "",
        move |ed, p| then(ed, Some(p)),
    );
    if let crate::editor::Mode::Command(cl) = &mut ed.mode {
        cl.secret = true;
    }
}

/// org-crypt-check-auto-save, for Fred's swap file; then `then`.
fn check_swap(ed: &mut Editor, then: impl FnOnce(&mut Editor) + Send + 'static) {
    let mode = super::sexp::option("org-crypt-disable-auto-save").map_or("ask".to_owned(), |v| {
        v.str().or(v.sym()).unwrap_or("t").to_owned()
    });
    match mode.as_str() {
        "nil" => {
            ed.set_msg("org-decrypt: Decrypting entry with the swap file enabled.  This may cause leakage.");
            then(ed)
        }
        "encrypt" => {
            if let Some(o) = &mut ed.org {
                o.reencrypt_on_save = true;
            }
            then(ed)
        }
        "ask" => super::menu(
            ed,
            "org-decrypt: The swap file may cause leakage. Disable it for current buffer?",
            vec![("y".into(), "yes".into()), ("n".into(), "no".into())],
            move |ed, k| {
                if k == "y" {
                    super::effect(ed, |s| s.org_disable_swap());
                }
                then(ed)
            },
        ),
        _ => {
            super::effect(ed, |s| s.org_disable_swap());
            then(ed)
        }
    }
}

/// Encrypt every tagged entry before a save (org-crypt-use-before-save-magic).
pub fn before_save(ed: &mut Editor) {
    let magic = super::options::bool("org-crypt-use-before-save-magic", false)
        || ed.org.as_ref().is_some_and(|o| o.reencrypt_on_save);
    if !magic || ed.org.is_none() {
        return;
    }
    let mut heads = crypt_entries(ed);
    heads.reverse();
    ed.undo.begin(ed.cur.pos());
    for h in heads {
        if key_for(ed, h).is_some()
            && let Err(e) = encrypt_at(ed, h, None)
        {
            ed.set_err(e);
        }
    }
    ed.undo.end(ed.cur.pos());
}

pub fn command(ed: &mut Editor, name: &str, _arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    Some(match name {
        "org-encrypt-entry" => match fold::back_to_heading(ed, l) {
            None => Ok(()),
            Some(h) => {
                let needed = key_for(ed, h).is_none();
                with_pass(ed, needed, move |ed, p| {
                    ed.undo.begin(ed.cur.pos());
                    if let Err(e) = encrypt_at(ed, h, p.as_deref()) {
                        ed.set_err(e);
                    }
                    ed.undo.end(ed.cur.pos());
                });
                Ok(())
            }
        },
        "org-decrypt-entry" => match fold::back_to_heading(ed, l) {
            Some(h) if encrypted_region(ed, h).is_some() => {
                let needed = key_for(ed, h).is_none();
                check_swap(ed, move |ed| {
                    with_pass(ed, needed, move |ed, p| {
                        ed.undo.begin(ed.cur.pos());
                        if let Err(e) = decrypt_at(ed, h, p.as_deref()) {
                            ed.set_err(e);
                        }
                        ed.undo.end(ed.cur.pos());
                    })
                });
                Ok(())
            }
            _ => Ok(()),
        },
        "org-encrypt-entries" | "org-decrypt-entries" => {
            let enc = name == "org-encrypt-entries";
            let heads = crypt_entries(ed);
            let needed = heads.iter().any(|&h| key_for(ed, h).is_none());
            let run = move |ed: &mut Editor| {
                with_pass(ed, needed, move |ed, p| {
                    ed.undo.begin(ed.cur.pos());
                    for &h in heads.iter().rev() {
                        let r = if enc {
                            encrypt_at(ed, h, p.as_deref())
                        } else {
                            decrypt_at(ed, h, p.as_deref())
                        };
                        if let Err(e) = r {
                            ed.set_err(e);
                            break;
                        }
                    }
                    ed.undo.end(ed.cur.pos());
                })
            };
            if enc {
                run(ed)
            } else {
                check_swap(ed, run)
            }
            Ok(())
        }
        "org-crypt-use-before-save-magic" => {
            super::options::put(
                "org-crypt-use-before-save-magic",
                toml::Value::Boolean(true),
            );
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;
    use super::*;

    fn gpg_ok() -> bool {
        Command::new("gpg")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn symmetric_round_trip() {
        if !gpg_ok() {
            eprintln!("skipped: gpg not installed");
            return;
        }
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module run serially on their own GNUPGHOME.
        unsafe { std::env::set_var("GNUPGHOME", home.path()) };
        let mut e = org("* Secret :crypt:\nhidden text\n** sub\nmore", "");
        encrypt_at(&mut e, 0, Some("pw")).unwrap();
        assert_eq!(e.buf.line(1), BEGIN);
        assert!(encrypted_region(&e, 0).is_some());
        decrypt_at(&mut e, 0, Some("pw")).unwrap();
        assert_eq!(e.buf.text(), "* Secret :crypt:\nhidden text\n** sub\nmore");
        // Wrong passphrase leaves the entry encrypted.
        encrypt_at(&mut e, 0, Some("pw")).unwrap();
        let _ = std::process::Command::new("gpgconf")
            .args(["--kill", "gpg-agent"])
            .env("GNUPGHOME", home.path())
            .output();
        assert!(decrypt_at(&mut e, 0, Some("wrong")).is_err());
        assert_eq!(e.buf.line(1), BEGIN);
        unsafe { std::env::remove_var("GNUPGHOME") };
    }
}
