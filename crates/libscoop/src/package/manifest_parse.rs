//! Custom serde (de)serializers for manifest data types.
//!
//! Scoop manifests use flexible formats (a field can be a single string or
//! an array, a license can be a string or a map, etc.). The custom
//! `Deserialize` implementations here, split out of [`super`] (`manifest.rs`),
//! handle those formats.

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;
use std::marker::PhantomData;

use super::{Checkver, HashString, License, Sourceforge, Vectorized};

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
                let (project, path) = match s.split_once('/') {
                    Some((a, b)) => (Some(a.to_owned()), b.to_owned()),
                    None => (None, s.to_owned()),
                };
                Ok(Sourceforge { project, path })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut project = None;
                let mut path: Result<String, A::Error> = Err(de::Error::missing_field("path"));

                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    match key.as_str() {
                        "project" => project = Some(value),
                        "path" => path = Ok(value),
                        _ => {
                            // skip invalid fields
                            map.next_value::<serde_json::Value>()?;
                            continue;
                        }
                    }
                }

                Ok(Sourceforge {
                    project,
                    path: path?,
                })
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
