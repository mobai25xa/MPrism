import type { GenerationState, MessageRecord, StreamEnvelope } from "../../lib/types";

export type StreamReduceResult =
  | { kind: "ignore" }
  | { kind: "gap" }
  | { kind: "update"; state: GenerationState }
  | { kind: "terminal"; state: GenerationState; status: "completed" | "stopped" | "error" };

export function createGenerationState(
  requestId: string,
  sessionId: string,
  assistantMessageId: string,
): GenerationState {
  return {
    requestId,
    sessionId,
    assistantMessageId,
    nextSequence: 0,
    reasoning: "",
    content: "",
    phase: "starting",
    error: null,
  };
}

export function reduceStreamEvent(
  state: GenerationState,
  envelope: StreamEnvelope,
): StreamReduceResult {
  if (
    envelope.request_id !== state.requestId ||
    envelope.session_id !== state.sessionId ||
    envelope.assistant_message_id !== state.assistantMessageId
  ) {
    return { kind: "ignore" };
  }
  if (envelope.sequence < state.nextSequence) {
    return { kind: "ignore" };
  }
  if (envelope.sequence > state.nextSequence) {
    return { kind: "gap" };
  }

  const next: GenerationState = {
    ...state,
    nextSequence: state.nextSequence + 1,
  };

  switch (envelope.event.type) {
    case "started":
      next.phase = "streaming";
      return { kind: "update", state: next };
    case "reasoning_delta":
      next.phase = "streaming";
      next.reasoning = `${state.reasoning}${envelope.event.text}`;
      return { kind: "update", state: next };
    case "content_delta":
      next.phase = "streaming";
      next.content = `${state.content}${envelope.event.text}`;
      return { kind: "update", state: next };
    case "usage":
      next.usage = envelope.event.usage;
      return { kind: "update", state: next };
    case "completed":
      return { kind: "terminal", state: next, status: "completed" };
    case "stopped":
      return { kind: "terminal", state: next, status: "stopped" };
    case "error":
      next.error = envelope.event.error;
      return { kind: "terminal", state: next, status: "error" };
    default:
      return { kind: "ignore" };
  }
}

export function optimisticUserMessage(
  sessionId: string,
  content: string,
  tempId: string,
): MessageRecord {
  return {
    schema_version: 1,
    id: tempId,
    session_id: sessionId,
    sequence: Number.MAX_SAFE_INTEGER - 1,
    role: "user",
    content,
    created_by_device_id: "local",
    created_at: new Date().toISOString(),
  };
}

export function streamingAssistantMessage(state: GenerationState): MessageRecord {
  return {
    schema_version: 1,
    id: state.assistantMessageId,
    session_id: state.sessionId,
    sequence: Number.MAX_SAFE_INTEGER,
    role: "assistant",
    content: state.content,
    reasoning: state.reasoning || null,
    status: state.phase === "cancelling" ? "stopped" : null,
    request_id: state.requestId,
    created_by_device_id: "local",
    created_at: new Date().toISOString(),
  };
}

export function shouldSendOnEnter(event: {
  key: string;
  shiftKey: boolean;
  isComposing?: boolean;
  nativeEvent?: { isComposing?: boolean };
}): boolean {
  if (event.key !== "Enter" || event.shiftKey) {
    return false;
  }
  if (event.isComposing || event.nativeEvent?.isComposing) {
    return false;
  }
  return true;
}

export function relativeTime(iso: string, nowMs = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) {
    return "";
  }
  const diffSec = Math.max(0, Math.floor((nowMs - then) / 1000));
  if (diffSec < 60) {
    return "刚刚";
  }
  const mins = Math.floor(diffSec / 60);
  if (mins < 60) {
    return `${mins} 分钟前`;
  }
  const hours = Math.floor(mins / 60);
  if (hours < 24) {
    return `${hours} 小时前`;
  }
  const days = Math.floor(hours / 24);
  if (days < 7) {
    return `${days} 天前`;
  }
  return new Date(then).toLocaleDateString();
}
