//! License data model and helpers.
//!
//! The [`License`] type, its accessors, SPDX soft-checking and `Display`
//! rendering, split out of [`super`] (`manifest.rs`).

use serde::Serialize;
use std::fmt;

use crate::constant::SPDX_LIST;

/// License information of a Scoop package.
#[derive(Clone, Debug, Serialize)]
pub struct License {
    /// The identifier of the license, which is intended to be a SPDX license.
    identifier: String,

    /// The url to the license text.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl License {
    /// Create a [`License`] representation.
    pub fn new(identifier: String, url: Option<String>) -> License {
        Self { identifier, url }
    }
    /// Return the identifier of this license.
    #[inline]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Check if this license is a valid SPDX identifier.
    #[inline]
    pub fn is_spdx(&self) -> bool {
        SPDX_LIST.contains(self.identifier())
    }

    /// Return the url to the license text of this license.
    #[inline]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

impl Default for License {
    /// An empty placeholder license, used when a manifest omits `license`.
    fn default() -> Self {
        License::new(String::new(), None)
    }
}

impl fmt::Display for License {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let url = self.url();

        if let Some(url) = url {
            write!(f, "{} ({})", self.identifier, url)
        } else if self.is_spdx() {
            write!(
                f,
                "{} (https://spdx.org/licenses/{}.html)",
                self.identifier, self.identifier
            )
        } else {
            write!(f, "{}", self.identifier)
        }
    }
}
