//! Integration tests for FileStore (always use temp directories).

use std::fs;

use mprism_desktop_lib::storage::{
    redact_log_message, ApiKeyUpdate, AssistantStatus, FileStore, MessageCorruption, MessageRecord,
    ModelRecord, ModelSnapshot, ModelSource, ProviderSnapshot, ProviderUpsert, SessionUpdate,
    StoredProtocol, ThemePreference, TitleSource, DEFAULT_SESSION_TITLE,
};
use uuid::Uuid;

fn temp_store() -> (tempfile::TempDir, FileStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::open(dir.path()).expect("open store");
    (dir, store)
}

#[test]
fn opens_layout_and_stable_device() {
    let (dir, store) = temp_store();
    let id1 = store.device_id();
    drop(store);
    let store2 = FileStore::open(dir.path()).unwrap();
    assert_eq!(store2.device_id(), id1);
    assert!(dir.path().join("settings.json").exists());
    assert!(dir.path().join("device.json").exists());
    assert!(dir.path().join("sessions").is_dir());
    assert!(dir.path().join("logs").is_dir());
}

#[test]
fn settings_theme_and_provider_upsert_key_states() {
    let (dir, store) = temp_store();
    let doc = store.set_theme(ThemePreference::Dark).unwrap();
    assert_eq!(doc.theme, ThemePreference::Dark);

    let model = ModelRecord {
        id: "gpt-example".into(),
        display_name: "GPT Example".into(),
        source: ModelSource::Discovery,
        temperature: Some(0.7),
        max_tokens: Some(1024),
    };

    let (doc, pubp) = store
        .upsert_provider(ProviderUpsert {
            id: None,
            name: "  My Provider  ".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://api.example.com/v1/".into(),
            api_key: ApiKeyUpdate::Replace("sk-secret-real-key-123".into()),
            models: vec![model.clone()],
        })
        .unwrap();

    assert_eq!(pubp.name, "My Provider");
    assert!(pubp.api_key_present);
    assert_eq!(pubp.base_url, "https://api.example.com/v1");
    assert_eq!(doc.default_provider_id, Some(pubp.id));
    assert_eq!(doc.default_model_id.as_deref(), Some("gpt-example"));

    let (_doc2, pub2) = store
        .upsert_provider(ProviderUpsert {
            id: Some(pubp.id),
            name: "Renamed".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://api.example.com/v1".into(),
            api_key: ApiKeyUpdate::Keep,
            models: vec![model.clone()],
        })
        .unwrap();
    assert!(pub2.api_key_present);
    let raw = fs::read_to_string(dir.path().join("settings.json")).unwrap();
    assert!(raw.contains("sk-secret-real-key-123"));

    let (_, pub3) = store
        .upsert_provider(ProviderUpsert {
            id: Some(pubp.id),
            name: "Renamed".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://api.example.com/v1".into(),
            api_key: ApiKeyUpdate::Clear,
            models: vec![model],
        })
        .unwrap();
    assert!(!pub3.api_key_present);
}

#[test]
fn provider_debug_and_public_never_show_key() {
    let (_dir, store) = temp_store();
    let (_, pubp) = store
        .upsert_provider(ProviderUpsert {
            id: None,
            name: "P".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "http://127.0.0.1:8080/v1".into(),
            api_key: ApiKeyUpdate::Replace("sk-should-not-appear".into()),
            models: vec![ModelRecord {
                id: "m".into(),
                display_name: "M".into(),
                source: ModelSource::Manual,
                temperature: None,
                max_tokens: None,
            }],
        })
        .unwrap();
    let dbg = format!("{pubp:?}");
    assert!(!dbg.contains("sk-should-not-appear"));
    let settings = store.load_settings().unwrap();
    let dbg2 = format!("{settings:?}");
    assert!(!dbg2.contains("sk-should-not-appear"));
}

