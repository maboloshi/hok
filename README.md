# hok

> Hok is a CLI implementation of [Scoop](https://scoop.sh/) in Rust

[![cicd][cicd-badge]][cicd]
[![release][release-badge]][releases]
[![crates-svg]][crates-url]
[![license][license-badge]](LICENSE)
[![downloads][downloads-badge]][releases]
[![docs-svg]][docs-url]

**[简体中文](./README-zh.md)**

> **Fork Notice**: This is a community-maintained fork of [chawyehsu/hok](https://github.com/chawyehsu/hok).
> The original author has paused development, so this fork continues independently with
> new features, optimizations, and fixes. Not intended to merge upstream.

## Install

```sh
# Build from source
git clone https://github.com/maboloshi/hok
cd hok
cargo build --release
./target/release/hok --help
```

## Commands

```raw
$ hok help
Hok is a CLI implementation of Scoop in Rust

Usage: hok.exe <COMMAND>

Commands:
  alias             List, add, or remove Scoop aliases
  bucket            Manage manifest buckets
  cache             Package cache management
  cat               Inspect the manifest of a package
  checkhashes       Verify and update manifest hashes
  checkup           Check for potential problems with installed packages
  checkurls         Check manifest URLs for validity
  checkver          Check manifest for a newer version
  ci-auto-pr        Auto-update manifests and create pull-requests via GitHub API (CI mode)
  cleanup           Cleanup apps by removing old versions
  completions       Generate shell completions
  config            Configuration management
  create            Create a manifest from a download URL
  depends           Show dependencies of a package
  download          Download apps in the cache folder and verify hashes
  export            Export installed packages list
  format-json       Format manifest JSON files in a bucket directory
  hold              Hold package(s) to disable changes
  home              Browse the homepage of a package
  import            Import installed packages from a file
  info              Show package(s) basic information
  install           Install package(s)
  list              List installed package(s)
  missing-checkver  Check bucket manifests missing checkver and autoupdate
  prefix            Show the directory where a package is installed
  reinstall         Reinstall a package
  reset             Reset an app to resolve conflicts (reapply shims, shortcuts, post_install)
  search            Search available package(s)
  shim              List or inspect shims
  status            Show the status of all installed apps
  unhold            Unhold package(s) to enable changes
  uninstall         Uninstall package(s)
  update            Fetch and update subscribed buckets, or upgrade installed package(s)
  upgrade           Upgrade installed package(s)
  virustotal        Check a package's download URL against VirusTotal
  which             Show the shim location(s) of a command
  help              Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
      --detail   Show detailed operation information for debugging
```

## Configuration

hok reads its config from `~/.config/hok/config.json` (a Scoop-compatible
`config.json`). On first run the supported keys are migrated once from
Scoop's own config file; afterwards only hok's file is used.

```sh
hok config list           # current config as pretty JSON
hok config list --all     # every supported key + current value + default
hok config set <key> <value>
hok config unset <key>    # remove a key (falls back to its default)
hok config --help         # full settings reference with defaults
```

### Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `aria2-enabled` | bool | `true` | Enable fragmented downloads (hok's built-in HTTP range splitter) |
| `aria2-split` | int | `5` | Number of connections used for each fragmented download |
| `aria2-max-connection-per-server` | int | `5` | Maximum connections to one server per download |
| `aria2-min-split-size` | size | `5M` | Minimum file size to trigger fragmented downloads (e.g. `5M`, `10M`) |
| `cache_path` | path | `$SCOOP_CACHE` or `<root>/cache` | Download cache directory |
| `cat_style` | string | *(empty)* | When set, use `bat` to display manifests (requires `bat` installed) |
| `default_architecture` | `64bit`\|`32bit`\|`arm64` | auto-detected | Preferred architecture used for installation |
| `editor` | string | *(empty)* | Text editor used by `hok config edit` (e.g. `code --wait`); falls back to `$EDITOR`, then the system default handler |
| `global_path` | path | `$SCOOP_GLOBAL` or `%ProgramData%\scoop` | Root directory for globally installed apps |
| `gh_token` | string | *(empty)* | GitHub API token used for authenticated requests |
| `ignore-failures` | bool | `true` | Continue multi-package operations despite individual failures |
| `ignore_running_processes` | bool | `false` | Proceed even if the app is running (warning instead of abort) |
| `language` | `auto`\|`en`\|`zh` | `auto` | CLI language for messages and help text |
| `last_update` | string | *(empty)* | Timestamp of the last bucket update (managed by hok; edit the file only) |
| `no-color` | bool | `false` | Disable colored output |
| `no_junction` | bool | `false` | Do not use the `current` junction; shims point to version dirs instead |
| `output-style` | `scoop`\|`pacman` | `scoop` | Output style of progress and status messages |
| `private_hosts` | array | *(empty)* | Private hosts needing extra auth headers (edit the file only) |
| `proxy` | string | `none` | Proxy: `none` \| `default` \| `currentuser` \| `[username:password@]host:port` |
| `root_path` | path | `$SCOOP` or `~/scoop` | Scoop root directory |
| `use_isolated_path` | bool\|string | *(empty)* | Store apps' PATH entries in a dedicated env var (`SCOOP_PATH` by default) |
| `use_sqlite_cache` | bool | `false` | Use SQLite database for manifest caching (Scoop-compatible schema) |
| `virustotal_api_key` | string | *(empty)* | VirusTotal API key used for uploading/scanning files |

Most keys are settable via `hok config set`; `last_update` and
`private_hosts` are only editable in the config file. Aliases are managed
with the `hok alias` command. Run `hok config list --all` for the effective
defaults on your machine.

## New Features (since original fork)

Compared to the original hok, this fork adds:

- **`--detail`** — global verbose flag shows per-package progress (extraction, shims, shortcuts)
- **checkver** — full implementation with regex, JSONPath, XPath, PowerShell script,
  reverse/replace, GitHub and SourceForge shortcuts, autoupdate with hash recomputation
- **reinstall** — uninstall + same-version reinstall with held-state preservation
- **Native shim** — `hok-shim.exe` replaces `.cmd` wrappers (GUI detection, job objects)
- **Pure Rust shortcuts** — `.lnk` writer using `shortcuts-rs` crate, no COM FFI, args/icon support
- **SQLite manifest cache** — `use_sqlite_cache` config, compatible with Scoop's schema
- **Resumable fragmented downloads** — partial parts resume via HTTP Range, no restart
- **`hok update` improvements** — 15-min cooldown, `--force` bypass, visible cache refresh
- **Batch failure isolation** — `ignore_failures` config keeps multi-package operations
  running even if individual packages fail (applies to install/upgrade/uninstall/cleanup)
- **Fixed upstream bug**: `reset` now correctly runs `post_install` scripts
  (original Scoop bug — Scoop skips post_install on reset)
- **cleanup** — remove old versions of installed packages
- **depends / prefix / which / checkup / shim** — new CLI commands
- **export / import** — export/import installed package lists as JSON
- **alias** — list/add/remove aliases with config persistence
- **create** — generate manifest skeleton from download URL
- **virustotal** — VirusTotal API v3 integration

## Development

Prerequisites: Git, Rust

```sh
git clone https://github.com/maboloshi/hok
cd hok
cargo build
cargo run -- help
```

## Code Quality

This fork maintains a strong focus on dependency hygiene. Notable improvements over upstream:

- **Removed heavy dependencies**: `chrono → time` (via a `jiff` intermediate step), `curl-static → ureq`, `futures` runtime, `sysinfo`, `once_cell`, `remove_dir_all`
- **Eliminated duplicate crate versions**: `unarc-rs` replaced (removed `sevenz-rust2 v0.20` + `zip v8` duplicates), `thiserror v1+v2` unified, `md-5/sha1/sha2 v0.10+v0.11` unified
- **Code deduplication**: Macroized repetitive accessor patterns (×16), extracted shared helpers (×5), consolidated 4 benchmark files into 1
- **Zero external C build dependencies for decompression**: All archive formats (7z, zip, tar, gz, bz2, xz, zst) handled by pure Rust crates; RAR via `unrar` (C++ unrar library); LZH/ISO via 7z.exe fallback

## Performance

Hok (also the libscoop backend) aims to provide a faster yet powerful alternative
to the original Scoop. Here are some random benchmarks captured in the Windows
Sandbox environment on my PC (AMD Ryzen 5 2600, 32G RAM, Windows 10).

```sh
# versions:
hok/dorado 0.1.0-beta.6
scoop-search/main 1.5.0
sfsu/extras 1.14.0
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

You may run the benchmarks yourself using provided benchmark scripts in the
[`scripts` directory]. Results may vary on different environments.

## License

**hok** © [Chawye Hsu](https://github.com/chawyehsu) and contributors.
Released under the [Apache-2.0](LICENSE) license.
For licenses of sub crates, see [COPYING](COPYING).

[cicd-badge]: https://github.com/maboloshi/hok/workflows/CICD/badge.svg
[cicd]: https://github.com/maboloshi/hok/actions/workflows/cicd.yml
[release-badge]: https://img.shields.io/github/v/release/maboloshi/hok
[releases]: https://github.com/maboloshi/hok/releases/latest
[crates-svg]: https://img.shields.io/crates/v/libscoop.svg
[crates-url]: https://crates.io/crates/libscoop
[license-badge]: https://img.shields.io/github/license/maboloshi/hok
[downloads-badge]: https://img.shields.io/github/downloads/maboloshi/hok/total
[docs-svg]: https://docs.rs/libscoop/badge.svg
[docs-url]: https://docs.rs/libscoop
[`scripts` directory]: scripts/benchmark/README.md
