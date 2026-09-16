import { describe, expect, it } from "vitest";

import { nextRetryDelay } from "./realtime";

describe("nextRetryDelay", () => {
  it("doubles reconnect delay until the cap", () => {
    expect(nextRetryDelay(500)).toBe(1000);
    expect(nextRetryDelay(5000)).toBe(10000);
    expect(nextRetryDelay(10000)).toBe(10000);
  });
});
