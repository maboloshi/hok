# Hok Architecture

> Audience: contributors and AI agents working on hok. This file describes the **shipped** structure.
> Runtime conventions, build gates and workflow rules live in [`AGENTS.md`](AGENTS.md);
> the doc map at the bottom says which file owns what.
>
> **Drift rule** — this file states *structure and relationships* only. Counts, module enumerations,
> constant values, version pins and upstream-patch history are **pointed at**, never copied: a copy
> drifts. (This file has already been wrong about the event-bus type, the crate list, and the
> `internal/` module count — each time because it restated something that lives in code.)

## Crate Dependency Graph

```
hok (binary)                         ← repository root crate, src/
├── libscoop (core library)          ← crates/libscoop/
│   ├── Scoop config, session, buckets
│   ├── Package download, manifest, resolve, sync
│   ├── Event-driven operation pipeline
│   └── Shim / shortcut / persist management
├── hok-shim (shim generation)       ← crates/hok-shim/  (no_std, Win32 FFI)
├── hok-i18n (i18n runtime)          ← crates/hok-i18n/  (build-time locale tables + lookup macro)
└── hok-i18n-derive (i18n derive)    ← crates/hok-i18n-derive/
```

## Layering (one-way dependencies)

```
CLI (hok)  →  facade (libscoop/src/lib.rs)  →  domain services (libscoop/src/*.rs)
           →  core business (package/)      →  utilities (internal/)
```

| Layer | Contents | Rule |
|---|---|---|
| CLI | `src/bin/hok.rs`, `src/cmd/`, `eventloop.rs`, `output.rs`, `cui.rs`, `i18n.rs`, `scoop_handler.rs` | Arg parsing, event loop, rendering, dispatch. **No business logic and no filesystem access** (except output). |
| Facade | `crates/libscoop/src/lib.rs` | `pub mod` / `pub use` only — no function bodies. |
| Domain services | `session` `config` `bucket` `cache` `network` `event` `handler` `shim` `shortcut` `persist` `env` `psmodule` `constant` `error` | Public API of the library. |
| Core business | `package/` — `mod.rs` (core types), `identity` `manifest*` `query` `resolve` `download` `sync*` `checkver*` `checkhashes` `checkurls` `operations/` | `pub(crate)`, selectively re-exported through `lib.rs`. May use utilities + domain services. |
| Utilities | `crates/libscoop/src/internal/` — domain-agnostic helpers: paths, hashing, network, archive decoding, PE/Inno/NSIS parsing | `pub(crate)`. **Must not depend on `package/` or domain-service types.** The module list lives in the directory — not enumerated here (enumeration drifts). |

Accepted exceptions (checked, do not widen):

- `internal/archive.rs` → `crate::event::Event` is the **only** utility-layer dependency on a domain
  type; it exists to report extraction progress.
- Domain services and `package/` reference each other (shim/shortcut/persist/env use `package::Package`
  data types; `package/operations` calls service entry points such as `shim::add`). The requirement is
  **acyclic**, not strictly one-way.
- `config::edit` stays in the library so a future `config edit` subcommand can reuse it; `cmd/config.rs`
  is only a caller.

## Main Data Flow

```
                    ┌──────────────────────────────────┐
                    │        hok (binary crate)         │
                    │                                  │
  ┌───────┐   ┌────▼────┐   ┌───────────┐   ┌────────┐│
  │ User  │──▶│ cmd::   │──▶│ operation │   │ Event  ││
  │ (CLI) │   │ start() │   │ (libscoop)│   │ Loop   ││
  └───────┘   └─────────┘   └─────┬─────┘   └───┬────┘│
                                  │              │     │
                                  │  events      │     │
                                  │──────────────▶     │
                                  │              │     │
                                  │◀──responses──│     │
                                  │              │     │
                                  │         ┌────▼──┐ │
                                  │         │ Event │ │
                                  │         │Handler│ │
                                  │         └───────┘ │
                                  │              │     │
                                  │         ┌────▼──┐ │
                                  │         │Output │ │
                                  │         │(cui/  │ │
                                  │         │output)│ │
                                  │         └───────┘ │
                    └──────────────────────────────────┘
```

