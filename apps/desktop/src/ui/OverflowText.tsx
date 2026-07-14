import type { TooltipPlacement } from "@mobai6462/components/tooltip";
import { useOverflowTooltip } from "./useOverflowTooltip";
import { cx } from "./cx";

export type OverflowTextProps = {
  text: string;
  className?: string;
  placement?: TooltipPlacement;
  /** Default: div. Use span inside flex/inline rows when needed. */
  as?: "div" | "span";
};

/**
 * Single-line ellipsis host with fytheme overflow-only tooltip.
 * Shares bindOverflowTooltip with Select via useOverflowTooltip.
 */
export function OverflowText({
  text,
  className,
  placement = "right",
  as = "div",
}: OverflowTextProps) {
  const overflowRef = useOverflowTooltip({ content: text, placement });

  if (as === "span") {
    return (
      <span ref={overflowRef} className={cx(className)}>
        {text}
      </span>
    );
  }

  return (
    <div ref={overflowRef} className={cx(className)}>
      {text}
    </div>
  );
}
