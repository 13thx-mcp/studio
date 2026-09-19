import { useEffect, useMemo, useState } from "react";

import type {
  ConnectionState,
} from "./realtime";
import type {
  ReconciliationView,
  UpdateComponent,
  UpdateInventory,
  UpdateTransaction,
} from "./types";
import {
  actionableUpdateError,
  applyConfirmation,
  canPrepareUpdate,
  componentImpact,
  isExpectedReconnect,
  isTerminalTransaction,
  phaseLabel,
  platformLabel,
  prepareConfirmation,
  providerLabel,
  rollbackMessage,
  targetVersionFor,
  transactionResult,
  updateCardStatus,
} from "./updates-state";

export interface UpdatesPanelProps {
  inventory: UpdateInventory[];
  reconciliation: ReconciliationView | null;
  transactions: Partial<Record<UpdateComponent, UpdateTransaction>>;
  runtimeStates: Partial<Record<UpdateComponent, string>>;
  connection: ConnectionState;
  pending: string | null;
  onCheckUpdates: () => Promise<void>;
  onPrepare: (component: UpdateComponent, version: string) => Promise<void>;
  onApply: (transaction: UpdateTransaction) => Promise<void>;
  onRefreshTransaction: (transactionId: string) => Promise<void>;
  onCheckReconciliation: () => Promise<void>;
  onApplyReconciliation: () => Promise<void>;
}

function formatTime(value: number | null | undefined): string {
  if (!value) return "never";
  return new Date(value).toLocaleString();
}

function canApplyTransaction(transaction: UpdateTransaction): boolean {
  if (transaction.phase === "staged") return true;
  return (
    transaction.component === "studio" &&
    ["activation_pending", "external_activating", "health_verifying"].includes(transaction.phase)
  );
}

function componentClassLabel(value: UpdateInventory["class"]): string {
  return value.replaceAll("_", " ");
}

