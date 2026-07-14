//! Integration tests against a local mock OpenAI-compatible server.

use futures_util::StreamExt;
use mprism_protocol::{
    ChatMessage, ChatRequest, ChatRole, OpenAiCompatibleAdapter, ProtocolAdapter,
    ProtocolErrorKind, ProtocolKind, ProviderEndpoint, StreamEvent,
};
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn endpoint(base: &str, key: &str) -> ProviderEndpoint {
    ProviderEndpoint::new(ProtocolKind::OpenAiChatCompletions, base, key).unwrap()
}

#[tokio::test]
async fn list_models_success_and_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("Authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [
                {"id": "alpha", "owned_by": "org"},
                {"id": "alpha"},
                {"id": "beta"}
            ]
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "alpha");
    assert_eq!(models[0].display_name, "alpha");
    assert_eq!(models[0].owned_by.as_deref(), Some("org"));
    assert_eq!(models[1].id, "beta");
}

#[tokio::test]
async fn list_models_without_authorization_when_key_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [{"id": "local-model"}]
        })))
        .mount(&server)
        .await;

    // Ensure request is accepted without requiring Authorization header.
    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1/", server.uri()), "");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models[0].id, "local-model");
}

#[tokio::test]
async fn list_models_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": {
                "message": "Incorrect API key provided: sk-leaked-secret",
                "code": "invalid_api_key"
            }
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-leaked-secret");
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Authentication);
    assert!(!err.retryable);
    assert!(!format!("{err:?}").contains("sk-leaked-secret"));
    assert!(!err.message.contains("sk-leaked-secret"));
}

#[tokio::test]
async fn list_models_missing_data_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list"
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Decode);
}

#[tokio::test]
async fn stream_chat_content_reasoning_usage_and_done() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header_exists("Authorization"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: Some(0.2),
        max_tokens: Some(64),
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut completed = false;
    let mut saw_usage = false;
    while let Some(item) = stream.next().await {
        match item.unwrap() {
            StreamEvent::ContentDelta { text } => content.push_str(&text),
            StreamEvent::ReasoningDelta { text } => reasoning.push_str(&text),
            StreamEvent::Usage(u) => {
                saw_usage = true;
                assert_eq!(u.total_tokens, Some(5));
            }
            StreamEvent::Completed { finish_reason } => {
                completed = true;
                assert_eq!(finish_reason.as_deref(), Some("stop"));
            }
        }
    }
    assert_eq!(content, "Hello");
    assert_eq!(reasoning, "think ");
    assert!(saw_usage);
    assert!(completed);
}

#[tokio::test]
async fn stream_chat_handles_chunked_and_crlf() {
    let server = MockServer::start().await;
    // Single response body still exercises parser path with CRLF separators.
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n\r\n: keep-alive\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"B\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut content = String::new();
    while let Some(item) = stream.next().await {
        if let StreamEvent::ContentDelta { text } = item.unwrap() {
            content.push_str(&text);
        }
    }
    assert_eq!(content, "AB");
}

#[tokio::test]
async fn stream_chat_unexpected_eof() {
    let server = MockServer::start().await;
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut saw_content = false;
    let mut saw_eof = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::ContentDelta { .. }) => saw_content = true,
            Err(err) => {
                assert_eq!(err.kind, ProtocolErrorKind::UnexpectedEof);
                saw_eof = true;
            }
            _ => {}
        }
    }
    assert!(saw_content);
    assert!(saw_eof);
}

#[tokio::test]
async fn stream_chat_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"message": "Rate limit reached", "code": "rate_limit_exceeded"}
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: None,
    };
    let err = match adapter.stream_chat(&ep, request).await {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ProtocolErrorKind::RateLimited);
    assert!(err.retryable);
    assert_eq!(err.http_status, Some(429));
}

#[tokio::test]
async fn stream_chat_provider_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: None,
    };
    let err = match adapter.stream_chat(&ep, request).await {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ProtocolErrorKind::ProviderUnavailable);
    assert!(err.retryable);
}

#[tokio::test]
async fn rejects_base_url_with_query() {
    let err = ProviderEndpoint::new(
        ProtocolKind::OpenAiChatCompletions,
        "https://api.example.com/v1?api_key=x",
        "k",
    )
    .unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::InvalidConfiguration);
}
