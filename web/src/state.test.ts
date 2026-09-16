import { describe, expect, it } from "vitest";

import { actionEnabled, formatUptime, mergeLogEntries } from "./state";
import type { LogEntry, ProcessStatus } from "./types";

const baseStatus: ProcessStatus = {
  id: "filesystem",
  name: "Filesystem",
  state: "stopped",
  pid: null,
  uptime_ms: null,
  restart_count: 0,
  crash_count: 0,
  last_exit_code: null,
  last_error: null,
};

function log(sequence: number, message: string): LogEntry {
  return {
    sequence,
    timestamp_ms: sequence,
    stream: "stdout",
    message,
  };
}

describe("mergeLogEntries", () => {
  it("deduplicates by sequence and sorts ascending", () => {
    expect(mergeLogEntries([log(2, "old")], [log(1, "first"), log(2, "new")])).toEqual([
      log(1, "first"),
      log(2, "new"),
    ]);
  });
});

describe("formatUptime", () => {
  it("formats null and elapsed time", () => {
    expect(formatUptime(null)).toBe("—");
    expect(formatUptime(3_661_000)).toBe("01:01:01");
  });
});

describe("actionEnabled", () => {
  it("reflects lifecycle state", () => {
    expect(actionEnabled(baseStatus, "start")).toBe(true);
    expect(actionEnabled(baseStatus, "stop")).toBe(false);

    const running = { ...baseStatus, state: "running" as const };
    expect(actionEnabled(running, "start")).toBe(false);
    expect(actionEnabled(running, "stop")).toBe(true);
    expect(actionEnabled(running, "restart")).toBe(true);

    const starting = { ...baseStatus, state: "starting" as const };
    expect(actionEnabled(starting, "restart")).toBe(false);
  });
});
