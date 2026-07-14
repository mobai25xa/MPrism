//! Atomic JSON write helpers.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use uuid::Uuid;

use super::error::{StorageError, StorageResult};
use super::paths::ensure_parent_dir;

/// Serialize `value` as pretty UTF-8 JSON (LF) and atomically replace `path`.
pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> StorageResult<()> {
    let mut body = serde_json::to_string_pretty(value)
        .map_err(|e| StorageError::Internal(format!("serialize failed: {e}")))?;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    // Normalize any CRLF that serde might not produce (serde uses LF).
    let bytes = body.into_bytes();
    atomic_write_bytes(path, &bytes)
}

pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> StorageResult<()> {
    ensure_parent_dir(path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let tmp = dir.join(format!("{name}.tmp-{}", Uuid::now_v7()));

    {
        let mut file = File::create(&tmp).map_err(|e| StorageError::io(&tmp, e))?;
        file.write_all(bytes)
            .map_err(|e| StorageError::io(&tmp, e))?;
        file.sync_all().map_err(|e| StorageError::io(&tmp, e))?;
    }

    match replace_file(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(StorageError::io(path, e))
        }
    }
}

fn replace_file(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    match fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            if !dest.exists() {
                return Err(err);
            }
            // Windows cannot rename over an existing file; swap via backup.
            let bak = dest.with_extension(format!("bak-{}", Uuid::now_v7().as_simple()));
            fs::rename(dest, &bak)?;
            match fs::rename(tmp, dest) {
                Ok(()) => {
                    let _ = fs::remove_file(&bak);
                    Ok(())
                }
                Err(e) => {
                    let _ = fs::rename(&bak, dest);
                    Err(e)
                }
            }
        }
    }
}

pub fn read_to_string(path: &Path) -> StorageResult<String> {
    let mut file = File::open(path).map_err(|e| StorageError::io(path, e))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|e| StorageError::io(path, e))?;
    // Strip UTF-8 BOM if present.
    if buf.starts_with('\u{feff}') {
        buf = buf.trim_start_matches('\u{feff}').to_string();
    }
    Ok(buf)
}

/// Remove `*.tmp-*` files under `root` older than 24 hours.
pub fn cleanup_stale_temp_files(root: &Path) -> StorageResult<usize> {
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    let mut removed = 0usize;
    cleanup_dir_temps(root, cutoff, &mut removed)?;
    if let Ok(sessions) = fs::read_dir(root.join("sessions")) {
        for entry in sessions.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                cleanup_dir_temps(&entry.path(), cutoff, &mut removed)?;
            }
        }
    }
    Ok(removed)
}

fn cleanup_dir_temps(dir: &Path, cutoff: SystemTime, removed: &mut usize) -> StorageResult<()> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(StorageError::io(dir, e)),
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Only files matching `<name>.tmp-<uuid>` pattern.
        if !name.contains(".tmp-") {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if modified < cutoff && fs::remove_file(&path).is_ok() {
            *removed += 1;
        }
    }
    Ok(())
}

/// Append a single line (without trailing newline in `line`) plus LF.
pub fn append_line(path: &Path, line: &str) -> StorageResult<()> {
    if line.contains('\n') || line.contains('\r') {
        return Err(StorageError::Internal(
            "JSONL record must not contain physical newlines".into(),
        ));
    }
    ensure_parent_dir(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| StorageError::io(path, e))?;
    file.write_all(line.as_bytes())
        .map_err(|e| StorageError::io(path, e))?;
    file.write_all(b"\n")
        .map_err(|e| StorageError::io(path, e))?;
    file.sync_all().map_err(|e| StorageError::io(path, e))?;
    Ok(())
}

#[allow(dead_code)]
pub fn list_tmp_paths(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(".tmp-") && entry.path().is_file() {
                out.push(entry.path());
            }
        }
    }
    out
}
