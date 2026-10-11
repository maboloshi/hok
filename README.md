# hok

> Hok is a CLI implementation of [Scoop](https://scoop.sh/) in Rust

[![cicd][cicd-badge]][cicd]
[![release][release-badge]][releases]
[![license][license-badge]](LICENSE)
[![downloads][downloads-badge]][releases]

**[简体中文](./README-zh.md)**

> **Fork Notice**: This is a community-maintained fork of [chawyehsu/hok](https://github.com/chawyehsu/hok).
> The original author has paused development, so this fork continues independently with
> new features, optimizations, and fixes. Not intended to merge upstream.

> Note: the `CICD` workflow is currently **disabled on this fork** (the release is built
> by `release.yml`), so the status badge above is expected to show no runs.

## Install

> Requirements: Windows, Git, Rust. The first build must produce the shim binary first
> (`crates/libscoop/build.rs` embeds it), so build the workspace as a whole.

```sh
git clone https://github.com/maboloshi/hok
cd hok
cargo build -p hok-shim   # the embedded shim; also built by the workspace build below
cargo build --release
./target/release/hok --help
```

The released `hok.exe` (x64, uploaded bare — no archive, no checksum) uses the `winsqlite3`
shape, linking the SQLite that ships with Windows. To reproduce that shape — or to build
the default bundled-amalgamation shape — use the supported script:

```sh
pwsh scripts/build-hok.ps1                       # bundled shape (default)
pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify   # the shape the releases ship
```

## Commands

```console
$ hok help
Subcommands:
  alias             List, add, or remove Scoop aliases
  ci-auto-pr        Auto-update manifests and create pull-requests via GitHub API (zero external binary, CI mode)
  bucket            Manage manifest buckets
  cache             Manage the download cache
  cat               Inspect the manifest of a package
  checkhashes       Verify and update manifest hashes
  checkup           Check for potential problems
  checkurls         Check manifest URLs for validity
  checkver          Check manifest for a newer version
  cleanup           Cleanup apps by removing old versions
  completions       Generate shell completions
  config            Configuration management
  create            Create a manifest from a download URL
  depends           Show dependencies of a package
  download          Download apps in the cache folder and verify hashes
  export            Export installed packages list
  formatjson        Format manifest JSON files in a bucket directory
  health            Check for potential problems with installed packages
  hold              Hold package(s) to disable changes
  home              Browse the homepage of a package
  import            Import installed packages from a Scoopfile
  info              Display information about an app
  install           Install package(s)
  list              List installed package(s)
  missing-checkver  Check bucket manifests missing checkver and autoupdate
  prefix            Show the directory where a package is installed
  reinstall         Reinstall a package
  search            Search available package(s)
  shim              List or inspect shims
  status            Show the status of all installed apps
  reset             Reset an app to resolve conflicts (reapply shims, shortcuts, post_install)
  unhold            Unhold package(s) to enable changes
  uninstall         Uninstall package(s)
  update            Fetch and update subscribed buckets, or upgrade installed package(s)
  upgrade           Upgrade installed package(s)
  virustotal        Check a package's download URL against VirusTotal
  which             Show the shim location(s) of a command
  help              Print this message or the help of the given subcommand(s)

Global Options:
  -v, --verbose...         Increase logging verbosity
  -q, --quiet...           Decrease logging verbosity
  -L, --lang <LANGUAGE>    Force CLI language (auto/en/zh)
  -N, --no-color           Disable colored output
  --confirm                Force the transaction confirmation gate, ignoring the confirm config
  -h, --help               Print help
  -V, --version            Print version
```

### Runs that write to disk require an explicit `-d`

`-d`/`--dir` still defaults to `.` (the current directory) — but **not for a run
that writes**. `formatjson`, and `checkhashes`/`checkver` with `-u/--update` or
`-f/--force`/`--force-update`, now refuse to start without `-d`:

```console
$ hok formatjson
error: formatjson writes to disk, so the target must be given explicitly: pass -d <dir>. (Read-only runs default to the current directory; a write run never scans it implicitly.)
$ hok formatjson -d ./bucket
  Formatting ./bucket/7zip.json...
Formatted 1 manifest(s).
```

This is a **breaking change** from previous hok releases, which silently scanned
the current directory. The reason is that the scan behind these commands
(`manifest_walker::discover`) collects *every* `*.json` file in the tree —
`package.json` is the only exclusion — and `formatjson` does not check whether a
file looks like a Scoop manifest. Run in a source checkout, a bare
`hok formatjson` rewrote `tsconfig.json`, `package-lock.json`, everything under
`target/` and even `~/.config/hok/config.json`. Upstream Scoop avoids this by
declaring `-Dir` `Mandatory = $true` (`bin/formatjson.ps1`,
`bin/checkhashes.ps1`, `bin/checkurls.ps1`, `bin/missing-checkver.ps1`) or by
throwing (`bin/checkver.ps1`).

`formatjson` additionally names every file it rewrites (upstream prints
nothing), so the blast radius is visible as it happens.

Read-only runs are untouched: `checkurls`, `missing-checkver`, a plain
`checkhashes` (no `-u`/`-f`) and a plain `checkver` (no `-u`/`-f`) all keep
scanning `.` when `-d` is omitted. That is a deliberate difference from
upstream, which requires `-Dir` for those too.

### `hok checkhashes` and the download cache

By default `checkhashes` downloads every URL again for each run and deletes its
temporary files afterwards, so a check always verifies what the server serves
right now — the same behaviour as official Scoop without `-UseCache`.

`-k`/`--keep-cache` is the equivalent of Scoop's `-UseCache`: the downloaded
files stay on disk (under the manifest's version) and the **next run reuses
them instead of downloading again**, which means an already out-of-date file
can be reported as `OK`. Use it only when you deliberately want to re-check a
local copy; add `-f`/`--force` to re-download anyway.

## Configuration

hok reads its config from `~/.config/hok/config.json` (a Scoop-compatible
`config.json`). On first run the supported keys are migrated once from
Scoop's own config file; afterwards only hok's file is used.

```sh
hok config                # the settings the config file holds (key = value)
hok config -j             # the same settings as pretty JSON
hok config -a             # every setting + current value + effective default
hok config <key>          # print one setting
hok config <key> <value>  # write one setting
hok config rm <key>       # remove a key (falls back to its default)
hok config edit           # open the config file in an editor (hok extension)
hok config --help         # full settings reference with defaults
```

Setting names are used exactly as listed below; there are no aliases. `-j` and
`-a` are hok extensions (`-a` is the only view of the settings hok does not
store yet).

### Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `aria2-enabled` | bool | `true` | Enable fragmented downloads (hok's built-in HTTP range splitter) |
| `aria2-split` | int | `5` | Number of connections used for each fragmented download |
| `aria2-max-connection-per-server` | int | `5` | Maximum connections to one server per download |
| `aria2-min-split-size` | size | `20M` | Minimum file size to trigger fragmented downloads (e.g. `5M`, `10M`); hok defaults to `20M`, official Scoop uses `5M` |
| `cache_path` | path | `$SCOOP_CACHE` or `<root>/cache` | Download cache directory |
| `cat_style` | string | *(empty)* | Components passed to `bat --style` when displaying manifests (requires `bat` installed; a value `bat` rejects falls back to plain JSON) |
| `default_architecture` | `64bit`\|`32bit`\|`arm64` | auto-detected | Preferred architecture used for installation |
| `editor` | string | *(empty)* | Text editor used by `hok config edit` (e.g. `code --wait`); falls back to `$EDITOR`, then the system default handler |
| `global_path` | path | `$SCOOP_GLOBAL` or `%ProgramData%\scoop` | Root directory for globally installed apps |
| `gh_token` | string | *(empty)* | GitHub API token used for authenticated requests |
| `ignore_failures` | bool | `false` | Continue multi-package operations despite individual failures |
| `ignore_running_processes` | bool | `false` | Proceed even if the app is running (warning instead of abort) |
| `jobs` | int | `4` | Concurrent package downloads |
| `language` | `auto`\|`en`\|`zh` | `auto` | CLI language for messages and help text |
| `last_update` | string | *(empty)* | Timestamp of the last bucket update (written by hok; settable, but normally left alone) |
| `no_color` | bool | `false` | Disable colored output |
| `confirm` | bool | `false` | Ask for confirmation before an operation that writes |
| `no_junction` | bool | `false` | Do not use the `current` junction; shims point to version dirs instead |
| `autostash_on_conflict` | bool | `false` | Stash uncommitted changes that block a repo pull during update (hok: every bucket pull; upstream: only Scoop's own repo) |
| `private_hosts` | array | *(empty)* | Private hosts needing extra auth headers (edit the file only) |
| `proxy` | string | `none` (unset) | Proxy: `none` \| `default` \| `currentuser` \| `[username:password@]host:port` |
| `root_path` | path | `$SCOOP` or `~/scoop` | Scoop root directory |
| `script_engine` | bool | `false` | Reuse one resident `pwsh` process for package scripts |
| `use_git_history` | bool | `true` | Resolve historical manifests through the bucket's git history |
| `use_isolated_path` | bool\|string | *(empty)* | Store apps' PATH entries in a dedicated env var (`SCOOP_PATH` by default) |
| `use_sqlite_cache` | bool | `false` | Use SQLite database for manifest caching (Scoop-compatible schema) |
| `virustotal_api_key` | string | *(empty)* | VirusTotal API key used for uploading/scanning files |

This table mirrors `hok config -a` (every settable key with its effective
default). Most keys are settable via `hok config <key> <value>`; `private_hosts`
is only editable in the config file. Aliases are managed with the `hok alias`
command.

## New Features (since original fork)

Compared to the original hok, this fork adds:

- **`-v/--verbose`, `-q/--quiet`, `-L/--lang`** — global flags: verbosity for
  per-package progress and debugging, and forcing the CLI language (`auto`/`en`/`zh`)
- **Localized CLI** — the in-house `hok-i18n` runtime replaces `rust-i18n`: help text,
  diagnostics and messages all come from build-time locale tables, and `hok help`
  renders the same command list in either language
- **Manifest history index** — `hok install app@version` resolves a historical manifest
  from the bucket's git history (SQLite index, incrementally maintained; `use_git_history`)
- **Transaction confirmation gate** — `--confirm` (global) / `-y, --assume-yes` (the sync
  commands) / the `confirm` config / TTY detection, so a run that modifies the installation
  can be reviewed first
- **Resident PowerShell engine** — `script_engine` reuses one `pwsh` process for package
  scripts instead of spawning one per script
- **checkver** — full implementation with regex, JSONPath, XPath, PowerShell script,
  reverse/replace, GitHub and SourceForge shortcuts, autoupdate with hash recomputation
- **reinstall** — uninstall + same-version reinstall with held-state preservation
- **Native shim** — `hok-shim.exe` replaces `.cmd` wrappers (GUI detection, job objects)
- **Pure Rust shortcuts** — `.lnk` writer using `shortcuts-rs` crate, no COM FFI, args/icon support
- **SQLite manifest cache** — `use_sqlite_cache` config, compatible with Scoop's schema,
  updated incrementally from each bucket's git diff
- **Resumable fragmented downloads** — partial parts resume via HTTP Range, no restart
- **`hok update` improvements** — concurrent bucket pulls, visible manifest-cache refresh
- **Batch failure isolation** — `ignore_failures` config keeps multi-package operations
  running even if individual packages fail (applies to install/uninstall/update/upgrade/reinstall;
  `cleanup` is fail-soft unconditionally)
- **Fixed upstream bug**: `reset` now correctly runs `post_install` scripts
  (original Scoop bug — Scoop skips post_install on reset)
- **cleanup** — remove old versions of installed packages
- **depends / prefix / which / checkup / shim** — new CLI commands
- **`shim -g` anywhere** — `-g/--global` parses before or after the subcommand like
  upstream's getopt (`hok shim list -g` == `hok shim -g list`, listing global shims
  only) and `hok shim list *` lists every shim. Shim arguments starting with `-`
  must follow a `--`, so `-g`/`--global` meant for the shim goes after it. This is
  a breaking change: `hok shim add x cmd.exe -g` now creates a *global* shim
  (previously `-g` was passed to the shim and the shim stayed local)
- **export / import** — export/import installed package lists as JSON
- **alias** — list/add/remove Scoop aliases; each alias is a runnable
  `shims\scoop-<name>.ps1` script (`hok <name> [args]` runs it)
- **create** — generate manifest skeleton from download URL
- **virustotal** — VirusTotal API v3 integration

## Development

Prerequisites: Git, Rust. `libscoop` (the core library crate) is workspace-internal — this
fork does not publish it to crates.io; its version tracks `Cargo.toml`.

```sh
git clone https://github.com/maboloshi/hok
cd hok
cargo build
cargo run -- help
cargo bench   # guardrail micro-benchmarks over libscoop pure functions and the event bus
```

## Design

The reasoning — technology choices, dependency policy, the two SQLite build shapes, the async runtime,
the shim and i18n implementation, and the benchmark numbers — lives in [`DESIGN.md`](DESIGN.md)
([中文](DESIGN-zh.md)). The shipped structure is in [`ARCHITECTURE.md`](ARCHITECTURE.md), and the
per-release history in [`CHANGELOG.md`](CHANGELOG.md).

In short: pure-Rust-first with stated exceptions (`git2` over `gix`, `zstd` for `.zst`), one major
version per **direct** dependency, `unsafe_code = deny` outside the Win32 FFI modules, and criterion
plus hok-vs-Scoop benchmarks as regression guardrails.

## License

**hok** © [Chawye Hsu](https://github.com/chawyehsu) and contributors.
Released under the [Apache-2.0](LICENSE) license.
For licenses of sub crates, see [COPYING](COPYING).

[cicd-badge]: https://github.com/maboloshi/hok/actions/workflows/cicd.yml/badge.svg
[cicd]: https://github.com/maboloshi/hok/actions/workflows/cicd.yml
[release-badge]: https://img.shields.io/github/v/release/maboloshi/hok
[releases]: https://github.com/maboloshi/hok/releases/latest
[license-badge]: https://img.shields.io/github/license/maboloshi/hok
[downloads-badge]: https://img.shields.io/github/downloads/maboloshi/hok/total
