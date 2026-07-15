import type {
  ApiKeyUpdateInput,
  ProtocolId,
  ReasoningEffortId,
  ReasoningModeId,
  StoredExtraHeader,
  StoredReasoningSettings,
  StoredToolChoice,
  StoredToolDefinition,
  ToolChoiceModeId,
} from "../../lib/types";
import { t } from "../../i18n";

export type ProviderFormState = {
  name: string;
  baseUrl: string;
  apiKeyInput: string;
  clearKey: boolean;
  isNew: boolean;
  apiKeyPresent: boolean;
};

export type FieldError = {
  field: "name" | "baseUrl" | "reasoning" | "tools" | "auth";
  message: string;
};

export function validateProviderDraft(state: ProviderFormState): FieldError | null {
  if (!state.name.trim()) {
    return { field: "name", message: t("settings.form.nameRequired") };
  }
  if (!state.baseUrl.trim()) {
    return { field: "baseUrl", message: t("settings.form.baseUrlRequired") };
  }
  try {
    const url = new URL(state.baseUrl.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return { field: "baseUrl", message: t("settings.form.baseUrlInvalid") };
    }
    if (url.search || url.hash) {
      return { field: "baseUrl", message: t("settings.form.baseUrlInvalid") };
    }
  } catch {
    return { field: "baseUrl", message: t("settings.form.baseUrlInvalid") };
  }
  return null;
}

export function normalizeReasoningMode(raw: string | null | undefined): ReasoningModeId {
  const mode = (raw ?? "auto").trim().toLowerCase();
  if (mode === "off" || mode === "on" || mode === "auto") {
    return mode;
  }
  return "auto";
}

/** Collapse auto/empty to null so settings stay V1-compatible. */
export function normalizeStoredReasoning(
  input: StoredReasoningSettings | null | undefined,
): StoredReasoningSettings | null {
  if (!input) {
    return null;
  }
  const mode = normalizeReasoningMode(String(input.mode));
  const effortRaw = input.effort?.toString().trim().toLowerCase() ?? "";
  const effort =
    effortRaw === ""
      ? null
      : (effortRaw as ReasoningEffortId | string);
  const budget =
    input.budget_tokens === null || input.budget_tokens === undefined
      ? null
      : Number(input.budget_tokens);
  if (mode === "auto" && !effort && (budget === null || Number.isNaN(budget))) {
    return null;
  }
  return {
    mode,
    effort,
    budget_tokens:
      budget === null || Number.isNaN(budget) || budget <= 0 ? null : Math.floor(budget),
  };
}

export function protocolReasoningHint(protocol: ProtocolId | string): string {
  switch (protocol) {
    case "openai_chat_completions":
      return t("settings.reasoning.hintChatCompletions");
    case "openai_responses":
      return t("settings.reasoning.hintResponses");
    case "anthropic_messages":
      return t("settings.reasoning.hintAnthropic");
    case "gemini_generate_content":
      return t("settings.reasoning.hintGemini");
    default:
      return t("settings.reasoning.hintGeneric");
  }
}

export function validateReasoningSettings(
  settings: StoredReasoningSettings | null | undefined,
  options: { reasoningControl: boolean },
): FieldError | null {
  if (!settings || normalizeReasoningMode(String(settings.mode)) === "auto") {
    return null;
  }
  if (!options.reasoningControl) {
    return {
      field: "reasoning",
      message: t("settings.reasoning.unsupportedControl"),
    };
  }
  if (settings.budget_tokens != null && settings.budget_tokens <= 0) {
    return {
      field: "reasoning",
      message: t("settings.reasoning.budgetInvalid"),
    };
  }
  return null;
}

export function normalizeToolChoiceMode(raw: string | null | undefined): ToolChoiceModeId {
  const mode = (raw ?? "auto").trim().toLowerCase();
  if (mode === "none" || mode === "required" || mode === "named" || mode === "auto") {
    return mode;
  }
  return "auto";
}

/** Empty tools → null choice for V1-compatible settings. */
export function normalizeStoredTools(
  tools: StoredToolDefinition[] | null | undefined,
  toolChoice: StoredToolChoice | null | undefined,
): { tools: StoredToolDefinition[]; tool_choice: StoredToolChoice | null } {
  const cleaned: StoredToolDefinition[] = [];
  const names = new Set<string>();
  for (const tool of tools ?? []) {
    const name = (tool.name ?? "").trim();
    if (!name || names.has(name)) {
      continue;
    }
    names.add(name);
    const description =
      tool.description == null || String(tool.description).trim() === ""
        ? null
        : String(tool.description).trim();
    cleaned.push({
      name,
      description,
      parameters: tool.parameters ?? { type: "object", properties: {} },
    });
  }
  if (cleaned.length === 0) {
    return { tools: [], tool_choice: null };
  }
  if (!toolChoice) {
    return { tools: cleaned, tool_choice: { mode: "auto", name: null } };
  }
  const mode = normalizeToolChoiceMode(String(toolChoice.mode));
  const named =
    mode === "named"
      ? (toolChoice.name?.toString().trim() || cleaned[0]?.name || null)
      : null;
  return {
    tools: cleaned,
    tool_choice: {
      mode,
      name: named,
    },
  };
}

