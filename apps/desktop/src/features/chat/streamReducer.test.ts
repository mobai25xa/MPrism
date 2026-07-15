import { describe, expect, it } from "vitest";
import {
  createGenerationState,
  formatFinishReason,
  formatTokenUsage,
  reduceStreamEvent,
  shouldSendOnEnter,
  streamingAssistantMessage,
} from "./streamReducer";
import type { StreamEnvelope } from "../../lib/types";

function env(
  sequence: number,
  event: StreamEnvelope["event"],
  overrides?: Partial<StreamEnvelope>,
): StreamEnvelope {
  return {
    schema_version: 1,
    request_id: "r1",
    session_id: "s1",
    assistant_message_id: "a1",
    sequence,
    event,
    ...overrides,
  };
}

describe("stream reducer", () => {
  it("applies ordered deltas and ignores duplicates", () => {
    let state = createGenerationState("r1", "s1", "a1");
    let result = reduceStreamEvent(state, env(0, { type: "started" }));
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;

    result = reduceStreamEvent(state, env(1, { type: "content_delta", text: "Hel" }));
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;
    expect(state.content).toBe("Hel");

    // duplicate
    result = reduceStreamEvent(state, env(1, { type: "content_delta", text: "Hel" }));
    expect(result.kind).toBe("ignore");

    result = reduceStreamEvent(state, env(2, { type: "reasoning_delta", text: "think" }));
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;
    expect(state.reasoning).toBe("think");

    result = reduceStreamEvent(state, env(3, { type: "completed", finish_reason: "stop" }));
    expect(result.kind).toBe("terminal");
  });

  it("detects sequence gaps", () => {
    const state = createGenerationState("r1", "s1", "a1");
    const result = reduceStreamEvent(state, env(2, { type: "content_delta", text: "x" }));
    expect(result.kind).toBe("gap");
  });

  it("accumulates tool call deltas and finished", () => {
    let state = createGenerationState("r1", "s1", "a1");
    let result = reduceStreamEvent(state, env(0, { type: "started" }));
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;

    result = reduceStreamEvent(
      state,
      env(1, {
        type: "tool_call_delta",
        id: "call_1",
        name: "lookup",
        arguments_delta: '{"q":',
        index: 0,
      }),
    );
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;
    expect(state.toolCalls).toHaveLength(1);
    expect(state.toolCalls[0].arguments).toBe('{"q":');

    result = reduceStreamEvent(
      state,
      env(2, {
        type: "tool_call_delta",
        id: "call_1",
        arguments_delta: "1}",
        index: 0,
      }),
    );
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;
    expect(state.toolCalls[0].arguments).toBe('{"q":1}');

    result = reduceStreamEvent(
      state,
      env(3, {
        type: "tool_call_finished",
        id: "call_1",
        name: "lookup",
        arguments: '{"q":1}',
        index: 0,
      }),
    );
    expect(result.kind).toBe("update");
    if (result.kind !== "update") return;
    state = result.state;
    expect(state.toolCalls[0].finished).toBe(true);
    expect(state.toolCalls[0].arguments).toBe('{"q":1}');
  });
});

describe("composer enter key", () => {
  it("sends on Enter without shift/ime", () => {
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: false })).toBe(true);
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: true })).toBe(false);
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: false, isComposing: true })).toBe(false);
    expect(shouldSendOnEnter({ key: "a", shiftKey: false })).toBe(false);
  });
});

describe("stream surface helpers", () => {
  it("formats finish reasons stably", () => {
    expect(formatFinishReason("stop")).toBe("stop");
    expect(formatFinishReason("tool_calls")).toBe("tool_calls");
    expect(formatFinishReason("other:weird")).toBe("other:weird");
    expect(formatFinishReason("weird")).toBe("other:weird");
    expect(formatFinishReason(null)).toBeNull();
  });

  it("formats usage without fabricating zeros", () => {
    expect(formatTokenUsage(null)).toBeNull();
    expect(formatTokenUsage({})).toBeNull();
    expect(
      formatTokenUsage({
        prompt_tokens: 3,
        completion_tokens: 2,
        total_tokens: 5,
        reasoning_tokens: 1,
        cached_tokens: null,
      }),
    ).toBe("prompt 3 · completion 2 · total 5 · reasoning 1");
  });

  it("maps generation tool calls into streaming assistant message", () => {
    let state = createGenerationState("r1", "s1", "a1");
    let result = reduceStreamEvent(state, env(0, { type: "started" }));
    if (result.kind !== "update") return;
    state = result.state;
    result = reduceStreamEvent(
      state,
      env(1, {
        type: "tool_call_finished",
        id: "c1",
        name: "lookup",
        arguments: '{"q":1}',
        index: 0,
      }),
    );
    if (result.kind !== "update") return;
    state = result.state;
    result = reduceStreamEvent(
      state,
      env(2, {
        type: "usage",
        usage: { prompt_tokens: 1, completion_tokens: 2, total_tokens: 3 },
      }),
    );
    if (result.kind !== "update") return;
    const message = streamingAssistantMessage(result.state);
    expect(message.tool_calls).toEqual([
      { id: "c1", name: "lookup", arguments: '{"q":1}', index: 0 },
    ]);
    expect(message.usage?.total_tokens).toBe(3);
  });
});
