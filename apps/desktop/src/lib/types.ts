export const IPC_SCHEMA_VERSION = 1;

export type ThemePreference = "system" | "light" | "dark";
export type AppPage = "chat" | "settings";
export type ProtocolId =
  | "openai_chat_completions"
  | "openai_responses"
  | "anthropic_messages"
  | "gemini_generate_content";
export type ModelSource = "discovery" | "manual";
export type MessageRole = "user" | "assistant";
export type AssistantStatus = "completed" | "stopped" | "error";
export type TitleSource = "default" | "auto" | "user";

export type AppError = {
  code: string;
  message: string;
  retryable: boolean;
  http_status?: number | null;
  provider_request_id?: string | null;
};

export type ReasoningModeId = "auto" | "off" | "on";
export type ReasoningEffortId =
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max";

export type StoredReasoningSettings = {
  /** auto | off | on — omit / auto ≡ no request control */
  mode: ReasoningModeId | string;
  effort?: ReasoningEffortId | string | null;
  budget_tokens?: number | null;
};

export type ModelRecord = {
  id: string;
  display_name: string;
  source: ModelSource;
  temperature: number | null;
  max_tokens: number | null;
  /** Model-level request reasoning; null/undefined ≡ auto */
  reasoning?: StoredReasoningSettings | null;
};

/** Provider-level tool definition (wire passthrough; app does not execute). */
export type StoredToolDefinition = {
  name: string;
  description?: string | null;
  /** JSON Schema object */
  parameters: Record<string, unknown> | object;
};

export type ToolChoiceModeId = "auto" | "none" | "required" | "named";

export type StoredToolChoice = {
  /** auto | none | required | named */
  mode: ToolChoiceModeId | string;
  name?: string | null;
};

export type StoredExtraHeader = {
  name: string;
  value: string;
};

export type ProviderPublic = {
  id: string;
  name: string;
  protocol: ProtocolId | string;
  base_url: string;
  api_key_present: boolean;
  models: ModelRecord[];
  /** Provider-level tools; omit/empty ≡ V1 no tools wire */
  tools?: StoredToolDefinition[];
  tool_choice?: StoredToolChoice | null;
  /** AuthOptions (3.7); omit/empty ≡ V1 */
  extra_headers?: StoredExtraHeader[];
  api_key_query_param?: string | null;
  created_at: string;
  updated_at: string;
  revision: number;
};

export type SessionMeta = {
  schema_version: number;
  id: string;
  title: string;
  title_source: TitleSource;
  system_prompt: string;
  last_provider_id: string | null;
  last_model_id: string | null;
  created_by_device_id: string;
  created_at: string;
  updated_at: string;
  revision: number;
  deleted_at: string | null;
};

export type ProviderSnapshot = {
  id: string;
  name: string;
};

export type ModelSnapshot = {
  id: string;
  display_name: string;
};

export type TokenUsageRecord = {
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  reasoning_tokens?: number | null;
  cached_tokens?: number | null;
};

/** Mirrors mprism_protocol ProtocolCapabilities for UI gating. */
export type ProtocolCapabilities = {
  protocol: ProtocolId | string;
  streaming: boolean;
  list_models: boolean;
  reasoning_output: boolean;
  reasoning_control: boolean;
  tools: boolean;
  vision_input: boolean;
  stream_usage: boolean;
  custom_headers: boolean;
  api_key_query: boolean;
};

export type ReasoningPolicyInput = {
  mode: ReasoningModeId | string;
  effort?: ReasoningEffortId | string | null;
  budget_tokens?: number | null;
};

export type ChatAttachmentInput = {
  id: string;
  media_type?: string | null;
};

export type ImportAttachmentInput = {
  schema_version: number;
  bytes: number[];
  media_type: string;
  original_name?: string | null;
};

export type AttachmentPublic = {
  id: string;
  media_type: string;
  byte_len: number;
  original_name?: string | null;
  created_at: string;
};

export type MessageAttachmentRef = {
  attachment_id: string;
  media_type?: string | null;
};

export type MessageErrorRecord = {
  code: string;
  message: string;
  retryable: boolean;
  http_status?: number | null;
  provider_request_id?: string | null;
  retry_after_ms?: number | null;
};

/** Persisted / streamed tool call (display only). */
export type StoredToolCall = {
  id: string;
  name: string;
  arguments: string;
  index?: number | null;
};

