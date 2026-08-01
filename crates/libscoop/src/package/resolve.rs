//! Dependency resolution for package operations.
//!
//! Uses the DAG from [`internal::dag`] to resolve package dependencies
//! into a deterministic, dependency-first execution order.
//!
//! # Design
//!
//! - **Lazy full scan**: Most packages have no dependencies, so a costly
//!   full-bucket scan is only triggered when at least one package declares
//!   dependencies. The scan queries all synced packages at once for
//!   efficient subsequent lookups.
//! - **Unique ordering**: The output is guaranteed to have no duplicates
//!   and is sorted so that dependencies appear before their dependents.
//! - **Cycle detection**: The [`DepGraph`] detects cyclic dependencies
//!   and returns a [`CyclicError`] if any are found.

use crate::{
    error::Fallible,
    event,
    internal::dag::DepGraph,
    package::{query, Package},
    Error, Session,
};

/// Resolve dependencies of the given packages.
///
/// # Note
///
/// This function ensures that packages are unique and sorted in dependency first
/// order.
pub(crate) fn resolve_dependencies(session: &Session, packages: &mut Vec<Package>) -> Fallible<()> {
    let mut graph = DepGraph::<String>::new();
    let mut to_resolve = packages.clone();

    // Only query all packages when there are actual dependencies to resolve.
    // Most packages have no deps, so this avoids a costly full scan.
    let has_deps = packages.iter().any(|p| !p.dependencies().is_empty());
    let synced = if has_deps {
        // For performance reason, a wildcard query is done here to get all the
        // available packages in one shot and then used for the following queries.
        query::query_synced(session, &["*"], &[])?
    } else {
        Vec::new()
    };

    loop {
        if to_resolve.is_empty() {
            break;
        }

        let mut tmp = vec![];
        tmp.append(&mut to_resolve);

        for pkg in tmp.into_iter() {
            let mut resolved = vec![];
            let deps = pkg.dependencies();

            if deps.is_empty() {
                graph.register_node(pkg.name().to_owned());
            } else {
                let queries = deps.iter().map(|d| d.as_str());

                for query in queries {
                    let mut matched = synced
                        .iter()
                        .filter(|p| {
                            let (query_bucket, query_name) =
                                query.split_once('/').unwrap_or(("", query));
                            let bucket_matched =
                                query_bucket.is_empty() || p.bucket() == query_bucket;
                            let name_matched = p.name() == query_name;
                            bucket_matched && name_matched
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    match matched.len() {
                        0 => return Err(Error::PackageNotFound(query.to_owned())),
                        1 => {
                            let p = matched.pop().unwrap();
                            if !(resolved.contains(&p)
                                || to_resolve.contains(&p)
                                || packages.contains(&p))
                            {
                                resolved.push(p);
                            }
                        }
                        _ => {
                            let (installed_candidate, mut matched) = matched
                                .into_iter()
                                .partition::<Vec<_>, _>(|p| p.is_strictly_installed());

                            // There are multiple candidates for the dependency
                            // package, we need to select one from them. If a
                            // candidate is installed, it will be selected
                            // preferentially as the dependency package.
                            if !installed_candidate.is_empty() {
                                matched = installed_candidate;
                            } else {
                                select_candidate(session, &mut matched)?;
                            }

                            let p = matched.pop().unwrap();
                            if !(resolved.contains(&p)
                                || to_resolve.contains(&p)
                                || packages.contains(&p))
                            {
                                resolved.push(p);
                            }
                        }
                    }
                }

                let dep_nodes = resolved
                    .iter()
                    .map(|p: &Package| p.name().to_owned())
                    .collect::<Vec<_>>();
                graph.register_deps(pkg.name().to_owned(), dep_nodes);
            }
            // Cyclic dependency check
            graph.check()?;

            to_resolve.append(&mut resolved);
        }

        packages.extend(to_resolve.clone());
    }

    // dependencies need to be installed before dependents
    packages.reverse();

    Ok(())
}

/// Select one from multiple package candidates, interactively if possible.
pub(crate) fn select_candidate(session: &Session, candidates: &mut Vec<Package>) -> Fallible<()> {
    let name = candidates[0].name().to_owned();

    // Sort candidates by package ident, in other words, by alphabetical order
    // of bucket name.
    candidates.sort_by_key(|p| p.ident().to_lowercase());

    // Only we can ask user/frontend to select one from multiple candidates
    // when the outbound tx is available for us to do an interactive q&a.
    if let Some(tx) = session.emitter() {
        let question = candidates.iter().map(|p| p.ident()).collect::<Vec<_>>();

        if tx
            .send(event::Event::PromptPackageCandidate(question))
            .is_ok()
        {
            // The unwrap is safe here because we have obtained the outbound tx,
            // so the inbound rx must be available.
            let rx = session.receiver().unwrap();

            while let Ok(answer) = rx.recv() {
                if let event::Event::PromptPackageCandidateResult(idx) = answer {
                    // bounds check
                    if idx < candidates.len() {
                        *candidates = vec![candidates[idx].clone()];

                        return Ok(());
                    }

                    return Err(Error::InvalidAnswer);
                }
            }
        }
    }

    // TODO: handle this case smartly using pre-defined bucket priority
    Err(Error::PackageMultipleCandidates(name))
}

/// Resolve unneeded dependencies of the given packages.
///
/// # Note
///
/// This function is used to resolve the unneeded dependencies of the given
/// packages. The unneeded dependencies are the dependencies that are not
/// depended by other installed packages.
///
/// The purpose is to support cascading removal of installed packages.
pub(crate) fn resolve_cascade(
    session: &Session,
    packages: &mut Vec<Package>,
    escape_hold: bool,
) -> Fallible<()> {
    let mut to_resolve = packages.clone();

    // For performance reason, a wildcard query is done here to get all the
    // installed packages in one shot and then used for the following queries.
    let installed = query::query_installed(session, &["*"], &[])?;

    loop {
        if to_resolve.is_empty() {
            break;
        }

        let tmp = to_resolve.clone();
        to_resolve = vec![];

        for pkg in tmp.into_iter() {
            // unneeded: the packages that are not depended by other installed
            // packages.
            let mut unneeded = vec![];

            let dep_names = pkg
                .dependencies()
                .into_iter()
                .map(super::extract_name)
                .collect::<Vec<_>>();

            for dep_name in dep_names {
                let mut result = installed
                    .iter()
                    .filter(|p| p.name() == dep_name)
                    .collect::<Vec<_>>();

                // The package dependency system of Scoop is not mandatory,
                // the dependency relationship is loose. For the original
                // Scoop implementation, it is allowed that a dependency may
                // be removed separately without checking its dependents.
                // This can cause the empty result of the query.
                if result.is_empty() {
                    continue;
                }

                // We queried the installed packages, it is impossible to
                // have more than one result here for an explicit package
                // name.
                assert_eq!(result.len(), 1);

                let dep_pkg = result.pop().unwrap();
                // The dependency package may be depended by other installed
                // packages.
                let mut dependents = vec![];
                installed.iter().for_each(|p| {
                    let be_dependent = p
                        .dependencies()
                        .iter()
                        .map(super::extract_name)
                        .any(|d| d == dep_pkg.name());
                    if be_dependent {
                        dependents.push(p.clone());
                    }
                });

                // `pkg` is already the package to be removed, not counted.
                dependents.retain(|p| p.name() != pkg.name());

                let needed = dependents
                    .iter()
                    .any(|p| !packages.contains(p) && !unneeded.contains(p));

                if !needed {
                    if dep_pkg.is_held() && !escape_hold {
                        return Err(Error::PackageCascadeRemoveHold(dep_pkg.name().to_owned()));
                    }
                    unneeded.push(dep_pkg.to_owned());
                }
            }

            unneeded.dedup();
            to_resolve.append(&mut unneeded);
        }

        packages.extend(to_resolve.clone());
    }

    packages.dedup();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const BASE_MANIFEST: &str =
        r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#;

    /// Create a temp root with a session rooted at it, plus a drop guard.
    fn setup(test_name: &str) -> (Session, PathBufGuard) {
        let root = crate::test_utils::tmpdir(&format!("resolve_{}", test_name));
        let session = crate::test_utils::test_session(&root);
        (session, PathBufGuard(root))
    }

    struct PathBufGuard(std::path::PathBuf);
    impl Drop for PathBufGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn pkg(bucket: &str, name: &str, depends: &[&str]) -> Package {
        let deps = if depends.is_empty() {
            String::new()
        } else {
            format!(
                r#", "depends": [{}]"#,
                depends
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let json = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"{}}}"#,
            deps
        );
        let manifest = crate::package::Manifest::from_json(name, &json).unwrap();
        Package::from(name, bucket, manifest)
    }

    /// Helper to write a bucket manifest with optional `depends`.
    fn write_manifest(root: &Path, bucket: &str, name: &str, depends: &[&str]) {
        let deps = if depends.is_empty() {
            String::new()
        } else {
            format!(
                r#", "depends": [{}]"#,
                depends
                    .iter()
                    .map(|d| format!("\"{}\"", d))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let json = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"{}}}"#,
            deps
        );
        crate::test_utils::write_bucket_manifest(root, bucket, name, &json);
    }

    // ── resolve_dependencies ──────────────────────────────────────────────────

    #[test]
    fn resolve_orders_dependencies_first() {
        let (session, root) = setup("order");
        write_manifest(&root.0, "main", "a", &["main/b"]);
        write_manifest(&root.0, "main", "b", &[]);

        let mut packages = vec![pkg("main", "a", &["main/b"])];
        resolve_dependencies(&session, &mut packages).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["b", "a"],
            "dependency must come before dependent"
        );
    }

    #[test]
    fn resolve_no_deps_is_identity() {
        let (session, _root) = setup("no_deps");

        let mut packages = vec![pkg("main", "solo", &[])];
        resolve_dependencies(&session, &mut packages).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["solo"]);
    }

    #[test]
    fn resolve_missing_dependency_errors() {
        let (session, root) = setup("missing_dep");
        write_manifest(&root.0, "main", "a", &["ghost"]);

        let mut packages = vec![pkg("main", "a", &["ghost"])];
        let err = resolve_dependencies(&session, &mut packages).unwrap_err();

        assert!(matches!(err, Error::PackageNotFound(name) if name == "ghost"));
    }

    #[test]
    fn resolve_multilevel_dependencies_expanded() {
        let (session, root) = setup("multilevel");
        write_manifest(&root.0, "main", "a", &["main/b"]);
        write_manifest(&root.0, "main", "b", &["main/c"]);
        write_manifest(&root.0, "main", "c", &[]);

        let mut packages = vec![pkg("main", "a", &["main/b"])];
        resolve_dependencies(&session, &mut packages).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["c", "b", "a"],
            "transitive deps expanded, deps first"
        );
    }

    #[test]
    fn resolve_duplicate_inputs_deduped() {
        let (session, root) = setup("dedup");
        write_manifest(&root.0, "main", "a", &["main/b"]);
        write_manifest(&root.0, "main", "b", &[]);

        let mut packages = vec![pkg("main", "a", &["main/b"]), pkg("main", "b", &[])];
        resolve_dependencies(&session, &mut packages).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["b", "a"], "no duplicates, deps first");
    }

    #[test]
    fn resolve_multiple_candidates_prefers_installed() {
        let (session, root) = setup("multi_installed");
        // Same package name in two buckets
        write_manifest(&root.0, "main", "a", &[]);
        write_manifest(&root.0, "alt", "a", &[]);
        // alt/a is the installed one
        crate::test_utils::mark_installed(&root.0, "a", "alt", BASE_MANIFEST, false);
        write_manifest(&root.0, "main", "b", &["a"]);

        let mut packages = vec![pkg("main", "b", &["a"])];
        resolve_dependencies(&session, &mut packages).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        let buckets = packages.iter().map(|p| p.bucket()).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(buckets, vec!["alt", "main"], "installed candidate wins");
    }

    #[test]
    fn resolve_multiple_candidates_without_emitter_errors() {
        let (session, root) = setup("multi_no_emitter");
        write_manifest(&root.0, "main", "a", &[]);
        write_manifest(&root.0, "alt", "a", &[]);
        write_manifest(&root.0, "main", "b", &["a"]);

        let mut packages = vec![pkg("main", "b", &["a"])];
        let err = resolve_dependencies(&session, &mut packages).unwrap_err();

        assert!(matches!(err, Error::PackageMultipleCandidates(name) if name == "a"));
    }

    // ── resolve_cascade ───────────────────────────────────────────────────────

    #[test]
    fn cascade_appends_unneeded_dependency() {
        let (session, root) = setup("cascade_unneeded");
        crate::test_utils::mark_installed(&root.0, "a", "main", BASE_MANIFEST, false);
        crate::test_utils::mark_installed(&root.0, "b", "main", BASE_MANIFEST, false);

        // a depends on b, and nothing else depends on b → b becomes unneeded
        let mut packages = vec![pkg("main", "a", &["b"])];
        resolve_cascade(&session, &mut packages, false).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn cascade_keeps_dependency_needed_by_others() {
        let (session, root) = setup("cascade_needed");
        crate::test_utils::mark_installed(&root.0, "a", "main", BASE_MANIFEST, false);
        // c depends on b (read from its installed manifest.json)
        let c_manifest = r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT", "depends": ["b"]}"#;
        crate::test_utils::mark_installed(&root.0, "c", "main", c_manifest, false);
        crate::test_utils::mark_installed(&root.0, "b", "main", BASE_MANIFEST, false);

        // a depends on b, but c also depends on b → b stays
        let mut packages = vec![pkg("main", "a", &["b"])];
        resolve_cascade(&session, &mut packages, false).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["a"], "b is still needed by c");
    }

    #[test]
    fn cascade_held_dependency_blocks_without_escape() {
        let (session, root) = setup("cascade_held");
        crate::test_utils::mark_installed(&root.0, "a", "main", BASE_MANIFEST, false);
        crate::test_utils::mark_installed(&root.0, "b", "main", BASE_MANIFEST, true);

        let mut packages = vec![pkg("main", "a", &["b"])];
        let err = resolve_cascade(&session, &mut packages, false).unwrap_err();

        assert!(matches!(err, Error::PackageCascadeRemoveHold(name) if name == "b"));
    }

    #[test]
    fn cascade_held_dependency_removed_when_escaping() {
        let (session, root) = setup("cascade_escape");
        crate::test_utils::mark_installed(&root.0, "a", "main", BASE_MANIFEST, false);
        crate::test_utils::mark_installed(&root.0, "b", "main", BASE_MANIFEST, true);

        let mut packages = vec![pkg("main", "a", &["b"])];
        resolve_cascade(&session, &mut packages, true).unwrap();

        let names = packages.iter().map(|p| p.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "b"]);
    }
}
