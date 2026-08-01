//! Runtime architecture detection.
//!
//! Mirrors Scoop's `Get-DefaultArchitecture` (lib/core.ps1), which determines
//! the **operating system** architecture at runtime instead of at compile
//! time:
//!
//! - `ProgramFiles(Arm)` env var present → `arm64` (Windows on ARM)
//! - `PROCESSOR_ARCHITEW6432` set → a 32-bit process on a 64-bit OS (WOW64)
//! - `PROCESSOR_ARCHITECTURE` → `AMD64` / `ARM64` / `x86`
//!
//! Unlike `cfg!(target_arch)`, this tracks the OS rather than the binary, so a
//! 32-bit build on a 64-bit OS (WOW64) or an emulated x64 build on Windows on
//! ARM selects the same manifest fields as the original Scoop.
//!
//! Two additional Scoop behaviours are mirrored here:
//!
//! - **Config override**: [`set_default_architecture()`] applies the
//!   `default_architecture` config (parsed by [`Arch::parse()`], equivalent
//!   to Scoop's `Format-ArchitectureString`) as a process-wide override on
//!   top of runtime detection.
//! - **ARM64 fallback**: [`Arch::supported()`] implements Scoop's
//!   `Get-SupportedArchitecture` — on ARM64 hosts whose manifest has no
//!   `arm64` field, Windows 11 (build ≥ 22000) falls back to `64bit`,
//!   Windows 10 to `32bit`.

use std::sync::OnceLock;

use crate::error::Fallible;

/// Process-wide override from the `default_architecture` config, if any.
static DEFAULT_ARCHITECTURE: OnceLock<Option<Arch>> = OnceLock::new();

/// A Scoop manifest architecture, named after the fields used in manifests
/// (`32bit`, `64bit`, `arm64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    Ia32,
    Amd64,
    Aarch64,
}

impl Arch {
    /// Return the current architecture: the `default_architecture` config
    /// override if set, otherwise the runtime-detected host architecture.
    pub fn current() -> Arch {
        if let Some(Some(arch)) = DEFAULT_ARCHITECTURE.get() {
            return *arch;
        }

        #[cfg(target_os = "windows")]
        {
            let arm_program_files = std::env::var_os("ProgramFiles(Arm)").is_some();
            let arch_w6432 = std::env::var("PROCESSOR_ARCHITEW6432").ok();
            let arch = std::env::var("PROCESSOR_ARCHITECTURE").ok();
            Arch::from_env(arm_program_files, arch_w6432.as_deref(), arch.as_deref())
        }
        #[cfg(not(target_os = "windows"))]
        {
            // Non-Windows fallback: the binary's own architecture.
            if cfg!(target_arch = "x86_64") {
                Arch::Amd64
            } else if cfg!(target_arch = "aarch64") {
                Arch::Aarch64
            } else {
                Arch::Ia32
            }
        }
    }

    /// Set the process-wide architecture override (from the
    /// `default_architecture` config). The first call wins; later calls are
    /// ignored, matching the single-session nature of the CLI.
    pub fn set_default_architecture(arch: Arch) {
        let _ = DEFAULT_ARCHITECTURE.set(Some(arch));
    }

    /// Parse an architecture string as accepted by Scoop's
    /// `Format-ArchitectureString` (used for the `default_architecture`
    /// config and CLI input).
    pub fn parse(value: &str) -> Fallible<Arch> {
        match value.to_ascii_lowercase().as_str() {
            "64bit" | "64" | "x64" | "amd64" | "x86_64" | "x86-64" => Ok(Arch::Amd64),
            "32bit" | "32" | "x86" | "i386" | "386" | "i686" => Ok(Arch::Ia32),
            "arm64" | "arm" | "aarch64" => Ok(Arch::Aarch64),
            _ => Err(crate::Error::Custom(format!(
                "invalid architecture: {value}"
            ))),
        }
    }

    /// Apply Scoop's `Get-SupportedArchitecture` fallback to the current
    /// architecture based on whether the manifest provides an `arm64` field.
    pub fn supported(current: Arch, manifest_has_arm64: bool) -> Arch {
        Arch::supported_with_build(current, manifest_has_arm64, windows_build_number())
    }

    /// Pure version of [`Arch::supported()`] with an explicit Windows build
    /// number, extracted for unit testing.
    pub fn supported_with_build(
        current: Arch,
        manifest_has_arm64: bool,
        windows_build: u32,
    ) -> Arch {
        if current == Arch::Aarch64 && !manifest_has_arm64 {
            // Windows 11 runs unmodified x64 apps on ARM (build ≥ 22000);
            // Windows 10 only x86 ones.
            if windows_build >= 22000 {
                Arch::Amd64
            } else {
                Arch::Ia32
            }
        } else {
            current
        }
    }

