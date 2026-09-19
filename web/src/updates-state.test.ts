import { describe, expect, it } from "vitest";

import type { UpdateInventory, UpdateTransaction } from "./types";
import {
  actionableUpdateError,
  applyConfirmation,
  canPrepareUpdate,
  componentImpact,
  isExpectedReconnect,
  phaseLabel,
  persistPendingTransaction,
  prepareConfirmation,
  readPendingTransaction,
  readPendingTransactions,
  rollbackMessage,
  targetVersionFor,
  transactionResult,
  updateCardStatus,
} from "./updates-state";

function inventory(overrides: Partial<UpdateInventory> = {}): UpdateInventory {
  return {
    component: "git",
    display_name: "Git MCP",
    class: "mcp_binary",
    provider: "github_13thx",
    platform: { os: "darwin", arch: "arm64" },
    host_mode: "runtime_only",
    source_present: false,
    installed_version: "1.0.0",
    running_version: "1.0.0",
    desired_version: "1.0.0",
    latest_version: "1.0.0",
    update_available: false,
    installation_health: "healthy",
    drift: "current",
    last_check: { checked_at_ms: 1, status: "ok", error: null },
    ...overrides,
  };
}

function transaction(overrides: Partial<UpdateTransaction> = {}): UpdateTransaction {
  return {
    transaction_id: "txn-git-test",
    component: "git",
    source_version: "1.0.0",
    target_version: "1.1.0",
    phase: "staged",
    was_running: true,
    rollback_succeeded: null,
    error: null,
    updated_at_ms: 1,
    ...overrides,
  };
}

class MemoryStorage {
  value: string | null = null;
  getItem() {
    return this.value;
  }
  setItem(_key: string, value: string) {
    this.value = value;
  }
  removeItem() {
    this.value = null;
  }
}

describe("updates state", () => {
  it("renders current, update-available, drift and broken states deterministically", () => {
    expect(updateCardStatus(inventory())).toEqual({
      label: "Current",
      attention: false,
      danger: false,
    });
    expect(updateCardStatus(inventory({ drift: "update_available" })).label).toBe(
      "Update available",
    );
    expect(updateCardStatus(inventory({ drift: "drifted" })).attention).toBe(true);
    expect(updateCardStatus(inventory({ drift: "broken" })).danger).toBe(true);
  });

  it("uses latest approved release as target and gates unsupported or broken updates", () => {
    const available = inventory({ latest_version: "1.1.0", update_available: true });
    expect(targetVersionFor(available)).toBe("1.1.0");
    expect(canPrepareUpdate(available, "1.1.0")).toBe(true);
    expect(canPrepareUpdate(available, "1.0.0")).toBe(false);
    expect(canPrepareUpdate(inventory({ installation_health: "broken" }), "1.1.0")).toBe(false);
    expect(canPrepareUpdate(inventory({ component: "tunnel" }), "1.1.0")).toBe(true);
    expect(
      canPrepareUpdate(
        inventory({ component: "tunnel", installed_version: "1.0.0" }),
        "1.0.0",
      ),
    ).toBe(true);
  });

  it("communicates stopped/unknown and control-path restart impact without fake progress", () => {
    expect(componentImpact("git", true)).toContain("stopped MCP remains stopped");
    expect(componentImpact("git", false)).toContain("not currently proven");
    expect(componentImpact("gateway", true)).toContain("control path");
    expect(componentImpact("studio", true)).toContain("expected to reconnect");
    expect(componentImpact("tunnel", true)).toContain("preserves config/credentials");
    expect(rollbackMessage("git")).toContain("Automatic rollback");
    expect(phaseLabel("gateway_catalog_verifying")).toBe("Verifying Gateway catalog");
  });

  it("builds prepare/apply confirmations with current target impact and rollback context", () => {
    const item = inventory({ component: "studio", display_name: "MCP Studio", installed_version: "1.0.0", running_version: "1.0.0" });
    const prepared = prepareConfirmation(item, "1.1.0", "serving dashboard");
    expect(prepared).toContain("Installed: 1.0.0");
    expect(prepared).toContain("Running: 1.0.0");
    expect(prepared).toContain("Target: 1.1.0");
    expect(prepared).toContain("serving dashboard");
    expect(prepared).toContain("Automatic rollback");

    const tunnelReinstall = prepareConfirmation(
      inventory({
        component: "tunnel",
        display_name: "OpenAI tunnel-client",
        installed_version: "0.0.14",
      }),
      "0.0.14",
      "running",
    );
    expect(tunnelReinstall).toContain("verified same-version force reinstall");

    const apply = applyConfirmation(item, transaction({ component: "studio", target_version: "1.1.0" }), "serving dashboard");
    expect(apply).toContain("Apply staged MCP Studio update?");
    expect(apply).toContain("interrupt the dashboard");
    expect(apply).toContain("queried after reconnect");
  });

  it("maps transaction result and expected reconnect state", () => {
    expect(
      isExpectedReconnect(transaction({ component: "studio", phase: "activation_pending" })),
    ).toBe(true);
    expect(
      isExpectedReconnect(transaction({ component: "gateway", phase: "gateway_reconnecting" })),
    ).toBe(true);
    expect(isExpectedReconnect(transaction({ phase: "activating" }))).toBe(false);
    expect(transactionResult(transaction({ phase: "rolled_back", rollback_succeeded: true }))).toBe(
      "Update failed; rollback succeeded",
    );
  });

  it("persists multiple pending transactions for reconnect and clears only terminal identity", () => {
    const storage = new MemoryStorage();
    const studio = transaction({
      transaction_id: "txn-studio-test",
      component: "studio",
      phase: "activation_pending",
    });
    const gateway = transaction({
      transaction_id: "txn-gateway-test",
      component: "gateway",
      phase: "gateway_reconnecting",
    });
    persistPendingTransaction(storage, studio);
    persistPendingTransaction(storage, gateway);

    expect(readPendingTransactions(storage)).toEqual([
      { transaction_id: "txn-studio-test", component: "studio" },
      { transaction_id: "txn-gateway-test", component: "gateway" },
    ]);
    expect(readPendingTransaction(storage)).toEqual({
      transaction_id: "txn-studio-test",
      component: "studio",
    });

    persistPendingTransaction(storage, { ...studio, phase: "completed" });
    expect(readPendingTransactions(storage)).toEqual([
      { transaction_id: "txn-gateway-test", component: "gateway" },
    ]);

    storage.value = JSON.stringify([
      { transaction_id: "txn-x", component: "unknown" },
      { transaction_id: "txn-git-ok", component: "git" },
    ]);
    expect(readPendingTransactions(storage)).toEqual([
      { transaction_id: "txn-git-ok", component: "git" },
    ]);
  });

  it("maps sanitized backend errors to actionable operator messages", () => {
    expect(actionableUpdateError("checksum_mismatch")).toContain("checksum");
    expect(actionableUpdateError("launcher_unavailable")).toContain("external Studio");
    expect(actionableUpdateError("unknown_future_error")).toBe("unknown future error");
    expect(actionableUpdateError(null)).toBeNull();
  });
});