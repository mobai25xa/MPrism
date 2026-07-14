import type { InputHTMLAttributes, ReactNode } from "react";
import { getInputClassName } from "@mobai6462/components/input";
import type { InputSize } from "@mobai6462/components/input";
import { cx } from "./cx";

export type UiInputProps = Omit<InputHTMLAttributes<HTMLInputElement>, "size" | "prefix"> & {
  size?: InputSize;
  error?: boolean;
  prefix?: ReactNode;
  suffix?: ReactNode;
};

export function Input({
  size = "md",
  error = false,
  disabled,
  className,
  prefix,
  suffix,
  ...rest
}: UiInputProps) {
  return (
    <div
      className={getInputClassName({
        size,
        disabled: !!disabled,
        error,
        className: cx("mprism-input", className),
      })}
    >
      <div className="myui-input__wrapper">
        {prefix ? <span className="myui-input__prefix">{prefix}</span> : null}
        <input className="myui-input__inner" disabled={disabled} {...rest} />
        {suffix ? <span className="myui-input__suffix">{suffix}</span> : null}
      </div>
    </div>
  );
}
