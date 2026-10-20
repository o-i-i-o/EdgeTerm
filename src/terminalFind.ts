import type {
  IBufferCell,
  IDecoration,
  IDisposable,
  IMarker,
  Terminal,
} from "@xterm/xterm";

import {
  compileQuery,
  DEFAULT_FIND_OPTIONS,
  findInText,
  SEARCH_HIGHLIGHT_LIMIT,
  type FindOptions,
  type SearchResults,
} from "./terminalSearch";

/** Find highlight colors; decorations take #RRGGBB only. */
export interface FindColors {
  match: string;
  activeMatch: string;
  /** Under the Find in Selection range. */
  scope: string;
}

/** A buffer position: an absolute row (scrollback included) and a column. */
interface Position {
  row: number;
  col: number;
}

/** From the first cell up to just past the last; may run across wrapped rows. */
interface Span {
  start: Position;
  end: Position;
}

/**
 * A span kept across scrolling. Its rows ride on markers, which xterm moves
 * as the scrollback is trimmed and disposes once their row is gone.
 */
interface TrackedSpan {
  buffer: "normal" | "alternate";
  start: IMarker;
  startCol: number;
  end: IMarker;
  endCol: number;
}

/** A buffer row and the wrapped rows continuing it, as one string. */
interface LogicalLine {
  row: number;
  text: string;
  /** Where each of its rows starts in `text`. */
  starts: number[];
}

const compare = (a: Position, b: Position) => a.row - b.row || a.col - b.col;

const sameSpan = (a: Span, b: Span) =>
  compare(a.start, b.start) === 0 && compare(a.end, b.end) === 0;

const disposeAll = (items: IDisposable[]) => {
  for (const item of items) item.dispose();
};

/** Output has to pause this long before the highlights are recomputed. */
const REFRESH_DELAY_MS = 200;

/**
 * Find in the terminal buffer: Match Case, Whole Word, Regular Expression and
 * Find in Selection (issue #95).
 *
 * It replaces xterm's search addon, which has no notion of a range to search
 * within, and whose whole-word test gave up on a row once its first hit sat
 * inside a longer word, and whose regexes matched `^` wherever the previous
 * hit ended. Its conventions are kept: the current match is the terminal
 * selection (so Copy takes it, and the next search starts from wherever the
 * user clicked), only the first SEARCH_HIGHLIGHT_LIMIT matches are
 * highlighted and counted, and the highlights are recomputed once output
 * pauses.
 *
 * Highlights are top-layer decorations, drawn over the selection, and never
 * overlap: the current match wears only its own. The Find in Selection range
 * is shaded on the bottom layer, under the selection, and only on the rows
 * in view — a decoration per row of a Select All range would be one per
 * scrollback line.
 */
export class TerminalFind {
  private options: FindOptions = { ...DEFAULT_FIND_OPTIONS };
  private pattern: RegExp | null = null;
  /** In reading order, from the top of the buffer or the range. */
  private matches: Span[] = [];
  /** Decorations of each entry of `matches`; the current one's are `activeDecorations`. */
  private highlights: IDecoration[][] = [];
  private activeIndex = -1;
  private active: TrackedSpan | null = null;
  private activeDecorations: IDecoration[] = [];
  /** Find in Selection's range. */
  private scope: TrackedSpan | null = null;
  private scopeDecorations: { decoration: IDecoration; x: number; width: number }[] =
    [];
  /**
   * The user's selection a search last replaced with a match. Find in
   * Selection turned on after typing a query takes this rather than the
   * match now selected.
   */
  private replaced: TrackedSpan | null = null;
  /** The selection changed since the engine last set it. */
  private selectionMoved = true;
  private settingSelection = false;
  /** The buffer changed since `matches` was computed. */
  private stale = true;
  /** Logical lines by first row; reading the whole scrollback on every keystroke is the expensive part. */
  private lines = new Map<number, LogicalLine>();
  private refreshTimer: number | null = null;
  private scopeSyncQueued = false;
  private cell: IBufferCell | undefined;
  private readonly listeners = new Set<(results: SearchResults) => void>();
  private readonly subscriptions: IDisposable[];

