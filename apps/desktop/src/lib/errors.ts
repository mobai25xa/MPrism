import { isAppError, type AppError } from "./types";
import { t } from "../i18n";

export function toAppError(error: unknown): AppError {
  if (isAppError(error)) {
    return error;
  }
  if (error instanceof Error && isAppError((error as Error & { cause?: unknown }).cause)) {
    return (error as Error & { cause: AppError }).cause;
  }
  // Tauri may wrap invoke errors as objects with message JSON string.
  if (typeof error === "object" && error !== null) {
    const maybe = error as { message?: unknown; data?: unknown };
    if (isAppError(maybe.data)) {
      return maybe.data;
    }
    if (typeof maybe.message === "string") {
      try {
        const parsed = JSON.parse(maybe.message) as unknown;
        if (isAppError(parsed)) {
          return parsed;
        }
      } catch {
        // fall through
      }
    }
  }
  return {
    code: "internal",
    message: t("error.unknown"),
    retryable: false,
  };
}

export function errorMessageByCode(error: AppError): string {
  switch (error.code) {
    case "validation":
      return error.message || t("error.validation");
    case "not_found":
      return t("error.notFound");
    case "conflict":
      return t("error.conflict");
    case "auth":
      return t("error.auth");
    case "rate_limited":
      return t("error.rateLimited");
    case "provider_unavailable":
      return t("error.providerUnavailable");
    case "timeout":
      return t("error.timeout");
    case "transport":
      return t("error.transport");
    case "protocol":
      return t("error.protocol");
    case "storage":
      return t("error.storage");
    case "cancelled":
      return t("error.cancelled");
    default:
      return error.message || t("error.unknown");
  }
}
