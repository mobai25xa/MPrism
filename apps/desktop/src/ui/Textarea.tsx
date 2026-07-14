import type { TextareaHTMLAttributes } from "react";
import { getTextareaClassName } from "@mobai6462/components/textarea";
import type { TextareaSize } from "@mobai6462/components/textarea";
import { cx } from "./cx";

export type UiTextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement> & {
  size?: TextareaSize;
  error?: boolean;
};

export function Textarea({
  size = "md",
  error = false,
  disabled,
  className,
  ...rest
}: UiTextareaProps) {
  return (
    <div
      className={getTextareaClassName({
        size,
        disabled: !!disabled,
        error,
        className: cx("mprism-textarea", className),
      })}
    >
      <textarea className="myui-textarea__inner" disabled={disabled} {...rest} />
    </div>
  );
}
