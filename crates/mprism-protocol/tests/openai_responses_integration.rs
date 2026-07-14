//! Integration tests against a local mock OpenAI Responses server.

use futures_util::StreamExt;
use mprism_protocol::{
    ChatMessage, ChatRequest, ChatRole, OpenAiResponsesAdapter, ProtocolAdapter, ProtocolErrorKind,
    ProtocolKind, ProviderEndpoint, StreamEvent,
};
use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn endpoint(base: &str, key: &str) -> ProviderEndpoint {
    ProviderEndpoint::new(ProtocolKind::OpenAiResponses, base, key).unwrap()
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

    let adapter = OpenAiResponsesAdapter::new().unwrap();
    assert_eq!(adapter.kind(), ProtocolKind::OpenAiResponses);
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "alpha");
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

    let adapter = OpenAiResponsesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1/", server.uri()), "");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models[0].id, "local-model");
}

#[tokio::test]
async fn list_models_401_redacts_secret() {
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

    let adapter = OpenAiResponsesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-leaked-secret");
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Authentication);
    assert!(!err.message.contains("sk-leaked-secret"));
}

#[tokio::test]
async fn stream_chat_semantic_sse_and_store_false() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"think \"}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"ignored\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"total_tokens\":5}}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header_exists("Authorization"))
        .and(body_partial_json(serde_json::json!({
            "stream": true,
            "store": false,
            "model": "m"
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiResponsesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![
            ChatMessage {
                role: ChatRole::System,
                content: "sys".into(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            },
        ],
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
                assert_eq!(u.prompt_tokens, Some(3));
                assert_eq!(u.completion_tokens, Some(2));
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
async fn stream_chat_handles_crlf_and_unknown_events() {
    let server = MockServer::start().await;
    let body = "data: {\"type\":\"response.queued\",\"response\":{}}\r\n\r\n: keep-alive\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"A\"}\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"B\"}\r\n\r\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\r\n\r\n";
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiResponsesAdapter::new().unwrap();
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
    let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiResponsesAdapter::new().unwrap();
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
async fn stream_chat_inline_error_event() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n",
        "data: {\"type\":\"error\",\"code\":\"server_error\",\"message\":\"boom\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiResponsesAdapter::new().unwrap();
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
    let mut saw_err = false;
    while let Some(item) = stream.next().await {
        if let Err(err) = item {
            assert_eq!(err.kind, ProtocolErrorKind::ProviderUnavailable);
            assert_eq!(err.provider_code.as_deref(), Some("server_error"));
            saw_err = true;
        }
    }
    assert!(saw_err);
}

#[tokio::test]
async fn stream_chat_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"message": "Rate limit reached", "code": "rate_limit_exceeded"}
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiResponsesAdapter::new().unwrap();
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
async fn rejects_wrong_protocol_kind() {
    let adapter = OpenAiResponsesAdapter::new().unwrap();
    let ep = ProviderEndpoint::new(
        ProtocolKind::OpenAiChatCompletions,
        "https://api.example.com/v1",
        "k",
    )
    .unwrap();
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Unsupported);
}
