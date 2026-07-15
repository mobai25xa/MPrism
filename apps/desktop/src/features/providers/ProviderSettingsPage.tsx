import { useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Button,
  OverflowText,
  Field,
  IconCheck,
  IconChevronLeft,
  IconChevronRight,
  IconDismiss,
  IconEye,
  IconEyeOff,
  IconPlus,
  IconSave,
  IconSearch,
  IconStar,
  IconStarFilled,
  IconTrash,
  Input,
  Modal,
  Select,
  Textarea,
  Tooltip,
  cx,
} from "../../ui";
import { t } from "../../i18n";
import type {
  ModelInfoPayload,
  ModelRecord,
  ProviderPublic,
  ReasoningEffortId,
  ReasoningModeId,
  StoredReasoningSettings,
  ToolChoiceModeId,
} from "../../lib/types";
import { useAppStore } from "../../app/store";
import {
  normalizeReasoningMode,
  normalizeStoredReasoning,
  normalizeToolChoiceMode,
  parseToolsJsonText,
  toolsToJsonText,
  protocolReasoningHint,
  validateAuthSettings,
  validateProviderDraft,
  validateReasoningSettings,
  validateToolsSettings,
} from "./formLogic";

const SIDEBAR_COLLAPSED_KEY = "mprism.settings.sidebarCollapsed";
const SIDEBAR_WIDTH_KEY = "mprism.settings.sidebarWidth";
const SIDEBAR_WIDTH_DEFAULT = 260;
const SIDEBAR_WIDTH_MIN = 200;
const SIDEBAR_WIDTH_MAX = 480;

function clampSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) {
    return SIDEBAR_WIDTH_DEFAULT;
  }
  return Math.min(SIDEBAR_WIDTH_MAX, Math.max(SIDEBAR_WIDTH_MIN, Math.round(width)));
}

function readSidebarCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function writeSidebarCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // ignore
  }
}

function readSidebarWidth(): number {
  try {
    const raw = localStorage.getItem(SIDEBAR_WIDTH_KEY);
    if (!raw) {
      return SIDEBAR_WIDTH_DEFAULT;
    }
    return clampSidebarWidth(Number(raw));
  } catch {
    return SIDEBAR_WIDTH_DEFAULT;
  }
}

function writeSidebarWidth(width: number): void {
  try {
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(clampSidebarWidth(width)));
  } catch {
    // ignore
  }
}

