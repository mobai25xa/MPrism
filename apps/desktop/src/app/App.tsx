import { useEffect, useMemo, useState } from "react";
import { NavRail } from "../components/NavRail";
import { ChatWorkspace } from "../features/chat/ChatWorkspace";
import { ProviderSettingsPage } from "../features/providers/ProviderSettingsPage";
import { applyResolvedTheme, resolveTheme } from "./theme";
import { useAppStore } from "./store";
import { Spinner } from "../ui";
import { t } from "../i18n";

export function App() {
  const ready = useAppStore((s) => s.ready);
  const bootError = useAppStore((s) => s.bootError);
  const page = useAppStore((s) => s.page);
  const themePreference = useAppStore((s) => s.theme);
  const setPage = useAppStore((s) => s.setPage);
  const setTheme = useAppStore((s) => s.setTheme);
  const bootstrap = useAppStore((s) => s.bootstrap);

  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setSystemDark(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const resolved = useMemo(() => {
    if (themePreference === "system") {
      return systemDark ? "dark" : "light";
    }
    return resolveTheme(themePreference);
  }, [themePreference, systemDark]);

  useEffect(() => {
    applyResolvedTheme(resolved);
  }, [resolved]);

  return (
    <div className="mprism-app" data-theme-preference={themePreference}>
      <NavRail
        page={page}
        onPageChange={setPage}
        themePreference={themePreference}
        onThemeChange={(theme) => {
          void setTheme(theme);
        }}
      />
      <main className="mprism-main">
        {!ready ? (
          <div className="mprism-center">
            <Spinner label={t("common.loading")} />
          </div>
        ) : bootError ? (
          <div className="mprism-center">{bootError}</div>
        ) : page === "chat" ? (
          <ChatWorkspace />
        ) : (
          <ProviderSettingsPage />
        )}
      </main>
    </div>
  );
}
