import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  SUBMENU_GRACE_MS,
  graceTriangle,
  insideTriangle,
  submenuKey,
  useSubmenuHover,
  type Box,
  type Triangle,
} from "./submenuHover";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

// The View menu's Theme row and its submenu in viewport pixels: the row runs
// from x 7 to 217 at y 300–328; the submenu opens 4px to its right and one
// row higher, with Dark level with Theme and Light below it.
const SUBMENU: Box = { left: 221, top: 293, right: 400, bottom: 363 };

describe("graceTriangle", () => {
  it("runs from the exit point to the submenu's near edge, widened by the margin", () => {
    expect(graceTriangle({ x: 180, y: 329 }, SUBMENU, 10)).toEqual([
      { x: 180, y: 329 },
      { x: 221, y: 283 },
      { x: 221, y: 373 },
    ]);
  });

  it("takes the right edge of a submenu that opens leftwards", () => {
    const flipped: Box = { left: 20, top: 293, right: 199, bottom: 363 };
    expect(graceTriangle({ x: 240, y: 329 }, flipped, 10)).toEqual([
      { x: 240, y: 329 },
      { x: 199, y: 283 },
      { x: 199, y: 373 },
    ]);
  });

  it("offers nothing when the pointer left above or below the submenu", () => {
    expect(graceTriangle({ x: 300, y: 370 }, SUBMENU)).toBeNull();
  });
});

describe("insideTriangle", () => {
  const triangle = graceTriangle({ x: 180, y: 329 }, SUBMENU) as Triangle;

  it("holds the exit point and every straight way to the submenu", () => {
    expect(insideTriangle({ x: 180, y: 329 }, triangle)).toBe(true);
    expect(insideTriangle({ x: 200, y: 335 }, triangle)).toBe(true);
    expect(insideTriangle({ x: 220, y: 300 }, triangle)).toBe(true);
  });

  it("excludes the rows above and below and the way back", () => {
    expect(insideTriangle({ x: 181, y: 345 }, triangle)).toBe(false);
    expect(insideTriangle({ x: 180, y: 310 }, triangle)).toBe(false);
    expect(insideTriangle({ x: 160, y: 329 }, triangle)).toBe(false);
  });
});

describe("submenuKey", () => {
  it("nests a level's index under its parent", () => {
    expect(submenuKey(undefined, 2)).toBe("2");
    expect(submenuKey("2", 0)).toBe("2.0");
  });
});

// A dropdown like View: a plain row, the Theme row with its submenu, another
// plain row, then a row whose submenu holds a nested submenu.
function Harness(): ReactElement {
  const submenu = useSubmenuHover();
  const button = (label: string) =>
    createElement("button", { key: label, className: "menu-entry" }, label);
  const entry = (key: string, label: string, ...children: ReactElement[]) =>
    createElement(
      "div",
      {
        key,
        className: `menu-submenu-entry${submenu.isOpen(key) ? " is-open" : ""}`,
        "data-key": key,
        ...submenu.entryProps(key),
      },
      createElement("div", { className: "menu-entry" }, label),
      createElement(
        "div",
        { className: "menu-dropdown menu-submenu" },
        ...children,
      ),
    );
  return createElement(
    "div",
    { className: "menu-dropdown", ...submenu.rootProps },
    button("Reveal Working Directory in Filer"),
    entry("1", "Theme", button("Dark"), button("Light")),
    button("Display Settings…"),
    entry("3", "More", button("Other"), entry("3.1", "Nested", button("Deep"))),
  );
}

