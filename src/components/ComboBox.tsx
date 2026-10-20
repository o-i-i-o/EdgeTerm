import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { Icon } from "./icons";

/** Space between the field and its list, and the least kept to the window edge. */
const LIST_GAP = 4;
const EDGE_MARGIN = 6;
/** About eleven rows; a longer list scrolls. */
const LIST_MAX_HEIGHT = 320;

/**
 * The options a combo box lists. Opened from the caret or the arrow keys it
 * offers all of them; once the user types, only those that start with what
 * the field holds. A list that would only repeat the field is no list.
 */
export function comboMatches(
  options: readonly string[],
  text: string,
  filtered: boolean,
): string[] {
  if (!filtered) return [...options];
  const prefix = text.toLowerCase();
  const matches = options.filter((option) =>
    option.toLowerCase().startsWith(prefix),
  );
  return matches.length === 1 && matches[0] === text ? [] : matches;
}

interface Placement {
  left: number;
  width: number;
  /** Below the field, or above it when only there is room. */
  top?: number;
  bottom?: number;
  maxHeight: number;
}

interface Props {
  id?: string;
  value: string;
  /** Every edit, typed or picked; the caller may clean it before storing. */
  onChange: (value: string) => void;
  options: readonly string[];
  placeholder?: string;
  inputMode?: HTMLAttributes<HTMLInputElement>["inputMode"];
  maxLength?: number;
  /** A unit or similar after the caret, inside the field's frame. */
  suffix?: ReactNode;
  /** The caret button's tooltip and accessible name. */
  listLabel: string;
}

/**
 * A text field with suggestions, drawn as one of the app's dropdowns rather
 * than the WebView's <datalist> popup, which WebView2 shows as a white
 * native list (issue #97). Typing narrows the list to what starts with the
 * text; the caret at the end, or ↓, lists everything with the current value
 * marked. ↑ / ↓ move, Enter takes the highlighted entry, Esc closes.
 *
 * Focus stays in the field the whole time: the list is pointed at, never
 * focused, so typing carries on. It hangs on <body> because a dragged dialog
 * carries a transform and would become the containing block of a fixed list.
 */
