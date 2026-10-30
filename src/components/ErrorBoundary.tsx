import { exit } from "@tauri-apps/plugin-process";
import {
  Component,
  useCallback,
  useState,
  type ErrorInfo,
  type ReactNode,
} from "react";

import { describeCrash, describeError } from "../crashReport";
import { useT } from "../i18n";

interface BoundaryProps {
  children: ReactNode;
}

interface BoundaryState {
  error: unknown;
  componentStack: string | null;
}

interface CrashScreenProps {
  error: unknown;
  componentStack: string | null;
  onRetry: () => void;
}

/**
 * The whole interface, replaced when rendering it threw. Windows and Linux draw
 * no native decorations (see `MenuBar.tsx`), so a blank window would leave no
 * button to close it with: this screen carries the only exit there is.
 */
const CrashScreen = ({ error, componentStack, onRetry }: CrashScreenProps) => {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const { message } = describeError(error);

  const copyDetails = useCallback(() => {
    navigator.clipboard
      .writeText(describeCrash({ error, componentStack }))
      .then(() => setCopied(true))
      .catch(() => setCopied(false));
  }, [error, componentStack]);

  return (
    <div className="crash-screen" role="alert">
      <div className="dialog">
        <div className="dialog-header">
          <span>{t("EdgeTerm hit an unexpected error")}</span>
        </div>
        <div className="dialog-body">
          <p className="crash-lead">
            {t(
              "The interface stopped rendering. Open sessions keep running; try again, or copy the details before quitting.",
            )}
          </p>
          <pre className="crash-message">{message}</pre>
        </div>
        <div className="dialog-footer">
          <button className="btn" type="button" onClick={copyDetails}>
            {copied ? t("Copied") : t("Copy details")}
          </button>
          <button
            className="btn"
            type="button"
            onClick={() => {
              void exit(0);
            }}
          >
            {t("Quit EdgeTerm")}
          </button>
          <button className="btn is-primary" type="button" onClick={onRetry}>
            {t("Try Again")}
          </button>
        </div>
      </div>
    </div>
  );
};

/**
 * Catches a render error anywhere below it. React has already unmounted the
 * tree by then, so the only way back into the interface is to mount it again —
 * which is what "Try again" does, and why it is worth offering even for a
 * failure that turns out to be deterministic.
 */
export class AppErrorBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { error: null, componentStack: null };

  static getDerivedStateFromError(error: unknown): Partial<BoundaryState> {
    return { error };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    // The component stack arrives here and nowhere else; the panel copies it
    // out through `describeCrash`, so keep it while the screen is up.
    console.error("EdgeTerm: the interface failed to render", error);
    this.setState({ componentStack: info.componentStack ?? null });
  }

  render() {
    if (this.state.error === null) {
      return this.props.children;
    }
    return (
      <CrashScreen
        error={this.state.error}
        componentStack={this.state.componentStack}
        onRetry={() => this.setState({ error: null, componentStack: null })}
      />
    );
  }
}
