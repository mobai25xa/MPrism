import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTauriBridge } from "../lib/tauri";
import { useAppStore } from "../app/store";
import type { BootstrapPayload, ModelInfoPayload, ProviderPublic } from "../lib/types";
import { IPC_SCHEMA_VERSION } from "../lib/types";

function provider(partial?: Partial<ProviderPublic>): ProviderPublic {
  return {
    id: "p1",
    name: "Provider One",
    protocol: "openai_chat_completions",
    base_url: "https://api.example.com/v1",
    api_key_present: true,
    models: [
      {
        id: "gpt-a",
        display_name: "GPT A",
        source: "discovery",
        temperature: null,
        max_tokens: null,
      },
    ],
    created_at: "2026-07-11T00:00:00Z",
    updated_at: "2026-07-11T00:00:00Z",
    revision: 1,
    ...partial,
  };
}

function bootstrapPayload(providers: ProviderPublic[] = []): BootstrapPayload {
  return {
    schema_version: IPC_SCHEMA_VERSION,
    theme: "system",
    default_provider_id: providers[0]?.id ?? null,
    default_model_id: providers[0]?.models[0]?.id ?? null,
    providers,
    sessions: [],
  };
}

describe("settings store mock IPC", () => {
  beforeEach(() => {
    useAppStore.setState({
      ready: false,
      bootError: null,
      page: "chat",
      theme: "system",
      providers: [],
      defaultProviderId: null,
      defaultModelId: null,
      selectedProviderId: null,
      draft: null,
      dirty: false,
      toast: null,
      formError: null,
      discovering: false,
      discovered: [],
      discoverError: null,
      modelSearch: "",
    });
  });

  it("bootstraps empty providers and never exposes api keys", async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === "bootstrap") {
        return bootstrapPayload([]);
      }
      throw new Error(`unexpected ${cmd}`);
    });
    setTauriBridge({ invoke: invoke as never });
    await useAppStore.getState().bootstrap();
    const state = useAppStore.getState();
    expect(state.ready).toBe(true);
    expect(state.providers).toEqual([]);
    expect(JSON.stringify(state)).not.toContain("sk-");
  });

  it("creates provider then discovers models via mock IPC", async () => {
    const saved = provider({
      id: "p-new",
      name: "New",
      api_key_present: true,
      models: [],
    });
    const models: ModelInfoPayload[] = [
      { id: "m1", display_name: "Model 1", owned_by: "org" },
      { id: "m2", display_name: "Model 2", owned_by: null },
    ];
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "bootstrap") {
        return bootstrapPayload([]);
      }
      if (cmd === "upsert_provider") {
        const input = args?.input as { api_key?: unknown; name?: string };
        // ensure no accidental plaintext field named api_key string
        expect(input.api_key).toEqual({ type: "replace", value: "sk-test" });
        return { ...saved, name: input.name ?? saved.name };
      }
      if (cmd === "discover_models") {
        return models;
      }
      throw new Error(`unexpected ${cmd}`);
    });
    setTauriBridge({ invoke: invoke as never });

    await useAppStore.getState().bootstrap();
    useAppStore.getState().startDraft();
    useAppStore.getState().updateDraft({
      name: "New",
      base_url: "https://api.example.com/v1",
      api_key_input: "sk-test",
    });
    const result = await useAppStore.getState().saveProvider();
    expect(result?.id).toBe("p-new");
    expect(useAppStore.getState().dirty).toBe(false);

    await useAppStore.getState().discoverModels();
    const state = useAppStore.getState();
    expect(state.discovered.map((m) => m.id)).toEqual(["m1", "m2"]);
    // retained models remain empty until user multi-selects
    expect(state.draft?.models).toEqual([]);
    // store snapshot must not keep raw key after save (input cleared via draftFromProvider)
    expect(state.draft?.api_key_input).toBe("");
    expect(JSON.stringify(state.providers)).not.toContain("sk-test");
  });

  it("keeps retained models when discover fails", async () => {
    const existing = provider({
      models: [
        {
          id: "keep-me",
          display_name: "Keep",
          source: "manual",
          temperature: null,
          max_tokens: null,
        },
      ],
    });
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === "bootstrap") {
        return bootstrapPayload([existing]);
      }
      if (cmd === "discover_models") {
        throw {
          code: "auth",
          message: "bad key",
          retryable: false,
        };
      }
      throw new Error(`unexpected ${cmd}`);
    });
    setTauriBridge({ invoke: invoke as never });
    await useAppStore.getState().bootstrap();
    await useAppStore.getState().discoverModels();
    const state = useAppStore.getState();
    expect(state.discoverError).toBeTruthy();
    expect(state.draft?.models.map((m) => m.id)).toEqual(["keep-me"]);
  });

  it("blocks provider switch while dirty", async () => {
    const a = provider({ id: "a", name: "A" });
    const b = provider({ id: "b", name: "B", models: [] });
    setTauriBridge({
      invoke: (async (cmd: string) => {
        if (cmd === "bootstrap") {
          return bootstrapPayload([a, b]);
        }
        throw new Error(cmd);
      }) as never,
    });
    await useAppStore.getState().bootstrap();
    useAppStore.getState().updateDraft({ name: "A changed" });
    expect(useAppStore.getState().dirty).toBe(true);
    expect(useAppStore.getState().selectProvider("b")).toBe(false);
    expect(useAppStore.getState().selectedProviderId).toBe("a");
  });
});