#[test]
fn delete_provider_repairs_defaults() {
    let (_dir, store) = temp_store();
    let model = ModelRecord {
        id: "m1".into(),
        display_name: "M1".into(),
        source: ModelSource::Manual,
        temperature: None,
        max_tokens: None,
    };
    let (_, a) = store
        .upsert_provider(ProviderUpsert {
            id: None,
            name: "A".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://a.example/v1".into(),
            api_key: ApiKeyUpdate::Clear,
            models: vec![model],
        })
        .unwrap();
    let (_, b) = store
        .upsert_provider(ProviderUpsert {
            id: None,
            name: "B".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://b.example/v1".into(),
            api_key: ApiKeyUpdate::Clear,
            models: vec![ModelRecord {
                id: "m2".into(),
                display_name: "M2".into(),
                source: ModelSource::Manual,
                temperature: None,
                max_tokens: None,
            }],
        })
        .unwrap();
    let doc = store.delete_provider(a.id).unwrap();
    assert_eq!(doc.providers.len(), 1);
    assert_eq!(doc.default_provider_id, Some(b.id));
    assert_eq!(doc.default_model_id.as_deref(), Some("m2"));
    let doc = store.delete_provider(b.id).unwrap();
    assert!(doc.default_provider_id.is_none());
    assert!(doc.default_model_id.is_none());
}

#[test]
fn future_settings_schema_is_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::open(dir.path()).unwrap();
    drop(store);
    let path = dir.path().join("settings.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["schema_version"] = serde_json::json!(99);
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let err = match FileStore::open(dir.path()) {
        Ok(_) => panic!("expected schema too new error"),
        Err(e) => e,
    };
    assert!(err.is_blocking());
}

#[test]
fn sessions_crud_auto_title_soft_delete_sort() {
    let (dir, store) = temp_store();
    let s1 = store.create_session(None).unwrap();
    assert_eq!(s1.title, DEFAULT_SESSION_TITLE);
    assert_eq!(s1.title_source, TitleSource::Default);

    let s2 = store.create_session(Some("手工标题".into())).unwrap();
    assert_eq!(s2.title_source, TitleSource::User);

    let msg =
        MessageRecord::new_user(s1.id, 0, "如何设计 Rust trait 系统", store.device_id()).unwrap();
    let msg = store.append_message_and_touch(msg, true).unwrap();
    assert_eq!(msg.sequence, 1);
    let meta = store.load_session_meta(s1.id).unwrap();
    assert_eq!(meta.title_source, TitleSource::Auto);
    assert!(meta.title.starts_with("如何设计"));
    assert!(meta.title.chars().count() <= 30);

    store
        .update_session(
            s1.id,
            SessionUpdate {
                title: Some("固定标题".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let msg2 =
        MessageRecord::new_user(s1.id, 0, "第二条消息不应改标题", store.device_id()).unwrap();
    store.append_message_and_touch(msg2, true).unwrap();
    let meta = store.load_session_meta(s1.id).unwrap();
    assert_eq!(meta.title, "固定标题");
    assert_eq!(meta.title_source, TitleSource::User);

    store.delete_session(s2.id).unwrap();
    let list = store.list_sessions().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, s1.id);
    assert!(dir.path().join("sessions").join(s2.id.to_string()).exists());
}

#[test]
fn messages_unicode_append_and_load() {
    let (_dir, store) = temp_store();
    let session = store.create_session(None).unwrap();
    let content = "你好 🌍 — async/await";
    let msg = MessageRecord::new_user(session.id, 0, content, store.device_id()).unwrap();
    store.append_message_and_touch(msg, true).unwrap();

    let assistant = MessageRecord::new_assistant(
        session.id,
        0,
        "回复内容",
        Some("思考过程".into()),
        AssistantStatus::Completed,
        Uuid::now_v7(),
        ProviderSnapshot {
            id: Uuid::now_v7(),
            name: "P".into(),
        },
        ModelSnapshot {
            id: "m".into(),
            display_name: "M".into(),
        },
        None,
        Some("stop".into()),
        None,
        store.device_id(),
    );
    store.append_message_and_touch(assistant, false).unwrap();

    let loaded = store.load_messages(session.id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, content);
    assert_eq!(loaded.messages[1].status, Some(AssistantStatus::Completed));
    assert_eq!(loaded.next_sequence, 3);
    assert!(!loaded.partially_corrupt);
}

#[test]
fn messages_mid_corrupt_and_duplicate_sequence() {
    let (dir, store) = temp_store();
    let session = store.create_session(None).unwrap();
    let m1 = MessageRecord::new_user(session.id, 1, "one", store.device_id()).unwrap();
    store.append_message_and_touch(m1.clone(), true).unwrap();
    let m2 = MessageRecord::new_user(session.id, 2, "two", store.device_id()).unwrap();
    store.append_message_and_touch(m2, false).unwrap();

    let path = dir
        .path()
        .join("sessions")
        .join(session.id.to_string())
        .join("messages.jsonl");
    let body = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    let mut new_body = format!("{}\nNOT-JSON\n{}\n", lines[0], lines[1]);
    new_body.push_str(&serde_json::to_string(&m1).unwrap());
    new_body.push('\n');
    fs::write(&path, new_body).unwrap();

    let loaded = store.load_messages(session.id).unwrap();
    assert!(loaded.partially_corrupt);
    assert_eq!(loaded.messages.len(), 2);
    assert!(loaded.warnings.len() >= 2);
}

#[test]
fn messages_trailing_corrupt_recovered() {
    let (dir, store) = temp_store();
    let session = store.create_session(None).unwrap();
    let m1 = MessageRecord::new_user(session.id, 1, "ok", store.device_id()).unwrap();
    store.append_message_and_touch(m1, true).unwrap();
    let path = dir
        .path()
        .join("sessions")
        .join(session.id.to_string())
        .join("messages.jsonl");
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str("{broken\n");
    fs::write(&path, body).unwrap();

    let loaded = store.load_messages(session.id).unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert!(loaded.partially_corrupt);
    assert!(loaded.warnings.iter().any(|w| matches!(
        w.corruption,
        MessageCorruption::TrailingCorruptRecovered { .. }
    )));
    let sess_dir = dir.path().join("sessions").join(session.id.to_string());
    let recovery = fs::read_dir(sess_dir).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with("messages.recovery.")
    });
    assert!(recovery);
    let reloaded = store.load_messages(session.id).unwrap();
    assert_eq!(reloaded.messages.len(), 1);
}

