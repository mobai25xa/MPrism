import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { RefCallback } from "react";
import { bindOverflowTooltip } from "@mobai6462/components/tooltip";
import type { OverflowTooltipHandle, TooltipPlacement } from "@mobai6462/components/tooltip";

export type OverflowTooltipBindOptions = {
  content: string | (() => string);
  placement?: TooltipPlacement;
  /** Default 0 to match previous local EllipsisTooltip (package default is 200). */
  showDelay?: number;
  hideDelay?: number;
  enabled?: boolean;
};

/**
 * Bind fytheme overflow-only tooltip to an existing ellipsis node (no wrapper).
 * Returns a callback ref so binding tracks real DOM attach/detach
 * (lists, portal dropdown options, React Strict Mode).
 *
 * Used by both sidebar titles (OverflowText) and Select trigger/options.
 */
export function useOverflowTooltip(
  options: OverflowTooltipBindOptions,
): RefCallback<HTMLElement> {
  const { content, placement = "top", showDelay = 0, hideDelay = 0, enabled = true } = options;
  const [node, setNode] = useState<HTMLElement | null>(null);
  const contentRef = useRef(content);
  contentRef.current = content;
  const handleRef = useRef<OverflowTooltipHandle | null>(null);
  const contentKey = typeof content === "string" ? content : undefined;

  const ref = useCallback<RefCallback<HTMLElement>>((el) => {
    setNode(el);
  }, []);

  useLayoutEffect(() => {
    handleRef.current?.destroy();
    handleRef.current = null;

    if (!node || !enabled) {
      return;
    }

    const handle = bindOverflowTooltip(node, {
      content: () => {
        const next = contentRef.current;
        return typeof next === "function" ? next() : next;
      },
      placement,
      showDelay,
      hideDelay,
    });
    handleRef.current = handle;

    return () => {
      handle.destroy();
      if (handleRef.current === handle) {
        handleRef.current = null;
      }
    };
  }, [node, placement, showDelay, hideDelay, enabled]);

  useLayoutEffect(() => {
    handleRef.current?.refresh();
  }, [contentKey, node, enabled]);

  return ref;
}