  constructor(
    private readonly term: Terminal,
    private colors: FindColors,
  ) {
    this.subscriptions = [
      term.onWriteParsed(() => this.bufferChanged()),
      term.onResize(() => {
        this.bufferChanged();
        this.queueScopeSync();
      }),
      term.buffer.onBufferChange(() => {
        this.bufferChanged();
        this.queueScopeSync();
      }),
      term.onScroll(() => this.queueScopeSync()),
      term.onSelectionChange(() => {
        if (!this.settingSelection) this.selectionMoved = true;
      }),
    ];
  }

  onResults(listener: (results: SearchResults) => void): IDisposable {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  /**
   * Search for `query` and make the next (or previous) match current.
   * `incremental` keeps the current match while the query is being typed,
   * when it still matches, instead of moving past it.
   */
  find(
    query: string,
    options: FindOptions,
    forward: boolean,
    incremental: boolean,
  ) {
    let pattern: RegExp | null = null;
    try {
      if (query) pattern = compileQuery(query, options);
    } catch {
      // A regular expression still being typed; nothing matches it yet.
    }
    const changed =
      this.stale ||
      pattern?.source !== this.pattern?.source ||
      pattern?.flags !== this.pattern?.flags ||
      options.wholeWord !== this.options.wholeWord;
    this.pattern = pattern;
    this.options = { ...options };
    this.cancelRefresh();

    const selection = this.selection();
    if (this.selectionMoved) {
      this.untrack(this.replaced);
      this.replaced = selection ? this.track(selection) : null;
    }

    if (changed) this.collect();
    let target: Span | null = null;
    if (pattern && this.matches.length > 0) {
      if (forward) {
        const from = incremental ? selection?.start : selection?.end;
        target = this.matchFrom(from ?? null);
      } else {
        const here =
          incremental && selection ? this.matchAt(selection.start) : null;
        target = here ?? this.matchBefore(selection?.start ?? null);
      }
    }
    if (changed) {
      this.setActive(target);
      this.redraw();
    } else {
      this.moveActive(target);
    }
    if (pattern) this.select(target);
    this.fire();
  }

  /**
   * Turn Find in Selection on or off; whether it is on afterwards. Its range
   * is the selection, or, when the selection is a match a search moved it
   * to, the selection the user had made before.
   */
  setScope(on: boolean): boolean {
    this.untrack(this.scope);
    this.scope = null;
    if (on) {
      const range = this.selectionMoved
        ? this.selection()
        : this.spanOf(this.replaced);
      if (range && compare(range.start, range.end) < 0) {
        this.scope = this.track(range);
      }
    }
    this.stale = true;
    this.syncScope();
    return this.scope !== null;
  }

  setColors(colors: FindColors) {
    this.colors = colors;
    // `matches` holds rows as they were when it was computed; output since
    // may have trimmed the scrollback under it.
    if (this.pattern) this.refresh();
    this.dropScopeDecorations();
    this.syncScope();
  }

  /** Forget the buffer as read; for changes that come without a write. */
  bufferChanged() {
    this.stale = true;
    this.lines.clear();
    if (!this.pattern) return;
    this.cancelRefresh();
    this.refreshTimer = window.setTimeout(
      () => this.refresh(),
      REFRESH_DELAY_MS,
    );
  }

  /** Drop the highlights and the range. The selection, the last match, stays. */
  clear() {
    this.cancelRefresh();
    this.pattern = null;
    this.matches = [];
    this.setActive(null);
    this.redraw();
    this.untrack(this.scope);
    this.scope = null;
    this.untrack(this.replaced);
    this.replaced = null;
    this.dropScopeDecorations();
    this.lines.clear();
    this.stale = true;
  }

  dispose() {
    this.clear();
    disposeAll(this.subscriptions);
    this.listeners.clear();
  }

  // --- matching -------------------------------------------------------------

  /** The first row of the logical line holding `row`. */
  private lineStart(row: number): number {
    const buffer = this.term.buffer.active;
    while (row > 0 && buffer.getLine(row)?.isWrapped) row--;
    return row;
  }

  private readLine(row: number): LogicalLine {
    const cached = this.lines.get(row);
    if (cached) return cached;
    const buffer = this.term.buffer.active;
    let text = "";
    const starts: number[] = [];
    for (let y = row; ; y++) {
      const line = buffer.getLine(y);
      if (!line) break;
      const next = buffer.getLine(y + 1);
      const continues = next?.isWrapped ?? false;
      let part = line.translateToString(!continues);
      // A wide character that did not fit at the end of a row moved to the
      // next, leaving an empty cell that is not a space in the text.
      if (continues && next?.getCell(0)?.getWidth() === 2) {
        const last = line.getCell(line.length - 1);
        if (last?.getCode() === 0 && last.getWidth() === 1) {
          part = part.slice(0, -1);
        }
      }
      starts.push(text.length);
      text += part;
      if (!continues) break;
    }
    const read = { row, text, starts };
    this.lines.set(row, read);
    return read;
  }

  /** The cell holding `text[offset]`: its buffer row, column and width. */
  private cellAt(line: LogicalLine, offset: number) {
    let index = line.starts.length - 1;
    while (index > 0 && line.starts[index] > offset) index--;
    const row = line.row + index;
    const bufferLine = this.term.buffer.active.getLine(row);
    this.cell ??= this.term.buffer.active.getNullCell();
    let rest = offset - line.starts[index];
    if (bufferLine) {
      for (let col = 0; col < bufferLine.length; col++) {
        const cell = bufferLine.getCell(col, this.cell);
        if (!cell) break;
        const width = cell.getWidth();
        // The second half of a wide character.
        if (width === 0) continue;
        // An empty cell reads as one space, as in translateToString.
        const length = cell.getChars().length || 1;
        if (rest < length) return { row, col, width };
        rest -= length;
      }
    }
    return { row, col: this.term.cols, width: 0 };
  }

  private toSpan(line: LogicalLine, start: number, end: number): Span {
    const first = this.cellAt(line, start);
    const last = this.cellAt(line, end - 1);
    return {
      start: { row: first.row, col: first.col },
      end: { row: last.row, col: last.col + last.width },
    };
  }

  /** Where matches may lie: the range, or the whole buffer; null when the range is gone. */
  private bounds(): Span | null {
    if (this.scope) return this.spanOf(this.scope, true);
    return {
      start: { row: 0, col: 0 },
      end: { row: this.term.buffer.active.length - 1, col: this.term.cols },
    };
  }

  private within(span: Span, bounds: Span) {
    return (
      compare(span.start, bounds.start) >= 0 &&
      compare(span.end, bounds.end) <= 0
    );
  }

  /** The matches of one logical line that lie within `bounds`. */
  private matchesIn(line: LogicalLine, bounds: Span): Span[] {
    if (!this.pattern) return [];
    return findInText(line.text, this.pattern, this.options.wholeWord)
      .map(([start, end]) => this.toSpan(line, start, end))
      .filter((span) => this.within(span, bounds));
  }

  /** The matches from the top, as many as get highlighted. */
  private collect() {
    this.stale = false;
    this.matches = [];
    const bounds = this.bounds();
    if (!this.pattern || !bounds) return;
    for (let row = this.lineStart(bounds.start.row); row <= bounds.end.row; ) {
      const line = this.readLine(row);
      for (const span of this.matchesIn(line, bounds)) {
        if (this.matches.length === SEARCH_HIGHLIGHT_LIMIT) return;
        this.matches.push(span);
      }
      row += line.starts.length;
    }
  }

  /**
   * The first match at or after `from`, else the first of all. Scanned from
   * the buffer rather than taken from `matches`, which stops at the
   * highlight limit.
   */
  private matchFrom(from: Position | null): Span | null {
    const bounds = this.bounds();
    if (!bounds) return null;
    if (from && compare(from, bounds.start) > 0) {
      for (let row = this.lineStart(from.row); row <= bounds.end.row; ) {
        const line = this.readLine(row);
        const found = this.matchesIn(line, bounds).find(
          (span) => compare(span.start, from) >= 0,
        );
        if (found) return found;
        row += line.starts.length;
      }
    }
    return this.matches[0] ?? null;
  }

  /** The last match before `before`, else the last of all. */
  private matchBefore(before: Position | null): Span | null {
    const bounds = this.bounds();
    if (!bounds) return null;
    const limit = before ?? { row: bounds.end.row + 1, col: 0 };
    const scan = (until: Position) => {
      let row = this.lineStart(Math.min(until.row, bounds.end.row));
      for (;;) {
        const line = this.readLine(row);
        const spans = this.matchesIn(line, bounds);
        for (let index = spans.length - 1; index >= 0; index--) {
          if (compare(spans[index].start, until) < 0) return spans[index];
        }
        if (row <= bounds.start.row) return null;
        row = this.lineStart(row - 1);
      }
    };
    return (
      scan(limit) ??
      (before ? scan({ row: bounds.end.row + 1, col: 0 }) : null)
    );
  }

  /** The match starting at `at`, if there is one. */
  private matchAt(at: Position): Span | null {
    const bounds = this.bounds();
    if (!bounds || at.row >= this.term.buffer.active.length) return null;
    const line = this.readLine(this.lineStart(at.row));
    return (
      this.matchesIn(line, bounds).find(
        (span) => compare(span.start, at) === 0,
      ) ?? null
    );
  }

  /** Recompute after output, keeping the current match while it still matches. */
  private refresh() {
    this.cancelRefresh();
    this.collect();
    const current = this.spanOf(this.active);
    const still = current && this.matchAt(current.start);
    this.setActive(still && sameSpan(still, current) ? current : null);
    this.redraw();
    this.fire();
  }

  private cancelRefresh() {
    if (this.refreshTimer === null) return;
    window.clearTimeout(this.refreshTimer);
    this.refreshTimer = null;
  }

  private fire() {
    const results = {
      resultIndex: this.activeIndex,
      resultCount: this.matches.length,
    };
    for (const listener of this.listeners) listener(results);
  }

  // --- selection --------------------------------------------------------------

  private selection(): Span | null {
    const range = this.term.getSelectionPosition();
    if (!range) return null;
    return {
      start: { row: range.start.y, col: range.start.x },
      end: { row: range.end.y, col: range.end.x },
    };
  }

  /** Select the match and scroll it into view; no match clears the selection. */
  private select(span: Span | null) {
    this.settingSelection = true;
    try {
      if (!span) {
        this.term.clearSelection();
      } else {
        const size =
          (span.end.row - span.start.row) * this.term.cols +
          span.end.col -
          span.start.col;
        this.term.select(span.start.col, span.start.row, size);
      }
    } finally {
      this.settingSelection = false;
    }
    this.selectionMoved = false;
    if (!span) return;
    const buffer = this.term.buffer.active;
    const { row } = span.start;
    if (row < buffer.viewportY || row >= buffer.viewportY + this.term.rows) {
      this.term.scrollLines(
        row - buffer.viewportY - Math.floor(this.term.rows / 2),
      );
    }
  }

  // --- tracking ---------------------------------------------------------------

  private track(span: Span): TrackedSpan {
    const buffer = this.term.buffer.active;
    const cursor = buffer.baseY + buffer.cursorY;
    return {
      buffer: buffer.type,
      start: this.term.registerMarker(span.start.row - cursor),
      startCol: span.start.col,
      end: this.term.registerMarker(span.end.row - cursor),
      endCol: span.end.col,
    };
  }

  private untrack(span: TrackedSpan | null) {
    span?.start.dispose();
    span?.end.dispose();
  }

  /**
   * Where a tracked span is now; null once it has scrolled away or belongs to
   * the other screen. A `partial` span (the range) keeps what is left of it
   * when its top has been trimmed.
   */
  private spanOf(span: TrackedSpan | null, partial = false): Span | null {
    if (
      !span ||
      span.buffer !== this.term.buffer.active.type ||
      span.end.isDisposed ||
      (span.start.isDisposed && !partial)
    ) {
      return null;
    }
    return {
      start: span.start.isDisposed
        ? { row: 0, col: 0 }
        : { row: span.start.line, col: span.startCol },
      end: { row: span.end.line, col: span.endCol },
    };
  }

  // --- decorations ------------------------------------------------------------

  /** One decoration per row the span covers. */
  private decorate(span: Span, color: string, layer: "top" | "bottom") {
    const buffer = this.term.buffer.active;
    const cursor = buffer.baseY + buffer.cursorY;
    const decorations: IDecoration[] = [];
    for (let row = span.start.row; row <= span.end.row; row++) {
      const x = row === span.start.row ? span.start.col : 0;
      const end = row === span.end.row ? span.end.col : this.term.cols;
      if (end <= x) continue;
      const marker = this.term.registerMarker(row - cursor);
      const decoration = this.term.registerDecoration({
        marker,
        x,
        width: end - x,
        backgroundColor: color,
        layer,
      });
      if (decoration) {
        decoration.onDispose(() => marker.dispose());
        decorations.push(decoration);
      } else {
        marker.dispose();
      }
    }
    return decorations;
  }

  /** Make `span` current, in `matches` or not; redraw() paints it. */
  private setActive(span: Span | null) {
    this.untrack(this.active);
    this.active = span ? this.track(span) : null;
    this.activeIndex = span
      ? this.matches.findIndex((match) => sameSpan(match, span))
      : -1;
  }

  private redraw() {
    for (const decorations of this.highlights) disposeAll(decorations);
    disposeAll(this.activeDecorations);
    this.highlights = this.matches.map((match, index) =>
      index === this.activeIndex
        ? []
        : this.decorate(match, this.colors.match, "top"),
    );
    const active = this.spanOf(this.active);
    this.activeDecorations = active
      ? this.decorate(active, this.colors.activeMatch, "top")
      : [];
  }

  /** Step to another match of the same search: only two matches repaint. */
  private moveActive(span: Span | null) {
    disposeAll(this.activeDecorations);
    if (this.activeIndex >= 0) {
      this.highlights[this.activeIndex] = this.decorate(
        this.matches[this.activeIndex],
        this.colors.match,
        "top",
      );
    }
    this.setActive(span);
    if (this.activeIndex >= 0) {
      disposeAll(this.highlights[this.activeIndex]);
      this.highlights[this.activeIndex] = [];
    }
    this.activeDecorations = span
      ? this.decorate(span, this.colors.activeMatch, "top")
      : [];
  }

  private queueScopeSync() {
    if (!this.scope || this.scopeSyncQueued) return;
    this.scopeSyncQueued = true;
    // onScroll fires once per line of a flood; one pass per parse slice
    // is enough, and a microtask still lands before the paint.
    queueMicrotask(() => {
      this.scopeSyncQueued = false;
      this.syncScope();
    });
  }

  /** Shade the range on the rows in view, reusing the decorations still right. */
  private syncScope() {
    const wanted = new Map<number, { x: number; width: number }>();
    const bounds = this.scope ? this.spanOf(this.scope, true) : null;
    if (bounds) {
      const top = this.term.buffer.active.viewportY;
      const last = Math.min(bounds.end.row, top + this.term.rows - 1);
      for (let row = Math.max(top, bounds.start.row); row <= last; row++) {
        const x = row === bounds.start.row ? bounds.start.col : 0;
        const end = row === bounds.end.row ? bounds.end.col : this.term.cols;
        if (end > x) wanted.set(row, { x, width: end - x });
      }
    }
    this.scopeDecorations = this.scopeDecorations.filter((entry) => {
      const { decoration } = entry;
      const row = decoration.marker.line;
      const want = wanted.get(row);
      if (
        !decoration.isDisposed &&
        want &&
        want.x === entry.x &&
        want.width === entry.width
      ) {
        wanted.delete(row);
        return true;
      }
      decoration.dispose();
      return false;
    });
    for (const [row, { x, width }] of wanted) {
      const [decoration] = this.decorate(
        { start: { row, col: x }, end: { row, col: x + width } },
        this.colors.scope,
        "bottom",
      );
      if (decoration) this.scopeDecorations.push({ decoration, x, width });
    }
  }

  private dropScopeDecorations() {
    for (const { decoration } of this.scopeDecorations) decoration.dispose();
    this.scopeDecorations = [];
  }
}