1. **CLI parse** — `cmd::start()` detects language, parses args, creates `Session`.
2. **Session init** — config loaded (with fallback chain); event bus created lazily.
3. **Operation dispatch** — e.g. `install::execute(args, &session)`.
4. **Operation execution** — `libscoop` operations emit events through `session.emitter()`. The library
   **never writes to stdout directly**: user-visible text goes through the synchronous `Output` sink,
   progress and interactive prompts through the asynchronous `tokio::sync::mpsc` event bus owned by `Session`.
5. **Event loop** — a tokio task (`eventloop::run_event_loop()`) selects over backend events and stdin,
   rendering progress/UIs or delegating to `EventHandler`.
6. **Handler output** — `ScoopHandler` (or a custom handler) formats events into the `output` module.

Event pairs follow `Start / Progress* / Done` (prompts use `PromptXxx` + `PromptXxxResult`); the frontend
relies on the `Done` event to retire that operation's UI.

## Async Runtime

hok is **fully async**: the CLI runs on a multi-threaded tokio runtime and every library path that
touches I/O, the network or a subprocess is an `async fn`.

| Concern | Implementation | Where |
|---|---|---|
| Entry | `#[tokio::main] async fn main` (`rt-multi-thread` + `macros` + `sync` + `signal`) | `src/bin/hok.rs` |
| Network | `Network` trait via `async-trait`; HTTP through **`reqwest`** (native-tls → schannel on Windows) | `libscoop/src/internal/network.rs` |
| Library surface | `async fn` throughout: sync / install / remove / download / checkver / checkurls / checkhashes / create / extract / script / bucket / auto_pr / virustotal / github | `libscoop/src/**` |
| Concurrency | `buffer_unordered` (checkver, fragmented download); `tokio::spawn` (bucket update) | `checkver.rs`, `package/download_fragmented.rs`, `bucket.rs` |
| Blocking offload | `tokio::task::spawn_blocking` at the **trait layer**, so call sites are unchanged: git2 clone/pull/ls-remote, archive decode (7z/zip/tar/rar), subprocess script runs, stdin prompts | `internal/git.rs`, `internal/archive.rs`, `operations/script.rs`, `eventloop.rs` |
| Cancellation | `tokio_util::sync::CancellationToken` created at startup, fed by `tokio::signal::ctrl_c`, carried on `Session` | `src/lib.rs`, `libscoop/src/session.rs` |
| Event bus | two **bounded `tokio::sync::mpsc`** channels (capacity = `constant::EVENT_BUS_CAPACITY`) | `libscoop/src/event.rs` |
| Event loop | a **tokio task** using `tokio::select!` (backend events ‖ stdin), not a dedicated thread | `src/eventloop.rs` |

### Backpressure, and the one hazard it leaves open

Producers use `try_send` (drop-on-full, error ignored) for **high-frequency** events and
`send().await` for **low-frequency / terminal** events. Download progress is additionally throttled
by a throttle (`download::ProgressThrottle`, mirroring upstream rake), so a slow consumer cannot flood the bus.

The consequence is deliberate but not free: on a bounded bus a *terminal* event sent with `try_send`
is dropped when the channel is full. `PackageDownloadDone` is emitted that way, so with a saturated
queue a progress bar can be left unfinished. That is recorded as an open item rather than papered
over — a queue-full terminal event must not be silently discarded.

### Cancellation semantics

- Cancelling **keeps resumable state**: fragmented and single-stream downloads retain `.parts` /
  `.download` for the next run; cleanup removes only genuine failure products.
- Subprocesses (`run_script`, `run_checkver_script`) are `kill`ed and `wait`ed before returning.
- `CursorGuard` (RAII) plus a done-event on every path restore the terminal cursor, cancel included.

## Install Pipeline

```
pre_install → extract → installer.script → link_current → shim → persist → post_install → metadata
```

