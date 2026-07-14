import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { getSelectClassName } from "@mobai6462/components/select";
import type {
  SelectPlacement as PackageSelectPlacement,
  SelectSize,
} from "@mobai6462/components/select";
import { cx } from "./cx";
import { useOverflowTooltip } from "./useOverflowTooltip";

export type SelectOption = {
  value: string;
  label: string;
  disabled?: boolean;
};

/** Matches @mobai6462/components: bottom | top | auto */
export type SelectPlacement = PackageSelectPlacement;

export type UiSelectProps = {
  options: SelectOption[];
  value?: string;
  placeholder?: string;
  disabled?: boolean;
  size?: SelectSize;
  className?: string;
  onChange?: (value: string) => void;
  "aria-label"?: string;
  /**
   * Menu direction.
   * - bottom: always down
   * - top: always up
   * - auto: prefer bottom; flip when space is insufficient
   */
  placement?: SelectPlacement;
};

const DROPDOWN_GAP = 4;
const DROPDOWN_MAX = 240;

const ARROW_SVG = (
  <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" aria-hidden>
    <path
      fill="currentColor"
      d="M6.3 9.3a1 1 0 0 1 1.4 0L12 13.58l4.3-4.3a1 1 0 1 1 1.4 1.42l-5 5a1 1 0 0 1-1.4 0l-5-5a1 1 0 0 1 0-1.42Z"
    />
  </svg>
);