export type MessageRecord = {
  schema_version: number;
  id: string;
  session_id: string;
  sequence: number;
  role: MessageRole;
  content: string;
  reasoning?: string | null;
  status?: AssistantStatus | null;
  request_id?: string | null;
  provider?: ProviderSnapshot | null;
  model?: ModelSnapshot | null;
  usage?: TokenUsageRecord | null;
  finish_reason?: string | null;
  /** Tool calls from stream; empty/missing on legacy messages. */
  tool_calls?: StoredToolCall[];
  /** Image attachment refs (no base64 in JSONL). */
  attachments?: MessageAttachmentRef[];
  error?: MessageErrorRecord | null;
  created_by_device_id: string;
  created_at: string;
  completed_at?: string | null;
};

export type LoadedSession = {
  schema_version: number;
  meta: SessionMeta;
  messages: MessageRecord[];
  partially_corrupt: boolean;
};

export type BootstrapPayload = {
  schema_version: number;
  theme: ThemePreference;
  default_provider_id: string | null;
  default_model_id: string | null;
  providers: ProviderPublic[];
  sessions: SessionMeta[];
};

export type ApiKeyUpdateInput =
  | { type: "keep" }
  | { type: "replace"; value: string }
  | { type: "clear" };

export type ProviderInput = {
  schema_version: number;
  id: string | null;
  name: string;
  protocol: ProtocolId;
  base_url: string;
  api_key: ApiKeyUpdateInput;
  models: ModelRecord[];
  tools?: StoredToolDefinition[];
  tool_choice?: StoredToolChoice | null;
  extra_headers?: StoredExtraHeader[];
  api_key_query_param?: string | null;
};

export type ProviderDraft = {
  schema_version: number;
  provider_id?: string | null;
  protocol?: ProtocolId | null;
  base_url?: string | null;
  api_key?: ApiKeyUpdateInput | null;
};

export type ModelInfoPayload = {
  id: string;
  display_name: string;
  owned_by: string | null;
};

export type UnitPayload = {
  schema_version: number;
};

export type UpdateSessionInput = {
  schema_version: number;
  title?: string | null;
  system_prompt?: string | null;
  set_last_provider_id?: boolean;
  last_provider_id?: string | null;
  set_last_model_id?: boolean;
  last_model_id?: string | null;
};

export type ToolDefinitionInput = {
  name: string;
  description?: string | null;
  parameters: Record<string, unknown> | object;
};

export type ToolChoiceInput = {
  mode: ToolChoiceModeId | string;
  name?: string | null;
};

export type ChatInput = {
  schema_version: number;
  session_id: string;
  provider_id: string;
  model_id: string;
  content: string;
  /** Omit or mode auto → V1-compatible (no request-side reasoning control). */
  reasoning?: ReasoningPolicyInput | null;
  /** Image attachments (3.5). */
  attachments?: ChatAttachmentInput[] | null;
  /** Optional per-request tools override; omit → provider settings. */
  tools?: ToolDefinitionInput[] | null;
  tool_choice?: ToolChoiceInput | null;
};

export type StreamEventPayload =
  | { type: "started" }
  | { type: "reasoning_delta"; text: string }
  | { type: "content_delta"; text: string }
  | {
      type: "tool_call_delta";
      id?: string | null;
      name?: string | null;
      arguments_delta: string;
      index?: number | null;
    }
  | {
      type: "tool_call_finished";
      id: string;
      name: string;
      arguments: string;
      index?: number | null;
    }
  | { type: "usage"; usage: TokenUsageRecord }
  | { type: "completed"; finish_reason?: string | null }
  | { type: "stopped" }
  | { type: "error"; error: MessageErrorRecord };

export type StreamEnvelope = {
  schema_version: number;
  request_id: string;
  session_id: string;
  assistant_message_id: string;
  sequence: number;
  event: StreamEventPayload;
};

export type CancelChatPayload = {
  schema_version: number;
  was_running: boolean;
};

export type GenerationPhase = "starting" | "streaming" | "cancelling";

export type ToolCallState = {
  id?: string | null;
  name?: string | null;
  arguments: string;
  index?: number | null;
  finished: boolean;
};

export type GenerationState = {
  requestId: string;
  sessionId: string;
  assistantMessageId: string;
  nextSequence: number;
  reasoning: string;
  content: string;
  toolCalls: ToolCallState[];
  usage?: TokenUsageRecord;
  phase: GenerationPhase;
  error?: MessageErrorRecord | null;
};

export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as AppError).code === "string" &&
    typeof (value as AppError).message === "string"
  );
}

