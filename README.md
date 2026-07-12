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
  alias        Manage Scoop aliases
  bucket       Manage manifest buckets
  cache        Package cache management
  cat          Inspect the manifest of a package
  checkhashes  Verify and update manifest hashes
  checkup      Check for potential problems with installed packages
  checkurls    Check URLs of manifests are accessible
  checkver     Check manifest for a newer version
  cleanup      Cleanup apps by removing old versions
  completions  Generate shell completions
  config       Configuration management
  create       Create a manifest from a download URL
  depends      Show dependencies of a package
  export       Export installed packages list
  formatjson   Format manifests and update them in-place
  hold         Hold package(s) to disable changes
  home         Browse the homepage of a package
  import       Import installed packages from a file
  info         Show package(s) basic information
  install      Install package(s)
  list         List installed package(s)
  missing-checkver  Check bucket manifests missing checkver/autoupdate
  prefix       Show the directory where a package is installed
  reset        Reset a package to reapply shims/shortcuts
  reinstall    Reinstall package(s) (uninstall then install)
  search       Search available package(s)
  shim         List or inspect shims
  status       Show status of installed package(s)
  unhold       Unhold package(s) to enable changes
  uninstall    Uninstall package(s)
  update       Fetch and update subscribed buckets, or upgrade package(s)
  upgrade      Upgrade installed package(s)
  virustotal   Check a package's download against VirusTotal
  which        Show the shim location(s) of a command
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
      --detail   Show detailed operation information for debugging
```

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
