//! Base64 codec — the single implementation used across `libscoop`.
//!
//! Scoop decodes base64 with .NET's `Convert.FromBase64String`
//! (`lib/autoupdate.ps1:194` for the `Digest` header, `:87` for hash files)
//! and encodes GitHub commit payloads with plain standard base64.  The
//! `base64` crate's `STANDARD` engine is **not** equivalent to
//! `FromBase64String`, so the contract here was measured against both
//! PowerShell hosts (Windows PowerShell 5.1 / pwsh 7.6.5 — identical results)
//! instead of assumed:
//!
//! | input | `Convert.FromBase64String` |
//! |---|---|
//! | `YQ==` | `61` |
//! | `Y Q = =` | `61` — whitespace is ignored anywhere |
//! | `YQ==` + trailing newline | `61` |
//! | `YR==` | `61` — non-zero trailing bits are **accepted** |
//! | `""` | empty |
//! | `YQ` / `YWJjZA` | `FormatException` — padding is required |
//! | `YQ=8` / `YQ$=` / `YQ==extra` | `FormatException` |
//!
//! [`decode`] therefore strips ASCII whitespace, requires canonical padding
//! and tolerates trailing bits — it accepts exactly what `FromBase64String`
//! accepts.  Individual call sites keep their own failure policy (Scoop's
//! `find_hash_in_headers` lets the exception escape, `find_hash_in_textfile`
//! swallows it): this module only decodes, it never decides.

use std::fmt::Write;

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine};

use crate::error::{Error, Fallible};

/// Engine mirroring `Convert.FromBase64String`: canonical padding required,
/// non-zero trailing bits tolerated.  Whitespace is stripped by [`decode`]
/// before the input reaches the engine.
static NET_ENGINE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(true),
);

/// Encode bytes as canonical padded standard base64.
///
/// Used for GitHub commit payloads (`createCommitOnBranch`), matching
/// Scoop's `[Convert]::ToBase64String`.
pub fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode standard base64 with `Convert.FromBase64String` semantics
/// (see the module docs for the measured behaviour table).
pub fn decode(input: &str) -> Fallible<Vec<u8>> {
    let cleaned: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    NET_ENGINE
        .decode(cleaned.as_bytes())
        .map_err(|e| Error::Custom(format!("invalid base64 input: {e}")))
}

/// Render bytes as lowercase hex, matching PowerShell's
/// `ForEach-Object { $_.ToString('x2') }` (leading zeros preserved).
pub fn to_lower_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{:02x}", byte);
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values are the measured .NET table in the module docs.
    #[test]
    fn decode_matches_dotnet_behaviour() {
        let ok: &[(&str, &str)] = &[
            ("YQ==", "61"),
            ("Y Q = =", "61"),
            ("YQ==\n", "61"),
            ("Y Q==", "61"),
            ("YR==", "61"),
            ("", ""),
            ("YWJj", "616263"),
            ("YWJjZA==", "61626364"),
        ];
        for (input, expected) in ok {
            assert_eq!(
                to_lower_hex(&decode(input).unwrap_or_else(|e| panic!("{input:?}: {e}"))),
                *expected,
                "input {input:?}"
            );
        }

        let rejected = ["YQ", "YWJjZA", "YQ=8", "YQ$=", "YQ==extra", "YQ=A", "YQ=A="];
        for input in rejected {
            assert!(
                decode(input).is_err(),
                "expected {input:?} to be rejected (as .NET does)"
            );
        }
    }

    #[test]
    fn encode_is_canonical_padded() {
        assert_eq!(encode(b"a"), "YQ==");
        assert_eq!(encode(b"abcd"), "YWJjZA==");
        assert_eq!(encode(b""), "");
        assert_eq!(decode(&encode(&[0xab; 32])).unwrap(), vec![0xab; 32]);
    }

    #[test]
    fn to_lower_hex_keeps_leading_zeros() {
        assert_eq!(to_lower_hex(&[0x00, 0x0a, 0xff]), "000aff");
        assert_eq!(to_lower_hex(&[]), "");
    }
}
