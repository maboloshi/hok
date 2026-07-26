# hok-shim — Native shim launcher for hok

A zero-dependency Windows executable that reads Scoop `.shim` metadata files and launches the target program.

## What it does

When hok installs a package with executables (`.exe`), it:

1. Writes `shims/{name}.exe` with this shim binary
2. Writes `shims/{name}.shim` with target info

When the user runs `{name}` from the command line, the shim reads the `.shim` file, resolves the path, and launches the real executable.

## Shim File Format

`.shim` files are plain text (UTF-8, CRLF or LF), one field per line with ` = ` separator. Lines starting with `#`, `;`, or `//`, as well as blank lines, are ignored as comments.

### Syntax

```text
path = <path to executable>
args = <arguments>
cwd = <working directory>
elevate = true|false|1|0|yes|no
NAME = <environment variable override>
```

### Fields

| Field | Required | Aliases | Description |
|-------|----------|---------|-------------|
| `path` | ✅ | | Target executable path. Supports `~\\..\\` relative to shim dir, or absolute |
| `args` | | | Arguments passed to the target |
| `cwd` | | `workdir` | Working directory for the target process |
| `elevate` | | `runas` | Request UAC elevation. Valid values: `true`, `1`, `yes` |
| Any other name | | | Environment variable set for the child process (key is case-insensitive) |

### Value Quoting

Values may be wrapped in double quotes (e.g. `path = "C:\Program Files\app.exe"`) or left unquoted.

### Variable Expansion

- **`%ENV%`** — Expands environment variables in `path`, `args`, `cwd`, and environment override values. Unknown variables (e.g. `%NONEXISTENT_VAR%`) are preserved as-is via `ExpandEnvironmentStringsW`.
- **`%~dp0`** — Expands to the **directory containing the target executable** with a trailing backslash. Applies to `args` and `cwd` only (not `path`).

### Argument Passing

User-provided runtime arguments are appended after those defined in `args`. For example, with `args = --verbose` in `.shim` and running `app.exe --output file`, the target receives `--verbose --output file`.

### Environment Variables

Any line whose key is not `path`, `args`, `cwd`, `workdir`, `elevate`, or `runas` is treated as an environment variable override for the child process. Keys are case-insensitive (handled via `SetEnvironmentVariableW`).

### Comments

| Prefix | Example |
|--------|---------|
| `#` | `# this is a comment` |
| `;` | `; also a comment` |
| `//` | `// also a comment` |

### Exit Codes

The shim waits for the child process to finish and forwards its exit code. If the shim fails internally, it exits with code 1.

## Features

- **GUI detection** — Hides console for GUI apps (`FreeConsole`)
- **Console attach** — Attaches to parent console for console apps (`AttachConsole`), avoiding costly console allocation (+416ms → +29ms overhead)
  - **Elevation** — Auto-relaunch via ``ShellExecuteExW`` ``runas`` when UAC is required; waits for the child and forwards its exit code
- **Job object** — `KILL_ON_JOB_CLOSE` ensures child process is cleaned up
- **Ctrl+C forwarding** — Ignores Ctrl+C in the shim so the child receives it
- **`.shim` parsing** — Parses `path`, `args`, `cwd`, `elevate`, and environment variable overrides
- **`~\\..\\` resolution** — Resolves relative paths against the shim's location

## Performance

Benchmarked with `C:\Windows\System32\whoami.exe` (console) via Python `subprocess.run` — 10 warmup + 30 measured runs. Architecture: x64. All shims use the same `.shim` file (`path = C:\Windows\System32\whoami.exe`).

| Implementation | Mean | Extra vs Direct | Size |
|---------------|------|----------------:|-----:|
| Direct (no shim) | 29.7 ms | — | — |
| **hok-shim (no_std)** | **51.7 ms** | **+22.0 ms** | **10 KB** 🥇 |
| Rust (upstream) | 54.9 ms | +24.7 ms | 121 KB |
| Zig (upstream) | 53.9 ms | +23.7 ms | 71 KB |
| C++ (upstream) | 56.9 ms | +26.6 ms | 158 KB |
| C# (upstream) | 107.3 ms | +77.0 ms | 14 KB |

hok-shim has the **smallest binary** (10 KB, 7–16× smaller than other native shims) and **competitive speed** (+22 ms overhead vs +24–27 ms for Rust/Zig/C++). All native shims perform similarly — the bottleneck is CreateProcessW, not shim logic.

Key optimization: `AttachConsole` for console targets avoids Windows allocating a new console (~400ms penalty) when a GUI-subsystem shim starts a console child.

For GUI targets (e.g. notepad), shim overhead is near-zero since no console allocation is needed. Direct measurement of GUI launch time is unreliable (dominated by GUI framework initialization), but all native shims show <10ms `Popen` overhead vs 60ms+ for direct AppX resolution.

## Benchmarking

To reproduce benchmark results:

```powershell
# Build hok-shim
cargo build -p hok-shim --release

# Run benchmark (requires Python)
python scripts/benchmark_shims.py

# For upstream comparison, download shims from:
# https://github.com/ScoopInstaller/Shim/releases
# and place in shim_test/{name}/shim.exe

## Specification Compliance

| Requirement | Status | Notes |
|-------------|--------|-------|
| `path` | ✅ | Supports `~\\..\\` and absolute paths |
| `args` | ✅ | |
| `cwd` / `workdir` | ✅ | Both field names accepted |
| `elevate` / `runas` | ✅ | Both field names accepted |
| Environment variable overrides | ✅ | Unknown fields set as env vars |
| Comments `#` `;` `//` | ✅ | |
| Value quoting `"..."` | ✅ | |
| `%ENV%` expansion | ✅ | Via `ExpandEnvironmentStringsW` |
| `%~dp0` expansion | ✅ | In `args` and `cwd` only; all occurrences replaced |
| User argument forwarding | ✅ | Appended after `args` from `.shim` |
| Exit code forwarding | ✅ | |
| UTF-8 BOM | ✅ | Automatically stripped |
| Case-insensitive keys | ✅ | |
| `~\\..\\` path resolution | ✅ | Resolved relative to shim directory |
| GUI subsystem detection | ✅ | Via PE header parsing (no `shell32`) |

## Implementation

- Language: Rust (`#![no_std]`, `#![no_main]`)
- Dependencies: **zero** — all raw Win32 FFI via `extern "system"`
- Memory: fully stack-allocated (no heap, no `alloc`)
- Binary size: **10 KB** release build (fully spec-compliant)
- Imported DLLs: `kernel32.dll` + `shell32.dll` (elevation only)
- Ctrl+C forwarding via `SetConsoleCtrlHandler(ignore_ctrl_c, TRUE)` — real handler function returns TRUE, letting child process receive the signal via console inheritance

## Build

```powershell
cargo build -p hok-shim --release
```

The binary is at `target/release/hok-shim.exe`.

## Embedding

Since hok 0.2.0, `hok-shim` is automatically built and **embedded into `hok.exe`** at compile time via `libscoop/build.rs`. No separate build step needed for normal usage.
