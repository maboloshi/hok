//! Dependency tree traversal for a package.
//!
//! Recursively walks the dependencies of a package and flattens them into an
//! indented tree, deduplicating packages that appear more than once — the
//! business logic behind the `hok depends` command.

use std::collections::HashSet;

use crate::{error::Fallible, QueryOption, Session};

/// A node in the flattened dependency tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyNode {
    /// Package name.
    pub name: String,
    /// Bucket the package lives in.
    pub bucket: String,
    /// Tree depth (0 = the queried package itself).
    pub depth: usize,
    /// Whether this package was already listed at a shallower depth.
    pub already_listed: bool,
}

/// Recursively collect the dependency tree of a package.
///
/// The returned nodes are ordered depth-first; the root package is the first
/// node (depth 0). Packages without a bucket prefix in their dependency
/// entries inherit the parent's bucket.
///
/// # Errors
///
/// Returns an error if querying any package in the tree fails.
pub fn dependencies_tree(
    session: &Session,
    name: &str,
    bucket: &str,
) -> Fallible<Vec<DependencyNode>> {
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    collect_deps(session, name, bucket, 0, &mut seen, &mut nodes)?;
    Ok(nodes)
}

fn collect_deps(
    session: &Session,
    name: &str,
    bucket: &str,
    depth: usize,
    seen: &mut HashSet<String>,
    nodes: &mut Vec<DependencyNode>,
) -> Fallible<()> {
    let already_listed = !seen.insert(name.to_string());
    nodes.push(DependencyNode {
        name: name.to_string(),
        bucket: bucket.to_string(),
        depth,
        already_listed,
    });
    if already_listed {
        return Ok(());
    }

    let query = format!("{bucket}/{name}");
    let pkgs = super::query::query(
        session,
        vec![query.as_str()],
        vec![QueryOption::Explicit],
        false,
    )?;
    let deps = pkgs.first().map(|p| p.dependencies()).unwrap_or_default();

    for dep in &deps {
        let (dep_bucket, dep_name) = dep
            .split_once('/')
            .map(|(b, n)| (b.to_string(), n.to_string()))
            .unwrap_or_else(|| (bucket.to_string(), dep.clone()));
        collect_deps(session, &dep_name, &dep_bucket, depth + 1, seen, nodes)?;
    }

    Ok(())
}
