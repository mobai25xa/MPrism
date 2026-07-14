import { describe, expect, it } from "vitest";
import {
  createGenerationState,
  reduceStreamEvent,
  shouldSendOnEnter,
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
});

describe("composer enter key", () => {
  it("sends on Enter without shift/ime", () => {
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: false })).toBe(true);
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: true })).toBe(false);
    expect(shouldSendOnEnter({ key: "Enter", shiftKey: false, isComposing: true })).toBe(false);
    expect(shouldSendOnEnter({ key: "a", shiftKey: false })).toBe(false);
  });
});
