import { create } from "zustand";
import type { AppPage } from "../components/NavRail";
import { errorMessageByCode, toAppError } from "../lib/errors";
import * as api from "../lib/tauri";
import type {
  AppError,
  BootstrapPayload,
  GenerationState,
  MessageRecord,
  ModelInfoPayload,
  ModelRecord,
  ProtocolId,
  ProviderPublic,
  SessionMeta,
  StreamEnvelope,
  ThemePreference,
} from "../lib/types";
import { IPC_SCHEMA_VERSION } from "../lib/types";
import { readThemePreference, writeThemePreference } from "../app/theme";
import { buildApiKeyUpdate } from "../features/providers/formLogic";
import {
  createGenerationState,
  reduceStreamEvent,
  streamingAssistantMessage,
} from "../features/chat/streamReducer";

export type ProviderDraftState = {
  name: string;
  protocol: ProtocolId;
  base_url: string;
  api_key_input: string;
  clear_key: boolean;
  models: ModelRecord[];
};

type AppStore = {
  ready: boolean;
  bootError: string | null;
  page: AppPage;
  theme: ThemePreference;
  providers: ProviderPublic[];
  defaultProviderId: string | null;
  defaultModelId: string | null;
  selectedProviderId: string | "draft" | null;
  draft: ProviderDraftState | null;
  dirty: boolean;
  toast: string | null;
  formError: string | null;
  discovering: boolean;
  discovered: ModelInfoPayload[];
  discoverError: string | null;
  modelSearch: string;

  sessions: SessionMeta[];
  activeSessionId: string | null;
  messagesBySession: Record<string, MessageRecord[]>;
  generations: Record<string, GenerationState>;
  draftsBySession: Record<string, string>;
  chatLoading: boolean;
  chatError: string | null;
  partiallyCorruptBySession: Record<string, boolean>;

  bootstrap: () => Promise<void>;
  setPage: (page: AppPage) => void;
  setTheme: (theme: ThemePreference) => Promise<void>;
  selectProvider: (id: string | "draft" | null) => boolean;
  startDraft: () => boolean;
  updateDraft: (patch: Partial<ProviderDraftState>) => void;
  markClean: () => void;
  setToast: (message: string | null) => void;
  setFormError: (message: string | null) => void;
  saveProvider: () => Promise<ProviderPublic | null>;
  deleteSelectedProvider: () => Promise<boolean>;
  discoverModels: () => Promise<void>;
  setModelSearch: (value: string) => void;
  setRetainedModels: (models: ModelRecord[]) => void;
  setDefaults: (providerId: string, modelId: string) => Promise<void>;
  clearDiscovered: () => void;

  selectSession: (sessionId: string | null) => Promise<void>;
  createSession: (title?: string) => Promise<SessionMeta | null>;
  renameSession: (sessionId: string, title: string) => Promise<boolean>;
  deleteSession: (sessionId: string) => Promise<boolean>;
  updateSystemPrompt: (sessionId: string, systemPrompt: string) => Promise<boolean>;
  updateSessionSelection: (
    sessionId: string,
    providerId: string | null,
    modelId: string | null,
  ) => Promise<boolean>;
  setComposerDraft: (sessionId: string, text: string) => void;
  sendMessage: (sessionId: string, content: string) => Promise<void>;
  stopGeneration: (sessionId: string) => Promise<void>;
  reloadSessionMessages: (sessionId: string) => Promise<void>;
};

const emptyDraft = (): ProviderDraftState => ({
  name: "",
  protocol: "openai_chat_completions",
  base_url: "",
  api_key_input: "",
  clear_key: false,
  models: [],
});

const KNOWN_PROTOCOLS: readonly ProtocolId[] = [
  "openai_chat_completions",
  "openai_responses",
  "anthropic_messages",
  "gemini_generate_content",
];

function normalizeProtocol(protocol: ProviderPublic["protocol"]): ProtocolId {
  return (KNOWN_PROTOCOLS as readonly string[]).includes(protocol)
    ? (protocol as ProtocolId)
    : "openai_chat_completions";
}

function draftFromProvider(provider: ProviderPublic): ProviderDraftState {
  return {
    name: provider.name,
    protocol: normalizeProtocol(provider.protocol),
    base_url: provider.base_url,
    api_key_input: "",
    clear_key: false,
    models: provider.models.map((model) => ({ ...model })),
  };
}

