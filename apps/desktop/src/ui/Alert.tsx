import type { ReactNode } from "react";
import { getAlertClassName } from "@mobai6462/components/alert";
import type { AlertType } from "@mobai6462/components/alert";

export type UiAlertProps = {
  type?: AlertType;
  children?: ReactNode;
  className?: string;
};

export function Alert({ type = "info", children, className }: UiAlertProps) {
  return (
    <div className={getAlertClassName({ type, className })}>
      <div className="myui-alert__content">
        <div className="myui-alert__description">{children}</div>
      </div>
    </div>
  );
}
