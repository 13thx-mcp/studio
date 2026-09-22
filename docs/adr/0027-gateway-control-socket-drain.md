# ADR 0027 — Private Gateway Control Socket and Drain Ownership

- **Status:** Accepted — P-06 control-socket viability VERIFIED; production implementation pending M7.2
- **Date:** 2026-09-21
- **Decision scope:** live Gateway drain/reload coordination with Studio

## Context

Gateway is launched as the MCP command owned by the secure tunnel runtime. Studio owns the `TunnelSupervisor`, not the live Gateway stdio peer.

The pre-M7 Studio reconciliation path cannot issue a command to the running Gateway. For Gateway-only configuration changes it writes files, waits for the Gateway watcher window, and verifies that Tunnel remains running. Gateway update similarly stops the Tunnel owner without first asking the live Gateway whether active mutations are safe to interrupt.

The existing public Gateway MCP administration tools are not a suitable Studio control dependency: Studio has no direct live MCP peer to that process, and routing a safety prerequisite through the same external tunnel/client path being drained would create a circular control dependency.

## Decision

Gateway exposes a **private, local Unix-domain control socket** dedicated to bounded lifecycle coordination.

### Location

The socket location is derived from the already trusted Gateway `--config-dir`:

```text
<config-dir-parent>/control/gateway.sock
```

For the deployed layout:

```text
runtime/gateway/
  servers.d/
  control/
    gateway.sock
```

Studio derives the same location from its trusted `runtime_root`; browser/API callers never provide a socket path.

This avoids adding arbitrary filesystem authority or a new TCP listener.

### Security boundary

- supported initially on the existing darwin-arm64/Unix runtime;
- dedicated control directory is private to the runtime user;
- socket is not exposed through Studio HTTP;
- requests use a small versioned typed protocol with a strict byte limit;
- no shell command, executable path, PID or arbitrary argv is accepted;
- unknown versions/actions/fields fail closed;
- a pre-existing non-socket/symlink path is rejected;
- same-user local access is within the current host trust boundary; peer-UID validation should be used where the platform API is available.

### Control protocol

Initial actions are closed and typed:

```text
status
drain
resume
reload
```

M7 freezes the wire framing as one request/response per connection using one UTF-8 JSON object terminated by LF. EOF is not request framing, and pipelining multiple commands on one connection is not supported. Production M7.2 must enforce a strict request byte limit before parsing and fail closed on unknown versions/actions/fields.

`status` returns bounded coordination state including process instance, drain generation/state, active/queued counts, oldest age and catalog generation/fingerprint. It does not return tool arguments/results.

`drain` accepts only server-defined reason enums and bounded policy/deadline fields. It returns a drain generation. Destructive callers proceed only after that exact generation reaches `DRAINED`.

`resume` requires the matching drain generation. It is used when a destructive operation is abandoned while the same Gateway process remains alive.

`reload` is a request to the Gateway's own coordinator. Gateway performs the required drain/catalog transition internally and returns the resulting catalog generation/fingerprint or a typed failure.

### Drain ownership

Gateway itself is authoritative for live drain state.

Studio/Fleet may request and observe drain, but they cannot declare it successful from process state, elapsed time or SQLite history.

A drain deadline failure prevents the destructive restart/update/reload that depended on it.

### Update behavior

Before stopping a running Tunnel/Gateway for Gateway binary activation, Studio must:

1. connect to the expected private socket;
2. prove the live process instance/binding;
3. request drain with reason `gateway_update`;
4. wait for the matching generation to reach `DRAINED`;
5. only then stop the owner.

If stop fails and the same Gateway remains alive, Studio attempts `resume` for that generation before returning failure.

If the process has exited, a new/rollback Gateway starts with fresh in-memory state; old drain state is not rehydrated from history.

### Reconciliation behavior

Gateway configuration reload is routed through the Gateway's internal reload/drain coordinator rather than inferred from a fixed watcher sleep.

The file watcher becomes another local producer of the same reload request; it must not bypass drain.

Studio reconciliation may request reload after all managed files are committed and verify the returned catalog identity.

## P-06 viability evidence

Gateway commit `60a0c00` contains the corrected permanent viability probe in `tests/m7_control_socket_viability.rs`.

Verified on 2026-09-21:

```text
cargo test --test m7_control_socket_viability --all-features
  3 passed; 0 failed

cargo fmt --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
  6 unit + 3 control-socket + 7 protocol-spike tests passed
```

The probe verifies short collision-free test socket paths, stale-path rejection, typed newline-framed round-trip behavior, and server-owned drain-generation transport. The earlier EOF-delimited probe could deadlock because both peers waited for EOF; LF framing removes that ambiguity.

This evidence resolves the M7.0 control-path **viability** prerequisite only. It does not claim that the M7.2 production drain server/client or destructive-update integration already exists.

## Consequences

- Gateway needs a bounded local control server and typed protocol.
- Studio needs a constrained local control client.
- M7.2 replaces fixed watcher-window assumptions with explicit reload/drain acknowledgement.
- Gateway update rollback keeps the existing M5 artifact/catalog authority; this ADR only adds a safety prerequisite before destructive stop.
- The control socket is not a remote-management API and does not solve future authentication/RBAC.

## Rejected alternatives

### Existing Gateway MCP admin tools only

Rejected because Studio does not own a direct live MCP peer and safety cannot depend on the external path being drained.

### Loopback TCP control port

Rejected because it adds port allocation/exposure and a broader local network surface with no benefit for same-host coordination.

### Signal files / SQLite drain flags

Rejected because polling files/history would create ambiguous ownership and could make stale persisted state appear authoritative after restart.

### Treat Tunnel stop as drain

Rejected because stopping the process can interrupt a dispatched mutation and provides no pre-stop safety proof.
