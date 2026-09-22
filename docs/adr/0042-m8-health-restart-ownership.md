# ADR 0042 — M8 Health Evidence and Single-Owner Restart Policy

- **Status:** Accepted for M8.2 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-10, P-11, health freshness, Studio/Tunnel restart policy

## Context

Component update managers already perform strong component-specific verification, while normal runtime status is fragmented across Supervisor, Tunnel, Gateway, inventory and Fleet.

M8 needs policy-readable health evidence and bounded restart behavior without creating a second restart owner for Gateway children.

## Decision

M8 introduces a typed internal `HealthSnapshot`:

```text
component
observed_at_ms
freshness_ms
state = healthy | degraded | unhealthy | unknown
liveness
readiness
identity
ownership
required_checks[]
failed_checks[]
```

The exact serialized browser DTO remains narrower and sanitized.

Health is evidence, not authority. Missing/stale/failed probes yield `unknown` or `unhealthy`; they are never coerced to healthy.

## Component adapters

### Gateway

Requires fresh private control status, expected running/drain state, catalog generation, child summaries and M8 capability availability where automatic mutation depends on it.

### Generic MCP

Combines installed binary identity with each known runtime owner:

- Studio Supervisor instance if registered/running;
- Gateway child summary/generation if Gateway manages the component.

Installed version alone does not prove running version.

### Tunnel

Keeps liveness, readiness, MCP discovery and control-plane poll health distinct. No one weak ping replaces the existing contract.

### Studio

Requires process identity/version, loopback health and loaded config identity.

### Fleet

Uses schema/render validation and installed control-bundle identity; it is not treated as a daemon.

## Freshness

Each adapter defines a bounded maximum evidence age in M8.0 implementation constants/config validation. Destructive automation re-probes immediately enough that stale cached UI evidence cannot authorize activation.

## Restart ownership

Gateway child restart/backoff/circuit remains exclusively Gateway-owned under M7.

Studio adds optional restart policy only for process instances it already owns directly:

- registered MCP processes supervised by Studio;
- Tunnel process supervised by Studio.

Supported initial values:

```text
disabled
on-failure
```

Default is `disabled`.

## Desired-running state

Auto restart only applies to an unrequested exit of a process whose owner state says `desired_running=true`.

Explicit Stop sets `desired_running=false` before signaling the process. That suppresses restart even if exit observation races.

Start/Restart/update recovery sets desired state explicitly.

## Backoff/circuit

Initial restart defaults:

```text
max attempts              3
stability window          30 s
initial backoff            1 s
max backoff               30 s
circuit cooldown          60 s
```

The owner persists only the policy/circuit metadata needed across Studio restart through AutomationState. PID/process ownership is re-probed and never restored from history.

A stability window resets consecutive failure count only after sustained health.

## Mutation suppression

When RuntimeOperationCoordinator shows an update/reconciliation/control owner that conflicts with a supervised process, its independent restart loop is suppressed.

Required rollback/recovery performed by the active transaction remains allowed.

## Alternatives considered

### One generic HTTP/process health check

Rejected because component semantics differ materially.

### Studio restarts Gateway children too

Rejected because it creates competing owners.

### Persist PIDs as restart authority

Rejected because PID reuse and process lifetime make persisted PID authority unsafe.

## Consequences

- automation gains explicit fresh health evidence;
- explicit stop cannot accidentally trigger auto restart;
- restart loops are bounded and owner-specific;
- M8.4 can reason about installed versus running identity across multiple owners.
