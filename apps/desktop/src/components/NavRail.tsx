import {
  Dropdown,
  IconChat,
  IconMoon,
  IconPalette,
  IconSettings,
  IconSun,
  Tooltip,
  cx,
} from "../ui";
import { t } from "../i18n";
import type { ThemePreference } from "../app/theme";

export type AppPage = "chat" | "settings";

type NavRailProps = {
  page: AppPage;
  onPageChange: (page: AppPage) => void;
  themePreference: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
};

export function NavRail({ page, onPageChange, themePreference, onThemeChange }: NavRailProps) {
  const themeIcon =
    themePreference === "dark" ? (
      <IconMoon size={18} />
    ) : themePreference === "light" ? (
      <IconSun size={18} />
    ) : (
      <IconPalette size={18} />
    );

  return (
    <nav className="myui-app-rail" aria-label={t("app.name")}>
      <Tooltip content={t("nav.chat")} placement="right">
        <button
          type="button"
          className={cx("myui-app-rail__item", page === "chat" && "is-active")}
          onClick={() => onPageChange("chat")}
          aria-current={page === "chat" ? "page" : undefined}
          aria-label={t("nav.chat")}
        >
          <IconChat size={18} />
        </button>
      </Tooltip>
      <Tooltip content={t("nav.settings")} placement="right">
        <button
          type="button"
          className={cx("myui-app-rail__item", page === "settings" && "is-active")}
          onClick={() => onPageChange("settings")}
          aria-current={page === "settings" ? "page" : undefined}
          aria-label={t("nav.settings")}
        >
          <IconSettings size={18} />
        </button>
      </Tooltip>
      <div className="myui-app-rail__spacer" />
      <Tooltip content={t("nav.theme")} placement="right">
        <Dropdown
          placement="top-start"
          items={[
            {
              key: "system",
              label: t("theme.system"),
              icon: <IconPalette size={16} />,
              disabled: themePreference === "system",
            },
            {
              key: "light",
              label: t("theme.light"),
              icon: <IconSun size={16} />,
              disabled: themePreference === "light",
            },
            {
              key: "dark",
              label: t("theme.dark"),
              icon: <IconMoon size={16} />,
              disabled: themePreference === "dark",
            },
          ]}
          onSelect={(key) => {
            if (key === "system" || key === "light" || key === "dark") {
              onThemeChange(key);
            }
          }}
        >
          <button type="button" className="myui-app-rail__item" aria-label={t("nav.theme")}>
            {themeIcon}
          </button>
        </Dropdown>
      </Tooltip>
    </nav>
  );
}
