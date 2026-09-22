# ADR 0030 — Filesystem v2 Revisions, Bounded Reads/Search and CAS Mutation

- **Status:** Accepted for M7.4 implementation
- **Date:** 2026-09-22
- **Decision scope:** P-05 Filesystem revision identity, range/search, compare-and-swap writes and deterministic patching

## Context

The committed Filesystem baseline is root-confined through cap-std and replacement writes already use a sibling temporary file, file sync and rename. It has only whole-file text read/write, directory listing and directory creation.

Existing `write_text_file(overwrite=true)` can overwrite an external edit because there is no content revision precondition. There is also no bounded range read, bounded text search, metadata/revision tool or deterministic patch operation.

M7 needs stale-write detection without weakening capability confinement.

## Revision identity

For every regular UTF-8 file accepted by the text API:

```text
revision = "sha256:" + lowercase_hex(SHA-256(exact_file_bytes))
```

The digest covers exact bytes, including line endings. Path, mtime and inode are not part of the revision.

Revision is content identity, not authorization.

Filesystem target version for this contract is **0.2.0**.

## Error model

v2 tools return machine-readable structured error data plus a bounded human-readable text message.

Stable error codes:

```text
INVALID_PATH
NOT_FOUND
NOT_FILE
NOT_DIRECTORY
NOT_UTF8
TOO_LARGE
BOUNDS_INVALID
SEARCH_LIMIT
READ_ONLY
ALREADY_EXISTS
EXPECTED_REVISION_REQUIRED
STALE_REVISION
PATCH_INVALID
IO_ERROR
```

`STALE_REVISION` includes the caller's expected revision and the observed current revision when it is safe/available. It does not include file contents.

## Tool contracts

### file_metadata

Request:

```json
{"path":"relative/file.txt"}
```

Response:

```json
{
  "path":"relative/file.txt",
  "kind":"file",
  "bytes":1234,
  "revision":"sha256:<hex>",
  "utf8":true
}
```

Directories may return `kind:"directory"`, `bytes:null`, `revision:null`; symlink/traversal behavior remains constrained by the existing capability policy.

### read_text_file

Request remains:

```json
{"path":"relative/file.txt"}
```

v2 response is structured while retaining bounded text content for ordinary MCP clients:

```json
{
  "path":"relative/file.txt",
  "content":"...",
  "bytes":1234,
  "revision":"sha256:<hex>"
}
```

The existing server-wide `max_file_bytes` remains the hard ceiling.

### read_text_file_range

Request:

```json
{
  "path":"relative/file.txt",
  "start_byte":0,
  "max_bytes":65536
}
```

Rules:

- `start_byte` is zero-based;
- `max_bytes` defaults to 65536 and may not exceed the configured range cap;
- start/end must fall on UTF-8 code-point boundaries;
- the server never returns bytes past the current file length;
- response includes the revision of the exact file version read.

Response:

```json
{
  "path":"relative/file.txt",
  "content":"...",
  "start_byte":0,
  "end_byte":123,
  "file_bytes":1234,
  "eof":false,
  "revision":"sha256:<hex>"
}
```

Byte ranges are chosen so search results and patch operations share one unambiguous coordinate system.

### search_text

Search is literal UTF-8 substring search in M7.4; regular expressions are not part of the first contract.

Request:

```json
{
  "path":".",
  "query":"needle",
  "max_matches":100,
  "max_total_bytes":1048576
}
```

`path` may name one file or one directory tree.

Default/hard behavior:

- `max_matches` default 100, hard max 1000;
- `max_total_bytes` default 1 MiB and may not exceed the server-configured search budget;
- each individual file remains subject to `max_file_bytes`;
- traversal is deterministic lexical relative-path order;
- non-UTF-8 files are skipped with a bounded skipped count, not decoded lossily;
- symlink/path confinement is unchanged;
- results stop deterministically at the first bound reached.

Each match returns:

```json
{
  "path":"relative/file.txt",
  "byte_start":120,
  "byte_end":126,
  "line":8,
  "column_bytes":4,
  "revision":"sha256:<hex>",
  "preview":"bounded line/context preview"
}
```

The response also returns `truncated`, `scanned_files`, `scanned_bytes` and `skipped_non_utf8`.

### write_text_file

Request extends the current shape:

```json
{
  "path":"relative/file.txt",
  "content":"replacement",
  "overwrite":true,
  "expected_revision":"sha256:<hex>"
}
```

Rules:

- creating a new file with `overwrite=false` does not require a revision;
- replacing an existing file requires `overwrite=true` **and** `expected_revision`;
- `overwrite=true` without `expected_revision` on an existing file returns `EXPECTED_REVISION_REQUIRED`;
- mismatch returns `STALE_REVISION` and performs no replacement;
- success returns the new revision and byte count.

This is an intentional safety-breaking change from Filesystem 0.1.x and is the reason for target version 0.2.0.

### patch_text_file

Patch uses ordered UTF-8 byte-range edits rather than shell/editor scripts or a free-form diff parser.

Request:

```json
{
  "path":"relative/file.txt",
  "expected_revision":"sha256:<hex>",
  "edits":[
    {"start_byte":10,"end_byte":15,"replacement":"new text"}
  ]
}
```

Rules:

- `expected_revision` is mandatory;
- edits are expressed against the expected file revision;
- `0 <= start_byte <= end_byte <= file_bytes`;
- boundaries must be UTF-8 code-point boundaries;
- edits must be sorted by `start_byte` and non-overlapping;
- maximum edit count is 256;
- total replacement bytes and resulting file bytes are bounded;
- application is deterministic;
- success returns new revision, old revision, new byte count and applied edit count.

## CAS mutation sequence

For replacement write/patch:

1. validate relative path/capability policy;
2. read bounded current bytes;
3. compute current revision and compare with `expected_revision`;
4. compute the bounded candidate bytes;
5. write a sibling temporary file with create-new;
6. sync the temporary file;
7. re-read/re-hash the destination immediately before replacement;
8. if the revision changed, delete temp and return `STALE_REVISION`;
9. atomically rename temp over destination;
10. sync the containing directory where supported by the capability-safe implementation;
11. return the new revision.

In-process mutations for the same normalized path are serialized so two MCP writers cannot both pass the same expected revision and silently overwrite each other.

External processes do not participate in that lock. The second revision check narrows the race and guarantees detection of ordinary external edits between read and commit preparation. M7 does **not** claim a kernel-level arbitrary-external-writer CAS primitive that POSIX rename does not provide; qualification must state this boundary explicitly rather than overclaim it.

## Confinement invariants

All v2 operations reuse the existing root capability and path normalization.

They must not:

- canonicalize an attacker-controlled path into authority outside the root;
- follow a symlink as a way to escape confinement;
- use a shell;
- accept arbitrary executable/editor scripts;
- expand search beyond the configured root.

## Verification requirements

Permanent tests must prove:

- deterministic SHA-256 revision;
- same bytes => same revision, any byte change => different revision;
- external edit before CAS commit yields `STALE_REVISION`;
- two concurrent server writers with the same revision produce one success and one stale conflict;
- stale conflict leaves external bytes unchanged;
- patch bounds/UTF-8 boundary/overlap validation;
- atomic replacement cleanup on injected write/sync/rename failure;
- range reads obey byte and UTF-8 boundaries;
- search obeys match/byte/file bounds and deterministic order;
- traversal/symlink confinement regressions remain green.

These map to R21-R27 and Q07.

## Consequences

- Filesystem becomes safe for read-modify-write agent workflows without treating an in-process mutex as correctness authority.
- Existing clients that overwrite files must first read metadata/content and send the observed revision.
- Patch coordinates can be produced directly from range/search outputs.
- Whole-file reads remain available for small files, while large workflows can use bounded range/search.
