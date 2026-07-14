import type { ApiKeyUpdateInput } from "../../lib/types";
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
  field: "name" | "baseUrl";
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
