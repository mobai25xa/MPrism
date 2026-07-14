import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeSanitize from "rehype-sanitize";
import rehypeHighlight from "rehype-highlight";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../../i18n";
import { useAppStore, resolveSelection, streamingAssistantMessage } from "../../app/store";
import type { MessageRecord, SessionMeta } from "../../lib/types";
import { relativeTime, shouldSendOnEnter } from "./streamReducer";
import {
  Alert,
  Button,
  Dropdown,
  Field,
  OverflowText,
  IconArrowDown,
  IconChevronLeft,
  IconChevronRight,
  IconDoc,
  IconPlus,
  IconRename,
  IconSend,
  IconStop,
  IconTrash,
  Input,
  Modal,
  Select,
  Spinner,
  Textarea,
  Tooltip,
  cx,
} from "../../ui";
import "../../styles/highlight-theme.css";

const SIDEBAR_COLLAPSED_KEY = "mprism.chat.sidebarCollapsed";
const SIDEBAR_WIDTH_KEY = "mprism.chat.sidebarWidth";
const SIDEBAR_WIDTH_DEFAULT = 260;
const SIDEBAR_WIDTH_MIN = 200;
const SIDEBAR_WIDTH_MAX = 480;
const ANCHOR_TOOLTIP_MAX = 120;
const ANCHOR_FULL_TEXT_MAX = 4000;

function outlineFullText(content: string): string {
  const text = content.replace(/\s+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
  if (!text) {
    return "…";
  }
  if (text.length <= ANCHOR_FULL_TEXT_MAX) {
    return text;
  }
  return `${text.slice(0, ANCHOR_FULL_TEXT_MAX)}…`;
}

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

function outlineTitle(content: string): string {
  const oneLine = content.replace(/\s+/g, " ").trim();
  if (!oneLine) {
    return "…";
  }
  if (oneLine.length <= ANCHOR_TOOLTIP_MAX) {
    return oneLine;
  }
  return `${oneLine.slice(0, ANCHOR_TOOLTIP_MAX)}…`;
}

function CodeBlock({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  const [copied, setCopied] = useState(false);
  const language = /language-(\w+)/.exec(className ?? "")?.[1] ?? "";
  const text = String(children).replace(/\n$/, "");
  return (
    <div>
      <div className="mprism-code-head">
        <span>{language || "code"}</span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            void navigator.clipboard.writeText(text).then(() => {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1200);
            });
          }}
        >
          {copied ? t("chat.copied") : t("chat.copyCode")}
        </Button>
      </div>
      <pre className={className}>
        <code className={className}>{children}</code>
      </pre>
    </div>
  );
}

function TypingDots() {
  return (
    <span className="mprism-typing" role="status" aria-label={t("chat.generating")}>
      <span className="mprism-typing__dot" />
      <span className="mprism-typing__dot" />
      <span className="mprism-typing__dot" />
    </span>
  );
}

function AssistantBody({ message, streaming }: { message: MessageRecord; streaming: boolean }) {
  const open = streaming && !!(message.reasoning && message.reasoning.length > 0);
  const showTyping = streaming && !message.content;
  return (
    <div className="mprism-assistant">
      <div className="mprism-assistant-meta">{message.model?.display_name ?? "assistant"}</div>
      {!!message.reasoning && (
        <details className="mprism-reasoning" open={open || undefined}>
          <summary>{t("chat.reasoning")}</summary>
          <div className="mprism-reasoning-body">{message.reasoning}</div>
        </details>
      )}
      {showTyping ? (
        <TypingDots />
      ) : message.content ? (
        <div className="mprism-markdown">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            rehypePlugins={[rehypeSanitize, rehypeHighlight]}
            components={{
              a: ({ href, children }) => (
                <a
                  href={href}
                  onClick={(event) => {
                    event.preventDefault();
                    if (!href) {
                      return;
                    }
                    if (!/^https?:|^mailto:/i.test(href)) {
                      return;
                    }
                    void openUrl(href).catch(() => {
                      window.open(href, "_blank", "noopener,noreferrer");
                    });
                  }}
                >
                  {children}
                </a>
              ),
              pre: ({ children }) => <>{children}</>,
              code: ({ className, children, ...props }) => {
                const isBlock = typeof className === "string" && className.includes("language-");
                if (isBlock) {
                  return <CodeBlock className={className}>{children}</CodeBlock>;
                }
                return (
                  <code className={className} {...props}>
                    {children}
                  </code>
                );
              },
              img: ({ alt, src }) => <span>{src ? `[image](${src})` : alt}</span>,
            }}
          >
            {message.content}
          </ReactMarkdown>
        </div>
      ) : null}
      {message.status === "stopped" && (
        <div className="mprism-muted">{t("chat.stopped")}</div>
      )}
      {message.status === "error" && (
        <Alert type="error">
          {t("chat.error")}: {message.error?.message ?? ""}
        </Alert>
      )}
    </div>
  );
}