function sortSessions(sessions: SessionMeta[]): SessionMeta[] {
  return [...sessions].sort((a, b) => {
    const ta = Date.parse(a.updated_at);
    const tb = Date.parse(b.updated_at);
    if (tb !== ta) {
      return tb - ta;
    }
    return b.id.localeCompare(a.id);
  });
}

function applyBootstrap(set: (partial: Partial<AppStore>) => void, payload: BootstrapPayload) {
  const firstProvider = payload.providers[0]?.id ?? null;
  const firstSession = payload.sessions[0]?.id ?? null;
  set({
    ready: true,
    bootError: null,
    theme: payload.theme,
    providers: payload.providers,
    defaultProviderId: payload.default_provider_id,
    defaultModelId: payload.default_model_id,
    selectedProviderId: firstProvider,
    draft: firstProvider
      ? draftFromProvider(payload.providers.find((p) => p.id === firstProvider)!)
      : null,
    dirty: false,
    sessions: sortSessions(payload.sessions),
    activeSessionId: firstSession,
  });
  writeThemePreference(payload.theme);
}

function upsertSession(sessions: SessionMeta[], meta: SessionMeta): SessionMeta[] {
  const next = sessions.filter((s) => s.id !== meta.id);
  next.push(meta);
  return sortSessions(next);
}

function resolveSelection(
  providers: ProviderPublic[],
  defaultProviderId: string | null,
  defaultModelId: string | null,
  session: SessionMeta | null,
): { providerId: string | null; modelId: string | null } {
  const providerId =
    (session?.last_provider_id &&
      providers.some((p) => p.id === session.last_provider_id) &&
      session.last_provider_id) ||
    (defaultProviderId && providers.some((p) => p.id === defaultProviderId)
      ? defaultProviderId
      : providers[0]?.id ?? null);
  if (!providerId) {
    return { providerId: null, modelId: null };
  }
  const provider = providers.find((p) => p.id === providerId);
  const models = provider?.models ?? [];
  const modelId =
    (session?.last_model_id && models.some((m) => m.id === session.last_model_id)
      ? session.last_model_id
      : null) ||
    (providerId === defaultProviderId &&
    defaultModelId &&
    models.some((m) => m.id === defaultModelId)
      ? defaultModelId
      : models[0]?.id ?? null);
  return { providerId, modelId };
}

