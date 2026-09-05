//! Installed-app source classification, shared by `status` and `list`.
//!
//! Axis 2 of the per-app state model: every installed app's relationship to
//! its *origin* bucket is collapsed into exactly one SourceState. This is
//! the single place that decides where an app came from / is still
//! resolvable, so `status` and `list` can never drift into two different
//! predicates (which is what happened before: `status` probed "no live
//! manifest" while `list` only checked "whole bucket directory gone").
//!
//! Cost model: the expensive part is scanning a bucket (to learn which
//! manifests are live vs. only under deprecated/), not classifying one app.
//! classify() walks each referenced bucket once - building a BucketIndex in
//! a single recursive pass - then answers every app of that bucket with two
//! O(1) set lookups. The previous code re-walked the bucket for every app.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::Session;
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

/// What one bucket currently contains, indexed once per call.
#[derive(Default)]
struct BucketIndex {
    /// App names (lower-cased) that have a live manifest outside `deprecated/`.
    live: HashSet<String>,
    /// App names (lower-cased) whose only manifest lives under `deprecated/`.
    deprecated: HashSet<String>,
}

impl BucketIndex {
    /// Recursively index a bucket directory, splitting live vs. deprecated.
    fn build(bucket_dir: &Path) -> BucketIndex {
        let mut idx = BucketIndex::default();
        walk(bucket_dir, false, &mut idx);
        idx
    }

    fn source(&self, app: &str) -> SourceState {
        let key = app.to_lowercase();
        if self.deprecated.contains(&key) {
            SourceState::Deprecated
        } else if self.live.contains(&key) {
            SourceState::Synced
        } else {
            SourceState::Orphaned
        }
    }
}

fn walk(dir: &Path, inside_deprecated: bool, idx: &mut BucketIndex) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let is_deprecated =
                inside_deprecated || path.file_name().is_some_and(|n| n == "deprecated");
            walk(&path, is_deprecated, idx);
        } else if path.extension().is_some_and(|e| e == "json") {
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
                continue;
            };
            if inside_deprecated {
                idx.deprecated.insert(stem);
            } else {
                idx.live.insert(stem);
            }
        }
    }
}

/// Classify every installed package; returns one SourceState per input
/// package (order preserved). Bucket-installed apps are grouped by their
/// origin bucket and each referenced bucket is walked at most once. Isolated
/// (URL/path) and broken-without-a-record apps never trigger a bucket walk.
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
        let idx = if bucket_dir.exists() {
            Some(BucketIndex::build(&bucket_dir))
        } else {
            None
        };
        for i in indexes {
            out[i] = match &idx {
                Some(idx) => idx.source(pkgs[i].name()),
                None => SourceState::BucketMissing,
            };
        }
    }

    out
}
