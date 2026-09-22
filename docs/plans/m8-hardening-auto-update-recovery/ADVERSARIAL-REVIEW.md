# M8 Adversarial Planning Review

This review tries to make the planned automation unsafe and records the required defense.

| Attack / failure | Unsafe shortcut | Required defense |
|---|---|---|
| Gateway returns unknown mutation then restarts | assume history ingestion caught it | durable Gateway safety hold before terminal response |
| tool policy changes after historical unknown | infer class from current tool name | persist server-owned class at admission |
| generic MCP binary replaced while Gateway child runs old inode | installed version == running version | targeted child restart + generation/identity proof |
| Studio prepares then crashes | reuse orphan staged dir blindly | previous-process staging never auto-apply |
| periodic timer wakes after 100 missed intervals | replay 100 checks/actions | collapse to one due evaluation |
| runtime coordinator busy | poll in tight loop | bounded defer/backoff |
| maintenance window closes during rollback | stop because window ended | recovery/rollback always completes |
| system clock moves backwards | treat stale time as open window | clock-sanity veto |
| provider latest changes between check and apply | use cached latest identity | staged fingerprint + source version revalidation |
| source checkout dirty | update runtime anyway silently | auto activation hard block; no Git repair |
| Fleet changes automation policy | old process continues old policy | Studio config restart freshness veto |
| Gateway updated | safety-hold file lost with binary/config | runtime state preserved/validated across update |
| state file corrupt | reset to manual and hide incident | explicit blocked automation state; no silent reset |
| staging disk fills | delete rollback/journal to free space | cleanup eligibility excludes recovery authority |
| reconciliation repeatedly fails | retry every interval forever | persisted circuit |
| circuit reset button | reset and immediately execute hidden action | reset state only; next action separately admitted |
| operator resolves unknown hold | automatically retry original tool | resolution never invokes child/tool |
| history unavailable | background action bypasses audit | shared admission blocks discretionary mutation |
| old Gateway lacks M8 fields | assume zero active/holds | capability unavailable = auto mutation blocked |
| explicit stop followed by crash monitor | restart stopped process | explicit-stop suppresses auto restart |
| update manager owns process while restart loop fires | competing starts/stops | single restart owner + operation suppression |
| self-update old process exits | two controllers activate same target | durable self-update finalization + one new controller |
| diagnostics include useful raw env | secret leakage | allowlist DTOs + canary byte scan |
| runtime-only source absent | treat as dirty/unhealthy | source hygiene skipped by design for source-less host |
| pre-release target needs release evidence | use generic tag | guarded prerelease release-tag support or change target |
| disabled automation during physical update | abandon half-mutated runtime | disable stops new admission, not required recovery |
| Gateway safety-hold persistence ENOSPC | return unknown but report automation safe | capability flips unavailable/fail-closed |

## Residual limits intentionally accepted in M8

- safety holds record uncertainty, not truth about whether the external side effect happened;
- hold resolution is an operator assertion and cannot reconstruct lost child state;
- UTC maintenance windows avoid DST ambiguity but are less ergonomic;
- process-scoped generic prepared authorization may re-download after Studio restart;
- M8 does not guarantee unattended compatibility across arbitrary major/breaking component releases; target/component safety checks and rollback remain required;
- diagnostics are for local operator support, not forensic completeness.

## Review gate

Before M8.0 closes, every row above must map to an accepted ADR and at least one R/Q verification row or be explicitly retired with rationale.
