import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { AppErrorBoundary } from "./components/ErrorBoundary";
import { installGlobalErrorHandlers } from "./crashReport";
import { applyFonts } from "./fonts";
import { installOverflowTooltip } from "./overflowTooltip";
import { useStore } from "./store";
import "./styles.css";

// The default stacks are picked per platform in fonts.ts (VS Code's editor
// and UI defaults) and the user may put a family of their own in front;
// publish both as --font-mono / --font-ui so CSS and xterm agree on one
// value. Applied before the first render, so the page never paints a frame
// in a font the user replaced.
applyFonts(
  useStore.getState().bufferFontFamily,
  useStore.getState().panelFontFamily,
);

// The WebView's native context menu exposes a Reload action. Reloading only
// resets the React page while the Rust session manager (and serial handles)
// keeps running, leaving invisible sessions behind. EdgeTerm provides its own
// context menus where needed, so suppress the native menu and its common
// keyboard shortcuts entirely.
document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener(
  "keydown",
  (event) => {
    const reloadShortcut =
      event.key === "F5" ||
      ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "r");
    if (reloadShortcut) event.preventDefault();
  },
  { capture: true },
);

installOverflowTooltip();

// Errors thrown outside React's render path — event handlers, timers, promise
// chains — never reach the error boundary; they are recorded here so the next
// crash report can carry them.
installGlobalErrorHandlers();

const container = document.getElementById("root");
if (!container) {
  throw new Error("index.html must provide the #root element");
}

ReactDOM.createRoot(container).render(
  <React.StrictMode>
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  </React.StrictMode>,
);
