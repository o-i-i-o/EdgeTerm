import { act, createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppErrorBoundary } from "./ErrorBoundary";

vi.mock("@tauri-apps/plugin-process", () => ({ exit: vi.fn() }));

// A render error reaches React's boundary and, separately, React's global error
// reporter, which is what a browser would hand to `window.onerror` — and what
// vitest counts as an uncaught exception. `reportError` is captured when
// react-dom loads, hence `vi.hoisted` rather than a plain assignment.
vi.hoisted(() => {
  (globalThis as { reportError?: (error: unknown) => void }).reportError =
    () => {};
});

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const Panel = () => createElement("div", null, "the interface");

const Exploding: ComponentType = () => {
  throw new Error("panel exploded");
};

/** Fails on its first render only, the way a race that has since settled does. */
let attempts = 0;
const Flaky: ComponentType = () => {
  attempts += 1;
  if (attempts === 1) {
    throw new Error("panel exploded");
  }
  return createElement("div", null, "the interface");
};

let container: HTMLDivElement;
let root: Root;

const mount = (child: Parameters<Root["render"]>[0]) => {
  act(() => {
    root.render(createElement(AppErrorBoundary, null, child));
  });
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  // `componentDidCatch` logs the failure; the assertions below are the check.
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("AppErrorBoundary", () => {
  it("renders its children while nothing has thrown", () => {
    mount(createElement(Panel));
    expect(container.textContent).toBe("the interface");
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("replaces the interface instead of leaving a blank window", () => {
    mount(createElement(Exploding));

    expect(container.querySelector('[role="alert"]')).not.toBeNull();
    expect(container.textContent).toContain("EdgeTerm hit an unexpected error");
    expect(container.textContent).toContain("panel exploded");
  });

  it("keeps a way out: Windows and Linux draw no window decorations", () => {
    mount(createElement(Exploding));

    const labels = Array.from(container.querySelectorAll("button")).map(
      (button) => button.textContent,
    );
    expect(labels).toEqual(["Copy details", "Quit EdgeTerm", "Try Again"]);
  });

  it("mounts the interface again when the user retries", () => {
    attempts = 0;
    mount(createElement(Flaky));
    const retry = Array.from(container.querySelectorAll("button")).find(
      (button) => button.textContent === "Try Again",
    );

    act(() => retry?.click());

    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.textContent).toBe("the interface");
  });
});