describe("useSubmenuHover", () => {
  let container: HTMLDivElement;
  let root: Root;

  const element = (selector: string): HTMLElement => {
    const found = container.querySelector<HTMLElement>(selector);
    if (!found) throw new Error(`no element matches ${selector}`);
    return found;
  };
  const row = (key: string) => element(`[data-key="${key}"] > .menu-entry`);
  const button = (label: string): HTMLElement => {
    const found = Array.from(container.querySelectorAll("button")).find(
      (candidate) => candidate.textContent === label,
    );
    if (!found) throw new Error(`no button labelled ${label}`);
    return found;
  };
  const isOpen = (key: string) =>
    element(`[data-key="${key}"]`).classList.contains("is-open");

  /**
   * Moves the pointer from one element to another (null: outside the menu)
   * the way the browser reports it: the boundary events, then the move.
   * React derives mouseenter / mouseleave from mouseout's relatedTarget.
   */
  const pointerMove = (
    from: Element | null,
    to: Element | null,
    x: number,
    y: number,
  ) => {
    const init = { bubbles: true, clientX: x, clientY: y };
    act(() => {
      if (from && from !== to) {
        from.dispatchEvent(
          new MouseEvent("mouseout", { ...init, relatedTarget: to }),
        );
      } else if (!from && to) {
        to.dispatchEvent(
          new MouseEvent("mouseover", { ...init, relatedTarget: null }),
        );
      }
      (to ?? document.body).dispatchEvent(new MouseEvent("mousemove", init));
    });
  };

  beforeEach(() => {
    vi.useFakeTimers();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root.render(createElement(Harness)));
    // jsdom lays nothing out; the Theme submenu gets the geometry above.
    element('[data-key="1"] > .menu-submenu').getBoundingClientRect = () =>
      ({
        ...SUBMENU,
        x: SUBMENU.left,
        y: SUBMENU.top,
        width: SUBMENU.right - SUBMENU.left,
        height: SUBMENU.bottom - SUBMENU.top,
        toJSON: () => ({}),
      }) as DOMRect;
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it("opens on the row and stays open across the gap and the row below", () => {
    pointerMove(null, row("1"), 100, 314);
    expect(isOpen("1")).toBe(true);
    // Heading down-right for Light, the pointer leaves the row through its
    // bottom edge onto the row below …
    pointerMove(row("1"), button("Display Settings…"), 190, 330);
    expect(isOpen("1")).toBe(true);
    pointerMove(
      button("Display Settings…"),
      button("Display Settings…"),
      210,
      333,
    );
    expect(isOpen("1")).toBe(true);
    // … and arrives.
    pointerMove(button("Display Settings…"), button("Light"), 230, 340);
    expect(isOpen("1")).toBe(true);
  });

  it("closes as soon as the pointer heads elsewhere", () => {
    pointerMove(null, row("1"), 100, 314);
    pointerMove(row("1"), button("Display Settings…"), 100, 330);
    // The exit point itself is still inside the triangle.
    expect(isOpen("1")).toBe(true);
    pointerMove(
      button("Display Settings…"),
      button("Display Settings…"),
      100,
      345,
    );
    expect(isOpen("1")).toBe(false);
  });

  it("closes when the pointer rests in the triangle for too long", () => {
    pointerMove(null, row("1"), 100, 314);
    pointerMove(row("1"), button("Display Settings…"), 190, 330);
    vi.advanceTimersByTime(SUBMENU_GRACE_MS - 100);
    pointerMove(
      button("Display Settings…"),
      button("Display Settings…"),
      200,
      332,
    );
    expect(isOpen("1")).toBe(true);
    // Each move inside the triangle restarts the clock …
    vi.advanceTimersByTime(SUBMENU_GRACE_MS - 100);
    pointerMove(
      button("Display Settings…"),
      button("Display Settings…"),
      205,
      333,
    );
    expect(isOpen("1")).toBe(true);
    // … but a rest runs it out.
    vi.advanceTimersByTime(SUBMENU_GRACE_MS + 1);
    pointerMove(
      button("Display Settings…"),
      button("Display Settings…"),
      210,
      334,
    );
    expect(isOpen("1")).toBe(false);
  });

  it("returning to the row cancels the pending close", () => {
    pointerMove(null, row("1"), 100, 314);
    pointerMove(row("1"), button("Display Settings…"), 190, 330);
    pointerMove(button("Display Settings…"), row("1"), 190, 320);
    vi.advanceTimersByTime(SUBMENU_GRACE_MS * 2);
    pointerMove(row("1"), row("1"), 150, 315);
    expect(isOpen("1")).toBe(true);
  });

  it("keeps the submenu after the pointer leaves the menu, until it comes back elsewhere", () => {
    pointerMove(null, row("1"), 100, 314);
    pointerMove(row("1"), null, 100, 250);
    expect(isOpen("1")).toBe(true);
    pointerMove(null, button("Display Settings…"), 100, 345);
    expect(isOpen("1")).toBe(false);
  });

  it("switches to a sibling submenu at once", () => {
    pointerMove(null, row("1"), 100, 314);
    pointerMove(row("1"), row("3"), 100, 400);
    expect(isOpen("1")).toBe(false);
    expect(isOpen("3")).toBe(true);
  });

  it("keeps the ancestors of an open nested submenu", () => {
    pointerMove(null, row("3"), 100, 400);
    pointerMove(row("3"), row("3.1"), 300, 400);
    expect(isOpen("3")).toBe(true);
    expect(isOpen("3.1")).toBe(true);
    // Leaving the nested entry for a sibling in the same submenu closes it
    // alone.
    pointerMove(row("3.1"), button("Other"), 300, 380);
    pointerMove(button("Other"), button("Other"), 300, 379);
    expect(isOpen("3.1")).toBe(false);
    expect(isOpen("3")).toBe(true);
  });
});
