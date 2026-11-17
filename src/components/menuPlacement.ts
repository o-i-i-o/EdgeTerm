/**
 * How a submenu must sit beside the row that opened it. Submenus normally
 * hang below the row (`top: -7px` in the stylesheet); near the bottom of the
 * window that would run past the edge, so the submenu flips above the row and
 * is capped to the room that is left.
 */
export interface SubmenuPlacement {
  /** Open above the row instead of below it. */
  up: boolean;
  /**
   * Height cap in pixels when neither side can show the whole submenu, so it
   * scrolls inside the window rather than past the edge. Null means it fits.
   */
  maxHeight: number | null;
}

/** Space kept between a menu edge and the window edge, matching `EDGE_MARGIN`. */
export const SUBMENU_EDGE_MARGIN = 6;
/** The stylesheet's `top: -7px`, read back to know where the submenu starts. */
export const SUBMENU_OFFSET = 7;

/**
 * Places `menuHeight` next to a row spanning `entryTop`…`entryBottom` in a
 * window `viewportHeight` tall. It stays below when it fits; when it does not
 * and there is more room above, it flips up. Either way the returned cap keeps
 * it inside the window.
 */
export function placeSubmenu({
  entryTop,
  entryBottom,
  menuHeight,
  viewportHeight,
  edgeMargin = SUBMENU_EDGE_MARGIN,
  offset = SUBMENU_OFFSET,
}: {
  entryTop: number;
  entryBottom: number;
  menuHeight: number;
  viewportHeight: number;
  edgeMargin?: number;
  offset?: number;
}): SubmenuPlacement {
  // The submenu's first row sits `offset` above the parent row's top when it
  // opens downwards, and its last row `offset` below that row when it flips.
  const below = viewportHeight - edgeMargin - (entryTop - offset);
  const above = entryBottom + offset - edgeMargin;
  const up = menuHeight > below && above > below;
  const room = Math.max(0, Math.floor(up ? above : below));
  return { up, maxHeight: menuHeight > room ? room : null };
}