const CHECK_SVG = (
  <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden>
    <path
      d="M5 12.5 10 17l9-10"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

type ResolvedPlacement = "bottom" | "top";

type PanelStyle = {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  transform: string;
};

function resolvePlacement(
  preferred: SelectPlacement,
  trigger: DOMRect,
  estimatedHeight: number,
): ResolvedPlacement {
  if (preferred === "top") {
    return "top";
  }
  if (preferred === "bottom") {
    return "bottom";
  }
  const needed = Math.min(estimatedHeight, DROPDOWN_MAX) + DROPDOWN_GAP;
  const spaceBelow = window.innerHeight - trigger.bottom;
  const spaceAbove = trigger.top;
  if (spaceBelow >= needed) {
    return "bottom";
  }
  if (spaceAbove >= needed) {
    return "top";
  }
  return spaceAbove > spaceBelow ? "top" : "bottom";
}

function panelStyleFor(trigger: DOMRect, placement: ResolvedPlacement): PanelStyle {
  const width = Math.max(trigger.width, 120);
  const viewportPad = 8;

  if (placement === "top") {
    const spaceAbove = trigger.top - viewportPad;
    const maxHeight = Math.max(80, Math.min(DROPDOWN_MAX, spaceAbove - DROPDOWN_GAP));
    return {
      top: trigger.top - DROPDOWN_GAP,
      left: trigger.left,
      width,
      maxHeight,
      transform: "translateY(-100%)",
    };
  }

  const spaceBelow = window.innerHeight - trigger.bottom - viewportPad;
  const maxHeight = Math.max(80, Math.min(DROPDOWN_MAX, spaceBelow - DROPDOWN_GAP));
  return {
    top: trigger.bottom + DROPDOWN_GAP,
    left: trigger.left,
    width,
    maxHeight,
    transform: "none",
  };
}

function SelectOptionLabel({ label }: { label: string }) {
  const labelRef = useOverflowTooltip({ content: label, placement: "right" });
  return (
    <span ref={labelRef} className="myui-select__option-label">
      {label}
    </span>
  );
}

export function Select({
  options,
  value = "",
  placeholder,
  disabled = false,
  size = "md",
  className,
  onChange,
  "aria-label": ariaLabel,
  placement = "bottom",
}: UiSelectProps) {
  const [open, setOpen] = useState(false);
  const [resolved, setResolved] = useState<ResolvedPlacement>(
    placement === "top" ? "top" : "bottom",
  );
  const [panel, setPanel] = useState<PanelStyle | null>(null);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const listId = useId();
  const selected = options.find((o) => o.value === value);
  const displayText = selected?.label ?? placeholder ?? "";
  const valueRef = useOverflowTooltip({
    content: displayText,
    placement: "top",
  });

  const updateGeometry = () => {
    const triggerEl = triggerRef.current;
    if (!triggerEl) {
      return;
    }
    const rect = triggerEl.getBoundingClientRect();
    const estimated =
      panelRef.current?.scrollHeight ||
      Math.min(DROPDOWN_MAX, Math.max(80, options.length * 34 + 8));
    const nextPlacement = resolvePlacement(placement, rect, estimated);
    setResolved(nextPlacement);
    setPanel(panelStyleFor(rect, nextPlacement));
  };

  useLayoutEffect(() => {
    if (!open) {
      setPanel(null);
      return;
    }
    updateGeometry();
    // second pass after portal paints for accurate scrollHeight / auto flip
    const raf = window.requestAnimationFrame(() => updateGeometry());
    return () => window.cancelAnimationFrame(raf);
  }, [open, placement, options.length, value]);

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
  }, [open, placement, options.length]);

  useEffect(() => {
    if (!open) {
      setHighlighted(null);
      return;
    }
    const initial =
      options.find((o) => o.value === value && !o.disabled)?.value ??
      options.find((o) => !o.disabled)?.value ??
      null;
    setHighlighted(initial);
  }, [open, value, options]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onDoc = (event: MouseEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const enabledValues = options.filter((o) => !o.disabled).map((o) => o.value);

  const moveHighlight = (delta: number) => {
    if (enabledValues.length === 0) {
      return;
    }
    const idx = highlighted ? enabledValues.indexOf(highlighted) : -1;
    const start = idx < 0 ? (delta > 0 ? -1 : 0) : idx;
    const next = Math.min(enabledValues.length - 1, Math.max(0, start + delta));
    setHighlighted(enabledValues[next] ?? null);
  };

  const selectValue = (next: string) => {
    const opt = options.find((o) => o.value === next);
    if (!opt || opt.disabled) {
      return;
    }
    onChange?.(next);
    setOpen(false);
  };

  const onTriggerKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) {
      return;
    }
    if (event.key === "Escape") {
      if (open) {
        event.preventDefault();
        setOpen(false);
      }
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      if (highlighted) {
        selectValue(highlighted);
      }
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      moveHighlight(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      moveHighlight(-1);
    }
  };

  const dropdown =
    open && panel
      ? createPortal(
          <div
            ref={panelRef}
            id={listId}
            className="myui-select__dropdown mprism-select-portal"
            role="listbox"
            data-placement={resolved}
            style={{
              position: "fixed",
              top: panel.top,
              left: panel.left,
              width: panel.width,
              maxHeight: panel.maxHeight,
              transform: panel.transform,
              zIndex: "var(--myui-select-z-index)" as unknown as number,
            }}
          >
            {options.length === 0 ? (
              <div className="myui-select__empty">No data</div>
            ) : (
              options.map((option) => {
                const active = option.value === value;
                const isHi = option.value === highlighted;
                return (
                  <div
                    key={option.value}
                    role="option"
                    aria-selected={active}
                    data-value={option.value}
                    className={cx(
                      "myui-select__option",
                      active && "is-selected",
                      option.disabled && "is-disabled",
                      isHi && "is-highlighted",
                    )}
                    onMouseEnter={() => {
                      if (!option.disabled) {
                        setHighlighted(option.value);
                      }
                    }}
                    onMouseDown={(event) => {
                      event.preventDefault();
                    }}
                    onClick={() => {
                      if (option.disabled) {
                        return;
                      }
                      selectValue(option.value);
                    }}
                  >
                    <span className="myui-select__option-check" aria-hidden>
                      {CHECK_SVG}
                    </span>
                    <SelectOptionLabel label={option.label} />
                  </div>
                );
              })
            )}
          </div>,
          document.body,
        )
      : null;

  return (
    <div
      ref={rootRef}
      className={getSelectClassName({
        size,
        disabled,
        focused: open,
        placement: resolved,
        className,
      })}
    >
      <button
        ref={triggerRef}
        type="button"
        className="myui-select__trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        aria-label={ariaLabel}
        onClick={() => !disabled && setOpen((v) => !v)}
        onKeyDown={onTriggerKeyDown}
      >
        <span
          ref={valueRef}
          className={cx("myui-select__value", !selected && "myui-select__placeholder")}
        >
          {displayText}
        </span>
        <span className="myui-select__suffix" aria-hidden>
          <span className="myui-select__arrow">{ARROW_SVG}</span>
        </span>
      </button>
      {dropdown}
    </div>
  );
}
