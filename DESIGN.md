# Design

> Audience: users evaluating hok, and contributors who want the reasoning rather than the layout.

**Scope** — this file owns **why** hok is built the way it is: the technology choices, their trade-offs
and the numbers behind them.

It does **not** restate:

- *where things live* (layers, data flow, pipelines, build/release) — [`ARCHITECTURE.md`](ARCHITECTURE.md);
- *what changed per release* — [`CHANGELOG.md`](CHANGELOG.md);
- *how to use it* — [`README.md`](README.md) ([中文](README-zh.md)).

## 1. Fork and upstream

Hok is a CLI implementation of [Scoop](https://scoop.sh/) in Rust, forked from
[chawyehsu/hok](https://github.com/chawyehsu/hok) after upstream paused. This fork continues
independently: feature parity work against official Scoop 0.6.0, an async runtime, full localization and
an engineering clean-up. It is **not** intended to merge upstream.

Two practical consequences:

- **Releases are manual.** `.github/workflows/release.yml` (`workflow_dispatch` plus a tag input) builds the
  winsqlite3 shape and uploads the single x64 `hok.exe` (no archive, no checksum);
  `cd_release_please` is gated on the upstream repository and never runs
  here, so version bumps and `CHANGELOG.md` are hand-maintained.
- **The `CICD` workflow is disabled manually on this fork**, so its badge shows no runs. The gates are
  run locally — see [`ARCHITECTURE.md`](ARCHITECTURE.md) → Building, Testing and Releasing.

## 2. Dependency policy

Pure Rust first — but with a stated boundary. When a pure-Rust option exists and performs, it wins. When
it does not, the C-backed or system option is taken deliberately and recorded here rather than hidden.

### 2.1 Replaced or removed

| Upstream | This fork | Why |
|---|---|---|
| `chrono` | `time` (via a `jiff` intermediate step) | Pure Rust and lighter; `time` covers the local offset and format parsing hok needs |
| `futures` executor + thread pool | tokio (`#[tokio::main]`) | The fan-out workloads (checkver, fragmented downloads, bucket pulls) are async, so the executor is unified on tokio; the `futures` crate remains only for `StreamExt` combinators |
| `sysinfo` | Win32 FFI (kernel32) | One process scan does not justify a whole system-monitoring crate |
| `curl` + `static-curl` | `ureq`, then **`reqwest`** (native-tls) | First removed the libcurl C build; the async refactor then moved the client to `reqwest`. GitHub and VirusTotal APIs use the same client |
| `once_cell` | std (`LazyLock`/`OnceCell`) | Standardized since Rust 1.70 |
| `remove_dir_all` | `std::fs::remove_dir_all` | Long-path support since Rust 1.74 |
| `unarc-rs` | `rars` (+ 7z.exe fallback) | The all-in-one extractor pulled an old `sevenz-rust2` and `zip`, duplicating the whole crypto chain |
| `thiserror` v1 + v2 | v2 only | One major version in the tree |
| `md-5` / `sha1` / `sha2` 0.10 + 0.11 | 0.11 only | One RustCrypto digest generation |
| `rust-i18n` | the in-house `hok-i18n` | Build-time locale tables instead of a runtime parser (§6) |
| `dirs` | std + Win32 FFI | Removed with the Windows dependency consolidation |
| `scoop_hash` (self-contained) | RustCrypto backend, now `libscoop::internal::hash` | ~4,700 handwritten hash lines were maintenance debt, not an asset |

### 2.2 Duplicate versions eliminated

`sevenz-rust2` (v0.20) and `zip` (v8) duplicates went away with `unarc-rs`; `thiserror`, `md-5`,
`sha1` and `sha2` now appear exactly once each in the dependency tree.

### 2.3 Current direct dependencies

Versions and the complete list live in the workspace `Cargo.toml` files — **this table says what each
group is for, never which version it pins.**

| Group | Crates | Note |
|---|---|---|
| Archives — pure Rust | `sevenz-rust2`, `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `rars` | One per format. `flate2` is pinned to the pure-Rust `zlib-rs` backend workspace-wide, and `bzip2` 0.6 uses `libbz2-rs-sys` — neither pulls in a C toolchain |
| Archives — C backend | `zstd` | Used for `.zst` only: the pure-Rust `ruzstd` decoder measured 1.4–3.5× slower when this backend was chosen (`ruzstd` is not in the tree) |
| Archives — external process | 7z.exe | Runtime fallback for rare formats (LZH, ISO) |
| Windows | `junction`, `winreg`, `windows-sys`, `innospect`, `nsis`, `shortcuts-rs` | Junction/registry/Win32 APIs, Inno Setup and NSIS installer parsing, `.lnk` writing |
| Network (async) | `reqwest` (native-tls) | Async HTTP for the `Network` trait, downloads and checkver; schannel on Windows, so the OS certificate store and proxy apply |
| Git | `git2` | libgit2 bindings; `gix` was evaluated and not adopted (§7) |
| SQLite cache | `rusqlite` (`bundled`) | Bundled amalgamation by default, with an opt-in `winsqlite3` shape (§4) |
| Serialization | `serde`, `serde_json`, `json5` | JSON / JSON5 |
| CLI | `clap`, `clap_complete`, `clap-verbosity-flag` | Argument parsing and completions |
| Terminal | `crossterm`, `indicatif` | Colours and progress |
| Infrastructure | `anyhow`, `thiserror`, `regex`, `time`, `tokio`, `futures`, `rayon`, `tracing`, … | `rayon` still parallelizes the CPU-bound bucket and manifest-history scans |

## 3. Archive backends

- **RAR is pure Rust and has no C++ fallback.** `.rar` extraction uses `rars` (RAR 1.3/1.4, RAR 1.5–4.x
  and RAR 5+, including volume sets and the RARVM filters). No RAR-related dependency is left in the tree
  at all — not even a dev-dependency — so `cargo test` compiles no C++ for RAR. The parity test compares
  against a committed golden baseline (`crates/libscoop/tests/fixtures/rar/expected-trees.txt`),
  historically derived from the `unrar` 0.5 (UnRAR 6.x) decoder and now regenerated with WinRAR's
  `Rar.exe` (`HOK_RAR_EXE`); the two agree byte for byte on this corpus. Coverage and known gaps:
  [`crates/libscoop/tests/fixtures/rar/README.md`](crates/libscoop/tests/fixtures/rar/README.md).
- **One format still needs C**: `.zst` goes through the `zstd` crate (see §2.3).
- 7z, zip, tar, gz, bz2 and xz are pure Rust; LZH/ISO fall back to 7z.exe at runtime.
- Inno Setup and NSIS installers are decoded by the registry crates `innospect` and `nsis`; the vendored
  forks were retired once the upstream PRs landed.

## 4. SQLite: two build shapes

The manifest cache uses `rusqlite`. A plain `cargo build` compiles the bundled amalgamation, which works
on x86/x64/arm64. The published shape instead links the `winsqlite3.dll` that ships with Windows (about
1 MB smaller):

```powershell
pwsh scripts/build-hok.ps1                            # bundled (the default, stated explicitly)
pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify   # the shape the releases ship
```

- The script stages `winsqlite3.h/.lib` from the Windows SDK into the gitignored `.winsqlite3/` and sets
  `LIBSQLITE3_SYS_USE_PKG_CONFIG` / `SQLITE3_LIB_DIR` / `SQLITE3_INCLUDE_DIR` **for its own process
  only** — no user environment variable, no `.cargo/config.toml` — so a later plain `cargo build` is
  bundled again.
- **x86 cannot use winsqlite3**: the SDK import library exports `__stdcall`-decorated names
  (`_sqlite3_column_name@8`) that `rusqlite`'s cdecl bindings cannot resolve, so i686 stays bundled.
- `crates/libscoop/build.rs` fails loudly on the two silent mistakes instead of building the wrong shape:
  `SQLITE3_LIB_DIR` set without `LIBSQLITE3_SYS_USE_PKG_CONFIG`, and x86 + linked mode.

## 5. Async runtime and concurrency

The CLI is fully async: `#[tokio::main]`, with every I/O-, network- or subprocess-touching library path
an `async fn`. The *shape* of that runtime — which module owns what, how cancellation and backpressure
work — is documented in [`ARCHITECTURE.md`](ARCHITECTURE.md) → Async Runtime. The reasoning is here:

- **Why async at all** — three workloads are naturally fan-out: checkver across many manifests,
  fragmented downloads (parallel HTTP `Range` chunks) and concurrent bucket pulls. A thread pool would
  pay a thread per in-flight request; `buffer_unordered` on tokio does not.
- **Why `reqwest` with `native-tls`, not rustls** — on Windows `native-tls` *is* schannel, i.e. the OS
  certificate store and proxy configuration, matching what official Scoop inherits from PowerShell. It
  also keeps `ring`/`rustls`/`webpki-roots` (and their `getrandom 0.2` pin) out of the tree. hok is
  Windows-only, so no non-Windows TLS backend is needed.
- **Why blocking work stays off the async threads** — git2, archive decoding, script runs and prompts are
  offloaded at the trait boundary with `tokio::task::spawn_blocking`, so call sites stay `async fn`.
- **Why CPU-bound scans stay on rayon** — the bucket and manifest-history scans are CPU-bound; an async
  runtime adds nothing there, so they keep using the rayon pool.

## 6. Shim and i18n design

**Native shim.** `hok-shim.exe` replaces the generated `.cmd` wrapper. It is `#![no_std]` with fixed
buffers and Win32 FFI, embedded into the hok binary, and built as two binaries that differ only in PE
subsystem: `hok-shim` (console) and `hok-shim-gui` (GUI — no console window, and the shell does not
wait).

| Implementation | Mean | Overhead | Size |
|---|---|---|---|
| Direct launch | 29.7 ms | — | — |
| **hok-shim (no_std)** | **51.7 ms** | **+22 ms** | **10 KB** |
| Rust (upstream) | 54.9 ms | +25 ms | 121 KB |
| Zig (upstream) | 53.9 ms | +24 ms | 71 KB |
| C++ (upstream) | 56.9 ms | +27 ms | 158 KB |
| C# (upstream) | 107.3 ms | +77 ms | 14 KB |

(whoami.exe, 30 runs. The fork's shim is the smallest by 7–16× and within the same band as the native
implementations. Key choices: `AttachConsole` avoids the ~400 ms cost of a GUI subsystem creating a new
console, and a job object guarantees the child tree is cleaned up.)

**i18n.** All user-facing strings go through the in-house `hok-i18n` runtime, which replaced `rust-i18n`
in 0.3.0. `crates/hok-i18n/build.rs` flattens `locales/en.yml` / `locales/zh.yml` into key-sorted lookup
tables at build time, so no YAML parser ships in the release binary; `hok-i18n-derive` patches the clap
tree so `--help` is localized too. Usage (`hok config language`, `-L/--lang`) is in
[`README.md`](README.md).

## 7. Deliberate limits

- **`git2`, not `gix`.** A pure-Rust Git would be the policy's first choice, but `gix` costs about
  20 minutes of build time here for the four operations hok needs (clone / pull / ls-remote / status).
  The pragmatic side wins — and the boundary is stated rather than hidden behind a flag.
- **Don't reinvent the wheel, and don't overpay for it either.** Standard algorithms (MD5/SHA) use
  existing crates; platform-specific APIs where no lightweight crate exists stay as a few dozen lines of
  raw FFI.
- **Scoop compatibility is not negotiable.** The SQLite cache schema, the `config.json` format and the
  autoupdate behaviour follow official Scoop.
- **`aria2-min-split-size` defaults to `20M`** where official Scoop uses `5M` (`lib/download.ps1`).
  Aligning it is a registered pending item (`hok`'s own downloader, not aria2c), not an unrecorded drift.

## 8. Engineering guardrails

- **`unsafe_code = deny`** crate-wide; the only exemptions are the `no_std` Win32 shim and the small FFI
  modules (`libscoop::internal::os`, `libscoop::permission`). Every `unsafe` is audited and classified in
  [`.agents/docs/active/audit/unsafe-audit.md`](.agents/docs/active/audit/unsafe-audit.md).
- **Zero-warning policy** — the tree builds with no warnings, and `cargo test --workspace` is green.
- **criterion guardrail benchmarks** — `cargo bench` covers the key pure functions (`compare_versions`,
  glob, hashing, manifest parsing, URL helpers) in the `pure_functions` target, plus an `event_emission`
  target over the event bus, so a refactor that regresses them fails loudly.

## 9. Performance

Hok (and the libscoop backend) aims to be a faster yet powerful alternative to official Scoop. The
numbers below were captured in a Windows Sandbox on the author's machine (AMD Ryzen 5 2600, 32 GB,
Windows 10) with hok 0.1.0-beta.6 — historical, so re-run the scripts to refresh:

```sh
# Benchmarking scoop bucket list
Benchmark 1: scoop bucket list
  Time (mean ± σ):      5.610 s ±  0.627 s    [User: 6.573 s, System: 3.520 s]
  Range (min … max):    4.784 s …  7.063 s    10 runs

Benchmark 2: hok bucket list
  Time (mean ± σ):     159.4 ms ±  28.3 ms    [User: 86.4 ms, System: 175.2 ms]
  Range (min … max):   140.0 ms … 252.1 ms    18 runs

Summary
  hok bucket list ran
   35.19 ± 7.38 times faster than scoop bucket list
```

How to reproduce:

- [`scripts/benchmark/`](scripts/benchmark/README.md) — the original Windows Sandbox benchmark suite.
- [`scripts/benchmark_shims.py`](scripts/benchmark_shims.py) — the shim launcher benchmark behind the
  table in §6 (`whoami.exe`, 30 runs).
- [`scripts/benchmark_vs_scoop.ps1`](scripts/benchmark_vs_scoop.ps1) — the host-direct guardrail
  benchmark: the same read-only commands on both tools, comparing exit codes and normalized output and
  reporting a timing ratio. Scoop's network/stateful commands run only with `-IncludeNetwork`; the rest
  are skipped when offline.
- `cargo bench` — the micro-benchmarks from §8.
