import { describe, expect, it } from "vitest";

import { isBackgroundEvent } from "./fork-background-events";

describe("isBackgroundEvent", () => {
  it("is true only for payloads flagged background: true", () => {
    expect(isBackgroundEvent({ background: true })).toBe(true);
    expect(isBackgroundEvent({ background: "true" })).toBe(false);
    expect(isBackgroundEvent({ failed_syncs: [] })).toBe(false);
    expect(isBackgroundEvent(null)).toBe(false);
    expect(isBackgroundEvent(undefined)).toBe(false);
    expect(isBackgroundEvent("background")).toBe(false);
  });
});
