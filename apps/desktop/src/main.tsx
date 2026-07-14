import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";

// Factory / fytheme skin (@mobai6462/theme)
// :root = dark; [data-theme=light|dark] set by applyResolvedTheme
import "@mobai6462/theme/base.css";
import "@mobai6462/theme/app-shell.css";
import "@mobai6462/components/button/style";
import "@mobai6462/components/input/style";
import "@mobai6462/components/textarea/style";
import "@mobai6462/components/select/style";
import "@mobai6462/components/modal/style";
import "@mobai6462/components/dropdown/style";
import "@mobai6462/components/tooltip/style";
import "@mobai6462/components/alert/style";
import "@mobai6462/components/loading/style";
import "@mobai6462/components/menu/style";
import "@mobai6462/components/back-top/style";
import "@mobai6462/components/timeline/style";

import "./styles/global.css";
import { applyResolvedTheme, readThemePreference, resolveTheme } from "./app/theme";

applyResolvedTheme(resolveTheme(readThemePreference()));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
