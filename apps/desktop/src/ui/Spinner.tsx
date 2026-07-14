import { cx } from "./cx";

export type UiSpinnerProps = {
  label?: string;
  className?: string;
  inline?: boolean;
};

export function Spinner({ label, className, inline = false }: UiSpinnerProps) {
  return (
    <div
      className={cx("mprism-spinner", inline && "mprism-spinner--inline", className)}
      role="status"
      aria-label={label}
    >
      <span className="myui-loading__spinner" />
      {label ? <span className="myui-loading__text">{label}</span> : null}
    </div>
  );
}
