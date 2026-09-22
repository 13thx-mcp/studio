# ADR 0045 — Guarded SemVer Prerelease Release Tags and Exact Evidence

- **Status:** Accepted for M8.0/M8.8 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-19, Git MCP 0.2.0 release control, M8 `v0.8.0-beta` closure

## Context

M8 roadmap target is `v0.8.0-beta`.

Current Git MCP guarded release tooling accepts only plain `MAJOR.MINOR.PATCH`. Using generic `git_tag` for M8 would bypass the guarded main-merge/release-evidence path.

The guarded path already requires:

- protected `main`;
- clean tree/index;
- two-parent no-ff merge HEAD;
- exact release evidence for enrolled repos;
- immutable existing tags.

## Decision

Git MCP target 0.2.0 extends guarded release-version parsing to SemVer core plus optional prerelease.

Accepted grammar:

```text
MAJOR.MINOR.PATCH
MAJOR.MINOR.PATCH-PRERELEASE
```

Core numeric components:

- decimal digits only;
- no leading zero unless exactly `0`.

Prerelease:

- one or more dot-separated identifiers;
- each identifier contains only ASCII alphanumeric or hyphen;
- empty identifiers are invalid;
- numeric prerelease identifiers cannot have leading zeroes.

Examples accepted:

```text
0.8.0-beta
0.8.0-beta.1
1.0.0-rc.2
```

Rejected:

```text
01.0.0
0.8.0-
0.8.0-beta..1
0.8.0-01
0.8.0+build
0.8.0-beta+build
```

Build metadata is intentionally not accepted by the guarded release tool in M8 to keep package/changelog/tag evidence identity exact and simple.

## Release-prep subject

Merge readiness requires exact:

```text
chore(release): prepare v<version>
```

including prerelease suffix.

For M8:

```text
chore(release): prepare v0.8.0-beta
```

## Evidence

Existing release evidence must prove PASS for exact current main HEAD.

For repositories whose release evidence schema does not currently carry version, HEAD identity plus exact release-prep subject/tag version is the required binding.

If/when evidence schema adds a version field, a mismatch must fail closed.

The guarded release tool never falls back to generic tagging when evidence validation fails.

## Existing tag immutability

If `v<version>` exists locally or remotely, normal release tagging does not move it.

Any local-only recovery uses existing guarded replacement tooling with publication proof.

Published tag recovery requires a new version rather than rewrite.

## Tool descriptions/contracts

Git MCP updates public descriptions that currently say plain `vMAJOR.MINOR.PATCH` to describe supported SemVer prerelease syntax accurately.

Generic `git_tag` remains available for non-release tagging but is not the approved enrolled-repository release path.

## Alternatives considered

### Change M8 target to 0.8.0

Rejected because the roadmap intentionally marks M8 as beta maturity.

### Use generic annotated tag for prereleases

Rejected because it bypasses exact release gates.

### Full SemVer including build metadata

Deferred. Build metadata is unnecessary for current release identity and complicates evidence/tag matching.

## Consequences

- M8 can close as `v0.8.0-beta` through the same guarded controls as stable versions;
- parser behavior is deterministic and permanently testable;
- no release-evidence bypass is introduced;
- Git MCP needs a small independent version bump to 0.2.0 when implemented.
