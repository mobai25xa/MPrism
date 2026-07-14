import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  BootstrapPayload,
  CancelChatPayload,
  ChatInput,
  LoadedSession,
  MessageRecord,
  ModelInfoPayload,
  ProviderDraft,
  ProviderInput,
  ProviderPublic,
  SessionMeta,
  StreamEnvelope,
  ThemePreference,
  UnitPayload,
  UpdateSessionInput,
} from "./types";
import { toAppError } from "./errors";

export type TauriBridge = {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  channel?<T>(onMessage: (payload: T) => void): { id: number } | ChannelLike<T>;
};

export type ChannelLike<T> = {
  onmessage: (response: T) => void;
  id?: number;
};

let bridge: TauriBridge = {
  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await tauriInvoke<T>(cmd, args);
    } catch (error) {
      throw toAppError(error);
    }
  },
};

export function setTauriBridge(next: TauriBridge | null): void {
  if (next) {
    bridge = next;
    return;
  }
  bridge = {
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      try {
        return await tauriInvoke<T>(cmd, args);
      } catch (error) {
        throw toAppError(error);
      }
    },
  };
}

export async function bootstrap(): Promise<BootstrapPayload> {
  return bridge.invoke<BootstrapPayload>("bootstrap");
}

export async function setTheme(theme: ThemePreference): Promise<ThemePreference> {
  return bridge.invoke<ThemePreference>("set_theme", { theme });
}

export async function upsertProvider(input: ProviderInput): Promise<ProviderPublic> {
  return bridge.invoke<ProviderPublic>("upsert_provider", { input });
}

export async function deleteProvider(providerId: string): Promise<UnitPayload> {
  return bridge.invoke<UnitPayload>("delete_provider", { providerId });
}

export async function setDefaults(
  providerId: string | null,
  modelId: string | null,
): Promise<UnitPayload> {
  return bridge.invoke<UnitPayload>("set_defaults", { providerId, modelId });
}

export async function discoverModels(draft: ProviderDraft): Promise<ModelInfoPayload[]> {
  return bridge.invoke<ModelInfoPayload[]>("discover_models", { draft });
}

export async function createSession(title?: string | null): Promise<SessionMeta> {
  return bridge.invoke<SessionMeta>("create_session", { title: title ?? null });
}

export async function listSessions(): Promise<SessionMeta[]> {
  return bridge.invoke<SessionMeta[]>("list_sessions");
}

export async function loadSession(sessionId: string): Promise<LoadedSession> {
  return bridge.invoke<LoadedSession>("load_session", { sessionId });
}

export async function updateSession(
  sessionId: string,
  input: UpdateSessionInput,
): Promise<SessionMeta> {
  return bridge.invoke<SessionMeta>("update_session", { sessionId, input });
}

export async function deleteSession(sessionId: string): Promise<UnitPayload> {
  return bridge.invoke<UnitPayload>("delete_session", { sessionId });
}

export async function startChat(
  input: ChatInput,
  onEvent: (envelope: StreamEnvelope) => void,
): Promise<MessageRecord> {
  const channel =
    typeof bridge.channel === "function"
      ? bridge.channel<StreamEnvelope>(onEvent)
      : new Channel<StreamEnvelope>(onEvent);
  return bridge.invoke<MessageRecord>("start_chat", {
    input,
    onEvent: channel,
  });
}

export async function cancelChat(requestId: string): Promise<CancelChatPayload> {
  return bridge.invoke<CancelChatPayload>("cancel_chat", { requestId });
}
