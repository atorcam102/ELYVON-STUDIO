import { error as logError } from "@tauri-apps/plugin-log";
import { Component, StrictMode, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { inTauri } from "./lib/ipc";
import "./styles/index.css";

function report(message: string) {
  console.error(message);
  if (inTauri) void logError(`[ui] ${message}`).catch(() => undefined);
}

window.addEventListener("error", (e) => report(`${e.message} at ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled rejection: ${String(e.reason)}`));

/** Last-resort boundary: keeps the native side (and unsaved work) alive. */
class ErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    report(`${error.stack ?? error.message}\n${info.componentStack ?? ""}`);
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="startup-error">
        <h1>Something went wrong in the interface</h1>
        <p>Your project is held by the native core and was not lost. Reload the interface to continue; unsaved changes are kept.</p>
        <button type="button" className="btn btn--primary btn--md" onClick={() => location.reload()}>
          Reload interface
        </button>
      </div>
    );
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