export function ComboBox({
  id,
  value,
  onChange,
  options,
  placeholder,
  inputMode,
  maxLength,
  suffix,
  listLabel,
}: Props) {
  const fieldRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = useId();
  // null while closed; otherwise whether the list is narrowed to the text.
  const [open, setOpen] = useState<{ filtered: boolean } | null>(null);
  const [active, setActive] = useState(-1);
  const [placement, setPlacement] = useState<Placement | null>(null);

  const matches = open ? comboMatches(options, value, open.filtered) : [];
  const shown = matches.length > 0;
  const isOpen = open !== null;
  const typed = open?.filtered ? value.length : 0;

  const close = () => {
    setOpen(null);
    setActive(-1);
  };

  const openAll = () => {
    setOpen({ filtered: false });
    setActive(options.indexOf(value));
  };

  const pick = (option: string) => {
    onChange(option);
    close();
  };

  // Below the field when the list fits there, else on whichever side has
  // more room, never over the field being typed into.
  useLayoutEffect(() => {
    if (!shown) {
      setPlacement(null);
      return;
    }
    const field = fieldRef.current?.getBoundingClientRect();
    const list = listRef.current;
    if (!field || !list) return;
    const wanted = Math.min(list.scrollHeight + 2, LIST_MAX_HEIGHT);
    const below = window.innerHeight - field.bottom - LIST_GAP - EDGE_MARGIN;
    const above = field.top - LIST_GAP - EDGE_MARGIN;
    const down = wanted <= below || below >= above;
    setPlacement({
      left: field.left,
      width: field.width,
      top: down ? field.bottom + LIST_GAP : undefined,
      bottom: down ? undefined : window.innerHeight - field.top + LIST_GAP,
      maxHeight: Math.min(wanted, down ? below : above),
    });
  }, [shown, matches.length]);

  // Keep the highlighted entry in sight. scrollTop rather than
  // scrollIntoView, which may scroll the page under a fixed list too.
  useLayoutEffect(() => {
    const list = listRef.current;
    const row = list?.children[active] as HTMLElement | undefined;
    if (!list || !row) return;
    if (row.offsetTop < list.scrollTop) {
      list.scrollTop = row.offsetTop - 6;
    } else if (
      row.offsetTop + row.offsetHeight >
      list.scrollTop + list.clientHeight
    ) {
      list.scrollTop = row.offsetTop + row.offsetHeight - list.clientHeight + 6;
    }
  }, [active, placement]);

  // The moments a native list goes away: a press elsewhere (the dialog's
  // backdrop keeps focus in the field, so blur alone does not cover it),
  // anything but the list itself scrolling, the window changing size or
  // losing focus.
  useEffect(() => {
    if (!isOpen) return;
    const onMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (fieldRef.current?.contains(target)) return;
      if (listRef.current?.contains(target)) return;
      close();
    };
    const onScroll = (event: Event) => {
      if (!listRef.current?.contains(event.target as Node)) close();
    };
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
    };
  }, [isOpen]);

  const step = (by: 1 | -1) => {
    if (!shown) {
      openAll();
      return;
    }
    setActive((current) =>
      current < 0
        ? by > 0
          ? 0
          : matches.length - 1
        : (current + by + matches.length) % matches.length,
    );
  };

  const listStyle: CSSProperties = placement
    ? {
        left: placement.left,
        width: placement.width,
        top: placement.top ?? "auto",
        bottom: placement.bottom ?? "auto",
        maxHeight: placement.maxHeight,
      }
    : // Measured before it is placed; never painted there.
      { visibility: "hidden" };

  return (
    <div className="input-with-suffix combo-field" ref={fieldRef}>
      <input
        ref={inputRef}
        id={id}
        type="text"
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={shown}
        aria-controls={listId}
        aria-activedescendant={
          shown && active >= 0 ? `${listId}-${active}` : undefined
        }
        autoComplete="off"
        spellCheck={false}
        inputMode={inputMode}
        maxLength={maxLength}
        placeholder={placeholder}
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
          setOpen({ filtered: true });
          setActive(-1);
        }}
        onBlur={close}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          switch (event.key) {
            case "ArrowDown":
            case "ArrowUp":
              event.preventDefault();
              step(event.key === "ArrowDown" ? 1 : -1);
              return;
            case "Enter":
              if (shown && active >= 0 && active < matches.length) {
                event.preventDefault();
                pick(matches[active]);
              }
              return;
            case "Escape":
              if (isOpen) {
                event.preventDefault();
                event.stopPropagation();
                close();
              }
              return;
          }
        }}
      />
      <button
        type="button"
        className={`combo-toggle${shown ? " is-open" : ""}`}
        // The field keeps the focus, so the list opens with the caret in it
        // and typing goes on narrowing it. Keyboard users have ↓.
        tabIndex={-1}
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => {
          if (shown) {
            close();
            return;
          }
          inputRef.current?.focus();
          openAll();
        }}
        title={listLabel}
        aria-label={listLabel}
      >
        <Icon name="chevron-down" />
      </button>
      {suffix && <span>{suffix}</span>}
      {shown &&
        createPortal(
          <div
            ref={listRef}
            id={listId}
            className={`menu-dropdown combo-list${placement?.bottom !== undefined ? " is-above" : ""}`}
            role="listbox"
            style={listStyle}
            // A press on an entry must not take the focus out of the field.
            onMouseDown={(event) => event.preventDefault()}
          >
            {matches.map((option, index) => (
              <div
                key={option}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={index === active}
                className={`combo-option${index === active ? " is-active" : ""}`}
                onMouseMove={() => {
                  if (index !== active) setActive(index);
                }}
                onClick={() => pick(option)}
              >
                <span className="menu-check" aria-hidden="true">
                  {option === value && <Icon name="check" />}
                </span>
                <span className="combo-option-label">
                  {typed > 0 && (
                    <span className="combo-match">{option.slice(0, typed)}</span>
                  )}
                  {option.slice(typed)}
                </span>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
