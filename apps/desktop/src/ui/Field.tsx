import type { ReactNode } from "react";
import { cx } from "./cx";

export type UiFieldProps = {
  label?: ReactNode;
  hint?: ReactNode;
  required?: boolean;
  children: ReactNode;
  className?: string;
  horizontal?: boolean;
};

export function Field({
  label,
  hint,
  required,
  children,
  className,
  horizontal = false,
}: UiFieldProps) {
  return (
    <div className={cx("mprism-field", horizontal && "mprism-field--horizontal", className)}>
      {label ? (
        <div className="mprism-field__label">
          {label}
          {required ? <span className="mprism-field__required">*</span> : null}
        </div>
      ) : null}
      <div className="mprism-field__control">
        {children}
        {hint ? <div className="mprism-field__hint">{hint}</div> : null}
      </div>
    </div>
  );
}
