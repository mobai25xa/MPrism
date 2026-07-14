//! messages.jsonl append and resilient load.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use super::atomic::append_line;
use super::error::{StorageError, StorageResult};
use super::paths::{check_schema_version, session_messages_path};
use super::types::MessageRecord;

#[derive(Debug, Clone)]
pub enum MessageCorruption {
    MidFileCorrupt { line_number: usize },
    TrailingCorruptRecovered { recovery_path: PathBuf },
    DuplicateSequence { sequence: u64 },
    DuplicateMessageId { id: Uuid },
    SessionIdMismatch { line_number: usize },
}

#[derive(Debug, Clone)]
pub struct MessageLoadWarning {
    pub corruption: MessageCorruption,
}

#[derive(Debug, Clone)]
pub struct LoadMessagesResult {
    pub messages: Vec<MessageRecord>,
    pub warnings: Vec<MessageLoadWarning>,
    pub partially_corrupt: bool,
    pub next_sequence: u64,
}

pub fn append_message(root: &Path, message: &MessageRecord) -> StorageResult<()> {
    check_schema_version(message.schema_version)?;
    if message.sequence == 0 {
        return Err(StorageError::validation("sequence 必须从 1 开始"));
    }
    let path = session_messages_path(root, message.session_id);
    let line = serde_json::to_string(message)
        .map_err(|e| StorageError::Internal(format!("message serialize: {e}")))?;
    append_line(&path, &line)
}

pub fn next_sequence(root: &Path, session_id: Uuid) -> StorageResult<u64> {
    Ok(load_messages(root, session_id)?.next_sequence)
}

pub fn load_messages(root: &Path, session_id: Uuid) -> StorageResult<LoadMessagesResult> {
    let path = session_messages_path(root, session_id);
    if !path.exists() {
        return Ok(LoadMessagesResult {
            messages: Vec::new(),
            warnings: Vec::new(),
            partially_corrupt: false,
            next_sequence: 1,
        });
    }

    let file = File::open(&path).map_err(|e| StorageError::io(&path, e))?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::io(&path, e))?;

    let last_non_empty = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, l)| !l.trim().is_empty())
        .map(|(i, _)| i);

    let mut messages = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_seq = std::collections::HashSet::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut partially_corrupt = false;
    let mut max_seq = 0u64;
    let mut truncate_at: Option<usize> = None;
    let mut recovered_line: Option<String> = None;

    for (idx, line) in lines.iter().enumerate() {
        let line_number = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match try_parse_message(trimmed, session_id) {
            ParseOutcome::Ok(msg) => {
                if !seen_ids.insert(msg.id) {
                    warnings.push(MessageLoadWarning {
                        corruption: MessageCorruption::DuplicateMessageId { id: msg.id },
                    });
                    partially_corrupt = true;
                    continue;
                }
                if !seen_seq.insert(msg.sequence) {
                    warnings.push(MessageLoadWarning {
                        corruption: MessageCorruption::DuplicateSequence {
                            sequence: msg.sequence,
                        },
                    });
                    partially_corrupt = true;
                    continue;
                }
                max_seq = max_seq.max(msg.sequence);
                messages.push(*msg);
            }
            ParseOutcome::SessionMismatch => {
                warnings.push(MessageLoadWarning {
                    corruption: MessageCorruption::SessionIdMismatch { line_number },
                });
                partially_corrupt = true;
            }
            ParseOutcome::BadJson => {
                if Some(idx) == last_non_empty {
                    recovered_line = Some(line.clone());
                    truncate_at = Some(idx);
                    partially_corrupt = true;
                } else {
                    warnings.push(MessageLoadWarning {
                        corruption: MessageCorruption::MidFileCorrupt { line_number },
                    });
                    partially_corrupt = true;
                }
            }
        }
    }

    if let (Some(idx), Some(raw)) = (truncate_at, recovered_line) {
        let recovery = recover_trailing_corrupt(&path, &raw)?;
        rewrite_without_line(&path, &lines, idx)?;
        warnings.push(MessageLoadWarning {
            corruption: MessageCorruption::TrailingCorruptRecovered {
                recovery_path: recovery,
            },
        });
    }

    messages.sort_by_key(|m| m.sequence);

    Ok(LoadMessagesResult {
        next_sequence: max_seq.saturating_add(1).max(1),
        messages,
        warnings,
        partially_corrupt,
    })
}

enum ParseOutcome {
    Ok(Box<MessageRecord>),
    BadJson,
    SessionMismatch,
}

fn try_parse_message(line: &str, session_id: Uuid) -> ParseOutcome {
    let msg: MessageRecord = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(_) => return ParseOutcome::BadJson,
    };
    if check_schema_version(msg.schema_version).is_err() {
        return ParseOutcome::BadJson;
    }
    if msg.session_id != session_id {
        return ParseOutcome::SessionMismatch;
    }
    ParseOutcome::Ok(Box::new(msg))
}

fn recover_trailing_corrupt(messages_path: &Path, raw_line: &str) -> StorageResult<PathBuf> {
    let dir = messages_path.parent().unwrap_or_else(|| Path::new("."));
    let ts = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
        .replace(':', "");
    let recovery = dir.join(format!("messages.recovery.{ts}.jsonl"));
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&recovery)
        .map_err(|e| StorageError::io(&recovery, e))?;
    f.write_all(raw_line.as_bytes())
        .map_err(|e| StorageError::io(&recovery, e))?;
    f.write_all(b"\n")
        .map_err(|e| StorageError::io(&recovery, e))?;
    f.sync_all().map_err(|e| StorageError::io(&recovery, e))?;
    Ok(recovery)
}

fn rewrite_without_line(path: &Path, lines: &[String], drop_idx: usize) -> StorageResult<()> {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        if idx == drop_idx {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    let tmp = path.with_extension(format!("rewrite-{}", Uuid::now_v7()));
    fs::write(&tmp, out.as_bytes()).map_err(|e| StorageError::io(&tmp, e))?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            if path.exists() {
                let bak = path.with_extension("bak-rewrite");
                fs::rename(path, &bak).map_err(|e2| StorageError::io(path, e2))?;
                if let Err(e2) = fs::rename(&tmp, path) {
                    let _ = fs::rename(&bak, path);
                    return Err(StorageError::io(path, e2));
                }
                let _ = fs::remove_file(&bak);
                Ok(())
            } else {
                Err(StorageError::io(path, e))
            }
        }
    }
}
