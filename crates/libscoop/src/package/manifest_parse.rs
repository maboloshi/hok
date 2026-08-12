//! Custom serde (de)serializers for manifest data types.
//!
//! Scoop manifests use flexible formats (a field can be a single string or
//! an array, a license can be a string or a map, etc.). The custom
//! `Deserialize` implementations here, split out of [`super`] (`manifest.rs`),
//! handle those formats.

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use super::{
    Checkver, HashExtraction, HashExtractionMode, HashString, License, Sourceforge, Vectorized,
};

////////////////////////////////////////////////////////////////////////////////
//  Custom (De)serializers
////////////////////////////////////////////////////////////////////////////////

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Vectorized<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VectorizedVisitor<T>(PhantomData<T>);
        impl<'de, T> Visitor<'de> for VectorizedVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = Vec<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("single item or array of items")
            }

            #[inline]
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                T::deserialize(serde_json::Value::String(s.to_owned()))
                    .map(|val| vec![val])
                    .map_err(|e| de::Error::custom(e))
            }

            fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
            where
                S: SeqAccess<'de>,
            {
                let mut ret = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(item) = seq.next_element()? {
                    ret.push(item)
                }
                Ok(ret)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut remap = serde_json::Map::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((k, v)) = map.next_entry()? {
                    remap.insert(k, v);
                }
                T::deserialize(serde_json::Value::Object(remap))
                    .map(|val| vec![val])
                    .map_err(de::Error::custom)
            }
        }

        Ok(Vectorized(
            deserializer.deserialize_any(VectorizedVisitor(PhantomData))?,
        ))
    }
}

impl<'de> Deserialize<'de> for HashExtraction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // `autoupdate.hash` may be a hash-extraction object, a plain string
        // (e.g. `"mode:json"` or an algorithm name like `"sha256"`), or an
        // array mixing both. Upstream's `HashHelper` accepts all three.
        // String forms carry no extraction fields here — the actual hash
        // computation runs on the raw JSON value in `checkver_hash`.
        struct HashExtractionVisitor;
        impl<'de> Visitor<'de> for HashExtractionVisitor {
            type Value = HashExtraction;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a hash extraction object or a string")
            }

            #[inline]
            fn visit_str<E>(self, _s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(HashExtraction {
                    find: None,
                    regex: None,
                    jsonpath: None,
                    xpath: None,
                    mode: None,
                    url: None,
                })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut find = None;
                let mut regex = None;
                let mut jsonpath = None;
                let mut xpath = None;
                let mut mode = None;
                let mut url = None;
                while let Some(key) = map.next_key::<String>()? {
                    let value: serde_json::Value = map.next_value()?;
                    // Non-string values (e.g. a number) must not be silently
                    // turned into empty strings — that would hide a malformed
                    // manifest. Fail the parse instead.
                    let expect_str = |field: &str| {
                        value.as_str().map(|s| s.to_owned()).ok_or_else(|| {
                            de::Error::custom(format!(
                                "checkver hash extraction '{field}' must be a string"
                            ))
                        })
                    };
                    match key.as_str() {
                        "find" => find = Some(expect_str("find")?),
                        "regex" => regex = Some(expect_str("regex")?),
                        "jp" | "jsonpath" => jsonpath = Some(expect_str("jsonpath")?),
                        "xpath" => xpath = Some(expect_str("xpath")?),
                        "mode" => {
                            mode = Some(
                                HashExtractionMode::deserialize(value)
                                    .map_err(de::Error::custom)?,
                            );
                        }
                        "url" => url = Some(expect_str("url")?),
                        _ => {}
                    }
                }
                Ok(HashExtraction {
                    find,
                    regex,
                    jsonpath,
                    xpath,
                    mode,
                    url,
                })
            }
        }

        deserializer.deserialize_any(HashExtractionVisitor)
    }
}

impl<'de> Deserialize<'de> for License {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LicenseVisitor;
        impl<'de> Visitor<'de> for LicenseVisitor {
            type Value = License;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a license string or a map with identifier field")
            }

