//! Lock protocol-capability-matrix-v2.md against adapter `capabilities()`.
//!
//! Y / P → corresponding flags true (P is still implemented + advertised).
//! N → flag false, and strong requests return Unsupported.

use mprism_protocol::{
    AnthropicMessagesAdapter, ChatMessage, ChatRequest, ChatRole, ContentPart,
    GeminiGenerateContentAdapter, OpenAiCompatibleAdapter, OpenAiResponsesAdapter, ProtocolAdapter,
    ProtocolCapabilities, ProtocolErrorKind, ReasoningMode, ReasoningPolicy, ToolDefinition,
};

fn assert_caps(got: ProtocolCapabilities, expected: ProtocolCapabilities, label: &str) {
    assert_eq!(got.streaming, expected.streaming, "{label}.streaming");
    assert_eq!(got.list_models, expected.list_models, "{label}.list_models");
    assert_eq!(
        got.reasoning_output, expected.reasoning_output,
        "{label}.reasoning_output"
    );
    assert_eq!(
        got.reasoning_control, expected.reasoning_control,
        "{label}.reasoning_control"
    );
    assert_eq!(got.tools, expected.tools, "{label}.tools");
    assert_eq!(
        got.vision_input, expected.vision_input,
        "{label}.vision_input"
    );
    assert_eq!(
        got.stream_usage, expected.stream_usage,
        "{label}.stream_usage"
    );
    assert_eq!(
        got.custom_headers, expected.custom_headers,
        "{label}.custom_headers"
    );
    assert_eq!(
        got.api_key_query, expected.api_key_query,
        "{label}.api_key_query"
    );
}

#[test]
fn matrix_openai_chat_completions() {
    let adapter = OpenAiCompatibleAdapter::new().unwrap();
    assert_caps(
        adapter.capabilities(),
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true,   // P: ecosystem + usage details
            reasoning_control: false, // N
            tools: true,
            vision_input: true,
            stream_usage: true, // P: include_usage + map when present
            custom_headers: true,
            api_key_query: true,
        },
        "chat_completions",
    );

    // N → Unsupported for On/Off reasoning control
    let on = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::text(ChatRole::User, "hi")],
        temperature: None,
        max_tokens: None,
        reasoning: Some(ReasoningPolicy {
            mode: ReasoningMode::On,
            effort: None,
            budget_tokens: None,
        }),
        tools: None,
        tool_choice: None,
    };
    assert_eq!(
        on.check_capabilities(&adapter.capabilities())
            .unwrap_err()
            .kind,
        ProtocolErrorKind::Unsupported
    );
}

#[test]
fn matrix_openai_responses() {
    let adapter = OpenAiResponsesAdapter::new().unwrap();
    assert_caps(
        adapter.capabilities(),
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true, // P: summary path
            reasoning_control: true,
            tools: true,
            vision_input: true,
            stream_usage: true,
            custom_headers: true,
            api_key_query: true,
        },
        "responses",
    );
}

#[test]
fn matrix_anthropic_messages() {
    let adapter = AnthropicMessagesAdapter::new().unwrap();
    assert_caps(
        adapter.capabilities(),
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true,
            reasoning_control: true,
            tools: true,
            vision_input: true,
            stream_usage: true, // P: when usage present
            custom_headers: true,
            api_key_query: true,
        },
        "anthropic",
    );
}

#[test]
fn matrix_gemini_generate_content() {
    let adapter = GeminiGenerateContentAdapter::new().unwrap();
    assert_caps(
        adapter.capabilities(),
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true,
            reasoning_control: true,
            tools: true,
            vision_input: true,
            stream_usage: true, // P: when usageMetadata present
            custom_headers: true,
            api_key_query: true,
        },
        "gemini",
    );
}

#[test]
fn y_capabilities_accept_tools_and_vision() {
    let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![
        Box::new(OpenAiCompatibleAdapter::new().unwrap()),
        Box::new(OpenAiResponsesAdapter::new().unwrap()),
        Box::new(AnthropicMessagesAdapter::new().unwrap()),
        Box::new(GeminiGenerateContentAdapter::new().unwrap()),
    ];
    for adapter in adapters {
        let caps = adapter.capabilities();
        assert!(caps.tools, "{:?} tools", adapter.kind());
        assert!(caps.vision_input, "{:?} vision", adapter.kind());
        let with_tools = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: Some(vec![ToolDefinition {
                name: "t".into(),
                description: None,
                parameters: serde_json::json!({}),
            }]),
            tool_choice: None,
        };
        assert!(
            with_tools.check_capabilities(&caps).is_ok(),
            "{:?} tools gate",
            adapter.kind()
        );

        // Gemini ImageUrl is Unsupported at encode time; gate only checks vision_input flag.
        let with_image = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text { text: "see".into() },
                    ContentPart::ImageBase64 {
                        media_type: "image/png".into(),
                        data: "aGVsbG8=".into(),
                    },
                ],
                tool_call_id: None,
                tool_calls: vec![],
            }],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        assert!(
            with_image.check_capabilities(&caps).is_ok(),
            "{:?} vision gate",
            adapter.kind()
        );
    }
}