function SessionListItem({
  session,
  active,
  onSelect,
  onRename,
  onDelete,
}: {
  session: SessionMeta;
  active: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(session.title);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPoint, setMenuPoint] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    setTitle(session.title);
  }, [session.title]);

  if (editing) {
    return (
      <div style={{ padding: 4 }}>
        <Input
          value={title}
          autoFocus
          onChange={(event) => setTitle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              onRename(title.trim() || session.title);
              setEditing(false);
            }
            if (event.key === "Escape") {
              setTitle(session.title);
              setEditing(false);
            }
          }}
          onBlur={() => {
            onRename(title.trim() || session.title);
            setEditing(false);
          }}
        />
      </div>
    );
  }

  return (
    <Dropdown
      block
      open={menuOpen}
      onOpenChange={setMenuOpen}
      placement="context"
      contextPoint={menuPoint}
      items={[
        { key: "rename", label: t("sessions.rename"), icon: <IconRename size={16} /> },
        { key: "delete", label: t("sessions.delete"), icon: <IconTrash size={16} />, danger: true },
      ]}
      onSelect={(key) => {
        if (key === "rename") {
          setEditing(true);
        }
        if (key === "delete") {
          onDelete();
        }
      }}
    >
      <button
        type="button"
        className={cx("myui-app-nav__item", active && "is-active")}
        onClick={onSelect}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenuPoint({ x: event.clientX, y: event.clientY });
          setMenuOpen(true);
        }}
      >
        <div className="myui-app-nav__item-main">
          <div className="myui-app-nav__item-text">
            <OverflowText text={session.title} className="myui-app-nav__item-title" placement="right" />
            <div className="myui-app-nav__item-meta">
              {relativeTime(session.updated_at)}
            </div>
          </div>
        </div>
      </button>
    </Dropdown>
  );
}