export default function UpdatesPanel({
  inventory,
  reconciliation,
  transactions,
  runtimeStates,
  connection,
  pending,
  onCheckUpdates,
  onPrepare,
  onApply,
  onRefreshTransaction,
  onCheckReconciliation,
  onApplyReconciliation,
}: UpdatesPanelProps) {
  const [targets, setTargets] = useState<Partial<Record<UpdateComponent, string>>>({});

  useEffect(() => {
    setTargets((current) => {
      const next = { ...current };
      for (const item of inventory) {
        if (!next[item.component] || next[item.component] === item.installed_version) {
          next[item.component] = targetVersionFor(item);
        }
      }
      return next;
    });
  }, [inventory]);

  const hostMode = inventory[0]?.host_mode ?? "runtime_only";
  const platform = inventory[0] ? platformLabel(inventory[0]) : "loading";
  const updateCount = inventory.filter((item) => item.update_available).length;
  const brokenCount = inventory.filter(
    (item) => item.drift === "broken" || item.installation_health === "broken",
  ).length;
  const restartCount = inventory.filter((item) => item.drift === "installed_restart_required").length;
  const lastCheck = useMemo(
    () =>
      inventory.reduce<number | null>(
        (latest, item) =>
          item.last_check.checked_at_ms && (!latest || item.last_check.checked_at_ms > latest)
            ? item.last_check.checked_at_ms
            : latest,
        null,
      ),
    [inventory],
  );

  const prepare = async (item: UpdateInventory) => {
    const target = targets[item.component]?.trim() ?? "";
    if (
      !window.confirm(
        prepareConfirmation(item, target, runtimeStates[item.component] ?? "unknown"),
      )
    ) {
      return;
    }
    await onPrepare(item.component, target);
  };

  const apply = async (item: UpdateInventory, transaction: UpdateTransaction) => {
    if (
      !window.confirm(
        applyConfirmation(item, transaction, runtimeStates[item.component] ?? "unknown"),
      )
    ) {
      return;
    }
    await onApply(transaction);
  };

  return (
    <section className="updates-panel" aria-label="Updates and Fleet">
      <div className="detail-header updates-heading">
        <div>
          <p className="eyebrow">Runtime distribution</p>
          <h2>Updates / Fleet</h2>
          <p className="panel-note">
            Manual, verified updates only. Runtime identity comes from deployed artifacts, not Git HEAD.
          </p>
        </div>
        <button
          disabled={pending !== null}
          onClick={() => void onCheckUpdates()}
        >
          {pending === "updates:check" ? "checking…" : "check releases"}
        </button>
      </div>

      <div className="fleet-summary-grid">
        <div><span>Host</span><strong>{reconciliation?.host_id ?? "local"}</strong></div>
        <div><span>Mode</span><strong>{hostMode.replace("_", " ")}</strong></div>
        <div><span>Platform</span><strong>{platform}</strong></div>
        <div><span>Updates</span><strong>{updateCount}</strong></div>
        <div><span>Restart drift</span><strong>{restartCount}</strong></div>
        <div><span>Broken</span><strong>{brokenCount}</strong></div>
        <div><span>Last release check</span><strong>{formatTime(lastCheck)}</strong></div>
        <div><span>Realtime</span><strong>{connection}</strong></div>
      </div>

      <section className="reconciliation-card" aria-label="Runtime configuration reconciliation">
        <div className="detail-header">
          <div>
            <p className="eyebrow">Fleet generated config</p>
            <h3>Runtime reconciliation</h3>
          </div>
          <span className={`update-badge update-${reconciliation?.state ?? "unknown"}`}>
            {reconciliation?.state.replaceAll("_", " ") ?? "loading"}
          </span>
        </div>
        <div className="reconciliation-grid">
          <div><span>Phase</span><strong>{reconciliation?.phase.replaceAll("_", " ") ?? "—"}</strong></div>
          <div><span>Generation</span><strong>{reconciliation?.generation ?? "—"}</strong></div>
          <div><span>Catalog</span><strong className="fingerprint">{reconciliation?.catalog_fingerprint?.slice(0, 16) ?? "unknown"}</strong></div>
          <div><span>Client catalog</span><strong>{reconciliation?.client_freshness.replaceAll("_", " ") ?? "unknown"}</strong></div>
        </div>
        {reconciliation?.affected_surfaces.length ? (
          <p className="panel-note">Affected: {reconciliation.affected_surfaces.join(", ")}</p>
        ) : null}
        {reconciliation?.studio_restart_required && (
          <div className="warning-banner">
            Studio configuration is reconciled on disk but this running Studio process has not
            proved loading the expected <code>studio.config</code> bytes. Restart Studio through
            the existing trusted service/launcher, reconnect, then check drift again. No automatic
            restart action is exposed here.
          </div>
        )}
        {reconciliation?.client_freshness === "refresh_pending" && (
          <div className="warning-banner">
            Local Gateway/catalog state changed. A connected client refresh is still pending/unproven.
          </div>
        )}
        {reconciliation?.state === "unmanaged_conflict" && (
          <div className="warning-banner">
            Unmanaged local edits were detected. M5.11 will not auto-adopt or overwrite them; operator review is required.
          </div>
        )}
        {reconciliation?.last_error && (
          <div className="inline-error">{actionableUpdateError(reconciliation.last_error)}</div>
        )}
        <div className="actions compact-actions">
          <button
            disabled={pending !== null}
            onClick={() => void onCheckReconciliation()}
          >
            {pending === "reconciliation:check" ? "checking…" : "check drift"}
          </button>
          <button
            disabled={
              pending !== null ||
              reconciliation?.state !== "managed_safe_drift" ||
              !reconciliation.safe_to_reconcile
            }
            onClick={() => {
              if (
                window.confirm(
                  "Apply the server-approved managed-safe reconciliation plan? Only Fleet-owned generated surfaces will be changed, with backend rollback semantics.",
                )
              ) {
                void onApplyReconciliation();
              }
            }}
          >
            {pending === "reconciliation:apply" ? "reconciling…" : "apply safe reconciliation"}
          </button>
        </div>
      </section>

      <div className="update-card-grid">
        {inventory.map((item) => {
          const status = updateCardStatus(item);
          const transaction = transactions[item.component];
          const target = targets[item.component] ?? targetVersionFor(item);
          const transactionBusy = transaction && !isTerminalTransaction(transaction.phase);
          const reconnecting = transaction ? isExpectedReconnect(transaction) : false;
          return (
            <article className="update-card" key={item.component}>
              <div className="detail-header update-card-header">
                <div>
                  <strong>{item.display_name}</strong>
                  <small>
                    {componentClassLabel(item.class)} · {providerLabel(item.provider)}
                  </small>
                </div>
                <span
                  className={[
                    "update-badge",
                    status.danger ? "update-broken" : status.attention ? "update-attention" : "update-current",
                  ].join(" ")}
                >
                  {status.label}
                </span>
              </div>

              <dl className="version-grid">
                <div><dt>Installed</dt><dd>{item.installed_version ?? "unknown"}</dd></div>
                <div><dt>Running</dt><dd>{item.running_version ?? "unknown"}</dd></div>
                <div><dt>Desired</dt><dd>{item.desired_version ?? "unknown"}</dd></div>
                <div><dt>Latest</dt><dd>{item.latest_version ?? "not checked"}</dd></div>
                <div><dt>Runtime</dt><dd>{runtimeStates[item.component] ?? "unknown"}</dd></div>
                <div><dt>Integrity</dt><dd>{item.installation_health}</dd></div>
                <div><dt>Check</dt><dd>{item.last_check.status}</dd></div>
                <div><dt>Checked</dt><dd>{formatTime(item.last_check.checked_at_ms)}</dd></div>
              </dl>

              {item.last_check.error && (
                <div className="inline-error">{actionableUpdateError(item.last_check.error)}</div>
              )}

              <label className="target-field">
                Target version
                <input
                  value={target}
                  disabled={Boolean(transactionBusy) || pending !== null}
                  onChange={(event) =>
                    setTargets((current) => ({
                      ...current,
                      [item.component]: event.target.value,
                    }))
                  }
                  placeholder="1.2.3"
                />
              </label>

              <p className="impact-copy">
                {componentImpact(item.component, item.running_version !== null)}
              </p>

              {transaction && (
                <div className="transaction-box">
                  <div className="transaction-heading">
                    <strong>{phaseLabel(transaction.phase)}</strong>
                    <code>{transaction.transaction_id}</code>
                  </div>
                  <p>{transactionResult(transaction)}</p>
                  <div className="transaction-meta">
                    <span>{transaction.source_version ?? "unknown"} → {transaction.target_version}</span>
                    <span>{formatTime(transaction.updated_at_ms)}</span>
                  </div>
                  {transaction.rollback_succeeded !== null && (
                    <p>
                      Rollback: {transaction.rollback_succeeded ? "verified" : "not verified"}
                    </p>
                  )}
                  {transaction.error && (
                    <div className="inline-error">{actionableUpdateError(transaction.error)}</div>
                  )}
                  {reconnecting && (
                    <div className="warning-banner">
                      Expected control-path reconnect. Do not submit a duplicate apply; this transaction will be re-queried after reconnect.
                    </div>
                  )}
                </div>
              )}

              <div className="actions compact-actions">
                <button
                  disabled={
                    pending !== null ||
                    Boolean(transactionBusy) ||
                    !canPrepareUpdate(item, target)
                  }
                  onClick={() => void prepare(item)}
                >
                  {pending === `updates:${item.component}:prepare` ? "preparing…" : "prepare"}
                </button>
                <button
                  disabled={
                    pending !== null ||
                    !transaction ||
                    !canApplyTransaction(transaction)
                  }
                  onClick={() => transaction && void apply(item, transaction)}
                >
                  {pending === `updates:${item.component}:apply`
                    ? "applying…"
                    : transaction?.component === "studio" &&
                        transaction.phase !== "staged"
                      ? "resume activation"
                      : "apply"}
                </button>
                {transaction && (
                  <button
                    disabled={pending !== null}
                    onClick={() => void onRefreshTransaction(transaction.transaction_id)}
                  >
                    refresh status
                  </button>
                )}
              </div>
              <p className="rollback-note">{rollbackMessage(item.component)}</p>
            </article>
          );
        })}
      </div>
    </section>
  );
}