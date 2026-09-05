//! Installed-app source classification, shared by `status` and `list`.
//!
//! Axis 2 of the per-app state model: every installed app's relationship to
//! its *origin* bucket is collapsed into exactly one SourceState. This is
//! the single place that decides where an app came from / is still
//! resolvable, so `status` and `list` can never drift into two different
//! predicates (which is what happened before: `status` probed "no live
//! manifest" while `list` only checked "whole bucket directory gone").
//!
//! Cost model: classifying never scans a whole bucket (that would read the
//! bucket's entire tree - including its `.git/` directory - on every
//! `list`/`status`). Instead each app is decided with a bounded handful of
//! `exists()` probes against the canonical manifest locations that Scoop
//! actually uses (the same [`Bucket::path_of_manifest`] lookup that loads the
//! origin manifest for upgradable checks, plus the known `deprecated`
//! folders), so per-bucket cost is O(apps) regardless of bucket size.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Session;
use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::package::Package;

/// Axis-2 classification of an installed app's source.
///
/// Mutually exclusive - an app is exactly one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    /// The origin bucket exists and carries a live manifest for the app.
    Synced,
    /// The origin bucket carries the manifest only under a `deprecated/` folder.
    Deprecated,
    /// The origin bucket exists but its live manifest for the app was deleted.
    Orphaned,
    /// The whole origin bucket directory no longer exists.
    BucketMissing,
    /// Installed from a URL / local path; there is no origin bucket to check.
    Isolated,
    /// Broken/partial install with no source record at all (neither a bucket
    /// nor a URL). Not "removed" - we simply do not know where it came from.
    UnknownBroken,
}

/// The `deprecated` folders Scoop keeps deprecated manifests in. A bucket
/// may hold them directly under `<bucket>/deprecated` or, in the categorized
/// layout, under `<bucket>/bucket/deprecated`. Only existing folders are
/// returned, so per-app probing stays a couple of cheap `exists()` calls.
fn deprecated_roots(bucket_dir: &Path) -> Vec<PathBuf> {
    let candidates = [
        bucket_dir.join("deprecated"),
        bucket_dir.join("bucket").join("deprecated"),
    ];
    candidates
        .into_iter()
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>()
}

/// Classify every installed package; returns one SourceState per input
/// package (order preserved). Bucket-installed apps are grouped by their
/// origin bucket and each bucket is resolved with cheap per-app `exists()`
/// probes - never a recursive scan of the whole bucket. Isolated
/// (URL/path) and broken-without-a-record apps never touch a bucket.
pub fn classify(session: &Session, pkgs: &[Package]) -> Vec<SourceState> {
    let mut out = vec![SourceState::Synced; pkgs.len()];

    // Group bucket-installed package indexes by their origin bucket.
    let mut by_bucket: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, p) in pkgs.iter().enumerate() {
        if p.is_install_failed() && p.installed_bucket().is_none() {
            // Broken with no source record at all.
            out[i] = SourceState::UnknownBroken;
        } else if p.bucket() == ISOLATED_PACKAGE_BUCKET {
            // URL / local-path install.
            out[i] = SourceState::Isolated;
        } else if let Some(bucket) = p.installed_bucket() {
            by_bucket.entry(bucket).or_default().push(i);
        }
        // else: defensive - keep Synced as the neutral default.
    }

    for (bucket, indexes) in by_bucket {
        let bucket_dir = session.bucket_dir(bucket);
        if !bucket_dir.is_dir() {
            for i in indexes {
                out[i] = SourceState::BucketMissing;
            }
            continue;
        }

        let deprecated_roots = deprecated_roots(&bucket_dir);
        // Bucket::from only fails if the directory vanished between the
        // is_dir() check and now - treat that as the bucket being missing.
        let Ok(bucket_obj) = Bucket::from(&bucket_dir) else {
            for i in indexes {
                out[i] = SourceState::BucketMissing;
            }
            continue;
        };

        for i in indexes {
            // Manifest filenames are conventionally lower-cased; match how
            // the bucket index keys were previously normalised.
            let name = pkgs[i].name().to_lowercase();
            let deprecated = deprecated_roots
                .iter()
                .any(|dir| dir.join(format!("{name}.json")).exists());
            if deprecated {
                out[i] = SourceState::Deprecated;
                continue;
            }
            out[i] = if bucket_obj.path_of_manifest(&name).is_some() {
                SourceState::Synced
            } else {
                SourceState::Orphaned
            };
        }
    }

    out
}