export function ChatWorkspace() {
  const providers = useAppStore((s) => s.providers);
  const sessions = useAppStore((s) => s.sessions);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const messagesBySession = useAppStore((s) => s.messagesBySession);
  const generations = useAppStore((s) => s.generations);
  const draftsBySession = useAppStore((s) => s.draftsBySession);
  const defaultProviderId = useAppStore((s) => s.defaultProviderId);
  const defaultModelId = useAppStore((s) => s.defaultModelId);
  const chatLoading = useAppStore((s) => s.chatLoading);
  const partiallyCorruptBySession = useAppStore((s) => s.partiallyCorruptBySession);
  const setPage = useAppStore((s) => s.setPage);
  const createSession = useAppStore((s) => s.createSession);
  const selectSession = useAppStore((s) => s.selectSession);
  const renameSession = useAppStore((s) => s.renameSession);
  const deleteSession = useAppStore((s) => s.deleteSession);
  const updateSystemPrompt = useAppStore((s) => s.updateSystemPrompt);
  const updateSessionSelection = useAppStore((s) => s.updateSessionSelection);
  const setComposerDraft = useAppStore((s) => s.setComposerDraft);
  const sendMessage = useAppStore((s) => s.sendMessage);
  const stopGeneration = useAppStore((s) => s.stopGeneration);

  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [promptOpen, setPromptOpen] = useState(false);
  const [promptDraft, setPromptDraft] = useState("");
  const [stickToBottom, setStickToBottom] = useState(true);
  const listRef = useRef<HTMLDivElement | null>(null);
  const anchorRailRef = useRef<HTMLDivElement | null>(null);
  const outlineTipHideTimerRef = useRef<number | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const resizeDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const [activeAnchorId, setActiveAnchorId] = useState<string | null>(null);
  const [outlineTipId, setOutlineTipId] = useState<string | null>(null);
  const [outlineTipBox, setOutlineTipBox] = useState<{
    top: number;
    left: number;
    maxWidth: number;
  } | null>(null);
  const anchorJumpingRef = useRef(false);
  const anchorJumpRafRef = useRef<number | null>(null);

  const clearOutlineTipHideTimer = () => {
    if (outlineTipHideTimerRef.current !== null) {
      window.clearTimeout(outlineTipHideTimerRef.current);
      outlineTipHideTimerRef.current = null;
    }
  };

  const showOutlineTip = (questionId: string) => {
    clearOutlineTipHideTimer();
    setOutlineTipId(questionId);
  };

  const scheduleHideOutlineTip = () => {
    clearOutlineTipHideTimer();
    outlineTipHideTimerRef.current = window.setTimeout(() => {
      setOutlineTipId(null);
      setOutlineTipBox(null);
      outlineTipHideTimerRef.current = null;
    }, 120);
  };

  const setSidebar = (collapsed: boolean) => {
    setSidebarCollapsed(collapsed);
    writeSidebarCollapsed(collapsed);
  };

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

  const hasProvider = providers.length > 0;
  const hasModel = providers.some((p) => p.models.length > 0);
  const activeSession = sessions.find((s) => s.id === activeSessionId) ?? null;
  const selection = resolveSelection(
    providers,
    defaultProviderId,
    defaultModelId,
    activeSession,
  );
  const provider = providers.find((p) => p.id === selection.providerId) ?? null;
  const models = provider?.models ?? [];
  const generation = activeSessionId ? generations[activeSessionId] : undefined;
  const baseMessages = activeSessionId ? messagesBySession[activeSessionId] ?? [] : [];
  const messages = useMemo(() => {
    if (!generation) {
      return baseMessages;
    }
    const filtered = baseMessages.filter((m) => m.id !== generation.assistantMessageId);
    return [...filtered, streamingAssistantMessage(generation)];
  }, [baseMessages, generation]);
  const userQuestions = useMemo(
    () => messages.filter((message) => message.role === "user"),
    [messages],
  );
  const outlineTipText = useMemo(() => {
    if (!outlineTipId) {
      return "";
    }
    const hit = userQuestions.find((q) => q.id === outlineTipId);
    return hit ? outlineFullText(hit.content) : "";
  }, [outlineTipId, userQuestions]);

  useLayoutEffect(() => {
    if (!outlineTipId || !anchorRailRef.current) {
      setOutlineTipBox(null);
      return;
    }
    const card = anchorRailRef.current;
    const item = card.querySelector<HTMLElement>(`[data-rail-id="${outlineTipId}"]`);
    if (!item) {
      setOutlineTipBox(null);
      return;
    }

    const updateBox = () => {
      const cardRect = card.getBoundingClientRect();
      const itemRect = item.getBoundingClientRect();
      const gap = 10;
      const maxWidth = Math.min(280, Math.max(160, window.innerWidth * 0.36));
      let left = cardRect.left - gap - maxWidth;
      left = Math.max(8, left);
      let top = itemRect.top;
      const maxTop = window.innerHeight - 8 - 48;
      top = Math.min(Math.max(8, top), Math.max(8, maxTop));
      setOutlineTipBox({ top, left, maxWidth });
    };

    updateBox();
    const onScrollOrResize = () => updateBox();
    window.addEventListener("resize", onScrollOrResize);
    window.addEventListener("scroll", onScrollOrResize, true);
    card.addEventListener("scroll", onScrollOrResize);
    return () => {
      window.removeEventListener("resize", onScrollOrResize);
      window.removeEventListener("scroll", onScrollOrResize, true);
      card.removeEventListener("scroll", onScrollOrResize);
    };
  }, [outlineTipId, userQuestions.length]);

  useEffect(() => {
    return () => clearOutlineTipHideTimer();
  }, []);

  const draft = activeSessionId ? draftsBySession[activeSessionId] ?? "" : "";
  const canSend =
    !!activeSessionId &&
    !!selection.providerId &&
    !!selection.modelId &&
    draft.trim().length > 0 &&
    !generation;

  const updateActiveAnchor = () => {
    const container = listRef.current;
    if (!container || anchorJumpingRef.current) {
      return;
    }
    const nodes = container.querySelectorAll<HTMLElement>("[data-anchor-id]");
    if (nodes.length === 0) {
      setActiveAnchorId(null);
      return;
    }
    const marker = container.scrollTop + 96;
    let nextId = nodes[0]?.dataset.anchorId ?? null;
    for (const node of nodes) {
      if (node.offsetTop <= marker) {
        nextId = node.dataset.anchorId ?? nextId;
      } else {
        break;
      }
    }
    setActiveAnchorId(nextId);
  };

  const finishAnchorJump = () => {
    if (anchorJumpRafRef.current !== null) {
      window.cancelAnimationFrame(anchorJumpRafRef.current);
      anchorJumpRafRef.current = null;
    }
    anchorJumpingRef.current = false;
    const container = listRef.current;
    if (container) {
      const distance = container.scrollHeight - container.scrollTop - container.clientHeight;
      setStickToBottom(distance < 80);
    }
    updateActiveAnchor();
  };

  const scrollToQuestion = (messageId: string) => {
    const container = listRef.current;
    const target = container?.querySelector<HTMLElement>(`[data-anchor-id="${messageId}"]`);
    if (!container || !target) {
      return;
    }
    if (anchorJumpRafRef.current !== null) {
      window.cancelAnimationFrame(anchorJumpRafRef.current);
      anchorJumpRafRef.current = null;
    }
    anchorJumpingRef.current = true;
    setStickToBottom(false);
    setActiveAnchorId(messageId);

    const maxTop = Math.max(0, container.scrollHeight - container.clientHeight);
    const containerRect = container.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const nextTop = Math.min(
      Math.max(0, container.scrollTop + (targetRect.top - containerRect.top) - 12),
      maxTop,
    );
    container.scrollTo({ top: nextTop, behavior: "smooth" });

    const startedAt = performance.now();
    const tick = () => {
      const el = listRef.current;
      if (!el) {
        finishAnchorJump();
        return;
      }
      const settled = Math.abs(el.scrollTop - nextTop) < 2;
      const timedOut = performance.now() - startedAt > 1200;
      if (settled || timedOut) {
        finishAnchorJump();
        return;
      }
      anchorJumpRafRef.current = window.requestAnimationFrame(tick);
    };
    anchorJumpRafRef.current = window.requestAnimationFrame(tick);
  };

  useEffect(() => {
    return () => {
      if (anchorJumpRafRef.current !== null) {
        window.cancelAnimationFrame(anchorJumpRafRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!stickToBottom || !listRef.current || anchorJumpingRef.current) {
      return;
    }
    listRef.current.scrollTop = listRef.current.scrollHeight;
  }, [messages, generation?.content, generation?.reasoning, stickToBottom]);

  useEffect(() => {
    updateActiveAnchor();
  }, [messages, activeSessionId]);

  useEffect(() => {
    const rail = anchorRailRef.current;
    if (!rail || userQuestions.length === 0) {
      return;
    }
    if (activeAnchorId) {
      const target = rail.querySelector<HTMLElement>(`[data-rail-id="${activeAnchorId}"]`);
      if (target) {
        const top = target.offsetTop;
        const bottom = top + target.offsetHeight;
        if (top < rail.scrollTop) {
          rail.scrollTop = top;
        } else if (bottom > rail.scrollTop + rail.clientHeight) {
          rail.scrollTop = bottom - rail.clientHeight;
        }
        return;
      }
    }
    rail.scrollTop = rail.scrollHeight;
  }, [userQuestions.length, activeAnchorId]);

  if (!hasProvider) {
    return (
      <div className="mprism-empty">
        <h3 className="mprism-title3">{t("chat.needProviderTitle")}</h3>
        <p className="mprism-body mprism-muted">{t("chat.needProviderBody")}</p>
        <Button variant="primary" onClick={() => setPage("settings")}>
          {t("chat.goSettings")}
        </Button>
      </div>
    );
  }

  if (!hasModel) {
    return (
      <div className="mprism-empty">
        <h3 className="mprism-title3">{t("chat.needModel")}</h3>
        <Button variant="primary" onClick={() => setPage("settings")}>
          {t("chat.goSettings")}
        </Button>
      </div>
    );
  }

  return (
    <div className="mprism-shell">
      {sidebarCollapsed ? (
        <aside className="myui-app-nav myui-app-nav--collapsed" aria-label={t("sessions.title")}>
          <Tooltip content={t("sessions.expand")} placement="bottom">
            <Button
              variant="ghost"
              icon={<IconChevronRight />}
              aria-label={t("sessions.expand")}
              onClick={() => setSidebar(false)}
            />
          </Tooltip>
          <Tooltip content={t("chat.newSession")} placement="bottom">
            <Button
              variant="ghost"
              icon={<IconPlus />}
              aria-label={t("chat.newSession")}
              onClick={() => void createSession()}
            />
          </Tooltip>
        </aside>
      ) : (
        <aside
          className="myui-app-nav myui-app-nav--resizable"
          style={{ width: sidebarWidth, minWidth: SIDEBAR_WIDTH_MIN, maxWidth: SIDEBAR_WIDTH_MAX }}
          aria-label={t("sessions.title")}
        >
          <div
            className="mprism-resize-handle"
            role="separator"
            aria-orientation="vertical"
            aria-label={t("sessions.resize")}
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
            <h3 className="mprism-title3">{t("sessions.title")}</h3>
            <div className="myui-app-nav__head-actions">
              <Tooltip content={t("chat.newSession")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconPlus />}
                  aria-label={t("chat.newSession")}
                  onClick={() => void createSession()}
                />
              </Tooltip>
              <Tooltip content={t("sessions.collapse")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconChevronLeft />}
                  aria-label={t("sessions.collapse")}
                  onClick={() => setSidebar(true)}
                />
              </Tooltip>
            </div>
          </div>
          <div className="myui-app-nav__body">
            {sessions.length === 0 ? (
              <div style={{ padding: 8 }} className="mprism-muted">
                {t("sessions.empty")}
              </div>
            ) : (
              sessions.map((session) => (
                <SessionListItem
                  key={session.id}
                  session={session}
                  active={session.id === activeSessionId}
                  onSelect={() => void selectSession(session.id)}
                  onRename={(title) => void renameSession(session.id, title)}
                  onDelete={() => setDeleteId(session.id)}
                />
              ))
            )}
          </div>
        </aside>
      )}

      <section className="myui-app-main">
        {!activeSession ? (
          <div className="mprism-empty">
            <h3 className="mprism-title3">{t("chat.startSessionTitle")}</h3>
            <p className="mprism-body mprism-muted">{t("chat.startSessionBody")}</p>
            <Button variant="primary" onClick={() => void createSession()}>
              {t("chat.newSession")}
            </Button>
          </div>
        ) : (
          <>
            <div className="myui-app-main__toolbar">
              <div className="myui-app-main__title mprism-ellipsis">{activeSession.title}</div>
              <Tooltip content={t("chat.systemPrompt")} placement="bottom">
                <Button
                  variant="ghost"
                  icon={<IconDoc />}
                  aria-label={t("chat.systemPrompt")}
                  onClick={() => {
                    setPromptDraft(activeSession.system_prompt ?? "");
                    setPromptOpen(true);
                  }}
                />
              </Tooltip>
            </div>

            {partiallyCorruptBySession[activeSession.id] && (
              <Alert type="warning">{t("chat.partialCorrupt")}</Alert>
            )}

            <div className="mprism-thread-row">
              <div
                className="mprism-messages"
                ref={listRef}
                onScroll={(event) => {
                  const el = event.currentTarget;
                  if (!anchorJumpingRef.current) {
                    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
                    setStickToBottom(distance < 80);
                  }
                  updateActiveAnchor();
                }}
              >
                <div className="mprism-messages-inner">
                  {chatLoading && messages.length === 0 ? (
                    <Spinner label={t("common.loading")} />
                  ) : messages.length === 0 ? (
                    <p className="mprism-body mprism-muted">{t("chat.emptySession")}</p>
                  ) : (
                    messages.map((message) =>
                      message.role === "user" ? (
                        <div
                          key={message.id}
                          className="mprism-user-bubble"
                          data-anchor-id={message.id}
                        >
                          {message.content}
                        </div>
                      ) : (
                        <AssistantBody
                          key={message.id}
                          message={message}
                          streaming={
                            !!generation && message.id === generation.assistantMessageId
                          }
                        />
                      ),
                    )
                  )}
                  <button
                    type="button"
                    className={cx("myui-back-top", "mprism-scroll-fab", !stickToBottom && "is-visible")}
                    aria-label={t("chat.scrollToBottom")}
                    onClick={() => {
                      setStickToBottom(true);
                      if (listRef.current) {
                        listRef.current.scrollTo({
                          top: listRef.current.scrollHeight,
                          behavior: "smooth",
                        });
                      }
                    }}
                  >
                    <IconArrowDown size={18} />
                  </button>
                </div>
              </div>

              {userQuestions.length > 0 && (
                <>
                  <nav
                    ref={anchorRailRef}
                    className="mprism-chat-outline"
                    aria-label={t("chat.outline")}
                    onMouseLeave={() => {
                      scheduleHideOutlineTip();
                    }}
                  >
                    <div
                      className="myui-timeline myui-timeline--right mprism-chat-outline__timeline"
                      role="list"
                    >
                      {userQuestions.map((question, index) => {
                        const active = activeAnchorId === question.id;
                        const isLast = index === userQuestions.length - 1;
                        const title = outlineTitle(question.content);
                        return (
                          <div
                            key={question.id}
                            role="listitem"
                            className={cx(
                              "myui-timeline__item",
                              active
                                ? "myui-timeline__item--process"
                                : "myui-timeline__item--default",
                              outlineTipId === question.id && "is-tip-open",
                            )}
                            data-rail-id={question.id}
                            data-status={active ? "process" : "default"}
                            onMouseEnter={() => showOutlineTip(question.id)}
                            onFocusCapture={() => showOutlineTip(question.id)}
                          >
                            <div
                              className={cx("myui-timeline__tail", isLast && "is-last")}
                              aria-hidden
                            />
                            <div className="myui-timeline__dot" aria-hidden />
                            <div className="myui-timeline__body">
                              <button
                                type="button"
                                className="myui-timeline__content mprism-chat-outline__content"
                                aria-current={active ? "true" : undefined}
                                aria-label={title}
                                onClick={() => scrollToQuestion(question.id)}
                              >
                                {title}
                              </button>
                            </div>
                            <button
                              type="button"
                              className="mprism-chat-outline__dot-hit"
                              aria-label={title}
                              aria-current={active ? "true" : undefined}
                              onClick={() => scrollToQuestion(question.id)}
                            />
                          </div>
                        );
                      })}
                    </div>
                  </nav>
                  {outlineTipId &&
                    outlineTipBox &&
                    outlineTipText &&
                    createPortal(
                      <div
                        className="myui-tooltip__popper is-open mprism-chat-outline-fulltip-popper"
                        role="tooltip"
                        style={{
                          position: "fixed",
                          top: outlineTipBox.top,
                          left: outlineTipBox.left,
                          width: outlineTipBox.maxWidth,
                          zIndex: "var(--myui-z-index-tooltip)" as unknown as number,
                        }}
                        onMouseEnter={() => {
                          clearOutlineTipHideTimer();
                          if (outlineTipId) {
                            showOutlineTip(outlineTipId);
                          }
                        }}
                        onMouseLeave={() => scheduleHideOutlineTip()}
                      >
                        <div className="mprism-chat-outline-fulltip">{outlineTipText}</div>
                      </div>,
                      document.body,
                    )}
                </>
              )}
            </div>

            <div className="mprism-composer-wrap">
              <div className="mprism-composer-box">
                <Textarea
                  className="mprism-textarea"
                  placeholder={t("chat.composerPlaceholder")}
                  value={draft}
                  disabled={!selection.providerId || !selection.modelId}
                  onChange={(event) => setComposerDraft(activeSession.id, event.target.value)}
                  onKeyDown={(event) => {
                    if (shouldSendOnEnter(event)) {
                      event.preventDefault();
                      if (canSend) {
                        void sendMessage(activeSession.id, draft);
                      }
                    }
                  }}
                />
                <div className="mprism-composer-toolbar">
                  <Select
                    className="mprism-composer-select"
                    size="sm"
                    placement="top"
                    placeholder={t("chat.selectProvider")}
                    value={selection.providerId ?? ""}
                    options={providers.map((p) => ({ value: p.id, label: p.name }))}
                    onChange={(providerId) => {
                      const nextProvider = providers.find((p) => p.id === providerId);
                      const modelId = nextProvider?.models[0]?.id ?? null;
                      void updateSessionSelection(activeSession.id, providerId, modelId);
                    }}
                  />
                  <Select
                    className="mprism-composer-select"
                    size="sm"
                    placement="top"
                    placeholder={t("chat.selectModel")}
                    value={selection.modelId ?? ""}
                    options={models.map((m) => ({ value: m.id, label: m.display_name }))}
                    onChange={(modelId) => {
                      void updateSessionSelection(
                        activeSession.id,
                        selection.providerId,
                        modelId,
                      );
                    }}
                  />
                  <div className="mprism-composer-spacer" />
                  {generation ? (
                    <Button
                      variant="primary"
                      size="sm"
                      icon={<IconStop size={14} />}
                      aria-label={t("chat.stop")}
                      disabled={generation.phase === "cancelling"}
                      onClick={() => void stopGeneration(activeSession.id)}
                    >
                      {generation.phase === "cancelling" ? t("chat.stopping") : t("chat.stop")}
                    </Button>
                  ) : (
                    <Button
                      variant="primary"
                      size="sm"
                      icon={<IconSend size={14} />}
                      aria-label={t("chat.send")}
                      disabled={!canSend}
                      onClick={() => void sendMessage(activeSession.id, draft)}
                    >
                      {t("chat.send")}
                    </Button>
                  )}
                </div>
              </div>
            </div>
          </>
        )}
      </section>

      <Modal
        open={!!deleteId}
        title={t("sessions.deleteTitle")}
        onClose={() => setDeleteId(null)}
        footer={
          <>
            <Button variant="default" onClick={() => setDeleteId(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                if (deleteId) {
                  void deleteSession(deleteId).then(() => setDeleteId(null));
                }
              }}
            >
              {t("sessions.confirmDelete")}
            </Button>
          </>
        }
      >
        {t("sessions.deleteBody")}
      </Modal>

      <Modal
        open={promptOpen}
        title={t("chat.systemPrompt")}
        onClose={() => setPromptOpen(false)}
        footer={
          <>
            <Button variant="default" onClick={() => setPromptOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                if (!activeSession) {
                  return;
                }
                void updateSystemPrompt(activeSession.id, promptDraft).then((ok) => {
                  if (ok) {
                    setPromptOpen(false);
                  }
                });
              }}
            >
              {t("chat.systemPromptSave")}
            </Button>
          </>
        }
      >
        <Field
          label={t("chat.systemPrompt")}
          hint={`${promptDraft.length} / 32000 · ${t("chat.systemPromptHint")}`}
        >
          <Textarea
            value={promptDraft}
            onChange={(event) => setPromptDraft(event.target.value.slice(0, 32000))}
            style={{ minHeight: 160 }}
          />
        </Field>
      </Modal>
    </div>
  );
}