`link_current` is `current`-junction aware: with `no_junction` enabled the layout switches to
versioned directories (`Session::current_dir_name` / `versioned_dir`), and shim, metadata, script
`$dir`, query, list and cleanup all follow it.

## Commands

`src/cmd/` holds one file per command handler; `build.rs` scans that directory and generates
`__cmd_reg__.rs`, so **adding a command = dropping a `.rs` file that exposes `execute()`** plus the
enum variant and match arm. The command count and list live in `src/cmd/` (`hok --help` shows the runtime result) — not hard-coded here.

## Key Architecture Decisions

| Decision | Rationale |
|---|---|
| **Event-driven operations** | Decouples backend logic (libscoop) from UI (hok). Allows different frontends (CLI, TUI, GUI) by swapping the event handler. |
| **Full-duplex event bus** | Backend emits progress events; frontend sends back user responses (confirm, select). Two bounded `tokio::sync::mpsc` channels = clean bidirectional flow. |
| **Session as handle** | `Session` holds only long-lived state (config, event bus). Operation state is ephemeral, keeping `Session` lightweight and reusable. |
| **Config fallback chain** | Tries known config paths in order; if all fail, creates a default config and emits a warning event. Graceful degradation. |
| **Auto-discovered commands** | `build.rs` scans `src/cmd/` and generates the `mod` declarations, removing a whole class of merge conflicts. |
| **Shim as a separate crate** | `hok-shim` is standalone (`no_std`, fixed buffers, Win32 FFI) so shim generation can be reused without the full dependency tree. |
| **Deny-by-default `unsafe`** | `unsafe_code = deny` crate-wide; the only exemptions are the `no_std` Win32 shim and the small FFI modules (`libscoop::internal::os`, `libscoop::permission`). Every `unsafe` is audited and classified. |

## Building, Testing and Releasing

See [`AGENTS.md`](AGENTS.md) for the full command list. The short version:

```powershell
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets
```

- CI (`.github/workflows/cicd.yml`) runs only on **pull requests** and **pushes to `main`** —
  a push to a feature branch does not run it, so run the gates locally before merging.
- Release is **manual**: `.github/workflows/release.yml` (`workflow_dispatch` with a `tag` input)
  builds the published shape with `pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify`.
- The version source of truth is `Cargo.toml` (workspace + package); released history lives in
  `CHANGELOG.md`. On this fork the `cd_release_please` job never runs (it is gated on the upstream
  repository), so version bumps and changelog entries are maintained by hand.

## New Contributor Guide

- **Start with**: `src/cmd/mod.rs` (entry point) → `src/eventloop.rs` (event loop pattern) →
  `crates/libscoop/src/session.rs` (core API).
- **To add a command**: see "Adding a new command" in `src/cmd/mod.rs`.
- **To add an event**: see "Extending" in `crates/libscoop/src/event.rs`.
- **i18n**: all user-facing strings go through the in-house `hok_i18n` runtime (`crates/hok-i18n/`);
  translations live in `locales/*.yml` and are flattened to key-sorted tables by that crate's
  `build.rs` at build time, so no YAML parser ends up in the release binary. `hok-i18n-derive` patches
  the clap tree so `--help` is translated too. (Upstream `rust-i18n` was replaced in `84fe376`.)
- **Module details**: each source file has a `//!` header with design rationale, extension guides and
  important notes. Run `cargo doc --open` or read the source directly.

## Doc Map

| File | Audience | Owns |
|---|---|---|
| [`README.md`](README.md) / [`README-zh.md`](README-zh.md) | users | Install, commands, configuration, behaviour |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | contributors, agents | Shipped structure (this file) |
| [`AGENTS.md`](AGENTS.md) | agents | Runtime conventions, build gates, workflow rules, doc routing |
| [`CHANGELOG.md`](CHANGELOG.md) | users | Released changes |
| [`.agents/skills/`](.agents/skills/) | agents | Reusable operational knowledge and pitfalls |
| [`.agents/docs/`](.agents/docs/) | maintainers | Specs, plans, audits, research, devlog, archive |
| [`docs/`](docs/) | outside readers | Long-lived reference material |
