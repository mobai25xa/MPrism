import { useEffect } from "react";
import type { ReactNode } from "react";
import { Button } from "./Button";
import { cx } from "./cx";

export type UiModalProps = {
  open: boolean;
  title?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  onClose?: () => void;
  className?: string;
  maskClosable?: boolean;
};

export function Modal({
  open,
  title,
  children,
  footer,
  onClose,
  className,
  maskClosable = true,
}: UiModalProps) {
  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose?.();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <div className={cx("myui-modal", "is-open", className)} role="presentation">
      <div
        className="myui-modal__mask"
        onClick={() => {
          if (maskClosable) {
            onClose?.();
          }
        }}
      />
      <div className="myui-modal__dialog" role="dialog" aria-modal="true">
        <div className="myui-modal__header">
          <h3 className="myui-modal__title">{title}</h3>
          <button
            type="button"
            className="myui-modal__close"
            aria-label="Close"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <div className="myui-modal__body">{children}</div>
        {footer !== undefined ? (
          <div className="myui-modal__footer">
            {footer ?? (
              <Button variant="default" onClick={onClose}>
                Close
              </Button>
            )}
          </div>
        ) : null}
      </div>
    </div>
  );
}
