import { afterEach, describe, expect, it, vi } from "vitest";

import {
  applyUpdate,
  checkReconciliation,
  checkUpdates,
  getUpdateTransaction,
  prepareUpdate,
} from "./api";

const ok = (body: unknown) =>
  Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );

afterEach(() => {
  vi.restoreAllMocks();
});

describe("updates API", () => {
  it("sends only version authority during prepare", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      ok({
        transaction_id: "txn-git-x",
        component: "git",
        source_version: "1.0.0",
        target_version: "1.1.0",
        phase: "staged",
        was_running: true,
        rollback_succeeded: null,
        error: null,
        updated_at_ms: 1,
      }),
    );

    await prepareUpdate("git", "1.1.0");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/updates/git/prepare");
    const body = String(fetchMock.mock.calls[0][1]?.body);
    expect(JSON.parse(body)).toEqual({ version: "1.1.0" });
    for (const forbidden of ["path", "url", "command", "pid", "checksum"]) {
      expect(body).not.toContain(forbidden);
    }
  });

  it("sends only the server-generated transaction id during apply", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      ok({
        transaction_id: "txn-studio-x",
        component: "studio",
        source_version: "1.0.0",
        target_version: "1.1.0",
        phase: "activation_pending",
        was_running: true,
        rollback_succeeded: null,
        error: null,
        updated_at_ms: 1,
      }),
    );

    await applyUpdate("studio", "txn-studio-x");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/updates/studio/apply");
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toEqual({
      transaction_id: "txn-studio-x",
    });
  });

  it("uses explicit check and durable transaction query endpoints", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
      if (String(input).includes("update-transactions")) {
        return ok({
          transaction_id: "txn-studio-x",
          component: "studio",
          source_version: "1.0.0",
          target_version: "1.1.0",
          phase: "completed",
          was_running: true,
          rollback_succeeded: null,
          error: null,
          updated_at_ms: 1,
        });
      }
      return ok([]);
    });

    await checkUpdates();
    await getUpdateTransaction("txn-studio-x");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/updates/check");
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[1][0]).toBe(
      "/api/update-transactions/txn-studio-x",
    );
  });

  it("checks reconciliation with no caller-provided plan body", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      ok({
        host_id: "aira",
        state: "synchronized",
        phase: "synchronized",
        safe_to_reconcile: false,
        affected_surfaces: [],
        gateway_reload_required: false,
        tunnel_restart_required: false,
        studio_restart_required: false,
        catalog_fingerprint: "abc",
        generation: 1,
        client_freshness: "unknown",
        rollback_succeeded: null,
        last_error: null,
        checked_at_ms: 1,
      }),
    );

    await checkReconciliation();
    expect(fetchMock.mock.calls[0][0]).toBe("/api/reconciliation/check");
    expect(fetchMock.mock.calls[0][1]?.method).toBe("POST");
    expect(fetchMock.mock.calls[0][1]?.body).toBeUndefined();
  });
});
