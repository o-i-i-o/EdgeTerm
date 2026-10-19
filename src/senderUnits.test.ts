import { describe, expect, it } from "vitest";

import { buildUnits, firstLine, splitLines } from "./senderUnits";

describe("Sender units", () => {
  it("sends a multi-line command one line at a time, each with the ending", () => {
    expect(buildUnits("cd /var/log\n\ntail -n 5 syslog\r\n", "lf")).toEqual([
      "cd /var/log\n",
      "tail -n 5 syslog\n",
    ]);
    expect(buildUnits("a\nb", "crlf")).toEqual(["a\r\n", "b\r\n"]);
    expect(buildUnits("a\nb", "none")).toEqual(["a", "b"]);
  });

  it("keeps a line's own leading whitespace and drops only blank lines", () => {
    expect(splitLines("  indented\n   \n\t\nlast")).toEqual([
      "  indented",
      "last",
    ]);
    expect(buildUnits("", "lf")).toEqual([]);
    expect(buildUnits("\n\n", "lf")).toEqual([]);
  });

  it("names a command after its first line", () => {
    expect(firstLine("\nuptime\ndf -h")).toBe("uptime");
    expect(firstLine("   ")).toBe("   ");
  });
});
