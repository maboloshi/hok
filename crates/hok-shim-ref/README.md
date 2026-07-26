# hok-shim-ref — Reference copy of the std-based hok-shim

This directory preserves the original `std::process::Command`-based implementation of hok-shim before it was rewritten in `#![no_std]`.

| Version | Approach | Binary Size |
|---------|----------|-------------|
| `crates/hok-shim` (current) | `#![no_std]`, raw `CreateProcessW`, fixed buffers | **10.5 KB** |
| `crates/hok-shim-ref` (this) | `std::process::Command`, raw FFI for GUI/Job/elevation | **310 KB** |

Kept for reference. The active implementation is at `crates/hok-shim/`.
