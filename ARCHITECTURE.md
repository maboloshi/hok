# Hok Architecture

## Crate Dependency Graph

```
hok (binary)
├── libscoop (core library)
│   ├── Scoop config, session, buckets
│   ├── Package download, manifest, resolve, sync
│   ├── Event-driven operation pipeline
│   └── Shim / shortcut / persist management
├── hok-shim (shim creation)
└── hok-i18n-derive (i18n derive macro)
```

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
2. **Session init** — Config loaded (with fallback chain), event bus created lazily.
3. **Operation dispatch** — e.g. `install::execute(args, &session)`.
4. **Operation execution** — `libscoop` operations emit events via `session.emitter()`.
5. **Event loop** — `eventloop::run_event_loop()` receives events; renders progress/UIs or delegates to `EventHandler`.
6. **Handler output** — `ScoopHandler` (or custom) formats events to `output` module.

## Key Architecture Decisions

| Decision | Rationale |
|---|---|
| **Event-driven operations** | Decouples backend logic (libscoop) from UI (hok). Allows different frontends (CLI, TUI, GUI) by swapping the event handler. |
| **Full-duplex event bus** | Backend emits progress events; frontend sends back user responses (confirm, select). Two flume channels = clean bidirectional flow. |
| **Session as handle** | `Session` holds only long-lived state (config, event bus). Operation state is ephemeral, keeping `Session` lightweight and reusable. |
| **Config fallback chain** | Tries known config paths in order; if all fail, creates default config + emits warning event. Graceful degradation. |
| **Auto-discovered commands** | `build.rs` scans `src/cmd/` for `.rs` files and generates `mod` declarations. Adding a command requires only 3 manual steps (file, enum variant, match arm). |
| **Shim as separate crate** | `hok-shim` is a standalone library so shim generation can be reused by external tools without pulling in the full hok dependency tree. |

## New Contributor Guide

- **Start with**: `src/cmd/mod.rs` (entry point) → `src/eventloop.rs` (event loop pattern) → `crates/libscoop/src/session.rs` (core API).
- **To add a command**: See "Adding a new command" in `src/cmd/mod.rs`.
- **To add an event**: See "Extending" in `crates/libscoop/src/event.rs`.
- **i18n**: All user-facing strings use `rust_i18n::t!()`. Add translations in `locales/`.
- **Module details**: Each source file has a `//!` header with design rationale, extension guides, and important notes. Run `cargo doc --open` or read the source directly.
