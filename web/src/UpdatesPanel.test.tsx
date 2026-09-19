import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import UpdatesPanel from "./UpdatesPanel";
import type { ReconciliationView, UpdateInventory, UpdateTransaction } from "./types";

const noop = async () => {};

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
    latest_version: "1.1.0",
    update_available: true,
    installation_health: "healthy",
    drift: "update_available",
    last_check: { checked_at_ms: 1, status: "ok", error: null },
    ...overrides,
  };
}

const reconciliation: ReconciliationView = {
  host_id: "aira",
  state: "managed_safe_drift",
  phase: "reconcile_ready",
  safe_to_reconcile: true,
  affected_surfaces: ["gateway.exec"],
  gateway_reload_required: true,
  tunnel_restart_required: false,
  studio_restart_required: false,
  studio_config_activation: "active",
  catalog_fingerprint: "0123456789abcdef0123456789abcdef",
  generation: 3,
  client_freshness: "refresh_pending",
  rollback_succeeded: null,
  last_error: null,
  checked_at_ms: 1,
};

function render(
  items: UpdateInventory[],
  transactions: Partial<Record<UpdateInventory["component"], UpdateTransaction>> = {},
  reconciliationView: ReconciliationView = reconciliation,
) {
  return renderToStaticMarkup(
    <UpdatesPanel
      inventory={items}
      reconciliation={reconciliationView}
      transactions={transactions}
      runtimeStates={{ git: "running", studio: "serving dashboard" }}
      connection="connected"
      pending={null}
      onCheckUpdates={noop}
      onPrepare={async () => {}}
      onApply={async () => {}}
      onRefreshTransaction={async () => {}}
      onCheckReconciliation={noop}
      onApplyReconciliation={noop}
    />,
  );
}

describe("UpdatesPanel", () => {
  it("renders current versions, update availability, drift and Fleet summary", () => {
    const html = render([
      inventory(),
      inventory({
        component: "filesystem",
        display_name: "Filesystem MCP",
        latest_version: "1.0.0",
        update_available: false,
        drift: "current",
      }),
      inventory({
        component: "blender",
        display_name: "Blender MCP",
        installation_health: "broken",
        drift: "broken",
      }),
    ]);
    expect(html).toContain("Updates / Fleet");
    expect(html).toContain("Git MCP");
    expect(html).toContain("1.0.0");
    expect(html).toContain("1.1.0");
    expect(html).toContain("Update available");
    expect(html).toContain("Broken");
    expect(html).toContain("runtime only");
    expect(html).toContain("refresh pending");
  });

  it("renders expected Studio reconnect transaction and rollback result without internal authority", () => {
    const transaction: UpdateTransaction = {
      transaction_id: "txn-studio-safe",
      component: "studio",
      source_version: "1.0.0",
      target_version: "1.1.0",
      phase: "activation_pending",
      was_running: true,
      rollback_succeeded: null,
      error: null,
      updated_at_ms: 1,
    };
    const html = render(
      [
        inventory({
          component: "studio",
          display_name: "MCP Studio",
          class: "service",
          running_version: "1.0.0",
        }),
      ],
      { studio: transaction },
    );
    expect(html).toContain("Activation pending");
    expect(html).toContain("Expected control-path reconnect");
    expect(html).toContain("txn-studio-safe");
    expect(html).not.toContain("download_url");
    expect(html).not.toContain("install_path");
    expect(html).not.toContain("content_b64");
  });

  it("renders persistent Studio restart-required guidance without a fake restart action", () => {
    const html = render(
      [inventory({ component: "studio", display_name: "MCP Studio", class: "service" })],
      {},
      {
        ...reconciliation,
        state: "synchronized",
        phase: "synchronized",
        safe_to_reconcile: false,
        affected_surfaces: [],
        gateway_reload_required: false,
        studio_restart_required: true,
        studio_config_activation: "restart_required",
        client_freshness: "unknown",
      },
    );
    expect(html).toContain("Studio configuration is reconciled on disk");
    expect(html).toContain("studio.config");
    expect(html).toContain("existing trusted service/launcher");
    expect(html).not.toContain(">restart Studio<");
  });

  it("shows tunnel as a versioned runtime with supported verified activation", () => {
    const html = render([
      inventory({
        component: "tunnel",
        display_name: "OpenAI tunnel-client",
        class: "upstream_runtime",
        provider: "github_openai",
        installed_version: "0.0.14",
        latest_version: "0.0.14",
        update_available: false,
      }),
    ]);
    expect(html).toContain("OpenAI tunnel-client");
    expect(html).toContain("preserves config/credentials");
    expect(html).not.toContain("not implemented by the current backend");
  });
});