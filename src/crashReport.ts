/** How many uncaught errors a report keeps; the oldest drop out first. */
const MAX_RECORDED = 10;

let recorded: string[] = [];

/**
 * Keeps one line per exception that escaped React. Event handlers, timers and
 * promise chains run outside the renderer, so an error thrown there never
 * reaches the error boundary: this buffer is the only place it survives long
 * enough to travel with the next crash report.
 */
export const recordUncaught = (summary: string) => {
  recorded.push(summary);
  if (recorded.length > MAX_RECORDED) {
    recorded = recorded.slice(-MAX_RECORDED);
  }
};

/** The recorded exceptions, oldest first. */
export const recordedUncaught = (): string[] => [...recorded];

/** Empties the buffer: a test's own state, or a report already handed over. */
export const forgetUncaught = () => {
  recorded = [];
};

/** `Error` → message and stack, for the `unknown` a `catch` hands over. */
export const describeError = (
  error: unknown,
): { message: string; stack: string } => {
  if (error instanceof Error) {
    return { message: error.message || error.name, stack: error.stack ?? "" };
  }
  try {
    return { message: String(error), stack: "" };
  } catch {
    // A value whose toString throws must not turn a crash into a second one.
    return { message: "unprintable error", stack: "" };
  }
};

/** One line for the buffer: what went wrong, and where if the runtime knew. */
const summarise = (message: string, where?: string): string =>
  where ? `${message} (${where})` : message;

/**
 * Installs the window-level handlers and returns the function that removes
 * them again. A browser that reports an error React already rendered an
 * overlay for is harmless — the two reports say the same thing.
 */
export const installGlobalErrorHandlers = (): (() => void) => {
  const onError = (event: ErrorEvent) => {
    const { message } = describeError(event.error ?? event.message);
    const where =
      event.filename && event.lineno > 0
        ? `${event.filename}:${event.lineno}`
        : undefined;
    recordUncaught(summarise(message, where));
    console.error("EdgeTerm: uncaught error", event.error ?? event.message);
  };

  const onRejection = (event: PromiseRejectionEvent) => {
    const { message, stack } = describeError(event.reason);
    // The second stack line is the caller, which is what places the failure.
    const caller = stack.split("\n")[1]?.trim();
    recordUncaught(summarise(message, caller));
    console.error("EdgeTerm: unhandled promise rejection", event.reason);
  };

  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
  };
};

export interface CrashDetails {
  error: unknown;
  /** React's component stack, when the error came from a render. */
  componentStack?: string | null;
}

/**
 * The text the crash screen copies: everything needed to place the failure,
 * so a report does not have to start with a round trip asking for it.
 */
export const describeCrash = ({
  error,
  componentStack,
}: CrashDetails): string => {
  const { message, stack } = describeError(error);
  const lines = [
    "EdgeTerm crash report",
    `time: ${new Date().toISOString()}`,
    `platform: ${navigator.userAgent}`,
    "",
    `message: ${message}`,
  ];
  if (stack) {
    lines.push("stack:", stack);
  }
  if (componentStack) {
    lines.push("component stack:", componentStack.trim());
  }
  const earlier = recordedUncaught();
  if (earlier.length > 0) {
    lines.push("", "earlier uncaught errors:", ...earlier);
  }
  return lines.join("\n");
};