            #[inline]
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                // Note: intentionally NOT validated against the SPDX list —
                // Scoop's schema allows non-SPDX identifiers such as
                // `Freeware`, `Proprietary`, `Public Domain` and `Shareware`.
                // See `License::is_spdx()` for soft checking.
                Ok(License::new(s.to_owned(), None))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut identifier: Result<String, A::Error> =
                    Err(de::Error::missing_field("identifier"));
                let mut url = None;

                // It is needed to explicitly specify types `<String, String>`
                // of the key and value for the `next_entry` method here,
                // otherwise the deserializer will complain about the invalid
                // type of the key, which is basically similar to:
                // https://github.com/influxdata/pbjson/issues/55
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    match key.as_str() {
                        "identifier" => identifier = Ok(value),
                        "url" => url = Some(value),
                        _ => {
                            // skip invalid fields
                            map.next_value::<serde_json::Value>()?;
                            continue;
                        }
                    }
                }

                Ok(License::new(identifier?, url))
            }
        }

        deserializer.deserialize_any(LicenseVisitor)
    }
}

impl<'de> Deserialize<'de> for Sourceforge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SourceforgeVisitor;
        impl<'de> Visitor<'de> for SourceforgeVisitor {
            type Value = Sourceforge;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a valid sourceforge check string or map with path field")
            }

            #[inline]
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                // Official checkver.ps1 regex `(?<project>[\w-]*)(/(?<path>.*))?`:
                // a string without '/' is the project, not the path.
                let (project, path) = match s.split_once('/') {
                    Some((a, b)) => (Some(a.to_owned()), Some(b.to_owned())),
                    None => (Some(s.to_owned()), None),
                };
                Ok(Sourceforge { project, path })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut project = None;
                let mut path = None;

                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    match key.as_str() {
                        "project" => project = Some(value),
                        "path" => path = Some(value),
                        _ => {
                            // skip invalid fields
                            map.next_value::<serde_json::Value>()?;
                            continue;
                        }
                    }
                }

                Ok(Sourceforge { project, path })
            }
        }

        deserializer.deserialize_any(SourceforgeVisitor)
    }
}

impl<'de> Deserialize<'de> for HashString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HashStringVisitor;
        impl<'de> Visitor<'de> for HashStringVisitor {
            type Value = HashString;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a valid hash string")
            }

            #[inline]
            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                HashString::new(s).map_err(|e| E::custom(e))
            }
        }

        deserializer.deserialize_any(HashStringVisitor)
    }
}

impl<'de> Deserialize<'de> for Checkver {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CheckverVisitor;
        impl<'de> Visitor<'de> for CheckverVisitor {
            type Value = Checkver;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a checkver string or a checkver map")
            }

            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let regex = match s {
                    "github" => Some("/releases/tag/(?:v|V)?([\\d.]+)".to_owned()),
                    _ => Some(s.to_owned()),
                };

                Ok(Checkver {
                    regex,
                    url: None,
                    jsonpath: None,
                    xpath: None,
                    reverse: None,
                    replace: None,
                    useragent: None,
                    script: None,
                    sourceforge: None,
                })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut regex = None;
                let mut url = None;
                let mut jsonpath = None;
                let mut xpath = None;
                let mut reverse = None;
                let mut replace = None;
                let mut useragent = None;
                let mut script = None;
                let mut sourceforge = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "github" => {
                            let prefix = map.next_value::<String>()?;
                            url = Some(format!("{}/releases/latest", prefix));
                            regex = Some("/releases/tag/(?:v|V)?([\\d.]+)".to_owned());
                        }
                        "re" | "regex" => regex = Some(map.next_value()?),
                        "url" => url = Some(map.next_value()?),
                        "jp" | "jsonpath" => jsonpath = Some(map.next_value()?),
                        "xpath" => xpath = Some(map.next_value()?),
                        "reverse" => reverse = Some(map.next_value()?),
                        "replace" => replace = Some(map.next_value()?),
                        "useragent" => useragent = Some(map.next_value()?),
                        "script" => script = Some(map.next_value()?),
                        "sourceforge" => sourceforge = Some(map.next_value()?),
                        _ => {
                            // skip invalid fields
                            map.next_value::<serde_json::Value>()?;
                            continue;
                        }
                    }
                }

                Ok(Checkver {
                    regex,
                    url,
                    jsonpath,
                    xpath,
                    reverse,
                    replace,
                    useragent,
                    script,
                    sourceforge,
                })
            }
        }

        deserializer.deserialize_any(CheckverVisitor)
    }
}

