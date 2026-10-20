/**
 * What the search overlay and the terminal's find engine (terminalFind.ts)
 * have to agree on. It lives apart from both so the overlay can read it
 * without pulling xterm into the start-up bundle (see `ensureController` in
 * actions.ts).
 */

/** Matches past this many are neither highlighted nor counted. */
export const SEARCH_HIGHLIGHT_LIMIT = 1000;

export interface SearchResults {
  /** Zero-based index of the selected match; -1 when it is not among the highlighted ones. */
  resultIndex: number;
  /** Highlighted match count, capped at SEARCH_HIGHLIGHT_LIMIT. */
  resultCount: number;
}

/** The find box's toggles, VS Code's three (issue #95). */
export interface FindOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
}

export const DEFAULT_FIND_OPTIONS: Readonly<FindOptions> = {
  caseSensitive: false,
  wholeWord: false,
  regex: false,
};

/**
 * The query as a global RegExp. Plain text is escaped, so both kinds of query
 * take the same path through `findInText`. Throws a SyntaxError for a
 * malformed regular expression.
 */
export function compileQuery(query: string, options: FindOptions): RegExp {
  const source = options.regex
    ? query
    : query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(source, options.caseSensitive ? "g" : "gi");
}

/** Why the query cannot be searched, or null when it can. */
export function queryError(query: string, options: FindOptions): string | null {
  if (!query || !options.regex) return null;
  try {
    compileQuery(query, options);
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

/** VS Code's default `editor.wordSeparators`; whitespace separates as well. */
const WORD_SEPARATORS = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

const isSeparator = (char: string) =>
  WORD_SEPARATORS.includes(char) || /\s/.test(char);

/**
 * Whether `text[start, end)` stands on its own as a word. As in VS Code, an
 * edge of the match that is itself a separator counts as a boundary, so
 * `-v` is a whole word in `ls -v` and `x=` in `x=1`.
 */
function isWholeWord(text: string, start: number, end: number): boolean {
  const left =
    start === 0 || isSeparator(text[start - 1]) || isSeparator(text[start]);
  const right =
    end === text.length || isSeparator(text[end]) || isSeparator(text[end - 1]);
  return left && right;
}

/**
 * Every match of `pattern` (from `compileQuery`) in `text`, as [start, end)
 * offsets in reading order. Empty matches are skipped — `a*` or `^` would
 * otherwise match between every character — and under `wholeWord` a match
 * inside a longer word is passed over without hiding a later one: `foo`
 * still finds the second word of `foobar foo`.
 */
export function findInText(
  text: string,
  pattern: RegExp,
  wholeWord: boolean,
): [number, number][] {
  const found: [number, number][] = [];
  pattern.lastIndex = 0;
  for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
    const start = match.index;
    const end = start + match[0].length;
    if (end === start || (wholeWord && !isWholeWord(text, start, end))) {
      pattern.lastIndex = start + 1;
      continue;
    }
    found.push([start, end]);
  }
  return found;
}

// The box also closes on Esc pressed in the terminal (issue #95), as VS
// Code's terminal find does. That key arrives in xterm's key filter, which
// is not React, so the open box registers how to close it here.
let closeFind: (() => void) | null = null;

/** Called by the open find box with its close action, and with null when it goes. */
export function setFindCloser(close: (() => void) | null): void {
  closeFind = close;
}

/** Closes the find box; false when it is not open. */
export function closeFindBox(): boolean {
  if (!closeFind) return false;
  closeFind();
  return true;
}
