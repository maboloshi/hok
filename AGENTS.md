# Workspace Overview — Scoop Ecosystem

Monorepo of sibling git projects: the original Scoop (PowerShell) package manager,
two independent Rust reimplementations (hok, hyperscoop), and shim test utilities.
Each project is its own git repo (no root git); `reasonix.toml` at repo root defines
the agent workspace/sandbox. **Current working directory = `hok/`.**

> **Language convention** — repo-root docs (this file, `ARCHITECTURE.md`, `README.md`,
> `CHANGELOG.md`) are **English**; the agent workspace (`.agents/**`) is **Chinese**. The per-location
> rule (`<name>-zh.md` counterparts, translations) lives in [`.agents/AGENTS.md`](.agents/AGENTS.md) → 语言约定.

```
Scoop/                  Original Scoop (PowerShell) package manager — v0.6.0
hok/                    hok CLI — Rust Scoop reimplementation (v0.2.0)  ← current working directory
hyperscoop/             hp CLI — Rust Scoop reimplementation (v4.2.0)
shim_test/              Shim testing sandbox (C#/C++/Rust/Zig variants)
hok_/, hok~/, hok~.zip  Historical hok backups/archives — do not modify
```

---

## 1. hyperscoop/ — hp CLI (not current dir)

Rust Windows package manager, edition 2024, Rust 1.91.0.

- Entry: `src/main.rs` → `Cli` (clap) → `hyperscoop_middle::invoke_*`.
- Crates: `.` (CLI hp: `command_args/`, `hyperscoop_middle/`, i18n, logging, self-update);
  `crates/command`, `crates/hscoop_macro` (proc), `crates/shim`, `crates/hash`.
- Locate: `src/command_args/` per-cmd args; `src/hyperscoop_middle/` logic; `src/i18n.rs` + `locales/`;
  `script/` automation; `hyperscoop_source_bucket/`.
- Commands: `cargo build --release`; `cargo test --workspace --all-features`; `cargo fmt --all`;
  `cargo clippy --workspace --all-targets -D warnings`; `cargo bench --bench hash`; `cargo run --example pg_bar`.
- Conventions: verbs mirror Scoop (`hp install`/`bucket`/`search`); `snake_case` fns / `CamelCase` types;
  i18n `tr!("key","中文","English")`; `unsafe` / `unreachable_pub` warned.

---

## 2. hok/ — hok CLI (current working directory)

