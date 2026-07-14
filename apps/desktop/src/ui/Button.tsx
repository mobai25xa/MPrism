import type { ButtonHTMLAttributes, ReactNode } from "react";
import { getButtonClassName } from "@mobai6462/components/button";
import type { ButtonSize, ButtonType } from "@mobai6462/components/button";

export type UiButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "type"> & {
  /**
   * Theme language:
   * cancel: default | ghost
   * confirm: secondary | outline | primary
   * alert: danger
   */
  variant?: ButtonType;
  size?: ButtonSize;
  htmlType?: ButtonHTMLAttributes<HTMLButtonElement>["type"];
  loading?: boolean;
  plain?: boolean;
  round?: boolean;
  /** Square icon-only hit (0.1.2 default for tool icons). Auto when icon-only. */
  iconOnly?: boolean;
  /** Round control; only when product needs avatar-like circle. */
  circle?: boolean;
  icon?: ReactNode;
};

export function Button({
  variant = "default",
  size = "md",
  htmlType = "button",
  loading = false,
  plain = false,
  round = false,
  iconOnly,
  circle = false,
  disabled,
  className,
  icon,
  children,
  ...rest
}: UiButtonProps) {
  const onlyIcon = !!icon && (children === undefined || children === null || children === false || children === "");
  // 0.1.2: icon tools use square hit (myui-button--icon), not circle.
  const useIconOnly = iconOnly ?? (onlyIcon && !circle);

  return (
    <button
      type={htmlType}
      className={getButtonClassName({
        type: variant,
        size,
        plain,
        round,
        iconOnly: useIconOnly,
        circle,
        disabled: !!disabled || loading,
        loading,
        className,
      })}
      disabled={disabled || loading}
      {...rest}
    >
      {loading ? <span className="myui-button__loading" aria-hidden /> : null}
      {icon ? <span className="myui-button__icon">{icon}</span> : null}
      {children}
    </button>
  );
}
