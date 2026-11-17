import { describe, expect, it } from "vitest";

import {
  placeSubmenu,
  SUBMENU_EDGE_MARGIN,
  SUBMENU_OFFSET,
} from "./menuPlacement";

const WINDOW = 800;

describe("placeSubmenu", () => {
  it("leaves a submenu below the row when it fits", () => {
    expect(
      placeSubmenu({
        entryTop: 100,
        entryBottom: 128,
        menuHeight: 200,
        viewportHeight: WINDOW,
      }),
    ).toEqual({ up: false, maxHeight: null });
  });

  it("flips a submenu up near the bottom of the window", () => {
    const placement = placeSubmenu({
      entryTop: 700,
      entryBottom: 728,
      menuHeight: 200,
      viewportHeight: WINDOW,
    });
    expect(placement.up).toBe(true);
    expect(placement.maxHeight).toBeNull();
  });

  it("caps a submenu that fits on neither side", () => {
    const placement = placeSubmenu({
      entryTop: 100,
      entryBottom: 128,
      menuHeight: 750,
      viewportHeight: WINDOW,
    });
    // More room below than above, so it stays down and scrolls.
    expect(placement.up).toBe(false);
    expect(placement.maxHeight).toBe(
      WINDOW - SUBMENU_EDGE_MARGIN - (100 - SUBMENU_OFFSET),
    );
  });

  it("counts the gap above the row when it flips", () => {
    const placement = placeSubmenu({
      entryTop: 500,
      entryBottom: 528,
      menuHeight: 310,
      viewportHeight: WINDOW,
    });
    // Above: 528 + 7 - 6 = 529; below: 800 - 6 - 493 = 301. Above wins.
    expect(placement).toEqual({ up: true, maxHeight: null });
  });

  it("never returns a negative cap on a tiny window", () => {
    const placement = placeSubmenu({
      entryTop: 20,
      entryBottom: 48,
      menuHeight: 600,
      viewportHeight: 30,
    });
    expect(placement.maxHeight).toBeGreaterThanOrEqual(0);
  });
});
