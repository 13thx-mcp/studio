# Planning-only validation and provenance

## Scope

This session audited the current Aira Studio repository and prepared Markdown planning documents only. It did not add a production SQLite dependency/migration, edit runtime source, run live control operations or modify Mirin. No version bump, new tag, push, merge, deployment or Git commit is part of this session.

Repository baseline and pre-write recheck both reported `main` at `6a6122abb41591f22874fb439eb8427edd649e53` with a clean worktree. Planning destinations were absent before creation. Locally cached origin/main matched; no remote fetch/public endpoint requalification was performed. Package versions remain 0.5.0, and the existing v0.5.0 tag was not created here.

## Candidate-schema design checks

The documented DDL was executed in an isolated **in-memory** database using the planning container's Python SQLite **3.46.1**. This checks relational syntax/constraints only. That engine is not the proposed production bundled engine, does not satisfy the plan's >=3.51.3 patched-engine qualification rule, and was not used to qualify WAL durability or concurrency. M6.1 must repeat the full production migration/fault matrix using its actual linked patched engine.

The candidate creates **26 application tables and 42 explicit indexes**. Checked: complete schema creation, foreign_key_check, integrity_check, duplicate source-ordinal rejection, missing-subject FK rejection, payload >4096 bytes rejection, negative ordinal rejection, invalid category rejection, transactional rollback and non-reuse of a committed AUTOINCREMENT sequence after deletion. These are documentation-fixture checks, not new permanent Rust tests.

Document checks cover resolving local Markdown links, all 13 required sections in each of the nine package plans, 44 unique requirement IDs and 16 qualification gates. The internal adversarial review records source prerequisites and explicit failure/coverage limits.

## Source integrity sentinels

SHA-256 values captured directly from Aira immediately before documentation writes:

| File | SHA-256 |
|---|---|
| Cargo.toml | add3a5f0ecd55cf920d599ddd7389b5d4a85cdabfb5fb7c75009523ba5ad24ce |
| Cargo.lock | 83a6022897ca5f8bb7b52055336f25c581a2921d4abf93cca998a814c715e897 |
| rust-toolchain.toml | 7553618f800829444096beea3dce84c3d07585efda504c19adb6787369bbd9e8 |
| web/package.json | cf9777cbcbc802f3350fab2883ee472b102bc18c2d976dfdb0bacb853b03e721 |
| web/pnpm-lock.yaml | 5c9cd61f7457ecfbc3babbdd2fe7a36945918e80c5719dec262237f7e2849efb |
| src/main.rs | 88cfcc1e0e5e12e604a63602b02b97e45bf568b0461505b14d7fa2445d950985 |
| src/storage/mod.rs | 899d58725a981bc3e4633a6efbf35f6d2cd13ded10c168fcf4ba89ebb1e242d7 |
| src/metrics/mod.rs | df2171800dc13141d2b7b34c8fd7bd11cc34865fa720d48bc789f321cbe5204b |

After writing, verify the same HEAD and hashes, no tracked production diff, only the new planning directory/status document, and no whitespace errors. Documentation is left uncommitted for review. A downloadable package is a copy of these planning files, not an alternate repository implementation.

## Not executed / not claimed

Rust fmt/check/Clippy/test/build, frontend lint/typecheck/test/build, bundled rusqlite/SQLite native build, RustSec/frontend dependency qualification, real filesystem ENOSPC, crash/rollback runtime tests, native two-process self-update, darwin-amd64 qualification and public-release/bootstrap proof were **not executed in this planning session**. Existing M5 results were read as historical evidence only. P-01/P-02 remain planned test-first source fixes. M6 implementation and publication closure are not established by this document.

Candidate DDL SHA-256 (fenced SQL bytes): `1f24c81fcff6e7275bcdb939c14fc99abc4d946c817c4d617ef2d7593c8b3b70`.