    /// The Scoop manifest field name for this architecture.
    pub fn name(self) -> &'static str {
        match self {
            Arch::Ia32 => "32bit",
            Arch::Amd64 => "64bit",
            Arch::Aarch64 => "arm64",
        }
    }

    /// Deterministic architecture resolution from raw environment inputs.
    ///
    /// Extracted for unit testing; [`current()`] is a thin wrapper over this.
    pub fn from_env(
        arm_program_files: bool,
        arch_w6432: Option<&str>,
        arch: Option<&str>,
    ) -> Arch {
        if arm_program_files {
            return Arch::Aarch64;
        }
        if arch_w6432.is_some() {
            return Arch::Amd64;
        }
        match arch {
            Some("AMD64") => Arch::Amd64,
            Some("ARM64") => Arch::Aarch64,
            _ => Arch::Ia32,
        }
    }
}

/// The Windows build number (e.g. 22631), read from the registry.
/// Returns 0 on non-Windows or when unreadable.
#[cfg(target_os = "windows")]
fn windows_build_number() -> u32 {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Non-Windows fallback: no ARM64 fallback decisions apply.
#[cfg(not(target_os = "windows"))]
fn windows_build_number() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_arm() {
        // ProgramFiles(Arm) wins even under WOW64
        assert_eq!(
            Arch::from_env(true, Some("AMD64"), Some("x86")),
            Arch::Aarch64
        );
    }

    #[test]
    fn test_from_env_wow64() {
        // 32-bit process on a 64-bit OS
        assert_eq!(
            Arch::from_env(false, Some("AMD64"), Some("x86")),
            Arch::Amd64
        );
    }

    #[test]
    fn test_from_env_amd64() {
        assert_eq!(Arch::from_env(false, None, Some("AMD64")), Arch::Amd64);
    }

    #[test]
    fn test_from_env_arm64() {
        assert_eq!(Arch::from_env(false, None, Some("ARM64")), Arch::Aarch64);
    }

    #[test]
    fn test_from_env_x86() {
        assert_eq!(Arch::from_env(false, None, Some("x86")), Arch::Ia32);
        assert_eq!(Arch::from_env(false, None, None), Arch::Ia32);
    }

    #[test]
    fn test_name() {
        assert_eq!(Arch::Ia32.name(), "32bit");
        assert_eq!(Arch::Amd64.name(), "64bit");
        assert_eq!(Arch::Aarch64.name(), "arm64");
    }

    #[test]
    fn test_parse_aliases() {
        assert_eq!(Arch::parse("64bit").unwrap(), Arch::Amd64);
        assert_eq!(Arch::parse("x64").unwrap(), Arch::Amd64);
        assert_eq!(Arch::parse("AMD64").unwrap(), Arch::Amd64);
        assert_eq!(Arch::parse("x86_64").unwrap(), Arch::Amd64);
        assert_eq!(Arch::parse("x86-64").unwrap(), Arch::Amd64);
        assert_eq!(Arch::parse("32bit").unwrap(), Arch::Ia32);
        assert_eq!(Arch::parse("x86").unwrap(), Arch::Ia32);
        assert_eq!(Arch::parse("i686").unwrap(), Arch::Ia32);
        assert_eq!(Arch::parse("arm64").unwrap(), Arch::Aarch64);
        assert_eq!(Arch::parse("aarch64").unwrap(), Arch::Aarch64);
        assert!(Arch::parse("mips").is_err(), "invalid architecture rejected");
    }

    #[test]
    fn test_supported_arm64_fallback() {
        // ARM64 host, manifest has arm64 → keep arm64 regardless of build
        assert_eq!(
            Arch::supported_with_build(Arch::Aarch64, true, 22000),
            Arch::Aarch64
        );
        assert_eq!(
            Arch::supported_with_build(Arch::Aarch64, true, 19045),
            Arch::Aarch64
        );
        // ARM64 host, no arm64 in manifest → Win11 (≥22000) → 64bit
        assert_eq!(
            Arch::supported_with_build(Arch::Aarch64, false, 22000),
            Arch::Amd64
        );
        assert_eq!(
            Arch::supported_with_build(Arch::Aarch64, false, 22631),
            Arch::Amd64
        );
        // ARM64 host, no arm64 in manifest → Win10 (<22000) → 32bit
        assert_eq!(
            Arch::supported_with_build(Arch::Aarch64, false, 19045),
            Arch::Ia32
        );
        // Non-ARM64 hosts are never downgraded
        assert_eq!(
            Arch::supported_with_build(Arch::Amd64, false, 19045),
            Arch::Amd64
        );
        assert_eq!(
            Arch::supported_with_build(Arch::Ia32, false, 22000),
            Arch::Ia32
        );
    }
}
