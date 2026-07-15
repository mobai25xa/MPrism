import { describe, expect, it } from "vitest";
import {
  buildApiKeyUpdate,
  normalizeStoredAuth,
  normalizeStoredReasoning,
  normalizeStoredTools,
  parseToolsJsonText,
  protocolReasoningHint,
  validateAuthSettings,
  validateProviderDraft,
  validateReasoningSettings,
  validateToolsSettings,
  type ProviderFormState,
} from "./formLogic";

const base: ProviderFormState = {
  name: "My Provider",
  baseUrl: "https://api.example.com/v1",
  apiKeyInput: "",
  clearKey: false,
  isNew: false,
  apiKeyPresent: true,
};

describe("provider form validation", () => {
  it("requires name and base url", () => {
    expect(validateProviderDraft({ ...base, name: "  " })?.field).toBe("name");
    expect(validateProviderDraft({ ...base, baseUrl: "" })?.field).toBe("baseUrl");
  });

  it("rejects query or fragment in base url", () => {
    expect(
      validateProviderDraft({ ...base, baseUrl: "https://api.example.com/v1?x=1" })?.field,
    ).toBe("baseUrl");
    expect(
      validateProviderDraft({ ...base, baseUrl: "https://api.example.com/v1#frag" })?.field,
    ).toBe("baseUrl");
  });

  it("accepts valid http(s) url", () => {
    expect(validateProviderDraft(base)).toBeNull();
    expect(
      validateProviderDraft({ ...base, baseUrl: "http://127.0.0.1:8080/v1/" }),
    ).toBeNull();
  });
});

describe("api key three-state", () => {
  it("keeps existing key when input empty and not cleared", () => {
    expect(
      buildApiKeyUpdate({
        ...base,
        apiKeyInput: "",
        clearKey: false,
        isNew: false,
      }),
    ).toEqual({ type: "keep" });
  });

  it("replaces key when user typed a value", () => {
    expect(
      buildApiKeyUpdate({
        ...base,
        apiKeyInput: "sk-new",
        clearKey: false,
        isNew: false,
      }),
    ).toEqual({ type: "replace", value: "sk-new" });
  });

  it("clears key when clear flag set", () => {
    expect(
      buildApiKeyUpdate({
        ...base,
        apiKeyInput: "",
        clearKey: true,
        isNew: false,
      }),
    ).toEqual({ type: "clear" });
  });

  it("uses empty replace for new provider without key", () => {
    expect(
      buildApiKeyUpdate({
        ...base,
        isNew: true,
        apiKeyInput: "",
        clearKey: false,
      }),
    ).toEqual({ type: "replace", value: "" });
  });
});

describe("reasoning settings helpers", () => {
  it("normalizes auto/empty to null for V1 compatibility", () => {
    expect(normalizeStoredReasoning(null)).toBeNull();
    expect(normalizeStoredReasoning({ mode: "auto" })).toBeNull();
    expect(normalizeStoredReasoning({ mode: "AUTO", effort: null, budget_tokens: null })).toBeNull();
  });

  it("keeps on/off with optional effort and budget", () => {
    expect(
      normalizeStoredReasoning({
        mode: "on",
        effort: "high",
        budget_tokens: 2048,
      }),
    ).toEqual({
      mode: "on",
      effort: "high",
      budget_tokens: 2048,
    });
    expect(normalizeStoredReasoning({ mode: "off" })).toEqual({
      mode: "off",
      effort: null,
      budget_tokens: null,
    });
  });

  it("rejects on/off when protocol lacks reasoning_control", () => {
    expect(
      validateReasoningSettings({ mode: "on", effort: "low" }, { reasoningControl: false })
        ?.field,
    ).toBe("reasoning");
    expect(
      validateReasoningSettings({ mode: "auto" }, { reasoningControl: false }),
    ).toBeNull();
    expect(
      validateReasoningSettings({ mode: "on" }, { reasoningControl: true }),
    ).toBeNull();
  });

  it("provides protocol-specific hints", () => {
    expect(protocolReasoningHint("openai_chat_completions")).toContain("Responses");
    expect(protocolReasoningHint("anthropic_messages")).toContain("1024");
  });
});

describe("tools settings helpers", () => {
  it("parses tools JSON array", () => {
    const parsed = parseToolsJsonText(
      JSON.stringify([
        {
          name: "get_weather",
          description: "weather",
          parameters: { type: "object", properties: {} },
        },
      ]),
    );
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.tools[0].name).toBe("get_weather");
    }
  });

  it("rejects non-object parameters", () => {
    const parsed = parseToolsJsonText(
      JSON.stringify([{ name: "x", parameters: [] }]),
    );
    expect(parsed.ok).toBe(false);
  });

  it("normalizes empty tools to null choice", () => {
    expect(normalizeStoredTools([], { mode: "auto" })).toEqual({
      tools: [],
      tool_choice: null,
    });
  });

  it("rejects tools when protocol lacks tools capability", () => {
    expect(
      validateToolsSettings(
        [{ name: "a", parameters: { type: "object" } }],
        { mode: "auto" },
        { toolsSupported: false },
      )?.field,
    ).toBe("tools");
  });

  it("requires named tool to exist", () => {
    expect(
      validateToolsSettings(
        [{ name: "a", parameters: { type: "object" } }],
        { mode: "named", name: "missing" },
        { toolsSupported: true },
      )?.field,
    ).toBe("tools");
  });
});

describe("auth settings helpers", () => {
  it("normalizes empty query and blank header rows", () => {
    expect(
      normalizeStoredAuth(
        [
          { name: "  ", value: "  " },
          { name: "X-A", value: "1" },
        ],
        "  ",
      ),
    ).toEqual({
      extra_headers: [{ name: "X-A", value: "1" }],
      api_key_query_param: null,
    });
  });

  it("rejects CR/LF in header values", () => {
    expect(
      validateAuthSettings(
        [{ name: "X-A", value: "bad\n" }],
        null,
        { customHeaders: true, apiKeyQuery: true },
      )?.field,
    ).toBe("auth");
  });

  it("respects capability flags", () => {
    expect(
      validateAuthSettings([{ name: "X-A", value: "1" }], null, {
        customHeaders: false,
        apiKeyQuery: true,
      })?.field,
    ).toBe("auth");
    expect(
      validateAuthSettings([], "key", {
        customHeaders: true,
        apiKeyQuery: false,
      })?.field,
    ).toBe("auth");
  });
});