Rust Scoop reimplementation, **edition 2024**, v0.2.0 (version source of truth: `Cargo.toml`); community-maintained fork of
[chawyehsu/hok](https://github.com/chawyehsu/hok) (upstream paused). Deep dive in `ARCHITECTURE.md`;
user guide in `README.md`; this section highlights only what those don't cover.

**Entry:** `src/bin/hok.rs` (binary), `src/lib.rs` (library).

- **Crates:** `.` (CLI hok: `src/cmd/`, `src/`); `crates/libscoop/` (core lib); `crates/hok-shim/`
  (`no_std` shim gen, Win32 FFI); `crates/hok-i18n/` (i18n runtime: build-time locale tables + lookup macro);
  `crates/hok-i18n-derive/` (derive patching clap tree).
  Inno Setup decoding uses the registry crate `innospect`, NSIS uses `nsis` — **neither is vendored**;
  version pins and upstream-patch history live in `crates/libscoop/Cargo.toml` (do not restate them
  elsewhere — a copy drifts).
- **Commands:** `cargo build` / `cargo test` (workspace auto-includes members); `cargo build/test -p libscoop`;
  `cargo check`.
- **Architecture:** `src/cmd/` per-cmd handlers; `build.rs` scans dir → auto-generates `__cmd_reg__.rs`
  (add a cmd = drop a `.rs` with `execute()` *and* add a variant to the `Command` enum in `src/cmd/mod.rs` —
  that enum, not the directory listing, is the command count). `src/eventloop.rs` async loop; `src/output.rs`
  style; `src/cui.rs` prompts; `src/scoop_handler.rs`; `src/i18n.rs` two-pass lang detect.
  `crates/libscoop/` install flow/extract/shim/persist/manifest/download/resolve/sync; `crates/libscoop/src/internal/`
  submodules (the module list lives in the directory — not enumerated here; it has drifted twice). `hok-shim` shared core in `crates/hok-shim/src/lib.rs` (`no_std`, fixed buffers; Scoop Shim format) + bins
  `hok-shim` (console) / `hok-shim-gui` differing only by PE subsystem.
- **Commands list:** `alias`, `ci-auto-pr`, `bucket`, `cache`, `cat`, `checkhashes`, `checkup`,
  `checkurls`, `checkver`, `cleanup`, `completions`, `config`, `create`, `depends`, `download`,
  `export`, `formatjson`, `health`, `hold`, `home`, `import`, `info`, `install`, `list`,
  `missing-checkver`, `prefix`, `reinstall`, `reset`, `search`, `shim`, `status`, `unhold`,
  `uninstall`, `update`, `upgrade`, `virustotal`, `which`
- **Conventions:** edition 2024, 4-space indent; `unsafe` FFI in hok-shim; i18n via the **in-house `hok-i18n`**
  (`crates/hok-i18n/` flattens `locales/en.yml` / `locales/zh.yml` into lookup tables at build time) + `hok-i18n-derive`; Scoop install flow
  `pre_install → extract → installer.script → link_current → shim → persist → post_install → metadata`;
  config `~/.config/hok/config.json` (migrated once from Scoop); lib never writes stdout directly —
  sync `Output` sink + async **`tokio::sync::mpsc`** event bus owned by `libscoop::Session` (async overview:
  `ARCHITECTURE.md` → Async Runtime — `#[tokio::main]`, `spawn_blocking`, `CancellationToken`, backpressure); hashing in
  `libscoop::internal::hash` (md-5/sha1/sha2 direct deps).

---

## 3. Scoop/ — Original Scoop (PowerShell)

Reference/backward-compat testing only ([ScoopInstaller/Scoop](https://github.com/ScoopInstaller/Scoop),
v0.6.0, PowerShell 5+). `bin/` entry (`scoop.ps1`), `lib/` core, `libexec/` command impls,
`supporting/` bundled tools, `test/` Pester.

---

## 4. shim_test/

Shim testing sandbox — four language impls: `C#_Shim_v0.1.1/`, `C++_Shim_v0.1.1/`, `Rust_Shim_v0.1.1/`,
`Zig_Shim_v0.1.2/`.

---

## Conventions (all projects)
- Scoop-compatible CLI verb naming; 4-space indent everywhere (Rust + configs); prefer `anyhow`.
- Two independent Rust Scoop reimpls (hok, hyperscoop) share no code.
- `justfile` automation in hyperscoop/ only. Workspace sandbox via `reasonix.toml`.

## Notes

### Housekeeping
- `hok_/`, `hok~/` and `hok~.zip` at the repo root are historical backups — ignore them.

### Always-on (no trigger — they shape every action)
- Global `token-efficient-processing`; global `file-editing` (UTF-8 without BOM, LF, trailing newline;
  never "read a fragment → `write` the whole file").
- Global `chinese-interaction` — the **interaction** language follows the user, artefacts follow the
  project (repo-root docs English, `.agents/**` Chinese; `.agents/AGENTS.md` → 语言约定).
- Global `no-patchwork-fixes` — fix the misplaced semantic instead of hiding it behind a flag.

### Gates (non-trivial changes)
- **Before implementing** — global `investigate-first`: research → report (evidence + options + a
  recommendation) → wait for an explicit go-ahead. When the request is ambiguous, ask; never treat
  "this is obviously a bug" as permission. Refactors and dedup additionally pass global
  `proactive-review` (its dedup threshold, §7) and `.agents/skills/hok-code-organization`
  (name the real problem first; the P2-6 case).
- **Before committing** — global `proactive-review`; repo-specific checklist
  `.agents/skills/hok-proactive-review`.
- **Before staging** — global `git-commit-practice`: one topic = one commit; run git with
  `-c core.autocrlf=false` (the worktree stays LF); write the message to a file and use
  `git commit -F <file>` — inline `-m` has been mangled by PowerShell before.
- **Deterministic wrap-up** (fmt / clippy / tests) — global `rust-deterministic-gate`; repo specifics
  `.agents/skills/hok-rust-gate-practice` (edition 2024, file-scoped `rustfmt`, never a whole-workspace
  `cargo fmt` while the tree already has diffs).
- **Before landing** (fill-in and records) — every session that lands something runs the wrap-up back-fill
  ([`.agents/skills/hok-worktree-parallel`](.agents/skills/hok-worktree-parallel.md) → 收尾四件套) and
  leaves exactly one devlog entry, which ships as its own wrap-up commit. Trigger, format and commit
  placement:
  [`.agents/docs/AGENTS.md`](.agents/docs/AGENTS.md) R7; the document checks must exit 0
  (`node .agents/scripts/devlog-index.mjs --check`, `doc-links.mjs`, `docs-index-check.mjs`);
  report archiving follows [`.agents/docs/AGENTS.md`](.agents/docs/AGENTS.md) R1–R9
  (procedure: `.agents/skills/hok-report-archive`).

### Session hygiene
- Global `session-scope-guard`: once a topic has clearly drifted from the opening subject *and* will
  keep expanding, **loudly** ask the user to open a new conversation (and propose its scope); do not
  grow the new topic before they confirm.

### Parallel work
- **Orchestration** — global `worktree-parallel` §9, repo specifics
  `.agents/skills/hok-agent-orchestration`. Before fanning work out to subagents: build a
  **file + semantic overlap matrix** (one owner per hot file, shared entry point or shared predicate),
  cap concurrency at 2–3, and make every prompt self-contained with hard prohibitions, known
  environment traps and a main-line review at block boundaries. The parent session owns the landing
  checklist and the handover document.
- **Worktrees** — global `worktree-parallel`, repo specifics
  `.agents/skills/hok-worktree-parallel` (script path, self-test, wrap-up quartet). The load-bearing
  rules are:
  - one conversation = one worktree = one topic, based on the main line's **current** tip;
  - the main line advances by linear `--ff-only` only — **merge commits are forbidden**; whoever lands
    later rebases and preserves both sides' semantics. Its **name** is machine-local git config, not the
    literal `main` (name, current value and how to reset: `.agents/skills/hok-worktree-parallel` →
    本仓事实);
  - each worktree builds into its own `target/`; verification trusts only a binary **you** built and
    copied out yourself — and a fresh worktree must run `cargo build -p hok-shim` first, or
    `crates/libscoop/build.rs:204` panics with "hok-shim binary not found";
  - `.agents/**` and this file are tracked, so every worktree has its own copy — edit and commit them
    normally. The **only** exception is the aggregate hotspot files, which are edited in the main tree
    only (list and reason: [`.agents/AGENTS.md`](.agents/AGENTS.md) → 硬约束 1);
    `skills/scoop-gap-audit.md` is methodology only since 2026-09-26 — nothing appends to it;
  - **never `git add -A` / `git add .`** (stage by topic, file by file) and **never `git clean -fd`**.

### Delivery and release
- CI (`.github/workflows/cicd.yml`) runs only on **pushes to `main`** or on **pull requests** — a push
  to a feature branch runs **no CI**, so run the gates locally before merging. See "Building, Testing
  and Releasing" in `ARCHITECTURE.md`.
- Releases are **manual**: `.github/workflows/release.yml` (`workflow_dispatch` plus a tag input) →
  `pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify`. The version source of truth is `Cargo.toml`;
  `cd_release_please` is gated on `chawyehsu/hok` and never runs on this fork, so version bumps and
  `CHANGELOG.md` are hand-maintained.

### Agent workspace
- AI collaboration artefacts live in **`.agents/`** (tracked; only `.agents/tmp/` and machine-local
  `*.local.md` are ignored). Three standing rule files, each auto-loaded when a file tool touches its
  directory: [`.agents/AGENTS.md`](.agents/AGENTS.md) (workspace), [`.agents/docs/AGENTS.md`](.agents/docs/AGENTS.md)
  (document standard), [`.agents/skills/AGENTS.md`](.agents/skills/AGENTS.md) (skill standard).
- **The gates above are carried by machine-local skills** (`~/.dsh/skills` on this machine:
  `investigate-first`, `proactive-review`, `rust-deterministic-gate`, `worktree-parallel`,
  `git-commit-practice`, …). They are deliberately **not** vendored here: a same-name copy under
  `.agents/skills/` would be shadowed by the higher-priority root and would only add a second,
  drifting definition. If that root is absent, those gates are unenforced — the hok-specific detail
  lives in `.agents/skills/hok-*` either way.
- **Never create `.dsh/`** (directory or link): with both project-level skill roots present, the
  rank-100 `.dsh/skills` shadows `.agents/skills`.
- Human-facing reference material goes in the repo-root `docs/` (index: [`docs/README.md`](docs/README.md)).
- **Committing `.agents/**`**: the first landing is one dedicated commit; after that the plan/audit/skill
  back-fill rides along with its code topic (one topic = one commit), while the devlog entry ships as its
  own wrap-up commit — it has to cite that topic's sha. Files are staged explicitly (never `git add -A` / `.`),
  and anything changed inside a worktree must land with that topic. Full rules:
  [`.agents/AGENTS.md`](.agents/AGENTS.md) → 「提交约定」.
- References into `.agents/**` from tracked files must be **repo-relative**; machine-local absolute paths
  inside historical records stay verbatim.
