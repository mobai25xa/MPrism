//! Integration tests against a local mock Gemini generateContent server.

use futures_util::StreamExt;
use mprism_protocol::{
    ChatMessage, ChatRequest, ChatRole, GeminiGenerateContentAdapter, ProtocolAdapter,
    ProtocolErrorKind, ProtocolKind, ProviderEndpoint, StreamEvent,
};
use wiremock::matchers::{body_partial_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn endpoint(base: &str, key: &str) -> ProviderEndpoint {
    ProviderEndpoint::new(ProtocolKind::GeminiGenerateContent, base, key).unwrap()
}

#[tokio::test]
async fn list_models_success_and_x_goog_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .and(header("x-goog-api-key", "AIza-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [
                {"name": "models/alpha", "displayName": "Alpha"},
                {"name": "models/alpha"},
                {"name": "models/beta"}
            ]
        })))
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    assert_eq!(adapter.kind(), ProtocolKind::GeminiGenerateContent);
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "AIza-test");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "alpha");
    assert_eq!(models[0].display_name, "Alpha");
    assert_eq!(models[1].id, "beta");
}

#[tokio::test]
async fn list_models_without_key_when_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [{"name": "models/local"}]
        })))
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta/", server.uri()), "");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models[0].id, "local");
}

#[tokio::test]
async fn list_models_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": {
                "code": 401,
                "message": "API key not valid: AIza-leaked-secret",
                "status": "UNAUTHENTICATED"
            }
        })))
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "AIza-leaked-secret");
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Authentication);
    assert_eq!(err.provider_code.as_deref(), Some("UNAUTHENTICATED"));
}

#[tokio::test]
async fn stream_chat_happy_path_and_request_shape() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"think \",\"thought\":true}]},\"index\":0}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]},\"index\":0}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo\"}]},\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2,\"totalTokenCount\":5}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-flash:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .and(header_exists("x-goog-api-key"))
        .and(body_partial_json(serde_json::json!({
            "systemInstruction": {
                "parts": [{"text": "sys"}]
            },
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]}
            ],
            "generationConfig": {
                "temperature": 0.2,
                "maxOutputTokens": 64
            }
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "AIza-test");
    let request = ChatRequest {
        model: "gemini-flash".into(),
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
                assert_eq!(u.total_tokens, Some(5));
            }
            StreamEvent::Completed { finish_reason } => {
                completed = true;
                assert_eq!(finish_reason.as_deref(), Some("STOP"));
            }
        }
    }
    assert_eq!(content, "Hello");
    assert_eq!(reasoning, "think ");
    assert!(saw_usage);
    assert!(completed);
}

#[tokio::test]
async fn stream_chat_handles_crlf() {
    let server = MockServer::start().await;
    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"A\"}]},\"index\":0}]}\r\n\r\n: keep-alive\r\n\r\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"B\"}]},\"finishReason\":\"STOP\",\"index\":0}]}\r\n\r\n";
    Mock::given(method("POST"))
        .and(path("/v1beta/models/m:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "");
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
    let body =
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"partial\"}]},\"index\":0}]}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1beta/models/m:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: Some(16),
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut saw_eof = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::ContentDelta { .. }) => {}
            Err(err) => {
                assert_eq!(err.kind, ProtocolErrorKind::UnexpectedEof);
                saw_eof = true;
            }
            Ok(other) => panic!("unexpected {other:?}"),
        }
    }
    assert!(saw_eof);
}

#[tokio::test]
async fn stream_chat_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/m:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("x-goog-request-id", "req_g")
                .set_body_json(serde_json::json!({
                    "error": {
                        "code": 429,
                        "message": "Resource exhausted",
                        "status": "RESOURCE_EXHAUSTED"
                    }
                })),
        )
        .mount(&server)
        .await;

    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1beta", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: Some(16),
    };
    let result = adapter.stream_chat(&ep, request).await;
    let err = match result {
        Ok(_) => panic!("expected HTTP error"),
        Err(err) => err,
    };
    assert_eq!(err.kind, ProtocolErrorKind::RateLimited);
    assert_eq!(err.provider_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
    assert_eq!(err.request_id.as_deref(), Some("req_g"));
    assert!(err.retryable);
}
