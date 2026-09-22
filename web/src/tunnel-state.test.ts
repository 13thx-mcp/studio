import { describe, expect, it } from "vitest";

import { mergeLogEntries, tunnelActionEnabled } from "./state";
import type { StudioEvent, TunnelLogEntry, TunnelStatus } from "./types";

const stoppedTunnel: TunnelStatus = {
  name: "Secure tunnel",
  state: "stopped",
  runtime_available: true,
  pid: null,
  uptime_ms: null,
  restart_count: 0,
  crash_count: 0,
  last_exit_code: null,
  last_error: null,
};

function tunnelLog(sequence: number, message: string): TunnelLogEntry {
  return {
    sequence,
    timestamp_ms: sequence,
    stream: "stdout",
    message,
  };
}

describe("tunnel lifecycle state", () => {
  it("enables and disables lifecycle actions by state", () => {
    expect(tunnelActionEnabled(stoppedTunnel, "start")).toBe(true);
    expect(tunnelActionEnabled(stoppedTunnel, "stop")).toBe(false);
    expect(tunnelActionEnabled(stoppedTunnel, "restart")).toBe(true);

    const running: TunnelStatus = {
      ...stoppedTunnel,
      state: "running",
      pid: 1234,
      uptime_ms: 5000,
    };
    expect(tunnelActionEnabled(running, "start")).toBe(false);
    expect(tunnelActionEnabled(running, "stop")).toBe(true);
    expect(tunnelActionEnabled(running, "restart")).toBe(true);
  });

  it("keeps stop available when an active tunnel runtime disappears", () => {
    const running: TunnelStatus = {
      ...stoppedTunnel,
      state: "running",
      runtime_available: false,
      pid: 1234,
    };

    expect(tunnelActionEnabled(running, "start")).toBe(false);
    expect(tunnelActionEnabled(running, "restart")).toBe(false);
    expect(tunnelActionEnabled(running, "stop")).toBe(true);
  });

  it("reconciles tunnel logs by sequence", () => {
    expect(
      mergeLogEntries([tunnelLog(2, "old")], [tunnelLog(1, "first"), tunnelLog(2, "new")]),
    ).toEqual([tunnelLog(1, "first"), tunnelLog(2, "new")]);
  });

  it("models reconnect snapshots and realtime tunnel updates without secret fields", () => {
    const snapshot: StudioEvent = {
      type: "snapshot",
      servers: [],
      tunnel: stoppedTunnel,
    };
    const update: StudioEvent = {
      type: "tunnel_status",
      status: { ...stoppedTunnel, state: "running", pid: 2222 },
    };

    expect(snapshot.tunnel.state).toBe("stopped");
    expect(update.status.state).toBe("running");
    expect(Object.keys(update.status)).not.toContain("secret");
    expect(Object.keys(update.status)).not.toContain("runtime");
    expect(Object.keys(update.status)).not.toContain("config_file");
  });
});
