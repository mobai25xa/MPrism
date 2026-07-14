import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { CSSProperties, ReactNode } from "react";
import { cx } from "./cx";

export type TooltipPlacement = "top" | "bottom" | "left" | "right";

export type UiTooltipProps = {
  content: ReactNode;
  children: ReactNode;
  className?: string;
  /** Default: top */
  placement?: TooltipPlacement;
  /** When true, never show the popper (layout wrapper only). */
  disabled?: boolean;
  /**
   * Render popper on document.body with fixed geometry.
   * Use inside overflow containers (sidebars) so tips are not clipped.
   */
  portal?: boolean;
};

const GAP = 8;
const VIEW_PAD = 8;

function computePortalStyle(rect: DOMRect, placement: TooltipPlacement): CSSProperties {
  switch (placement) {
    case "bottom":
      return {
        top: rect.bottom + GAP,
        left: Math.min(
          Math.max(VIEW_PAD, rect.left + rect.width / 2),
          window.innerWidth - VIEW_PAD,
        ),
        transform: "translateX(-50%)",
      };
    case "left":
      return {
        top: rect.top + rect.height / 2,
        left: Math.max(VIEW_PAD, rect.left - GAP),
        transform: "translate(-100%, -50%)",
      };
    case "right":
      return {
        top: rect.top + rect.height / 2,
        left: Math.min(window.innerWidth - VIEW_PAD, rect.right + GAP),
        transform: "translateY(-50%)",
      };
    case "top":
    default:
      return {
        top: Math.max(VIEW_PAD, rect.top - GAP),
        left: Math.min(
          Math.max(VIEW_PAD, rect.left + rect.width / 2),
          window.innerWidth - VIEW_PAD,
        ),
        transform: "translate(-50%, -100%)",
      };
  }
}

export function Tooltip({
  content,
  children,
  className,
  placement = "top",
  disabled = false,
  portal = false,
}: UiTooltipProps) {
  const [open, setOpen] = useState(false);
  const [box, setBox] = useState<CSSProperties | null>(null);
  const id = useId();
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const show = open && !disabled && content != null && content !== false && content !== "";

  const updateBox = () => {
    const el = rootRef.current;
    if (!el) {
      return;
    }
    setBox(computePortalStyle(el.getBoundingClientRect(), placement));
  };

  useLayoutEffect(() => {
    if (!show || !portal) {
      setBox(null);
      return;
    }
    updateBox();
  }, [show, portal, placement, content]);

  useEffect(() => {
    if (!show || !portal) {
      return;
    }
    const onMove = () => updateBox();
    window.addEventListener("resize", onMove);
    window.addEventListener("scroll", onMove, true);
    return () => {
      window.removeEventListener("resize", onMove);
      window.removeEventListener("scroll", onMove, true);
    };
  }, [show, portal, placement]);

  const popper = show ? (
    <span
      id={id}
      role="tooltip"
      className={cx(
        "myui-tooltip__popper",
        "is-open",
        portal ? "mprism-tooltip-portal" : "mprism-tooltip-popper",
        !portal && `mprism-tooltip-popper--${placement}`,
      )}
      style={
        portal && box
          ? {
              position: "fixed",
              zIndex: "var(--myui-z-index-tooltip)" as unknown as number,
              ...box,
            }
          : undefined
      }
    >
      {content}
    </span>
  ) : null;

  return (
    <span
      ref={rootRef}
      className={cx("myui-tooltip", className)}
      onMouseEnter={() => {
        if (!disabled) {
          setOpen(true);
        }
      }}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => {
        if (!disabled) {
          setOpen(true);
        }
      }}
      onBlur={() => setOpen(false)}
    >
      <span className="myui-tooltip__trigger" aria-describedby={show ? id : undefined}>
        {children}
      </span>
      {portal && popper ? createPortal(popper, document.body) : popper}
    </span>
  );
}
