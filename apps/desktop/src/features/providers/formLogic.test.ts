import { describe, expect, it } from "vitest";
import {
  buildApiKeyUpdate,
  validateProviderDraft,
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

