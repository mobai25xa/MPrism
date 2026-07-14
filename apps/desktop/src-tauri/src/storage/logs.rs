//! Application log writer with key/content redaction.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::error::{StorageError, StorageResult};
use super::paths::logs_dir;

const MAX_LOG_FILES: usize = 7;

pub struct AppLogger {
    root: PathBuf,
    lock: Mutex<()>,
}

impl AppLogger {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn info(&self, message: &str) {
        let _ = self.write_line("INFO", message);
    }

    pub fn warn(&self, message: &str) {
        let _ = self.write_line("WARN", message);
    }

    pub fn error(&self, message: &str) {
        let _ = self.write_line("ERROR", message);
    }

    fn write_line(&self, level: &str, message: &str) -> StorageResult<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let dir = logs_dir(&self.root);
        fs::create_dir_all(&dir).map_err(|e| StorageError::io(&dir, e))?;
        self.prune_old_logs(&dir)?;

        let now = OffsetDateTime::now_utc();
        let day = format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            u8::from(now.month()),
            now.day()
        );
        let path = dir.join(format!("mprism.{day}.log"));
        let ts = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "unknown".into());
        let safe = redact_log_message(message);
        let line = format!("{ts} {level} {safe}\n");

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StorageError::io(&path, e))?;
        file.write_all(line.as_bytes())
            .map_err(|e| StorageError::io(&path, e))?;
        Ok(())
    }

    fn prune_old_logs(&self, dir: &Path) -> StorageResult<()> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .map_err(|e| StorageError::io(dir, e))?
            .flatten()
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("mprism.")
                    && e.file_name().to_string_lossy().ends_with(".log")
            })
            .collect();
        files.sort_by_key(|e| e.file_name());
        while files.len() > MAX_LOG_FILES {
            if let Some(oldest) = files.first() {
                let path = oldest.path();
                let _ = fs::remove_file(&path);
                files.remove(0);
            } else {
                break;
            }
        }
        Ok(())
    }
}

/// Best-effort redaction for accidental secrets in log lines.
pub fn redact_log_message(message: &str) -> String {
    let mut out = message.to_string();
    // Bearer tokens
    if let Some(idx) = out.to_ascii_lowercase().find("bearer ") {
        let rest = &out[idx + 7..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end > 0 {
            let start = idx + 7;
            out.replace_range(start..start + end, "***");
        }
    }
    // sk- style keys
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 3 < chars.len()
            && chars[i] == 's'
            && chars[i + 1] == 'k'
            && chars[i + 2] == '-'
            && chars[i + 3].is_ascii_alphanumeric()
        {
            result.push_str("sk-***");
            i += 3;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}
