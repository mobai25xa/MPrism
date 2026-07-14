import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTauriBridge } from "../lib/tauri";
import { useAppStore } from "./store";
import type {
  BootstrapPayload,
  LoadedSession,
  MessageRecord,
  ProviderPublic,
  SessionMeta,
  StreamEnvelope,
} from "../lib/types";
import { IPC_SCHEMA_VERSION } from "../lib/types";

function provider(): ProviderPublic {
  return {
    id: "p1",
    name: "Provider",
    protocol: "openai_chat_completions",
    base_url: "https://api.example.com/v1",
    api_key_present: true,
    models: [
      {
        id: "m1",
        display_name: "Model",
        source: "manual",
        temperature: null,
        max_tokens: null,
      },
    ],
    created_at: "2026-07-11T00:00:00Z",
    updated_at: "2026-07-11T00:00:00Z",
    revision: 1,
  };
}

function session(id: string, title = "新会话"): SessionMeta {
  return {
    schema_version: 1,
    id,
    title,
    title_source: "default",
    system_prompt: "",
    last_provider_id: "p1",
    last_model_id: "m1",
    created_by_device_id: "d1",
    created_at: "2026-07-11T00:00:00Z",
    updated_at: "2026-07-11T00:00:00Z",
    revision: 1,
    deleted_at: null,
  };
}

function bootstrapPayload(sessions: SessionMeta[] = []): BootstrapPayload {
  return {
    schema_version: IPC_SCHEMA_VERSION,
    theme: "system",
    default_provider_id: "p1",
    default_model_id: "m1",
    providers: [provider()],
    sessions,
  };
}

describe("chat workspace mock IPC", () => {
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
      sessions: [],
      activeSessionId: null,
      messagesBySession: {},
      generations: {},
      draftsBySession: {},
      chatLoading: false,
      chatError: null,
      partiallyCorruptBySession: {},
    });
  });

  it("creates session and streams assistant content", async () => {
    const s1 = session("s1", "hello");
    let channelHandler: ((payload: StreamEnvelope) => void) | null = null;
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "bootstrap") return bootstrapPayload([]);
      if (cmd === "create_session") return s1;
      if (cmd === "update_session") return { ...s1, ...(args as object) };
      if (cmd === "load_session") {
        const messages: MessageRecord[] = [
          {
            schema_version: 1,
            id: "u1",
            session_id: "s1",
            sequence: 1,
            role: "user",
            content: "hi",
            created_by_device_id: "d1",
            created_at: "2026-07-11T00:00:01Z",
          },
          {
            schema_version: 1,
            id: "a1",
            session_id: "s1",
            sequence: 2,
            role: "assistant",
            content: "hello world",
            reasoning: "think",
            status: "completed",
            created_by_device_id: "d1",
            created_at: "2026-07-11T00:00:02Z",
            completed_at: "2026-07-11T00:00:03Z",
          },
        ];
        return {
          schema_version: 1,
          meta: s1,
          messages,
          partially_corrupt: false,
        } satisfies LoadedSession;
      }
      if (cmd === "start_chat") {
        const onEvent = args?.onEvent as { onmessage?: (p: StreamEnvelope) => void };
        // support both Channel and mock
        const emit = (envelope: StreamEnvelope) => {
          if (typeof onEvent?.onmessage === "function") {
            onEvent.onmessage(envelope);
          } else if (channelHandler) {
            channelHandler(envelope);
          }
        };
        emit({
          schema_version: 1,
          request_id: "r1",
          session_id: "s1",
          assistant_message_id: "a1",
          sequence: 0,
          event: { type: "started" },
        });
        emit({
          schema_version: 1,
          request_id: "r1",
          session_id: "s1",
          assistant_message_id: "a1",
          sequence: 1,
          event: { type: "content_delta", text: "hello " },
        });
        emit({
          schema_version: 1,
          request_id: "r1",
          session_id: "s1",
          assistant_message_id: "a1",
          sequence: 2,
          event: { type: "content_delta", text: "world" },
        });
        emit({
          schema_version: 1,
          request_id: "r1",
          session_id: "s1",
          assistant_message_id: "a1",
          sequence: 3,
          event: { type: "completed", finish_reason: "stop" },
        });
        return {
          schema_version: 1,
          id: "a1",
          session_id: "s1",
          sequence: 2,
          role: "assistant",
          content: "hello world",
          status: "completed",
          created_by_device_id: "d1",
          created_at: "2026-07-11T00:00:02Z",
        } satisfies MessageRecord;
      }
      throw new Error(`unexpected ${cmd}`);
    });

    setTauriBridge({
      invoke: invoke as never,
      channel: (onMessage) => {
        channelHandler = onMessage as (payload: StreamEnvelope) => void;
        return { onmessage: onMessage, id: 1 };
      },
    });

    await useAppStore.getState().bootstrap();
    await useAppStore.getState().createSession();
    expect(useAppStore.getState().activeSessionId).toBe("s1");
    await useAppStore.getState().sendMessage("s1", "hi");
    const messages = useAppStore.getState().messagesBySession.s1;
    expect(messages.some((m) => m.role === "user" && m.content === "hi")).toBe(true);
    expect(messages.some((m) => m.role === "assistant" && m.content === "hello world")).toBe(
      true,
    );
    expect(useAppStore.getState().generations.s1).toBeUndefined();
  });

  it("keeps two session generation maps independent", async () => {
    setTauriBridge({
      invoke: (async (cmd: string) => {
        if (cmd === "bootstrap") {
          return bootstrapPayload([session("s1"), session("s2")]);
        }
        if (cmd === "load_session") {
          return {
            schema_version: 1,
            meta: session("s1"),
            messages: [],
            partially_corrupt: false,
          };
        }
        throw new Error(cmd);
      }) as never,
    });
    await useAppStore.getState().bootstrap();
    useAppStore.setState({
      generations: {
        s1: {
          requestId: "r1",
          sessionId: "s1",
          assistantMessageId: "a1",
          nextSequence: 1,
          reasoning: "",
          content: "one",
          phase: "streaming",
        },
        s2: {
          requestId: "r2",
          sessionId: "s2",
          assistantMessageId: "a2",
          nextSequence: 1,
          reasoning: "",
          content: "two",
          phase: "streaming",
        },
      },
    });
    expect(useAppStore.getState().generations.s1.content).toBe("one");
    expect(useAppStore.getState().generations.s2.content).toBe("two");
  });
});
