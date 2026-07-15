//! Integration tests against a local mock Anthropic Messages server.

use futures_util::StreamExt;
use mprism_protocol::{
    AnthropicMessagesAdapter, ChatMessage, ChatRequest, ChatRole, ProtocolAdapter,
    ProtocolErrorKind, ProtocolKind, ProviderEndpoint, StreamEvent,
};
use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn endpoint(base: &str, key: &str) -> ProviderEndpoint {
    ProviderEndpoint::new(ProtocolKind::AnthropicMessages, base, key).unwrap()
}

#[tokio::test]
async fn list_models_success_and_x_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-api-key", "sk-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "alpha", "display_name": "Alpha"},
                {"id": "alpha"},
                {"id": "beta"}
            ]
        })))
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    assert_eq!(adapter.kind(), ProtocolKind::AnthropicMessages);
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let models = adapter.list_models(&ep).await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "alpha");
    assert_eq!(models[0].display_name, "Alpha");
    assert_eq!(models[1].id, "beta");
}

#[tokio::test]
async fn list_models_without_x_api_key_when_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "local-model"}]
        })))
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
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
            "type": "error",
            "error": {
                "type": "authentication_error",
                "message": "Invalid API key: sk-leaked-secret"
            }
        })))
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-leaked-secret");
    let err = adapter.list_models(&ep).await.unwrap_err();
    assert_eq!(err.kind, ProtocolErrorKind::Authentication);
    assert_eq!(err.provider_code.as_deref(), Some("authentication_error"));
    assert!(!err.message.contains("sk-leaked-secret"));
}

#[tokio::test]
async fn stream_chat_happy_path_and_request_shape() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"think \"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header_exists("x-api-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(serde_json::json!({
            "stream": true,
            "model": "m",
            "max_tokens": 64,
            "system": "sys"
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "sk-test");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![
            ChatMessage::text(ChatRole::System, "sys"),
            ChatMessage::text(ChatRole::User, "hi"),
        ],
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
                assert_eq!(u.prompt_tokens, Some(3));
                assert_eq!(u.completion_tokens, Some(2));
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
async fn stream_chat_default_max_tokens_when_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_partial_json(serde_json::json!({
            "stream": true,
            "max_tokens": 4096
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n",
                )),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
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
    let mut content = String::new();
    while let Some(item) = stream.next().await {
        if let StreamEvent::ContentDelta { text } = item.unwrap() {
            content.push_str(&text);
        }
    }
    assert_eq!(content, "ok");
}

#[tokio::test]
async fn stream_chat_handles_chunked_and_crlf() {
    let server = MockServer::start().await;
    let body = "event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"A\"}}\r\n\r\n: keep-alive\r\n\r\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"B\"}}\r\n\r\ndata: {\"type\":\"message_stop\"}\r\n\r\n";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: Some(16),
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
    let body =
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: Some(16),
        reasoning: None,
        tools: None,
        tool_choice: None,
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
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("request-id", "req_abc")
                .set_body_json(serde_json::json!({
                    "type": "error",
                    "error": {
                        "type": "rate_limit_error",
                        "message": "rate limited"
                    }
                })),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: Some(16),
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let result = adapter.stream_chat(&ep, request).await;
    let err = match result {
        Ok(_) => panic!("expected HTTP error"),
        Err(err) => err,
    };
    assert_eq!(err.kind, ProtocolErrorKind::RateLimited);
    assert_eq!(err.provider_code.as_deref(), Some("rate_limit_error"));
    assert_eq!(err.request_id.as_deref(), Some("req_abc"));
    assert!(err.retryable);
}

#[tokio::test]
async fn stream_chat_finish_reasons() {
    for (reason, expected) in [
        ("end_turn", mprism_protocol::FinishReason::Stop),
        ("max_tokens", mprism_protocol::FinishReason::Length),
        ("tool_use", mprism_protocol::FinishReason::ToolCalls),
    ] {
        let server = MockServer::start().await;
        let body = format!(
            "data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{reason}\"}}}}\n\ndata: {{\"type\":\"message_stop\"}}\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let adapter = AnthropicMessagesAdapter::new().unwrap();
        let ep = endpoint(&format!("{}/v1", server.uri()), "k");
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: Some(16),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
        let mut finished = None;
        let mut n = 0usize;
        while let Some(item) = stream.next().await {
            if let StreamEvent::Completed { finish_reason } = item.unwrap() {
                n += 1;
                finished = Some(finish_reason);
            }
        }
        assert_eq!(n, 1);
        assert_eq!(finished, Some(expected));
    }
}

#[tokio::test]
async fn stream_chat_drop_without_completed() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicMessagesAdapter::new().unwrap();
    let ep = endpoint(&format!("{}/v1", server.uri()), "k");
    let request = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: Some(16),
        reasoning: None,
        tools: None,
        tool_choice: None,
    };
    let mut stream = adapter.stream_chat(&ep, request).await.unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, StreamEvent::ContentDelta { .. }));
    drop(stream);
}