/// Deserialize the `bin` field tolerantly.
///
/// Accepts Scoop's standard forms — `"foo.exe"`, `["foo.exe"]`,
/// `[["foo.exe", "name", "arg"]]` — plus the object form
/// `{"file": "...", "name": "...", "args": [...]}` that some third-party
/// manifests use. Object entries are normalized to the `(file, name,
/// args...)` tuple the shim layer already understands; entries that cannot
/// be understood are dropped so a single bad `bin` item never invalidates
/// the whole manifest (upstream Scoop aborts at install time, hok
/// tolerates at parse time).
pub fn deserialize_bin<'de, D>(
    deserializer: D,
) -> Result<Option<Vectorized<Vectorized<String>>>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(Vectorized(vec![Vectorized(vec![s])]))),
        serde_json::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    serde_json::Value::String(s) => out.push(Vectorized(vec![s])),
                    serde_json::Value::Array(arr) => {
                        let def: Vec<String> = arr
                            .into_iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                        if !def.is_empty() {
                            out.push(Vectorized(def));
                        }
                    }
                    serde_json::Value::Object(map) => {
                        // Object form `{"file", "name", "args"}` — normalize
                        // to the (file, name, args...) tuple.
                        if let Some(file) = map.get("file").and_then(|v| v.as_str()) {
                            let mut def = vec![file.to_owned()];
                            if let Some(name) = map.get("name").and_then(|v| v.as_str()) {
                                def.push(name.to_owned());
                            }
                            if let Some(args) = map.get("args").and_then(|v| v.as_array()) {
                                def.extend(
                                    args.iter().filter_map(|v| v.as_str().map(String::from)),
                                );
                            }
                            out.push(Vectorized(def));
                        }
                        // No `file`: unparseable, drop the entry.
                    }
                    _ => {}
                }
            }
            Ok(Some(Vectorized(out)))
        }
        _ => Err(de::Error::custom(
            "bin must be a string, an array of strings, or an array of arrays",
        )),
    }
}

/// Deserialize a hook script field (`pre_install` / `post_install` /
/// `pre_uninstall` / `post_uninstall`) tolerantly.
///
/// Scoop's schema defines these as a string or an array of strings, but
/// some third-party manifests use an object form `{"script": [...]}`. The
/// official checkver parses manifests with `ConvertFrom-Json` and never
/// validates the schema, so such manifests still work there; hok
/// normalizes the object form to its `script` value instead of rejecting
/// the whole manifest at parse time (matching the `deserialize_bin`
/// tolerance for `bin` object entries).
pub fn deserialize_hook_script<'de, D>(
    deserializer: D,
) -> Result<Option<Vectorized<String>>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let scripts: Vec<String> = match value {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::String(s) => vec![s],
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| de::Error::custom("hook script array items must be strings"))
            })
            .collect::<Result<_, _>>()?,
        serde_json::Value::Object(map) => match map.get("script") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            Some(_) => {
                return Err(de::Error::custom(
                    "hook script object's `script` must be a string or an array of strings",
                ))
            }
            None => {
                return Err(de::Error::custom(
                    "hook script object must contain a `script` field",
                ))
            }
        },
        _ => return Err(de::Error::custom(
            "hook script must be a string, an array of strings, or an object with a `script` field",
        )),
    };
    Ok(Some(Vectorized(scripts)))
}

/// Deserialize an object map whose values are coerced to strings.
///
/// The official schema types `env_set` / `cookie` as plain objects with no
/// value-type restriction; hok stores them as `HashMap<String, String>`.
/// Non-string values (numbers, booleans) are stringified instead of
/// rejecting the whole manifest.
pub fn deserialize_string_map<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let map = HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let out = map
        .into_iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            (k, s)
        })
        .collect();
    Ok(Some(out))
}