export const useAppStore = create<AppStore>((set, get) => ({
  ready: false,
  bootError: null,
  page: "chat",
  theme: readThemePreference(),
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

  async bootstrap() {
    try {
      const payload = await api.bootstrap();
      applyBootstrap(set, payload);
      const firstSession = payload.sessions[0]?.id;
      if (firstSession) {
        await get().selectSession(firstSession);
      }
    } catch (error) {
      const appError = toAppError(error);
      set({
        ready: true,
        bootError: errorMessageByCode(appError),
      });
    }
  },

  setPage(page) {
    set({ page });
  },

  async setTheme(theme) {
    writeThemePreference(theme);
    set({ theme });
    try {
      const saved = await api.setTheme(theme);
      set({ theme: saved });
      writeThemePreference(saved);
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
    }
  },

  selectProvider(id) {
    const state = get();
    if (state.dirty) {
      return false;
    }
    if (id === "draft") {
      set({
        selectedProviderId: "draft",
        draft: state.draft ?? emptyDraft(),
        formError: null,
        discoverError: null,
        discovered: [],
      });
      return true;
    }
    if (!id) {
      set({
        selectedProviderId: null,
        draft: null,
        formError: null,
        discoverError: null,
        discovered: [],
      });
      return true;
    }
    const provider = state.providers.find((item) => item.id === id);
    if (!provider) {
      return false;
    }
    set({
      selectedProviderId: id,
      draft: draftFromProvider(provider),
      dirty: false,
      formError: null,
      discoverError: null,
      discovered: [],
      modelSearch: "",
    });
    return true;
  },

  startDraft() {
    const state = get();
    if (state.dirty) {
      return false;
    }
    set({
      selectedProviderId: "draft",
      draft: emptyDraft(),
      dirty: true,
      formError: null,
      discoverError: null,
      discovered: [],
      modelSearch: "",
    });
    return true;
  },

  updateDraft(patch) {
    const current = get().draft ?? emptyDraft();
    set({
      draft: { ...current, ...patch },
      dirty: true,
      formError: null,
    });
  },

  markClean() {
    set({ dirty: false });
  },

  setToast(message) {
    set({ toast: message });
  },

  setFormError(message) {
    set({ formError: message });
  },

  async saveProvider() {
    const state = get();
    const draft = state.draft;
    if (!draft) {
      return null;
    }
    const isDraft = state.selectedProviderId === "draft";
    const apiKey = buildApiKeyUpdate({
      name: draft.name,
      baseUrl: draft.base_url,
      apiKeyInput: draft.api_key_input,
      clearKey: draft.clear_key,
      isNew: isDraft,
      apiKeyPresent: !isDraft,
    });

    try {
      const saved = await api.upsertProvider({
        schema_version: IPC_SCHEMA_VERSION,
        id: isDraft ? null : state.selectedProviderId,
        name: draft.name,
        protocol: draft.protocol,
        base_url: draft.base_url,
        api_key: apiKey,
        models: draft.models,
      });
      const providers = isDraft
        ? [...state.providers, saved]
        : state.providers.map((item) => (item.id === saved.id ? saved : item));
      let defaultProviderId = state.defaultProviderId;
      let defaultModelId = state.defaultModelId;
      if (defaultProviderId) {
        const found = providers.find((p) => p.id === defaultProviderId);
        if (!found || !found.models.some((m) => m.id === defaultModelId)) {
          const first = providers.find((p) => p.models.length > 0);
          defaultProviderId = first?.id ?? null;
          defaultModelId = first?.models[0]?.id ?? null;
        }
      } else {
        const first = providers.find((p) => p.models.length > 0);
        defaultProviderId = first?.id ?? null;
        defaultModelId = first?.models[0]?.id ?? null;
      }
      set({
        providers,
        selectedProviderId: saved.id,
        draft: draftFromProvider(saved),
        dirty: false,
        formError: null,
        defaultProviderId,
        defaultModelId,
      });
      return saved;
    } catch (error) {
      const appError = toAppError(error);
      set({ formError: errorMessageByCode(appError) });
      return null;
    }
  },

  async deleteSelectedProvider() {
    const state = get();
    if (!state.selectedProviderId || state.selectedProviderId === "draft") {
      set({ selectedProviderId: null, draft: null, dirty: false });
      return true;
    }
    try {
      await api.deleteProvider(state.selectedProviderId);
      const providers = state.providers.filter((p) => p.id !== state.selectedProviderId);
      const next = providers[0] ?? null;
      let defaultProviderId = state.defaultProviderId;
      let defaultModelId = state.defaultModelId;
      if (defaultProviderId === state.selectedProviderId) {
        const first = providers.find((p) => p.models.length > 0);
        defaultProviderId = first?.id ?? null;
        defaultModelId = first?.models[0]?.id ?? null;
      }
      set({
        providers,
        selectedProviderId: next?.id ?? null,
        draft: next ? draftFromProvider(next) : null,
        dirty: false,
        formError: null,
        defaultProviderId,
        defaultModelId,
        discovered: [],
        discoverError: null,
      });
      return true;
    } catch (error) {
      const appError = toAppError(error);
      set({ formError: errorMessageByCode(appError) });
      return false;
    }
  },

  async discoverModels() {
    const state = get();
    const draft = state.draft;
    if (!draft) {
      return;
    }
    if (state.selectedProviderId === "draft" || state.dirty) {
      set({ formError: null, discoverError: null });
      throw Object.assign(new Error("must_save"), {
        code: "validation",
        message: "must_save",
        retryable: false,
      } satisfies AppError);
    }
    set({ discovering: true, discoverError: null });
    try {
      const models = await api.discoverModels({
        schema_version: IPC_SCHEMA_VERSION,
        provider_id: state.selectedProviderId,
        protocol: draft.protocol,
        base_url: draft.base_url,
        api_key: draft.clear_key
          ? { type: "clear" }
          : draft.api_key_input.trim()
            ? { type: "replace", value: draft.api_key_input }
            : { type: "keep" },
      });
      set({ discovered: models, discovering: false });
    } catch (error) {
      const appError = toAppError(error);
      set({
        discovering: false,
        discoverError: errorMessageByCode(appError),
      });
    }
  },

  setModelSearch(value) {
    set({ modelSearch: value });
  },

  setRetainedModels(models) {
    const draft = get().draft;
    if (!draft) {
      return;
    }
    set({
      draft: { ...draft, models },
      dirty: true,
    });
  },

  async setDefaults(providerId, modelId) {
    try {
      await api.setDefaults(providerId, modelId);
      set({
        defaultProviderId: providerId,
        defaultModelId: modelId,
      });
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
    }
  },

  clearDiscovered() {
    set({ discovered: [], discoverError: null });
  },

  async selectSession(sessionId) {
    if (!sessionId) {
      set({ activeSessionId: null, chatError: null });
      return;
    }
    set({ activeSessionId: sessionId, chatLoading: true, chatError: null });
    try {
      const loaded = await api.loadSession(sessionId);
      set((state) => ({
        chatLoading: false,
        sessions: upsertSession(state.sessions, loaded.meta),
        messagesBySession: {
          ...state.messagesBySession,
          [sessionId]: loaded.messages,
        },
        partiallyCorruptBySession: {
          ...state.partiallyCorruptBySession,
          [sessionId]: loaded.partially_corrupt,
        },
      }));
    } catch (error) {
      const appError = toAppError(error);
      set({
        chatLoading: false,
        chatError: errorMessageByCode(appError),
      });
    }
  },

  async createSession(title) {
    try {
      const meta = await api.createSession(title ?? null);
      set((state) => ({
        sessions: upsertSession(state.sessions, meta),
        activeSessionId: meta.id,
        messagesBySession: {
          ...state.messagesBySession,
          [meta.id]: [],
        },
        draftsBySession: {
          ...state.draftsBySession,
          [meta.id]: "",
        },
      }));
      const selection = resolveSelection(
        get().providers,
        get().defaultProviderId,
        get().defaultModelId,
        meta,
      );
      if (selection.providerId && selection.modelId) {
        await get().updateSessionSelection(meta.id, selection.providerId, selection.modelId);
      }
      return meta;
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
      return null;
    }
  },

  async renameSession(sessionId, title) {
    try {
      const meta = await api.updateSession(sessionId, {
        schema_version: IPC_SCHEMA_VERSION,
        title,
      });
      set((state) => ({
        sessions: upsertSession(state.sessions, meta),
      }));
      return true;
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
      return false;
    }
  },

  async deleteSession(sessionId) {
    try {
      await api.deleteSession(sessionId);
      const state = get();
      const remaining = state.sessions.filter((s) => s.id !== sessionId);
      const nextId =
        state.activeSessionId === sessionId ? remaining[0]?.id ?? null : state.activeSessionId;
      const { [sessionId]: _m, ...messagesBySession } = state.messagesBySession;
      const { [sessionId]: _g, ...generations } = state.generations;
      const { [sessionId]: _d, ...draftsBySession } = state.draftsBySession;
      set({
        sessions: remaining,
        activeSessionId: nextId,
        messagesBySession,
        generations,
        draftsBySession,
      });
      if (nextId) {
        await get().selectSession(nextId);
      }
      return true;
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
      return false;
    }
  },

  async updateSystemPrompt(sessionId, systemPrompt) {
    try {
      const meta = await api.updateSession(sessionId, {
        schema_version: IPC_SCHEMA_VERSION,
        system_prompt: systemPrompt,
      });
      set((state) => ({ sessions: upsertSession(state.sessions, meta) }));
      return true;
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
      return false;
    }
  },

  async updateSessionSelection(sessionId, providerId, modelId) {
    try {
      // Backend expects Option fields; send explicit values.
      const meta = await api.updateSession(sessionId, {
        schema_version: IPC_SCHEMA_VERSION,
        set_last_provider_id: true,
        last_provider_id: providerId,
        set_last_model_id: true,
        last_model_id: modelId,
      });
      set((state) => ({ sessions: upsertSession(state.sessions, meta) }));
      return true;
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
      return false;
    }
  },

  setComposerDraft(sessionId, text) {
    set((state) => ({
      draftsBySession: {
        ...state.draftsBySession,
        [sessionId]: text,
      },
    }));
  },

  async sendMessage(sessionId, content) {
    const state = get();
    if (state.generations[sessionId]) {
      return;
    }
    const session = state.sessions.find((s) => s.id === sessionId) ?? null;
    const selection = resolveSelection(
      state.providers,
      state.defaultProviderId,
      state.defaultModelId,
      session,
    );
    if (!selection.providerId || !selection.modelId) {
      set({ toast: errorMessageByCode({ code: "validation", message: "", retryable: false }) });
      return;
    }
    const trimmed = content.trim();
    if (!trimmed) {
      return;
    }

    const tempUserId = `temp-user-${Date.now()}`;
    const optimistic: MessageRecord = {
      schema_version: 1,
      id: tempUserId,
      session_id: sessionId,
      sequence: Date.now(),
      role: "user",
      content: trimmed,
      created_by_device_id: "local",
      created_at: new Date().toISOString(),
    };
    set((s) => ({
      draftsBySession: { ...s.draftsBySession, [sessionId]: "" },
      messagesBySession: {
        ...s.messagesBySession,
        [sessionId]: [...(s.messagesBySession[sessionId] ?? []), optimistic],
      },
      chatError: null,
    }));

    try {
      const finalAssistant = await api.startChat(
        {
          schema_version: IPC_SCHEMA_VERSION,
          session_id: sessionId,
          provider_id: selection.providerId,
          model_id: selection.modelId,
          content: trimmed,
        },
        (envelope: StreamEnvelope) => {
          const current = get().generations[sessionId];
          if (!current) {
            if (envelope.event.type === "started" || envelope.sequence === 0) {
              const created = createGenerationState(
                envelope.request_id,
                envelope.session_id,
                envelope.assistant_message_id,
              );
              const reduced = reduceStreamEvent(created, envelope);
              if (reduced.kind === "update" || reduced.kind === "terminal") {
                set((s) => ({
                  generations: { ...s.generations, [sessionId]: reduced.state },
                }));
              }
            }
            return;
          }
          const reduced = reduceStreamEvent(current, envelope);
          if (reduced.kind === "ignore") {
            return;
          }
          if (reduced.kind === "gap") {
            void get().reloadSessionMessages(sessionId);
            set((s) => {
              const { [sessionId]: _, ...rest } = s.generations;
              return { generations: rest };
            });
            return;
          }
          if (reduced.kind === "update") {
            set((s) => ({
              generations: { ...s.generations, [sessionId]: reduced.state },
            }));
            return;
          }
          // terminal events still wait for invoke resolve for final persistence
          set((s) => ({
            generations: { ...s.generations, [sessionId]: reduced.state },
          }));
        },
      );

      // reload session to get user + assistant persisted messages and title
      const loaded = await api.loadSession(sessionId);
      set((s) => {
        const { [sessionId]: _, ...restGen } = s.generations;
        return {
          generations: restGen,
          sessions: upsertSession(s.sessions, loaded.meta),
          messagesBySession: {
            ...s.messagesBySession,
            [sessionId]: loaded.messages,
          },
          partiallyCorruptBySession: {
            ...s.partiallyCorruptBySession,
            [sessionId]: loaded.partially_corrupt,
          },
        };
      });
      void finalAssistant;
    } catch (error) {
      const appError = toAppError(error);
      // keep optimistic user if backend may have persisted; reload best-effort
      try {
        const loaded = await api.loadSession(sessionId);
        set((s) => {
          const { [sessionId]: _, ...restGen } = s.generations;
          return {
            generations: restGen,
            sessions: upsertSession(s.sessions, loaded.meta),
            messagesBySession: {
              ...s.messagesBySession,
              [sessionId]: loaded.messages,
            },
            toast: errorMessageByCode(appError),
          };
        });
      } catch {
        set((s) => {
          const { [sessionId]: _, ...restGen } = s.generations;
          return {
            generations: restGen,
            toast: errorMessageByCode(appError),
          };
        });
      }
    }
  },

  async stopGeneration(sessionId) {
    const gen = get().generations[sessionId];
    if (!gen) {
      return;
    }
    set((s) => ({
      generations: {
        ...s.generations,
        [sessionId]: { ...gen, phase: "cancelling" },
      },
    }));
    try {
      await api.cancelChat(gen.requestId);
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
    }
  },

  async reloadSessionMessages(sessionId) {
    try {
      const loaded = await api.loadSession(sessionId);
      set((s) => ({
        sessions: upsertSession(s.sessions, loaded.meta),
        messagesBySession: {
          ...s.messagesBySession,
          [sessionId]: loaded.messages,
        },
        partiallyCorruptBySession: {
          ...s.partiallyCorruptBySession,
          [sessionId]: loaded.partially_corrupt,
        },
      }));
    } catch (error) {
      const appError = toAppError(error);
      set({ toast: errorMessageByCode(appError) });
    }
  },
}));

export { resolveSelection, streamingAssistantMessage };