export function ProviderSettingsPage() {
  const {
    providers,
    selectedProviderId,
    draft,
    dirty,
    formError,
    discovering,
    discovered,
    discoverError,
    modelSearch,
    defaultProviderId,
    defaultModelId,
    protocolCapabilities,
    startDraft,
    selectProvider,
    updateDraft,
    saveProvider,
    deleteSelectedProvider,
    discoverModels,
    setModelSearch,
    setRetainedModels,
    setDefaults,
    setFormError,
    markClean,
    ensureProtocolCapabilities,
  } = useAppStore();

  const [unsavedOpen, setUnsavedOpen] = useState(false);
  const [pendingSelect, setPendingSelect] = useState<string | "draft" | "draft-new" | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualId, setManualId] = useState("");
  const [manualName, setManualName] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [manualError, setManualError] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const resizeDragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const setSidebar = (collapsed: boolean) => {
    setSidebarCollapsed(collapsed);
    writeSidebarCollapsed(collapsed);
  };

  useEffect(() => {
    if (!draft?.protocol) {
      return;
    }
    void ensureProtocolCapabilities(draft.protocol);
  }, [draft?.protocol, ensureProtocolCapabilities]);

  const reasoningControl =
    protocolCapabilities[draft?.protocol ?? ""]?.reasoning_control ?? false;
  const toolsSupported = protocolCapabilities[draft?.protocol ?? ""]?.tools ?? false;
  const customHeadersSupported =
    protocolCapabilities[draft?.protocol ?? ""]?.custom_headers ?? false;
  const apiKeyQuerySupported =
    protocolCapabilities[draft?.protocol ?? ""]?.api_key_query ?? false;
  const [toolsJsonText, setToolsJsonText] = useState("");
  const [toolsJsonError, setToolsJsonError] = useState<string | null>(null);
  const [authOpen, setAuthOpen] = useState(false);

  useEffect(() => {
    if (!draft) {
      setToolsJsonText("");
      setToolsJsonError(null);
      return;
    }
    setToolsJsonText(toolsToJsonText(draft.tools));
    setToolsJsonError(null);
  }, [draft?.name, draft?.protocol, selectedProviderId, draft?.tools]);

  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const drag = resizeDragRef.current;
      if (!drag) {
        return;
      }
      const next = clampSidebarWidth(drag.startWidth + (event.clientX - drag.startX));
      setSidebarWidth(next);
    };
    const onUp = () => {
      if (!resizeDragRef.current) {
        return;
      }
      resizeDragRef.current = null;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      setSidebarWidth((current) => {
        writeSidebarWidth(current);
        return current;
      });
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, []);

  const onResizePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    resizeDragRef.current = {
      startX: event.clientX,
      startWidth: sidebarWidth,
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

  const selectedProvider: ProviderPublic | null = useMemo(() => {
    if (!selectedProviderId || selectedProviderId === "draft") {
      return null;
    }
    return providers.find((p) => p.id === selectedProviderId) ?? null;
  }, [providers, selectedProviderId]);

  const filteredDiscovered = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    if (!q) {
      return discovered;
    }
    return discovered.filter(
      (m) =>
        m.id.toLowerCase().includes(q) ||
        m.display_name.toLowerCase().includes(q) ||
        (m.owned_by ?? "").toLowerCase().includes(q),
    );
  }, [discovered, modelSearch]);

  const trySelect = (id: string | "draft" | null) => {
    if (dirty) {
      setPendingSelect(id);
      setUnsavedOpen(true);
      return;
    }
    selectProvider(id);
    setShowKey(false);
  };

  const onCreate = () => {
    if (dirty) {
      setPendingSelect("draft-new");
      setUnsavedOpen(true);
      return;
    }
    startDraft();
    setShowKey(false);
  };

  const handleSave = async (): Promise<boolean> => {
    if (!draft) {
      return false;
    }
    const validation = validateProviderDraft({
      name: draft.name,
      baseUrl: draft.base_url,
      apiKeyInput: draft.api_key_input,
      clearKey: draft.clear_key,
      isNew: selectedProviderId === "draft",
      apiKeyPresent: !!selectedProvider?.api_key_present,
    });
    const validationMessage = validation?.message ?? null;
    if (validationMessage) {
      setFormError(validationMessage);
      return false;
    }
    for (const model of draft.models) {
      const reasoningError = validateReasoningSettings(model.reasoning, {
        reasoningControl,
      });
      if (reasoningError) {
        setFormError(`${model.id}: ${reasoningError.message}`);
        return false;
      }
    }
    const toolsError = validateToolsSettings(draft.tools, draft.tool_choice, {
      toolsSupported,
    });
    if (toolsError) {
      setFormError(toolsError.message);
      return false;
    }
    const authError = validateAuthSettings(draft.extra_headers, draft.api_key_query_param, {
      customHeaders: customHeadersSupported,
      apiKeyQuery: apiKeyQuerySupported,
    });
    if (authError) {
      setFormError(authError.message);
      return false;
    }
    const saved = await saveProvider();
    if (saved) {
      setShowKey(false);
      return true;
    }
    return false;
  };

  const handleDiscover = async () => {
    if (!draft) {
      return;
    }
    if (selectedProviderId === "draft" || dirty) {
      setFormError(t("settings.form.mustSaveBeforeDiscover"));
      return;
    }
    try {
      await discoverModels();
    } catch {
      setFormError(t("settings.form.mustSaveBeforeDiscover"));
    }
  };

  const toggleDiscovered = (model: ModelInfoPayload, checked: boolean) => {
    if (!draft) {
      return;
    }
    if (checked) {
      if (draft.models.some((m) => m.id === model.id)) {
        return;
      }
      setRetainedModels([
        ...draft.models,
        {
          id: model.id,
          display_name: model.display_name || model.id,
          source: "discovery",
          temperature: null,
          max_tokens: null,
        },
      ]);
      return;
    }
    setRetainedModels(draft.models.filter((m) => m.id !== model.id));
  };

  const addManual = () => {
    if (!draft) {
      return;
    }
    const id = manualId.trim();
    if (!id) {
      setManualError(t("settings.models.manualId"));
      return;
    }
    if (draft.models.some((m) => m.id === id)) {
      setManualError(t("settings.models.duplicate"));
      return;
    }
    const name = manualName.trim() || id;
    setRetainedModels([
      ...draft.models,
      {
        id,
        display_name: name,
        source: "manual",
        temperature: null,
        max_tokens: null,
      },
    ]);
    setManualOpen(false);
    setManualId("");
    setManualName("");
    setManualError(null);
  };

  const updateModelField = (
    modelId: string,
    patch: Partial<Pick<ModelRecord, "temperature" | "max_tokens" | "display_name" | "reasoning">>,
  ) => {
    if (!draft) {
      return;
    }
    setRetainedModels(
      draft.models.map((model) => (model.id === modelId ? { ...model, ...patch } : model)),
    );
  };

  const updateModelReasoning = (
    modelId: string,
    patch: Partial<StoredReasoningSettings>,
  ) => {
    if (!draft) {
      return;
    }
    setRetainedModels(
      draft.models.map((model) => {
        if (model.id !== modelId) {
          return model;
        }
        const current: StoredReasoningSettings = model.reasoning ?? {
          mode: "auto",
          effort: null,
          budget_tokens: null,
        };
        return {
          ...model,
          reasoning: normalizeStoredReasoning({ ...current, ...patch }),
        };
      }),
    );
  };

  const removeModel = (modelId: string) => {
    if (!draft) {
      return;
    }
    setRetainedModels(draft.models.filter((model) => model.id !== modelId));
  };

  const renderProviderCard = (id: string | "draft", title: string, subtitle: string) => {
    const active = selectedProviderId === id;
    const isDirtySelected = active && dirty;
    return (
      <div
        key={id}
        className={cx("myui-app-nav__item", active && "is-active")}
        role="button"
        tabIndex={0}
        onClick={() => trySelect(id)}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            trySelect(id);
          }
        }}
      >
        <div className="myui-app-nav__item-main">
          <div className="myui-app-nav__item-text">
            <OverflowText text={title} className="myui-app-nav__item-title" placement="right" />
            <div className="myui-app-nav__item-meta">{subtitle}</div>
          </div>
          {active ? (
            isDirtySelected ? (
              <Tooltip content={t("settings.form.save")} placement="bottom">
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<IconSave size={16} />}
                  aria-label={t("settings.form.save")}
                  onClick={(event) => {
                    event.stopPropagation();
                    void handleSave();
                  }}
                />
              </Tooltip>
            ) : (
              <Tooltip content={t("settings.form.savedHint")} placement="bottom">
                <span className="mprism-saved-icon" aria-label={t("settings.form.savedHint")}>
                  <IconCheck size={18} />
                </span>
              </Tooltip>
            )
          ) : null}
        </div>
      </div>
    );
  };

  return (
    <div className="mprism-shell">
      {sidebarCollapsed ? (
        <aside className="myui-app-nav myui-app-nav--collapsed" aria-label={t("settings.providerList")}>
          <Tooltip content={t("settings.expand")} placement="bottom">
            <Button
              variant="ghost"
              icon={<IconChevronRight />}
              aria-label={t("settings.expand")}
              onClick={() => setSidebar(false)}
            />
          </Tooltip>
          <Tooltip content={t("settings.newProvider")} placement="bottom">
            <Button
              variant="ghost"
              icon={<IconPlus />}
              aria-label={t("settings.newProvider")}
              onClick={onCreate}
            />
          </Tooltip>
          {dirty && (
            <Tooltip content={t("settings.form.save")} placement="bottom">
              <Button
                variant="ghost"
                icon={<IconSave />}
                aria-label={t("settings.form.save")}
                onClick={() => void handleSave()}
              />
            </Tooltip>
          )}
        </aside>
      ) : (
        <aside
          className="myui-app-nav myui-app-nav--resizable"
          style={{ width: sidebarWidth, minWidth: SIDEBAR_WIDTH_MIN, maxWidth: SIDEBAR_WIDTH_MAX }}
          aria-label={t("settings.providerList")}
        >
          <div
            className="mprism-resize-handle"
            role="separator"
            aria-orientation="vertical"
            aria-label={t("settings.resize")}
            aria-valuemin={SIDEBAR_WIDTH_MIN}
            aria-valuemax={SIDEBAR_WIDTH_MAX}
            aria-valuenow={sidebarWidth}
            tabIndex={0}
            onPointerDown={onResizePointerDown}
            onKeyDown={(event) => {
              if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
                return;
              }
              event.preventDefault();
              const delta = event.key === "ArrowRight" ? 16 : -16;
              setSidebarWidth((current) => {
                const next = clampSidebarWidth(current + delta);
                writeSidebarWidth(next);
                return next;
              });
            }}
          />
          <div className="myui-app-nav__head">
            <h3 className="mprism-title3">{t("settings.title")}</h3>
            <div className="myui-app-nav__head-actions">
              {dirty ? (
                <Tooltip content={t("settings.form.save")} placement="bottom">
                  <Button
                    variant="ghost"
                    icon={<IconSave />}
                    aria-label={t("settings.form.save")}
                    onClick={() => void handleSave()}
                  />
                </Tooltip>
              ) : draft ? (
                <Tooltip content={t("settings.form.savedHint")} placement="bottom">
                  <span className="mprism-saved-icon" aria-label={t("settings.form.savedHint")}>
                    <IconCheck size={18} />
                  </span>
                </Tooltip>
              ) : null}
              <Tooltip content={t("settings.newProvider")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconPlus />}
                  aria-label={t("settings.newProvider")}
                  onClick={onCreate}
                />
              </Tooltip>
              <Tooltip content={t("settings.collapse")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconChevronLeft />}
                  aria-label={t("settings.collapse")}
                  onClick={() => setSidebar(true)}
                />
              </Tooltip>
            </div>
          </div>
          <div className="myui-app-nav__body">
            {selectedProviderId === "draft" &&
              renderProviderCard(
                "draft",
                draft?.name.trim() || t("settings.draftLabel"),
                t("settings.draftLabel"),
              )}
            {providers.map((provider) =>
              renderProviderCard(
                provider.id,
                provider.name,
                t("settings.modelCount", { count: provider.models.length }),
              ),
            )}
          </div>
        </aside>
      )}

      {!draft ? (
        <section className="mprism-empty">
          <h3 className="mprism-title3">{t("settings.emptyTitle")}</h3>
          <p className="mprism-body mprism-muted">{t("settings.emptyBody")}</p>
          <Button variant="primary" onClick={onCreate}>
            {t("settings.newProvider")}
          </Button>
        </section>
      ) : (
        <section className="mprism-detail">
          <div className="mprism-detail-header">
            <h3 className="mprism-title3" style={{ flex: 1, minWidth: 0 }}>
              {selectedProviderId === "draft"
                ? draft.name.trim() || t("settings.draftLabel")
                : selectedProvider?.name}
            </h3>
            {selectedProviderId !== "draft" && (
              <Tooltip content={t("settings.form.delete")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconTrash />}
                  aria-label={t("settings.form.delete")}
                  onClick={() => setDeleteOpen(true)}
                />
              </Tooltip>
            )}
          </div>

          {formError && <Alert type="error">{formError}</Alert>}

          <div className="mprism-form-grid">
            <Field label={t("settings.form.name")} required horizontal>
              <Input
                value={draft.name}
                onChange={(event) => updateDraft({ name: event.target.value })}
              />
            </Field>
            <Field label={t("settings.form.protocol")} horizontal>
              <Select
                placement="auto"
                value={draft.protocol}
                options={[
                  {
                    value: "openai_chat_completions",
                    label: t("settings.form.protocolOpenai"),
                  },
                  {
                    value: "openai_responses",
                    label: t("settings.form.protocolOpenaiResponses"),
                  },
                  {
                    value: "anthropic_messages",
                    label: t("settings.form.protocolAnthropicMessages"),
                  },
                  {
                    value: "gemini_generate_content",
                    label: t("settings.form.protocolGeminiGenerateContent"),
                  },
                ]}
                onChange={(protocol) => {
                  if (
                    protocol === "openai_chat_completions" ||
                    protocol === "openai_responses" ||
                    protocol === "anthropic_messages" ||
                    protocol === "gemini_generate_content"
                  ) {
                    updateDraft({ protocol });
                  }
                }}
              />
            </Field>
            <Field label={t("settings.form.baseUrl")} required horizontal>
              <Input
                value={draft.base_url}
                placeholder="https://api.example.com/v1"
                onChange={(event) => updateDraft({ base_url: event.target.value })}
              />
            </Field>
            <Field
              label={t("settings.form.apiKey")}
              horizontal
              hint={
                selectedProvider?.api_key_present
                  ? t("settings.form.apiKeyHintSaved")
                  : t("settings.form.apiKeyHintNew")
              }
            >
              <div className="mprism-key-row">
                <Input
                  type={showKey ? "text" : "password"}
                  value={draft.api_key_input}
                  onChange={(event) =>
                    updateDraft({ api_key_input: event.target.value, clear_key: false })
                  }
                  autoComplete="off"
                />
                <Tooltip content={showKey ? t("settings.form.hideKey") : t("settings.form.showKey")} placement="bottom">
                  <Button
                    variant="ghost"
                    icon={showKey ? <IconEyeOff /> : <IconEye />}
                    aria-label={showKey ? t("settings.form.hideKey") : t("settings.form.showKey")}
                    onClick={() => setShowKey((v) => !v)}
                  />
                </Tooltip>
                <Tooltip content={t("settings.form.clearKey")} placement="bottom">
                  <Button
                    variant="ghost"
                    icon={<IconDismiss />}
                    aria-label={t("settings.form.clearKey")}
                    onClick={() => updateDraft({ api_key_input: "", clear_key: true })}
                  />
                </Tooltip>
              </div>
            </Field>
            <div className="mprism-action-row">
              <Button
                variant="outline"
                loading={discovering}
                disabled={discovering}
                onClick={() => void handleDiscover()}
              >
                {discovering ? t("settings.models.discovering") : t("settings.form.discover")}
              </Button>
              <Button variant="outline" onClick={() => setManualOpen(true)}>
                {t("settings.models.manualAdd")}
              </Button>
            </div>
          </div>

          <div>
            <h3 className="mprism-title3" style={{ marginTop: 8 }}>
              {t("settings.models.retained")}
            </h3>
            {draft.models.length === 0 ? (
              <p className="mprism-body mprism-muted">{t("settings.models.emptyRetained")}</p>
            ) : (
              <div className="mprism-model-list" aria-label={t("settings.models.retained")}>
                {draft.models.map((model) => {
                  const isDefault =
                    defaultProviderId === selectedProviderId &&
                    defaultModelId === model.id &&
                    selectedProviderId !== "draft";
                  return (
                    <div key={model.id} className="mprism-model-card">
                      <div className="mprism-model-top">
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div className="mprism-model-id">{model.id}</div>
                          <div className="mprism-muted">
                            {model.source === "manual"
                              ? t("settings.models.source.manual")
                              : t("settings.models.source.discovery")}
                            {isDefault ? ` · ${t("settings.models.isDefault")}` : ""}
                          </div>
                        </div>
                        <div className="mprism-row">
                          {selectedProviderId !== "draft" && (
                            <Tooltip
                              placement="bottom"
                              content={
                                isDefault
                                  ? t("settings.models.isDefault")
                                  : t("settings.models.setDefault")
                              }
                            >
                              <Button
                                variant="ghost"
                                size="sm"
                                icon={isDefault ? <IconStarFilled size={16} /> : <IconStar size={16} />}
                                disabled={isDefault}
                                aria-label={t("settings.models.setDefault")}
                                onClick={() => {
                                  void setDefaults(selectedProviderId as string, model.id);
                                }}
                              />
                            </Tooltip>
                          )}
                          <Tooltip content={t("settings.models.remove")} placement="bottom">
                            <Button
                              variant="ghost"
                              size="sm"
                              icon={<IconTrash size={16} />}
                              aria-label={t("settings.models.remove")}
                              onClick={() => removeModel(model.id)}
                            />
                          </Tooltip>
                        </div>
                      </div>
                      <div className="mprism-model-fields">
                        <Field label={t("settings.models.displayName")}>
                          <Input
                            value={model.display_name}
                            onChange={(event) => {
                              updateModelField(model.id, {
                                display_name: event.target.value,
                              });
                            }}
                            onBlur={(event) => {
                              const next = event.target.value.trim() || model.id;
                              if (next !== model.display_name) {
                                updateModelField(model.id, { display_name: next });
                              }
                            }}
                          />
                        </Field>
                        <Field label={t("settings.models.temperature")}>
                          <Input
                            type="number"
                            value={model.temperature?.toString() ?? ""}
                            placeholder="0-2"
                            onChange={(event) => {
                              const raw = event.target.value.trim();
                              updateModelField(model.id, {
                                temperature: raw === "" ? null : Number(raw),
                              });
                            }}
                          />
                        </Field>
                        <Field label={t("settings.models.maxTokens")}>
                          <Input
                            type="number"
                            value={model.max_tokens?.toString() ?? ""}
                            onChange={(event) => {
                              const raw = event.target.value.trim();
                              updateModelField(model.id, {
                                max_tokens: raw === "" ? null : Number(raw),
                              });
                            }}
                          />
                        </Field>
                      </div>
                      <div className="mprism-model-fields" style={{ marginTop: 8 }}>
                        <Field
                          label={t("settings.reasoning.mode")}
                          hint={
                            reasoningControl
                              ? t("settings.reasoning.requestVsResponse")
                              : protocolReasoningHint(draft.protocol)
                          }
                        >
                          <Select
                            placement="auto"
                            value={normalizeReasoningMode(model.reasoning?.mode)}
                            disabled={!reasoningControl}
                            options={[
                              {
                                value: "auto",
                                label: t("settings.reasoning.modeAuto"),
                              },
                              {
                                value: "off",
                                label: t("settings.reasoning.modeOff"),
                              },
                              {
                                value: "on",
                                label: t("settings.reasoning.modeOn"),
                              },
                            ]}
                            onChange={(mode) => {
                              if (mode === "auto" || mode === "off" || mode === "on") {
                                updateModelReasoning(model.id, {
                                  mode: mode as ReasoningModeId,
                                });
                              }
                            }}
                          />
                        </Field>
                        {reasoningControl &&
                          normalizeReasoningMode(model.reasoning?.mode) === "on" && (
                            <>
                              <Field label={t("settings.reasoning.effort")}>
                                <Select
                                  placement="auto"
                                  value={(model.reasoning?.effort as string) || ""}
                                  options={[
                                    {
                                      value: "",
                                      label: t("settings.reasoning.effortNone"),
                                    },
                                    { value: "minimal", label: "minimal" },
                                    { value: "low", label: "low" },
                                    { value: "medium", label: "medium" },
                                    { value: "high", label: "high" },
                                    { value: "xhigh", label: "xhigh" },
                                    { value: "max", label: "max" },
                                  ]}
                                  onChange={(effort) => {
                                    updateModelReasoning(model.id, {
                                      effort: (effort || null) as ReasoningEffortId | null,
                                    });
                                  }}
                                />
                              </Field>
                              <Field
                                label={t("settings.reasoning.budget")}
                                hint={protocolReasoningHint(draft.protocol)}
                              >
                                <Input
                                  type="number"
                                  value={model.reasoning?.budget_tokens?.toString() ?? ""}
                                  placeholder={t("settings.reasoning.budgetPlaceholder")}
                                  onChange={(event) => {
                                    const raw = event.target.value.trim();
                                    updateModelReasoning(model.id, {
                                      budget_tokens:
                                        raw === "" ? null : Number(raw),
                                    });
                                  }}
                                />
                              </Field>
                            </>
                          )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <div style={{ marginTop: 16 }}>
            <button
              type="button"
              className="mprism-linkish"
              onClick={() => setAuthOpen((v) => !v)}
              style={{
                background: "none",
                border: "none",
                padding: 0,
                cursor: "pointer",
                color: "inherit",
                font: "inherit",
                textAlign: "left",
              }}
            >
              <h3 className="mprism-title3" style={{ margin: 0 }}>
                {t("settings.auth.title")} {authOpen ? "▾" : "▸"}
              </h3>
            </button>
            <p className="mprism-body mprism-muted">{t("settings.auth.advanced")}</p>
            {authOpen && (
              <div className="mprism-form-grid" style={{ marginTop: 8 }}>
                <Alert type="warning">{t("settings.auth.warning")}</Alert>
                <Field label={t("settings.auth.extraHeaders")}>
                  {!customHeadersSupported ? (
                    <p className="mprism-muted">{t("settings.auth.headersUnsupported")}</p>
                  ) : (
                    <div className="mprism-form-grid">
                      {draft.extra_headers.map((header, index) => (
                        <div key={`hdr-${index}`} className="mprism-key-row">
                          <Input
                            value={header.name}
                            placeholder={t("settings.auth.headerName")}
                            onChange={(event) => {
                              const next = draft.extra_headers.map((item, i) =>
                                i === index ? { ...item, name: event.target.value } : item,
                              );
                              updateDraft({ extra_headers: next });
                            }}
                          />
                          <Input
                            value={header.value}
                            placeholder={t("settings.auth.headerValue")}
                            onChange={(event) => {
                              const next = draft.extra_headers.map((item, i) =>
                                i === index ? { ...item, value: event.target.value } : item,
                              );
                              updateDraft({ extra_headers: next });
                            }}
                          />
                          <Tooltip content={t("settings.auth.removeHeader")} placement="bottom">
                            <Button
                              variant="ghost"
                              icon={<IconTrash size={16} />}
                              aria-label={t("settings.auth.removeHeader")}
                              onClick={() => {
                                updateDraft({
                                  extra_headers: draft.extra_headers.filter((_, i) => i !== index),
                                });
                              }}
                            />
                          </Tooltip>
                        </div>
                      ))}
                      <div className="mprism-action-row">
                        <Button
                          variant="outline"
                          onClick={() =>
                            updateDraft({
                              extra_headers: [...draft.extra_headers, { name: "", value: "" }],
                            })
                          }
                        >
                          {t("settings.auth.addHeader")}
                        </Button>
                      </div>
                    </div>
                  )}
                </Field>
                <Field
                  label={t("settings.auth.apiKeyQuery")}
                  hint={
                    apiKeyQuerySupported
                      ? t("settings.auth.apiKeyQueryHint")
                      : t("settings.auth.queryUnsupported")
                  }
                >
                  <Input
                    value={draft.api_key_query_param ?? ""}
                    disabled={!apiKeyQuerySupported}
                    placeholder={t("settings.auth.apiKeyQueryPlaceholder")}
                    onChange={(event) =>
                      updateDraft({
                        api_key_query_param: event.target.value,
                      })
                    }
                  />
                </Field>
              </div>
            )}
          </div>

          <div style={{ marginTop: 16 }}>
            <h3 className="mprism-title3">{t("settings.tools.title")}</h3>
            <p className="mprism-body mprism-muted">{t("settings.tools.hint")}</p>
            {!toolsSupported ? (
              <Alert type="warning">{t("settings.tools.unsupported")}</Alert>
            ) : (
              <div className="mprism-form-grid">
                <Field label={t("settings.tools.jsonLabel")} hint={t("settings.tools.hint")}>
                  <Textarea
                    rows={8}
                    value={toolsJsonText}
                    placeholder={t("settings.tools.jsonPlaceholder")}
                    onChange={(event) => {
                      setToolsJsonText(event.target.value);
                      setToolsJsonError(null);
                    }}
                  />
                </Field>
                <div className="mprism-action-row">
                  <Button
                    variant="outline"
                    onClick={() => {
                      const parsed = parseToolsJsonText(toolsJsonText);
                      if (!parsed.ok) {
                        setToolsJsonError(parsed.message);
                        return;
                      }
                      setToolsJsonError(null);
                      const nextChoice =
                        parsed.tools.length === 0
                          ? null
                          : draft.tool_choice ?? { mode: "auto" as ToolChoiceModeId, name: null };
                      updateDraft({
                        tools: parsed.tools,
                        tool_choice: nextChoice,
                      });
                    }}
                  >
                    {t("settings.tools.applyJson")}
                  </Button>
                </div>
                {toolsJsonError && <Alert type="error">{toolsJsonError}</Alert>}
                <Field label={t("settings.tools.toolChoice")}>
                  <Select
                    placement="auto"
                    value={normalizeToolChoiceMode(draft.tool_choice?.mode)}
                    disabled={draft.tools.length === 0}
                    options={[
                      { value: "auto", label: t("settings.tools.choiceAuto") },
                      { value: "none", label: t("settings.tools.choiceNone") },
                      { value: "required", label: t("settings.tools.choiceRequired") },
                      { value: "named", label: t("settings.tools.choiceNamed") },
                    ]}
                    onChange={(mode) => {
                      if (
                        mode === "auto" ||
                        mode === "none" ||
                        mode === "required" ||
                        mode === "named"
                      ) {
                        updateDraft({
                          tool_choice: {
                            mode: mode as ToolChoiceModeId,
                            name:
                              mode === "named"
                                ? draft.tool_choice?.name || draft.tools[0]?.name || null
                                : null,
                          },
                        });
                      }
                    }}
                  />
                </Field>
                {normalizeToolChoiceMode(draft.tool_choice?.mode) === "named" && (
                  <Field label={t("settings.tools.namedTool")}>
                    <Select
                      placement="auto"
                      value={draft.tool_choice?.name || draft.tools[0]?.name || ""}
                      options={draft.tools.map((tool) => ({
                        value: tool.name,
                        label: tool.name,
                      }))}
                      onChange={(name) => {
                        updateDraft({
                          tool_choice: {
                            mode: "named",
                            name,
                          },
                        });
                      }}
                    />
                  </Field>
                )}
              </div>
            )}
          </div>

          {(discovered.length > 0 || discoverError) && (
            <div>
              <h3 className="mprism-title3" style={{ marginTop: 8 }}>
                {t("settings.models.discoverResults")}
              </h3>
              {discoverError && (
                <Alert type="warning">
                  {t("settings.models.discoverFailed")} {discoverError}
                </Alert>
              )}
              <Field label={t("settings.models.search")}>
                <Input
                  prefix={<IconSearch size={16} />}
                  value={modelSearch}
                  onChange={(event) => setModelSearch(event.target.value)}
                />
              </Field>
              <div
                className="mprism-discover-list"
                aria-label={t("settings.models.discoverResults")}
              >
                {filteredDiscovered.map((model) => {
                  const checked = draft.models.some((m) => m.id === model.id);
                  return (
                    <label key={model.id} className="mprism-discover-row">
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(event) => toggleDiscovered(model, event.target.checked)}
                        aria-label={model.id}
                      />
                      <span className="mprism-ellipsis">{model.display_name}</span>
                      <span className="mprism-model-id">{model.id}</span>
                    </label>
                  );
                })}
              </div>
            </div>
          )}
        </section>
      )}

      <Modal
        open={unsavedOpen}
        title={t("settings.form.unsavedTitle")}
        onClose={() => setUnsavedOpen(false)}
        footer={
          <>
            <Button variant="default" onClick={() => setUnsavedOpen(false)}>
              {t("settings.form.cancel")}
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                markClean();
                setUnsavedOpen(false);
                if (pendingSelect === "draft-new") {
                  startDraft();
                } else {
                  selectProvider(pendingSelect === "draft" ? "draft" : pendingSelect);
                }
                setPendingSelect(null);
                setShowKey(false);
              }}
            >
              {t("settings.form.discard")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                void handleSave().then((ok) => {
                  if (!ok) {
                    return;
                  }
                  setUnsavedOpen(false);
                  if (pendingSelect === "draft-new") {
                    startDraft();
                  } else if (pendingSelect !== null) {
                    selectProvider(pendingSelect === "draft" ? "draft" : pendingSelect);
                  }
                  setPendingSelect(null);
                });
              }}
            >
              {t("settings.form.saveAndContinue")}
            </Button>
          </>
        }
      >
        {t("settings.form.unsavedBody")}
      </Modal>

      <Modal
        open={deleteOpen}
        title={t("settings.form.deleteConfirmTitle")}
        onClose={() => setDeleteOpen(false)}
        footer={
          <>
            <Button variant="default" onClick={() => setDeleteOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                void deleteSelectedProvider().then((ok) => {
                  if (ok) {
                    setDeleteOpen(false);
                  }
                });
              }}
            >
              {t("settings.form.confirmDelete")}
            </Button>
          </>
        }
      >
        {t("settings.form.deleteConfirmBody")}
      </Modal>

      <Modal
        open={manualOpen}
        title={t("settings.models.manualTitle")}
        onClose={() => setManualOpen(false)}
        footer={
          <>
            <Button variant="default" onClick={() => setManualOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="outline" onClick={addManual}>
              {t("settings.models.manualSubmit")}
            </Button>
          </>
        }
      >
        <div className="mprism-form-grid">
          <Field label={t("settings.models.manualId")} required horizontal>
            <Input value={manualId} onChange={(event) => setManualId(event.target.value)} />
          </Field>
          <Field label={t("settings.models.manualName")} horizontal>
            <Input value={manualName} onChange={(event) => setManualName(event.target.value)} />
          </Field>
          {manualError && <Alert type="error">{manualError}</Alert>}
        </div>
      </Modal>
    </div>
  );
}
