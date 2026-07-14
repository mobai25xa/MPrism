import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { CSSProperties, ReactNode } from "react";
import { cx } from "./cx";

export type DropdownItem = {
  key: string;
  label: ReactNode;
  disabled?: boolean;
  danger?: boolean;
  icon?: ReactNode;
};

export type DropdownPlacement =
  | "bottom-start"
  | "bottom-end"
  | "top-start"
  | "top-end"
  | "auto"
  | "context";

export type UiDropdownProps = {
  items: DropdownItem[];
  children: ReactNode;
  onSelect?: (key: string) => void;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  placement?: DropdownPlacement;
  contextPoint?: { x: number; y: number } | null;
  className?: string;
  /** Stretch to parent width (session/provider list cards). */
  block?: boolean;
};

type MenuBox = {
  top: number;
  left: number;
  minWidth: number;
  transform: string;
};

const GAP = 4;
const VIEW_PAD = 8;

function resolveAutoVertical(trigger: DOMRect, estimatedHeight = 140): "top" | "bottom" {
  const spaceBelow = window.innerHeight - trigger.bottom - VIEW_PAD;
  const spaceAbove = trigger.top - VIEW_PAD;
  if (spaceBelow >= estimatedHeight + GAP) {
    return "bottom";
  }
  if (spaceAbove >= estimatedHeight + GAP) {
    return "top";
  }
  return spaceAbove > spaceBelow ? "top" : "bottom";
}

function computeMenuBox(
  trigger: DOMRect,
  placement: Exclude<DropdownPlacement, "context" | "auto"> | "top-start" | "bottom-start",
  menuWidth: number,
): MenuBox {
  const minWidth = Math.max(trigger.width, menuWidth, 140);
  const alignEnd = placement.endsWith("end");
  const openTop = placement.startsWith("top");

  let left = alignEnd ? trigger.right - minWidth : trigger.left;
  left = Math.min(Math.max(VIEW_PAD, left), window.innerWidth - minWidth - VIEW_PAD);

  if (openTop) {
    return {
      top: trigger.top - GAP,
      left,
      minWidth,
      transform: "translateY(-100%)",
    };
  }

  return {
    top: trigger.bottom + GAP,
    left,
    minWidth,
    transform: "none",
  };
}

export function Dropdown({
  items,
  children,
  onSelect,
  open: controlledOpen,
  onOpenChange,
  placement = "bottom-start",
  contextPoint = null,
  className,
  block = false,
}: UiDropdownProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const setOpen = (next: boolean) => {
    onOpenChange?.(next);
    if (controlledOpen === undefined) {
      setUncontrolledOpen(next);
    }
  };

  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLUListElement | null>(null);
  const [menuBox, setMenuBox] = useState<MenuBox | null>(null);

  const updateGeometry = () => {
    if (placement === "context") {
      if (!contextPoint) {
        setMenuBox(null);
        return;
      }
      setMenuBox({
        top: contextPoint.y,
        left: contextPoint.x,
        minWidth: 140,
        transform: "none",
      });
      return;
    }

    const trigger = triggerRef.current;
    if (!trigger) {
      return;
    }
    const rect = trigger.getBoundingClientRect();
    const estimated = menuRef.current?.offsetHeight ?? items.length * 36 + 16;
    const vertical =
      placement === "auto"
        ? resolveAutoVertical(rect, estimated)
        : placement.startsWith("top")
          ? "top"
          : "bottom";
    const align: "start" | "end" =
      placement === "auto"
        ? "start"
        : placement.endsWith("end")
          ? "end"
          : "start";
    const resolved = `${vertical}-${align}` as "top-start" | "top-end" | "bottom-start" | "bottom-end";
    const width = menuRef.current?.offsetWidth ?? 160;
    setMenuBox(computeMenuBox(rect, resolved, width));
  };

  useLayoutEffect(() => {
    if (!open) {
      setMenuBox(null);
      return;
    }
    updateGeometry();
    const raf = window.requestAnimationFrame(() => updateGeometry());
    return () => window.cancelAnimationFrame(raf);
  }, [open, placement, contextPoint, items.length]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onScrollOrResize = () => updateGeometry();
    window.addEventListener("resize", onScrollOrResize);
    window.addEventListener("scroll", onScrollOrResize, true);
    return () => {
      window.removeEventListener("resize", onScrollOrResize);
      window.removeEventListener("scroll", onScrollOrResize, true);
    };
  }, [open, placement, contextPoint, items.length]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onDoc = (event: MouseEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target) || menuRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const menuStyle: CSSProperties | undefined = menuBox
    ? {
        position: "fixed",
        top: menuBox.top,
        left: menuBox.left,
        minWidth: menuBox.minWidth,
        transform: menuBox.transform,
        zIndex: "var(--myui-dropdown-z-index)" as unknown as number,
      }
    : undefined;

  const menu =
    open && menuStyle
      ? createPortal(
          <ul
            ref={menuRef}
            className="myui-dropdown__menu"
            role="menu"
            style={menuStyle}
          >
            {items.map((item) => (
              <li
                key={item.key}
                role="menuitem"
                className={cx(
                  "myui-dropdown__item",
                  item.disabled && "is-disabled",
                  item.danger && "myui-dropdown__item--danger",
                )}
                onMouseDown={(event) => {
                  // avoid outside mousedown closing before click
                  event.preventDefault();
                }}
                onClick={() => {
                  if (item.disabled) {
                    return;
                  }
                  onSelect?.(item.key);
                  setOpen(false);
                }}
              >
                {item.icon ? <span className="mprism-menu-icon">{item.icon}</span> : null}
                <span>{item.label}</span>
              </li>
            ))}
          </ul>,
          document.body,
        )
      : null;

  return (
    <div
      ref={rootRef}
      className={cx("myui-dropdown", open && "is-open", block && "myui-dropdown--block", className)}
    >
      <div
        ref={triggerRef}
        className={cx("myui-dropdown__trigger", block && "myui-dropdown__trigger--block")}
        onClick={() => {
          if (placement !== "context") {
            setOpen(!open);
          }
        }}
      >
        {children}
      </div>
      {menu}
    </div>
  );
}
