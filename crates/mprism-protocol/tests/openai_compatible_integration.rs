//! Integration tests against a local mock OpenAI-compatible server.

use futures_util::StreamExt;
use mprism_protocol::{
    AuthOptions, ChatMessage, ChatRequest, ChatRole, OpenAiCompatibleAdapter, ProtocolAdapter,
    ProtocolErrorKind, ProtocolKind, ProviderEndpoint, StreamEvent,
};
use wiremock::matchers::{header, header_exists, method, path, query_param};
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: Some(0.2),
        max_tokens: Some(64),
        reasoning: None,
        tools: None,
        tool_choice: None,
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
                assert_eq!(finish_reason, mprism_protocol::FinishReason::Stop);
            }
            StreamEvent::ToolCallDelta { .. } | StreamEvent::ToolCallFinished { .. } => {}
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
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
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "7")
                .insert_header("x-request-id", "req_rate_1")
                .set_body_json(serde_json::json!({
                    "error": {"message": "Rate limit reached", "code": "rate_limit_exceeded"}
                })),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let err = match adapter.stream_chat(&ep, request).await {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ProtocolErrorKind::RateLimited);
    assert!(err.retryable);
    assert_eq!(err.http_status, Some(429));
    assert_eq!(err.retry_after_ms, Some(7_000));
    assert_eq!(err.request_id.as_deref(), Some("req_rate_1"));
}

#[tokio::test]
async fn stream_chat_context_length_exceeded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "This model's maximum context length is 8192 tokens",
                "code": "context_length_exceeded"
            }
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let err = match adapter.stream_chat(&ep, request).await {
        Ok(_) => panic!("expected error"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ProtocolErrorKind::ContextLengthExceeded);
    assert!(!err.retryable);
}

#[tokio::test]
async fn list_models_sends_extra_headers_and_api_key_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("Authorization", "Bearer sk-test"))
        .and(header("X-Custom-Client", "mprism"))
        .and(query_param("api_key", "sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [{"id": "m1"}]
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    let mut ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    ep.auth = AuthOptions {
        extra_headers: vec![("X-Custom-Client".into(), "mprism".into())],
        api_key_query_param: Some("api_key".into()),
    };
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models[0].id, "m1");
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
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

#[tokio::test]
async fn stream_chat_finish_reason_length() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"x\"},\"finish_reason\":\"length\"}]}\n\n",
        "data: [DONE]\n\n",
    );
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut finished = None;
    while let Some(item) = stream.next().await {
        if let StreamEvent::Completed { finish_reason } = item.unwrap() {
            finished = Some(finish_reason);
        }
    }
    assert_eq!(finished, Some(mprism_protocol::FinishReason::Length));
}

#[tokio::test]
async fn stream_chat_finish_reason_content_filter_and_tool_calls() {
    for (raw, expected) in [
        (
            "content_filter",
            mprism_protocol::FinishReason::ContentFilter,
        ),
        ("tool_calls", mprism_protocol::FinishReason::ToolCalls),
    ] {
        let server = MockServer::start().await;
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{raw}\"}}]}}\n\ndata: [DONE]\n\n"
        );
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
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
        let mut finished = None;
        let mut completed_count = 0usize;
        while let Some(item) = stream.next().await {
            if let StreamEvent::Completed { finish_reason } = item.unwrap() {
                completed_count += 1;
                finished = Some(finish_reason);
            }
        }
        assert_eq!(completed_count, 1);
        assert_eq!(finished, Some(expected));
    }
}

#[tokio::test]
async fn stream_chat_sends_stream_options_include_usage() {
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(serde_json::json!({
            "stream": true,
            "stream_options": { "include_usage": true }
        })))
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let mut completed = false;
    while let Some(item) = stream.next().await {
        if matches!(item.unwrap(), StreamEvent::Completed { .. }) {
            completed = true;
        }
    }
    assert!(completed);
}

#[tokio::test]
async fn stream_chat_idle_timeout() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Headers OK, then body stalls: idle timeout applies to stream bytes (§5).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 8192];
        let _ = socket.read(&mut buf).await;
        let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
        let _ = socket.write_all(headers).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let adapter =
        OpenAiCompatibleAdapter::with_stream_idle_timeout(Duration::from_millis(300)).unwrap();
    let ep = endpoint(&format!("http://{addr}/v1"), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let item = stream.next().await.expect("timeout event");
    let err = item.expect_err("expected timeout error");
    assert_eq!(err.kind, ProtocolErrorKind::Timeout);
    assert!(err.retryable);
}

#[tokio::test]
async fn stream_chat_drop_without_completed() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        // No [DONE]; client will drop after first delta.
        "data: {\"choices\":[{\"delta\":{\"content\":\"more\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
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
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, StreamEvent::ContentDelta { .. }));
    // Drop stream: must not invent Completed{Stop}.
    drop(stream);
}