#[test]
fn rejects_base_url_with_query() {
    let (_dir, store) = temp_store();
    let err = store
        .upsert_provider(ProviderUpsert {
            id: None,
            name: "X".into(),
            protocol: StoredProtocol::OpenAiChatCompletions,
            base_url: "https://api.example.com/v1?x=1".into(),
            api_key: ApiKeyUpdate::Clear,
            models: vec![],
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("query") || msg.contains("校验") || msg.contains("Base URL"));
}

#[test]
fn log_redaction_strips_keys() {
    let s = redact_log_message("auth Bearer sk-abc123DEF and sk-other_key-99 failed");
    assert!(!s.contains("sk-abc123DEF"));
    assert!(!s.contains("sk-other_key-99"));
    assert!(s.contains("***"));
}

#[test]
fn corrupt_session_does_not_block_others() {
    let (dir, store) = temp_store();
    let good = store.create_session(Some("good".into())).unwrap();
    let bad_id = Uuid::now_v7();
    let bad_dir = dir.path().join("sessions").join(bad_id.to_string());
    fs::create_dir_all(&bad_dir).unwrap();
    fs::write(bad_dir.join("meta.json"), "{not json").unwrap();
    let list = store.list_sessions().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, good.id);
}

#[test]
fn never_uses_real_user_mprism_in_tests() {
    let (dir, store) = temp_store();
    let root = store.root().to_path_buf();
    let home_mprism = dirs::home_dir().unwrap().join(".mprism");
    assert_ne!(root, home_mprism);
    assert!(root.starts_with(dir.path()) || root == dir.path());
}
