import { beforeEach, describe, expect, it } from "vitest";

import {
  describeCrash,
  describeError,
  forgetUncaught,
  recordUncaught,
  recordedUncaught,
} from "./crashReport";

describe("crash report", () => {
  beforeEach(() => {
    forgetUncaught();
  });

  it("describes a thrown Error with its message and stack", () => {
    const described = describeError(new Error("boom"));
    expect(described.message).toBe("boom");
    expect(described.stack).toContain("crashReport.test.ts");
  });

  it("describes a thrown non-error value", () => {
    expect(describeError("just a string").message).toBe("just a string");
    expect(describeError(undefined).message).toBe("undefined");
  });

  it("keeps the message, the component stack and the earlier errors", () => {
    recordUncaught("first failure (/src/App.tsx:12)");
    recordUncaught("second failure");

    const report = describeCrash({
      error: new Error("render exploded"),
      componentStack: "\n    in Panel\n    in App\n",
    });

    expect(report).toContain("EdgeTerm crash report");
    expect(report).toContain("message: render exploded");
    expect(report).toContain("component stack:");
    // The blank line React puts around the stack is trimmed off, its own
    // indentation is kept: it is what makes the trace readable.
    expect(report).toContain("in Panel\n    in App");
    expect(report).toContain("earlier uncaught errors:");
    expect(report).toContain("first failure (/src/App.tsx:12)");
    expect(report).toContain("second failure");
  });

  it("omits the sections a render-less failure has nothing for", () => {
    const report = describeCrash({ error: "nothing to see" });
    expect(report).toContain("message: nothing to see");
    expect(report).not.toContain("component stack:");
    expect(report).not.toContain("earlier uncaught errors:");
  });

  it("drops the oldest uncaught errors past the cap", () => {
    for (let index = 0; index < 13; index += 1) {
      recordUncaught(`failure ${index}`);
    }

    const kept = recordedUncaught();
    expect(kept).toHaveLength(10);
    expect(kept[0]).toBe("failure 3");
    expect(kept[9]).toBe("failure 12");
  });

  it("hands out a copy, so a caller cannot edit the buffer", () => {
    recordUncaught("failure");
    recordedUncaught().push("injected");
    expect(recordedUncaught()).toEqual(["failure"]);
  });
});
