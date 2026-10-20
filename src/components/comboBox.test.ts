import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ComboBox, comboMatches } from "./ComboBox";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const RATES = ["9600", "19200", "115200", "230400", "921600"];

describe("comboMatches", () => {
  it("offers everything when the list was opened rather than typed into", () => {
    expect(comboMatches(RATES, "115200", false)).toEqual(RATES);
  });

  it("narrows to the options that start with the typed text", () => {
    expect(comboMatches(RATES, "9", true)).toEqual(["9600", "921600"]);
    expect(comboMatches(RATES, "", true)).toEqual(RATES);
    expect(comboMatches(["en_US.UTF-8"], "EN_", true)).toEqual(["en_US.UTF-8"]);
  });

  it("drops a list that would only repeat the field", () => {
    expect(comboMatches(RATES, "115200", true)).toEqual([]);
    expect(comboMatches(RATES, "1152000", true)).toEqual([]);
  });
});

describe("ComboBox", () => {
  let container: HTMLDivElement;
  let root: Root;

  function Harness({ initial }: { initial: string }) {
    const [value, setValue] = useState(initial);
    return createElement(ComboBox, {
      value,
      onChange: setValue,
      options: RATES,
      listLabel: "Rates",
    });
  }

  const input = () => container.querySelector("input") as HTMLInputElement;
  const toggle = () =>
    container.querySelector(".combo-toggle") as HTMLButtonElement;
  const rows = () =>
    [...document.querySelectorAll(".combo-list .combo-option")].map(
      (row) => row.textContent,
    );
  const activeRow = () =>
    document.querySelector(".combo-option.is-active")?.textContent ?? null;

  const type = (text: string) =>
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(input(), text);
      input().dispatchEvent(new Event("input", { bubbles: true }));
    });
  const key = (name: string) =>
    act(() => {
      input().dispatchEvent(
        new KeyboardEvent("keydown", { key: name, bubbles: true }),
      );
    });

  beforeEach(() => {
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it("lists what starts with the typed text, and Enter takes the highlighted one", () => {
    act(() => root.render(createElement(Harness, { initial: "" })));
    expect(rows()).toEqual([]);

    type("9");
    expect(rows()).toEqual(["9600", "921600"]);
    expect(activeRow()).toBeNull();
    expect(
      [...document.querySelectorAll(".combo-match")].map((m) => m.textContent),
    ).toEqual(["9", "9"]);

    key("ArrowDown");
    key("ArrowDown");
    expect(activeRow()).toBe("921600");
    key("Enter");
    expect(input().value).toBe("921600");
    expect(rows()).toEqual([]);
  });

  it("opens on the whole list from the caret, with the current value marked", () => {
    act(() => root.render(createElement(Harness, { initial: "115200" })));
    act(() => toggle().click());
    expect(rows()).toEqual(RATES);
    expect(activeRow()).toBe("115200");
    expect(document.querySelectorAll(".combo-option .menu-check .icon")).toHaveLength(1);
    expect(document.activeElement).toBe(input());

    act(() => toggle().click());
    expect(rows()).toEqual([]);
  });

  it("opens on ↓ too, and closes on Esc or a press elsewhere", () => {
    act(() => root.render(createElement(Harness, { initial: "19200" })));
    key("ArrowDown");
    expect(rows()).toEqual(RATES);
    expect(activeRow()).toBe("19200");
    key("ArrowUp");
    expect(activeRow()).toBe("9600");
    key("Escape");
    expect(rows()).toEqual([]);
    expect(input().value).toBe("19200");

    key("ArrowDown");
    act(() => {
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(rows()).toEqual([]);
  });

  it("takes a clicked entry", () => {
    act(() => root.render(createElement(Harness, { initial: "" })));
    type("2");
    const row = [...document.querySelectorAll(".combo-option")].find(
      (r) => r.textContent === "230400",
    ) as HTMLElement;
    act(() => {
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      row.click();
    });
    expect(input().value).toBe("230400");
    expect(rows()).toEqual([]);
  });
});