export function parseToolsJsonText(raw: string): {
  ok: true;
  tools: StoredToolDefinition[];
} | {
  ok: false;
  message: string;
} {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, tools: [] };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return { ok: false, message: t("settings.tools.jsonInvalid") };
  }
  if (!Array.isArray(parsed)) {
    return { ok: false, message: t("settings.tools.jsonMustBeArray") };
  }
  const tools: StoredToolDefinition[] = [];
  for (const item of parsed) {
    if (!item || typeof item !== "object") {
      return { ok: false, message: t("settings.tools.itemInvalid") };
    }
    const record = item as Record<string, unknown>;
    const name = String(record.name ?? "").trim();
    if (!name) {
      return { ok: false, message: t("settings.tools.nameRequired") };
    }
    const parameters = record.parameters;
    if (
      parameters === null ||
      parameters === undefined ||
      typeof parameters !== "object" ||
      Array.isArray(parameters)
    ) {
      return { ok: false, message: t("settings.tools.parametersObject") };
    }
    tools.push({
      name,
      description:
        record.description == null || String(record.description).trim() === ""
          ? null
          : String(record.description),
      parameters: parameters as object,
    });
  }
  return { ok: true, tools };
}

export function toolsToJsonText(tools: StoredToolDefinition[] | null | undefined): string {
  if (!tools || tools.length === 0) {
    return "";
  }
  return JSON.stringify(
    tools.map((tool) => ({
      name: tool.name,
      ...(tool.description ? { description: tool.description } : {}),
      parameters: tool.parameters,
    })),
    null,
    2,
  );
}

export function validateToolsSettings(
  tools: StoredToolDefinition[] | null | undefined,
  toolChoice: StoredToolChoice | null | undefined,
  options: { toolsSupported: boolean },
): FieldError | null {
  const list = tools ?? [];
  if (list.length === 0) {
    if (toolChoice && normalizeToolChoiceMode(String(toolChoice.mode)) !== "auto") {
      return { field: "tools", message: t("settings.tools.choiceWithoutTools") };
    }
    return null;
  }
  if (!options.toolsSupported) {
    return { field: "tools", message: t("settings.tools.unsupported") };
  }
  const names = new Set<string>();
  for (const tool of list) {
    const name = (tool.name ?? "").trim();
    if (!name) {
      return { field: "tools", message: t("settings.tools.nameRequired") };
    }
    if (names.has(name)) {
      return { field: "tools", message: t("settings.tools.duplicateName", { name }) };
    }
    names.add(name);
    if (
      !tool.parameters ||
      typeof tool.parameters !== "object" ||
      Array.isArray(tool.parameters)
    ) {
      return { field: "tools", message: t("settings.tools.parametersObject") };
    }
  }
  if (!toolChoice) {
    return null;
  }
  const mode = normalizeToolChoiceMode(String(toolChoice.mode));
  if (mode === "named") {
    const named = toolChoice.name?.toString().trim() ?? "";
    if (!named || !names.has(named)) {
      return { field: "tools", message: t("settings.tools.namedMissing") };
    }
  }
  return null;
}

export function normalizeStoredAuth(
  extraHeaders: StoredExtraHeader[] | null | undefined,
  apiKeyQueryParam: string | null | undefined,
): { extra_headers: StoredExtraHeader[]; api_key_query_param: string | null } {
  const headers: StoredExtraHeader[] = [];
  for (const header of extraHeaders ?? []) {
    const name = (header.name ?? "").trim();
    if (!name && !(header.value ?? "").trim()) {
      continue;
    }
    headers.push({
      name,
      value: header.value ?? "",
    });
  }
  const query = (apiKeyQueryParam ?? "").trim();
  return {
    extra_headers: headers,
    api_key_query_param: query === "" ? null : query,
  };
}

export function validateAuthSettings(
  extraHeaders: StoredExtraHeader[] | null | undefined,
  apiKeyQueryParam: string | null | undefined,
  options: { customHeaders: boolean; apiKeyQuery: boolean },
): FieldError | null {
  const headers = extraHeaders ?? [];
  if (headers.length > 0 && !options.customHeaders) {
    return { field: "auth", message: t("settings.auth.headersUnsupported") };
  }
  for (const header of headers) {
    const name = (header.name ?? "").trim();
    if (!name) {
      return { field: "auth", message: t("settings.auth.headerNameRequired") };
    }
    if (/[\r\n]/.test(name) || /[\r\n]/.test(header.value ?? "")) {
      return { field: "auth", message: t("settings.auth.headerCrlf") };
    }
  }
  const query = (apiKeyQueryParam ?? "").trim();
  if (query) {
    if (!options.apiKeyQuery) {
      return { field: "auth", message: t("settings.auth.queryUnsupported") };
    }
    if (/[\r\n=&]/.test(query)) {
      return { field: "auth", message: t("settings.auth.queryInvalid") };
    }
  }
  return null;
}

export function buildApiKeyUpdate(state: ProviderFormState): ApiKeyUpdateInput {
  if (state.clearKey) {
    return { type: "clear" };
  }
  if (state.apiKeyInput.trim().length > 0) {
    return { type: "replace", value: state.apiKeyInput };
  }
  if (state.isNew) {
    return { type: "replace", value: "" };
  }
  return { type: "keep" };
}

export function mergeDiscoveredSelection(
  retained: { id: string }[],
  discoveredId: string,
  checked: boolean,
): string[] {
  const ids = new Set(retained.map((m) => m.id));
  if (checked) {
    ids.add(discoveredId);
  } else {
    ids.delete(discoveredId);
  }
  return [...ids];
}

export function ensureUniqueModelId(
  existing: { id: string }[],
  candidate: string,
): { ok: true } | { ok: false; reason: "duplicate" | "empty" } {
  const id = candidate.trim();
  if (!id) {
    return { ok: false, reason: "empty" };
  }
  if (existing.some((m) => m.id === id)) {
    return { ok: false, reason: "duplicate" };
  }
  return { ok: true };
}
