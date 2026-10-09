//! Git operations.
//!
//! Currently uses `git2` (libgit2 bindings). A pure-Rust `gix` version was
//! attempted but abandoned because gix's dependency tree is prohibitively
//! large (aws-lc-rs, quinn-proto, h2, icu — causing 20+ min first build).

use git2::{CredentialType, FetchOptions, Repository};
use regex::Regex;
use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    path::Path,
    rc::Rc,
    result::Result,
    sync::LazyLock,
};

use crate::error::{Error, Fallible};

/// The credential callback every remote operation shares: an explicit
/// username when one is requested, the configured credential helper for
/// user/password, otherwise libgit2's default resolution.
fn remote_callbacks() -> git2::RemoteCallbacks<'static> {
    let mut cb = git2::RemoteCallbacks::new();

    cb.credentials(
        move |url, username, cred| -> Result<git2::Cred, git2::Error> {
            let user = username.unwrap_or("git");
            let cfg = &(git2::Config::open_default()?);

            if cred.contains(CredentialType::USERNAME) {
                git2::Cred::username(user)
            } else if cred.contains(CredentialType::USER_PASS_PLAINTEXT) {
                git2::Cred::credential_helper(cfg, url, username)
            } else if cred.contains(CredentialType::DEFAULT) {
                git2::Cred::default()
            } else {
                Err(git2::Error::from_str("no authentication available"))
            }
        },
    );

    cb
}

/// The proxy settings for a configured `proxy` value, if any.
fn proxy_options(proxy: Option<&str>) -> Option<git2::ProxyOptions<'static>> {
    let mut url = proxy?.to_owned();

    if !(url.starts_with("http") || url.starts_with("socks")) {
        url.insert_str(0, "http://");
    }

    let mut po = git2::ProxyOptions::new();
    po.url(url.as_str());
    Some(po)
}

fn fetch_options(proxy: Option<&str>) -> FetchOptions<'static> {
    let mut fo = FetchOptions::new();
    fo.remote_callbacks(remote_callbacks());

    if let Some(po) = proxy_options(proxy) {
        fo.proxy_options(po);
    }

    fo
}

/// Normalize a git remote URL the way upstream's `Convert-RepositoryUri`
/// does (`lib/buckets.ps1:83-102`).
///
/// Every clone URL form git accepts — `https://host/user/repo(.git)`,
/// `git@host:user/repo.git`, `ssh://user@host[:port]/user/repo`, with
/// credentials and/or a port — collapses to `provider/user/repo`, so two
/// spellings of the same repository compare equal. A `None` return means
/// upstream would reject the string as "not a valid Git URL!".
///
/// This is the single entry point for both the add-time validation and the
/// duplicate-repository scan (`lib/buckets.ps1:135-147`), so the two can
/// never disagree about what a repository is.
pub fn normalize_git_uri(uri: &str) -> Option<String> {
    // The pattern is upstream's verbatim. PowerShell's `-match` is
    // case-insensitive, hence `(?i)`; the crate's leftmost-first search and
    // backtracking otherwise agree with .NET on every form in the test table.
    static URI: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)(?:@|/{1,3})(?:www\.|.*@)?(?P<provider>[^/]+?)(?::\d+)?[:/](?P<user>.+)/(?P<repo>.+?)(?:\.git)?/?$",
        )
        .expect("the upstream Convert-RepositoryUri pattern compiles")
    });

    let caps = URI.captures(uri)?;
    Some(format!(
        "{}/{}/{}",
        &caps["provider"], &caps["user"], &caps["repo"]
    ))
}

pub fn clone_repo<S, P>(remote_url: S, path: P, proxy: Option<S>) -> Fallible<()>
where
    S: AsRef<str>,
    P: AsRef<Path>,
{
    let proxy = proxy.as_ref().map(|s| s.as_ref());
    let mut repo_builder = git2::build::RepoBuilder::new();
    repo_builder.fetch_options(fetch_options(proxy));

    repo_builder
        .clone(remote_url.as_ref(), path.as_ref())
        .map_err(|e| e.into())
        .map(|_| {})
}

/// `git ls-remote <url>`: connect to the remote and take its ref
/// advertisement without cloning anything — upstream's `Checking repo... `
/// precheck before `bucket add` creates the bucket directory
/// (`lib/buckets.ps1:149-154`).
///
/// A detached remote needs no repository and leaves no temporary clone
/// behind; the connection carries the same credentials and proxy a fetch
/// would use.
pub fn ls_remote<S>(remote_url: S, proxy: Option<S>) -> Fallible<()>
where
    S: AsRef<str>,
{
    let proxy = proxy.as_ref().map(|s| s.as_ref());
    let mut remote = git2::Remote::create_detached(remote_url.as_ref())?;
    // The connect handshake is what decides whether the URL is a repository at
    // all: it resolves the transport, authenticates and reads the ref
    // advertisement, so a dead host, a missing repository or a non-repo path
    // all fail here. Deliberately no `RemoteConnection::list()` afterwards:
    // git2 0.19 builds that slice with `from_raw_parts(null, 0)` for a remote
    // advertising no refs (an empty repository), which trips Rust's
    // non-null-pointer precondition and aborts the process.
    let _connection = remote.connect_auth(
        git2::Direction::Fetch,
        Some(remote_callbacks()),
        proxy_options(proxy),
    )?;

    Ok(())
}

// Update a bucket repo with `git pull` semantics (Scoop-compatible):
// - already up to date           -> Ok
// - fast-forwardable             -> fast-forward the local branch
// - diverged but mergeable       -> 3-way merge, auto-commit the merge
// - merge conflict               -> leave conflict markers, return an error
// - unrelated histories          -> error
// Local modifications that would be overwritten by the update abort it
// (matching `git pull`); otherwise they are preserved. With `autostash` set
// (the `autostash_on_conflict` config, upstream `libexec/scoop-update.ps1:113`)
// those modifications are stashed and the update proceeds; the stash is left
// in place, so the user recovers it with `git stash pop`.
pub fn pull<P, S>(path: P, proxy: Option<S>, autostash: bool) -> Fallible<()>
where
    P: AsRef<Path>,
    S: AsRef<str>,
{
    let proxy = proxy.as_ref().map(|s| s.as_ref());
    let mut repo = Repository::open(path.as_ref())?;
    let mut origin = repo.find_remote("origin")?;

    // Capture old HEAD before fetch. The symmetrical refspec would update the
    // local branch during fetch, making head_id == fetch_commit.id() and the
    // early return always trigger — so we save it first.
    let head_id = repo.head()?.target().unwrap();
    let ref_name = repo
        .head()
        .ok()
        .and_then(|h| h.name().map(|n| n.to_owned()))
        .unwrap_or_else(|| "refs/heads/master".to_owned());

    // Fetch to remote-tracking ref so the local branch is NOT updated
    // until we decide how to integrate (fast-forward or merge).
    let branch = ref_name.strip_prefix("refs/heads/").unwrap_or(&ref_name);
    origin.fetch(
        &[format!("+{ref_name}:refs/remotes/origin/{branch}").as_str()],
        Some(&mut fetch_options(proxy)),
        None,
    )?;
    // The remote handle borrows `repo`; release it now so the autostash below
    // can take the mutable borrow it needs.
    drop(origin);

    let fetch_id = repo.find_reference("FETCH_HEAD")?.peel_to_commit()?.id();

    if fetch_id == head_id {
        return Ok(()); // already up to date
    }

    let (analysis, _preference) = {
        let fetch_annotated = repo.find_annotated_commit(fetch_id)?;
        repo.merge_analysis(&[&fetch_annotated])?
    };

    // `git pull` refuses to overwrite local changes: fail before touching the
    // working tree if any file the update would change is locally modified.
    // With `autostash` on, those changes are stashed (and left stashed)
    // instead, so the update can proceed. Only a genuine local-changes verdict
    // may take that branch — every other `ensure_updateable` failure (an
    // unreadable object, a status query error) is returned as-is rather than
    // masked by a stash that would only report "nothing to stash".
    match ensure_updateable(&repo, head_id, fetch_id) {
        Ok(()) => {}
        Err(Error::BucketUpdateLocalChanges(_)) if autostash => {
            let stash_message = stash_local_changes(&mut repo)?;
            // The stash is deliberately not re-applied (upstream leaves it
            // too), so say how to get the work back.
            tracing::warn!(
                "uncommitted changes blocked the bucket update; stashed as '{stash_message}'. \
                 Recover them with `git stash pop` in {}",
                path.as_ref().display()
            );
        }
        Err(err) => return Err(err),
    }

    if analysis.is_fast_forward() {
        // Fast-forward: hard-reset the branch to the fetched commit (updates
        // the ref, index and working tree in one step). Safe because
        // ensure_updateable() above guarantees no local modification would
        // be overwritten.
        let reset_target: git2::Object =
            repo.find_object(fetch_id, Some(git2::ObjectType::Commit))?;
        repo.reset(&reset_target, git2::ResetType::Hard, None)?;

        Ok(())
    } else if analysis.is_normal() {
        // Diverged: perform a 3-way merge like `git pull` (merge mode).
        // SAFE checkout aborts if the merge would overwrite local changes.
        let fetch_annotated = repo.find_annotated_commit(fetch_id)?;
        let mut merge_opts = git2::MergeOptions::new();
        let mut co = git2::build::CheckoutBuilder::new();
        repo.merge(&[&fetch_annotated], Some(&mut merge_opts), Some(&mut co))?;

        let mut index = repo.index()?;
        if index.has_conflicts() {
            // Leave conflict markers in the working tree (like git), then fail.
            // Default conflict style is the standard merge style
            // (<<<<<<< / ======= / >>>>>>>), matching `git merge`.
            let mut conflict_co = git2::build::CheckoutBuilder::new();
            conflict_co.allow_conflicts(true);
            repo.checkout_index(None, Some(&mut conflict_co))?;
            repo.cleanup_state()?;
            return Err(Error::BucketUpdateMergeConflict(ref_name));
        }

        // No conflicts: write the merged tree and create the merge commit.
        // `signature` reads user.name / user.email from git config and fails
        // like `git commit` when they are unset.
        let head_commit = repo.head()?.peel_to_commit()?;
        let remote_commit = repo.find_commit(fetch_id)?;
        let sig = repo.signature()?;
        let tree_id = index.write_tree_to(&repo)?;
        let tree = repo.find_tree(tree_id)?;
        let message = format!("Merge remote-tracking branch 'origin/{branch}' into {branch}");
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &message,
            &tree,
            &[&head_commit, &remote_commit],
        )?;
        repo.cleanup_state()?;

        Ok(())
    } else {
        // NONE / UNBORN: cannot integrate (e.g. unrelated histories).
        Err(Error::BucketUpdateNotFastForward(ref_name))
    }
}

/// Fail (like `git pull`) if the update would overwrite local changes:
/// every file whose content differs between `head_id` and `target` must be
/// clean in both the index and the working tree.
///
/// `target` is the fetched commit id rather than a [`git2::Commit`]: the caller
/// stashes the blocking changes (which needs a mutable borrow of the
/// repository) between this check and the integration, and a live `Commit`
/// would keep the repository immutably borrowed across it.
fn ensure_updateable(repo: &Repository, head_id: git2::Oid, target: git2::Oid) -> Fallible<()> {
    let old_tree = repo.find_commit(head_id)?.tree()?;
    let new_tree = repo.find_commit(target)?.tree()?;
    let diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;

    let mut status_by_path: HashMap<String, git2::Status> = HashMap::new();
    for entry in repo.statuses(None)?.iter() {
        if let Some(path) = entry.path() {
            status_by_path.insert(path.to_owned(), entry.status());
        }
    }

    let mut local = Vec::new();
    for delta in diff.deltas() {
        let path = delta.old_file().path().or(delta.new_file().path());
        if let Some(path) = path {
            // A path absent from the status list has no local modification —
            // this includes files newly added on the remote, which do not
            // exist locally at all and are simply created by the update
            // (`git pull` semantics). `git_status_file` would instead reject
            // those with GIT_ENOTFOUND, so we use one full status pass here.
            let key = path.to_string_lossy().into_owned();
            if let Some(status) = status_by_path.get(&key)
                && !status.is_empty()
            {
                local.push(key);
            }
        }
    }

    if local.is_empty() {
        Ok(())
    } else {
        Err(Error::BucketUpdateLocalChanges(local.join(", ")))
    }
}

/// Stash the local changes that block an update, leaving the stash in place.
///
/// Mirrors upstream's `autostash_on_conflict` branch
/// (`libexec/scoop-update.ps1:113-121`): untracked files are included (`-u`)
/// and the message is `WIP at <round-trip timestamp>` — the same shape
/// `[System.DateTime]::Now.ToString('o')` produces.
///
/// Returns the stash message so the caller can tell the user how to recover
/// the changes. A stash that cannot be created is an error: the update must
/// never proceed after failing to save the user's work.
fn stash_local_changes(repo: &mut Repository) -> Fallible<String> {
    let stasher = repo.signature()?;
    let message = format!(
        "WIP at {}",
        crate::internal::time::format_last_update(time::OffsetDateTime::now_utc())
    );

    repo.stash_save(
        &stasher,
        &message,
        Some(git2::StashFlags::INCLUDE_UNTRACKED),
    )
    .map_err(|e| {
        Error::Custom(format!(
            "failed to stash the local changes blocking the bucket update: {e}"
        ))
    })?;

    Ok(message)
}

/// Get the URL of `remote` in the repository rooted exactly at `repo_path`.
pub fn remote_url_of<S>(repo_path: &Path, remote: S) -> Fallible<Option<String>>
where
    S: AsRef<str>,
{
    let repo = Repository::open(repo_path)?;
    remote_url_from(&repo, remote.as_ref())
}

/// Get the URL of `remote` in the repository that *contains* `path`,
/// searching parent directories the way the `git` CLI does.
///
/// Unlike [`remote_url_of`], `path` does not have to be the repository root:
/// this mirrors `git remote get-url <remote>` run with `path` as the working
/// directory. Prefer [`remote_url_of`] when the path is known to be a
/// repository root, so an unrelated enclosing repository is never picked up.
pub fn remote_url_containing<S>(path: &Path, remote: S) -> Fallible<Option<String>>
where
    S: AsRef<str>,
{
    let repo = Repository::discover(path)?;
    remote_url_from(&repo, remote.as_ref())
}

/// Shared tail of the two `remote_url_*` lookups above.
fn remote_url_from(repo: &Repository, remote: &str) -> Fallible<Option<String>> {
    let remote = repo.find_remote(remote)?;
    Ok(remote.url().map(|s| s.to_owned()))
}

/// Get the HEAD commit author time of a git repo as Unix seconds.
///
/// Returns `None` if the repository is not available or has no commits.
pub fn head_commit_time<P: AsRef<Path>>(path: P) -> Option<i64> {
    let repo = Repository::open(path.as_ref()).ok()?;
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    Some(commit.time().seconds())
}

/// Get the current HEAD commit id (hex) of a repository, or `None` if the
/// repository is unavailable or has no commits.
pub fn head_commit_id<P: AsRef<Path>>(path: P) -> Option<String> {
    let repo = Repository::open(path.as_ref()).ok()?;
    Some(repo.head().ok()?.target()?.to_string())
}

/// Upper bound on commits visited while following a file's history. The walk
/// stops at the first commit that is not "treesame" with a parent, so the cap
/// is only a safety net for a pathological history.
const MAX_COMMITS_SCANNED: usize = 200_000;

/// The most recent commit that changed `file`, mirroring
/// `git log -1 -s --format=%aD#%an -- <file>`.
///
/// This is the metadata `scoop info` shows as `Updated at` / `Updated by`
/// (libexec/scoop-info.ps1:123-134). Upstream shells out to git and falls back
/// to the file's mtime + owner; hok resolves it through libgit2 instead, so it
/// never needs git on PATH — the same rule the rest of hok follows (buckets are
/// cloned with libgit2).
///
/// The walk reproduces git's default history simplification with tree-entry
/// lookups instead of comparing each commit to its parent with a full diff:
///
/// - a commit whose blob for the path equals a parent's is not a change — keep
///   following that parent and ignore the others (this is what makes a merge
///   that merely carried the change into HEAD invisible, exactly as
///   `git log -- <path>` simplifies it away);
/// - the first commit that differs from **every** parent is the answer;
/// - the root commit only counts when the path already exists there.
///
/// Picking the first parent's diff alone instead reported the wrong author for
/// a merge, and diffing every commit against its parent loaded whole bucket
/// trees per commit (`hok info git` took 1.69 s, against 0.63 s here with
/// `-v` at 1.97 s against 0.91 s): the blob is carried from one commit to the
/// next, so a linear history looks each one up once, and an identical tree id
/// settles a parent without any object lookup at all.
///
/// # Returns
///
/// The commit author's Unix timestamp (seconds) and name, or `None` when the
/// file is outside a work tree, no commit in the reachable history changed it,
/// or the repository cannot be read.
pub fn last_commit_for_file<P: AsRef<Path>>(file: P) -> Option<(i64, String)> {
    let file = file.as_ref();
    let repo = Repository::discover(file.parent()?).ok()?;
    // Tree paths are relative to the work tree root and use `/` separators.
    let rel = file.strip_prefix(repo.workdir()?).ok()?;
    let rel = rel.to_string_lossy().replace('\\', "/");

    let mut commit = repo.head().ok()?.peel_to_commit().ok()?;
    let mut blob = blob_at(&commit, &rel);

    for _ in 0..MAX_COMMITS_SCANNED {
        if commit.parent_count() == 0 {
            return blob.is_some().then(|| author_of(&commit));
        }

        let mut treesame = None;
        for index in 0..commit.parent_count() {
            // A parent that cannot be read (shallow clone, corruption) leaves
            // the history unverifiable — report "no metadata" rather than
            // guessing a commit.
            let parent = commit.parent(index).ok()?;
            // An identical tree id means the commit changed nothing at all
            // relative to that parent, so the path cannot have changed either.
            if parent.tree_id() == commit.tree_id() {
                treesame = Some((parent, blob));
                break;
            }
            let parent_blob = blob_at(&parent, &rel);
            if parent_blob == blob {
                treesame = Some((parent, parent_blob));
                break;
            }
        }

        match treesame {
            Some((parent, parent_blob)) => {
                commit = parent;
                // The next iteration would only look this up again.
                blob = parent_blob;
            }
            None => return Some(author_of(&commit)),
        }
    }

    None
}

/// The blob a commit's tree carries for `path`, or `None` when the commit does
/// not contain it.
fn blob_at(commit: &git2::Commit<'_>, path: &str) -> Option<git2::Oid> {
    commit
        .tree()
        .ok()?
        .get_path(Path::new(path))
        .ok()
        .map(|entry| entry.id())
}

/// The author of a commit, as `(Unix seconds, name)`.
fn author_of(commit: &git2::Commit<'_>) -> (i64, String) {
    let author = commit.author();
    (
        author.when().seconds(),
        author.name().unwrap_or_default().to_owned(),
    )
}

/// A manifest file that changed between two commits of a bucket repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDiffEntry {
    /// Repo-relative path with `/` separators.
    pub path: String,
    /// True if the file was deleted by the update (no longer on disk).
    pub deleted: bool,
}

/// List the manifest files whose content changed between `from_commit` and the
/// current HEAD of the repository at `path`.
///
/// This backs the incremental SQLite manifest cache: after a bucket pull we
/// only re-parse the manifests that actually changed instead of re-reading
/// every manifest in the bucket. Added and modified files are reported with
/// their new path; deleted files carry `deleted: true` (their content is gone).
///
/// Rename detection is not configured: the diff is built without options
/// (`diff_tree_to_tree(.., .., None)`) and is never run through
/// `find_similar`, so a renamed manifest is reported as the old path with
/// `deleted: true` **and** the new path with `deleted: false` — never as a
/// single rename entry naming the new path.
pub fn diff_manifest_changes<P: AsRef<Path>>(
    path: P,
    from_commit: &str,
) -> Fallible<Vec<ManifestDiffEntry>> {
    let repo = Repository::open(path.as_ref())?;
    let from_tree = repo
        .revparse_single(from_commit)?
        .peel_to_commit()?
        .tree()?;
    let head_tree = repo.head()?.peel_to_commit()?.tree()?;
    let diff = repo.diff_tree_to_tree(Some(&from_tree), Some(&head_tree), None)?;

    let mut out = Vec::new();
    for delta in diff.deltas() {
        let deleted = matches!(delta.status(), git2::Delta::Deleted);
        let candidate = if deleted {
            delta.old_file().path()
        } else {
            delta.new_file().path()
        };
        let Some(p) = candidate else {
            continue;
        };
        let s = p.to_string_lossy().into_owned();
        // Only manifest JSON files; ignore e.g. a repo-level package.json.
        if s.ends_with(".json") && !s.ends_with("package.json") {
            out.push(ManifestDiffEntry { path: s, deleted });
        }
    }
    Ok(out)
}

// ─── Manifest-history walk primitives ──────────────────────────────────────
//
// Backs the `hok_manifest_history` index (`package/manifest_history.rs`): one
// pass down the first-parent chain, one tree diff per step, so the seed scan
// never re-reads whole trees per app (the O(commits × apps) shape that made
// the per-app pickaxe route infeasible). Rename detection is deliberately off —
// a renamed manifest arrives as old-path deleted + new-path added, exactly like
// `diff_manifest_changes`; the index keys on the file name and keeps the old
// path in `path` so a row can be traced back.
//
// The per-step diff is [`HistoryRepo::diff_trees`], which compares parsed trees
// and memoizes them by OID instead of calling `diff_tree_to_tree` — see
// `manifest_deltas` for why that is what makes the walk affordable.
//
// The first-parent choice mirrors `last_commit_for_file`'s treesame
// simplification: bucket histories are near-linear, and a merge that merely
// carried a change into HEAD is invisible there already.
//
/// Safety cap on the first-parent walk (the seed scan visits every commit in
/// the chain; a pathological history must not loop forever). Generous over
/// the measured 175k-commit worst case (main 81k + extras 93k, research §2).
const MAX_HISTORY_COMMITS: usize = 500_000;

/// An open bucket repository for the manifest-history index.
///
/// The seed and the incremental scan each read thousands of blobs, so both
/// hold one handle for a whole pass: opening the repository per blob would
/// re-read its config on every manifest. This is also the seam that keeps
/// `git2` out of `package/` — the index builder works with this type, not
/// with libgit2 directly.
pub struct HistoryRepo {
    repo: Repository,
    trees: RefCell<TreeCache>,
}

impl HistoryRepo {
    /// Open the repository at `path`, or `None` when the directory is not a
    /// repository (a plain directory bucket has no history to index).
    pub fn open<P: AsRef<Path>>(path: P) -> Option<HistoryRepo> {
        Repository::open(path.as_ref())
            .ok()
            .map(|repo| HistoryRepo {
                repo,
                trees: RefCell::new(TreeCache::new()),
            })
    }

    /// The parsed entries of tree `oid`, memoized by OID (see [`TreeCache`]).
    fn tree(&self, oid: git2::Oid) -> Option<Rc<TreeData>> {
        if let Some(tree) = self.trees.borrow().trees.get(&oid) {
            return Some(Rc::clone(tree));
        }
        // The cache borrow is released before the load: `find_tree` calls back
        // into the repository, and the recursion below re-enters `tree`.
        let tree = Rc::new(load_tree(&self.repo, oid)?);
        self.trees.borrow_mut().insert(oid, Rc::clone(&tree));
        Some(tree)
    }

    /// Whether tree `oid` is currently memoized. Test-only: production has no
    /// reason to ask, it relies on the next step hitting the entry.
    #[cfg(test)]
    fn is_cached(&self, oid: &str) -> bool {
        parse_oid(oid).is_some_and(|oid| self.trees.borrow().trees.contains_key(&oid))
    }

    /// The current HEAD commit as 40-hex, or `None` when the repository has
    /// no commits.
    pub fn head(&self) -> Option<String> {
        Some(
            self.repo
                .head()
                .ok()?
                .peel_to_commit()
                .ok()?
                .id()
                .to_string(),
        )
    }

    /// Whether `oid` is reachable from `head` (including `head` itself),
    /// walking first parents — the same chain [`HistoryRepo::walk_chain`]
    /// traverses. A commit from a rewritten or re-cloned history reports
    /// `false`, so the caller re-seeds instead of walking from a baseline
    /// that no longer describes this repository.
    pub fn is_ancestor(&self, head: &str, oid: &str) -> bool {
        // Commits cross this boundary as 40-hex, like `head_commit_id` and
        // `diff_manifest_changes`: `package/` never has to name a git2 type.
        let (Some(head), Some(oid)) = (parse_oid(head), parse_oid(oid)) else {
            return false;
        };
        // Anchor the walk at `head` and descend first parents looking for
        // `oid` — the same direction as `walk`. Starting from `oid` instead
        // would answer the opposite question ("is head an ancestor of oid")
        // and reject every stored baseline the walk legitimately visits.
        let Ok(mut commit) = self.repo.find_commit(head) else {
            return false;
        };
        for _ in 0..=MAX_HISTORY_COMMITS {
            if commit.id() == oid {
                return true;
            }
            if commit.parent_count() == 0 {
                return false;
            }
            // First parent only — matches the walk this validates.
            let Ok(parent) = commit.parent(0) else {
                return false;
            };
            commit = parent;
        }
        false
    }

    /// The UTF-8 text of blob `oid`, or `None` when it cannot be read — the
    /// object was pruned (`gc --prune`) or is not UTF-8. Callers treat `None`
    /// as "skip this manifest", never as a failure.
    pub fn blob_text(&self, oid: &str) -> Option<String> {
        let blob = self.repo.find_blob(parse_oid(oid)?).ok()?;
        std::str::from_utf8(blob.content()).ok().map(str::to_owned)
    }

    /// Walk the first-parent chain from `head` down, yielding one entry per
    /// commit — **without reading a single tree**.
    ///
    /// Split from [`HistoryRepo::manifest_deltas`] so a caller can walk the
    /// chain cheaply, fan the expensive tree-diff and blob work out to a
    /// thread pool, and still keep the walk itself sequential and ordered. On
    /// a real bucket that split is the whole point: 92,616 commits of chain
    /// walking take under a second, while diffing their trees takes ~177 s
    /// (93% of a seed).
    ///
    /// - `start`: resume point (inclusive). A seed interrupted mid-chain
    ///   resumes at the commit it last recorded, which is re-processed — the
    ///   index rows are written with `INSERT OR IGNORE`, so that is a no-op.
    ///   `None`, or a commit that cannot be read, starts at `head`.
    /// - `until`: stop **after** yielding this commit (inclusive) — this is
    ///   what bounds an incremental scan at its stored baseline.
    ///
    /// A repository that cannot be walked yields a short (possibly empty)
    /// walk; the caller treats "did not reach the stop commit" as "baseline
    /// not advanced", which re-scans next time.
    pub fn walk_chain(
        &self,
        head: &str,
        start: Option<&str>,
        until: Option<&str>,
    ) -> impl Iterator<Item = HistoryCommit> + '_ {
        struct State<'r> {
            next: Option<git2::Commit<'r>>,
            until: Option<git2::Oid>,
            steps: usize,
            done: bool,
        }

        impl Iterator for State<'_> {
            type Item = HistoryCommit;

            fn next(&mut self) -> Option<HistoryCommit> {
                if self.done {
                    return None;
                }
                let commit = self.next.take()?;
                let id = commit.id();
                // The stop commit is yielded, then the walk ends (inclusive).
                if self.until == Some(id) {
                    self.done = true;
                }
                self.steps += 1;
                if self.steps > MAX_HISTORY_COMMITS {
                    self.done = true;
                    return None;
                }

                // `tree_id` is read out of the commit object: no tree is
                // loaded here, which is what keeps this walk cheap.
                let parent = commit.parent(0).ok();
                let entry = HistoryCommit {
                    commit: id.to_string(),
                    tree: commit.tree_id().to_string(),
                    parent_tree: parent.as_ref().map(|parent| parent.tree_id().to_string()),
                    committed_at: commit.author().when().seconds(),
                };

                // Advance: first parent, else root (no parent → next() ends).
                self.next = parent;

                Some(entry)
            }
        }

        // Resolve the starting commit: `start` when it exists, else `head`.
        let start_commit = start
            .and_then(parse_oid)
            .and_then(|oid| self.repo.find_commit(oid).ok())
            .or_else(|| parse_oid(head).and_then(|oid| self.repo.find_commit(oid).ok()));

        State {
            next: start_commit,
            until: until.and_then(parse_oid),
            steps: 0,
            done: false,
        }
    }

    /// The manifest deltas between two of this repository's trees, as 40-hex
    /// tree OIDs.
    ///
    /// `old_tree` is the first parent's tree; `None` for a root commit, which
    /// then reports every manifest in `new_tree` as an addition.
    ///
    /// This is the expensive half of the walk — one full tree diff per commit
    /// — so it takes plain OIDs and does not borrow the chain walk: callers
    /// run it across a thread pool, each worker with its own [`HistoryRepo`].
    ///
    /// The comparison is [`HistoryRepo::diff_trees`], not
    /// `Repository::diff_tree_to_tree`: the seed diffs commit N against N+1
    /// and then N+1 against N+2, so the same tree is the *new* side of one
    /// step and the *old* side of the next. libgit2 loads both sides itself
    /// and its object cache keeps trees of up to 4 KB, far below a real
    /// bucket's ~100 KB `bucket/` subtree, so that read is always paid twice
    /// (plan §2: 100.3 s against 57.1 s for one read, extras). Diffing parsed
    /// trees from [`HistoryRepo::tree`] pays it once.
    pub fn manifest_deltas(&self, new_tree: &str, old_tree: Option<&str>) -> Vec<HistoryDelta> {
        let Some(new_tree) = parse_oid(new_tree).and_then(|oid| self.tree(oid)) else {
            return Vec::new();
        };
        let old_tree = old_tree.and_then(parse_oid).and_then(|oid| self.tree(oid));

        let mut out = Vec::new();
        self.diff_trees(old_tree.as_deref(), &new_tree, "", &mut out);
        // libgit2 expands both trees before it compares them, so its deltas
        // come out in path order (the merge of two path-sorted streams).
        // Recursing per directory can order an entry differently — a file
        // sorts before the directory it collides with, but the directory's
        // name takes part in the tree comparison as `name/` — so the handful
        // of deltas of one commit is re-sorted into path order. The sort is
        // stable, which keeps a path that yields both a delete and an add (a
        // type change) in libgit2's order.
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Diff two parsed trees one directory at a time, appending the manifest
    /// deltas of their leaves to `out`.
    ///
    /// This is the merged walk libgit2's diff engine performs over two fully
    /// expanded trees: entries are matched by name in git's tree order, a
    /// directory present on one side only contributes one delta per file, and
    /// a subtree whose id matches on both sides is skipped without being read.
    fn diff_trees(
        &self,
        old: Option<&TreeData>,
        new: &TreeData,
        prefix: &str,
        out: &mut Vec<HistoryDelta>,
    ) {
        let old_len = old.map_or(0, |tree| tree.entries.len());
        let mut oi = 0;
        let mut ni = 0;

        loop {
            match (oi < old_len, ni < new.entries.len()) {
                (false, false) => return,
                (true, false) => {
                    self.push_entry(old.expect("a tree with entries"), oi, prefix, true, out);
                    oi += 1;
                }
                (false, true) => {
                    self.push_entry(new, ni, prefix, false, out);
                    ni += 1;
                }
                (true, true) => {
                    let old_tree = old.expect("a tree with entries");
                    match base_name_compare(old_tree, oi, new, ni) {
                        Ordering::Less => {
                            self.push_entry(old_tree, oi, prefix, true, out);
                            oi += 1;
                        }
                        Ordering::Greater => {
                            self.push_entry(new, ni, prefix, false, out);
                            ni += 1;
                        }
                        Ordering::Equal => {
                            self.push_matched(old_tree, oi, new, ni, prefix, out);
                            oi += 1;
                            ni += 1;
                        }
                    }
                }
            }
        }
    }

    /// Append one entry present on a single side: the entry itself when it is
    /// a file, every manifest under it when it is a directory.
    fn push_entry(
        &self,
        tree: &TreeData,
        index: usize,
        prefix: &str,
        deleted: bool,
        out: &mut Vec<HistoryDelta>,
    ) {
        let entry = &tree.entries[index];
        if entry.mode & MODE_TYPE_MASK == MODE_TREE {
            let Some(subtree) = self.tree(entry.oid) else {
                // An unreadable subtree drops its deltas, like an unreadable
                // blob: the index is derived data and a broken object must not
                // fail the walk (`blob_text` answers `None` for the same
                // reason).
                return;
            };
            let child = child_prefix(prefix, tree.name(index));
            for index in 0..subtree.entries.len() {
                self.push_entry(&subtree, index, &child, deleted, out);
            }
        } else {
            self.push_leaf(tree, index, prefix, deleted, out);
        }
    }

    /// Append one manifest leaf, dropping it when the manifest filter does.
    fn push_leaf(
        &self,
        tree: &TreeData,
        index: usize,
        prefix: &str,
        deleted: bool,
        out: &mut Vec<HistoryDelta>,
    ) {
        let entry = &tree.entries[index];
        // The filter's two rules are both about the end of the path, so the
        // entry name — the path's last component — decides them without
        // building the path first. Only the few leaves that pass pay for one.
        let name = String::from_utf8_lossy(tree.name(index));
        if !is_manifest_path(&name) {
            return;
        }
        out.push(HistoryDelta {
            path: format!("{prefix}{name}"),
            deleted,
            blob_oid: (!deleted && entry.oid != git2::Oid::zero()).then(|| entry.oid.to_string()),
        });
    }

    /// Append the deltas of an entry present on both sides under one name.
    fn push_matched(
        &self,
        old: &TreeData,
        oi: usize,
        new: &TreeData,
        ni: usize,
        prefix: &str,
        out: &mut Vec<HistoryDelta>,
    ) {
        let old_entry = &old.entries[oi];
        let new_entry = &new.entries[ni];
        let old_is_tree = old_entry.mode & MODE_TYPE_MASK == MODE_TREE;
        let new_is_tree = new_entry.mode & MODE_TYPE_MASK == MODE_TREE;

        match (old_is_tree, new_is_tree) {
            (true, true) => {
                // Two identical subtrees cannot differ, and looking at the id
                // first is what keeps a commit that touched no manifest from
                // reading `bucket/` at all.
                if old_entry.oid != new_entry.oid
                    && let (Some(old_tree), Some(new_tree)) =
                        (self.tree(old_entry.oid), self.tree(new_entry.oid))
                {
                    let child = child_prefix(prefix, new.name(ni));
                    self.diff_trees(Some(&old_tree), &new_tree, &child, out);
                }
            }
            (false, false) => {
                if old_entry.mode & MODE_TYPE_MASK != new_entry.mode & MODE_TYPE_MASK {
                    // git splits a change of basic file type — a symlink
                    // becoming a regular file, a submodule becoming a blob —
                    // into a delete and an add, not into one type-change
                    // delta.
                    self.push_leaf(old, oi, prefix, true, out);
                    self.push_leaf(new, ni, prefix, false, out);
                } else if old_entry.oid == new_entry.oid
                    && old_entry.mode == new_entry.mode
                    && old_entry.oid != git2::Oid::zero()
                {
                    // Unchanged.
                } else {
                    // A content change, and a mode-only change
                    // (100644 <-> 100755) alike: git reports both as modified.
                    self.push_leaf(new, ni, prefix, false, out);
                }
            }
            // A file on one side and a directory on the other. libgit2 sees
            // the file's path first (it sorts before everything under the
            // directory), so the file's delta is the delete or the add.
            (false, true) => {
                self.push_leaf(old, oi, prefix, true, out);
                self.push_entry(new, ni, prefix, false, out);
            }
            (true, false) => {
                self.push_leaf(new, ni, prefix, false, out);
                self.push_entry(old, oi, prefix, true, out);
            }
        }
    }
}

/// Parse a 40-hex commit or blob id, rejecting anything else.
fn parse_oid(hex: &str) -> Option<git2::Oid> {
    git2::Oid::from_str(hex).ok()
}

/// One commit of the first-parent chain, as [`HistoryRepo::walk_chain`] yields
/// it: the commit, its tree and its first parent's tree as 40-hex OIDs, and the
/// author time. The trees are only *read* later, by
/// [`HistoryRepo::manifest_deltas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCommit {
    /// The commit itself, 40-hex.
    pub commit: String,
    /// The commit's root tree, 40-hex.
    pub tree: String,
    /// The first parent's root tree, 40-hex — `None` for a root commit, which
    /// is also how a completed seed recognises the end of the chain.
    pub parent_tree: Option<String>,
    /// Author time of `commit`, Unix seconds — the `committed_at` the index
    /// records for the versions this commit introduced.
    pub committed_at: i64,
}

/// A manifest file that changed between two adjacent commits of a bucket repo,
/// with the blob OID on the **new** side (present for added/modified deltas;
/// `None` for deleted ones — the content is gone from that commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryDelta {
    /// Repo-relative path with `/` separators.
    pub path: String,
    /// True when the update removed the file (no content on the new side).
    pub deleted: bool,
    /// Blob OID on the new side of the delta, when there is one (40-hex).
    pub blob_oid: Option<String>,
}

/// The rule that decides which paths of a bucket tree are manifests, shared
/// with [`diff_manifest_changes`] and `manifest_walker`'s discovery: every
/// `.json` except `package.json`. A bucket may keep its manifests at the
/// repository root or under a category directory — `Bucket::path_of_manifest`
/// accepts both — so requiring a `bucket/` prefix would silently index nothing
/// for those buckets. A JSON that is not a manifest is dropped later, when its
/// version fails to parse.
fn is_manifest_path(path: &str) -> bool {
    path.ends_with(".json") && !path.ends_with("package.json")
}

/// Object-type bits of a git filemode (`S_IFMT`).
const MODE_TYPE_MASK: i32 = 0o170000;
/// The filemode of a subtree (git's `S_IFDIR`).
const MODE_TREE: i32 = 0o040000;

/// How many parsed trees one [`HistoryRepo`] keeps.
///
/// The walk needs exactly two at a time — the commit's tree and its first
/// parent's — and the next step asks for the parent's again, so two entries
/// would do; the slack absorbs a shared subtree and the odd re-read without
/// ever holding much: a `bucket/` tree with its ~2,400 entries is about
/// 110 KB.
const TREE_CACHE_CAPACITY: usize = 8;

/// Parsed trees, keyed by tree OID, most recently inserted last.
///
/// This is the memoization the seed walk runs on: without it every tree is
/// read twice, once as each step's new side and once as the next step's old
/// side, and reading a tree is what the walk mostly spends its time on
/// (plan §2: ~0.57 ms for a `bucket/` subtree, ~5.5 µs per entry).
struct TreeCache {
    trees: HashMap<git2::Oid, Rc<TreeData>>,
    order: VecDeque<git2::Oid>,
}

impl TreeCache {
    fn new() -> TreeCache {
        TreeCache {
            trees: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, oid: git2::Oid, tree: Rc<TreeData>) {
        if self.trees.insert(oid, tree).is_none() {
            self.order.push_back(oid);
        }
        while self.order.len() > TREE_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.trees.remove(&oldest);
            }
        }
    }
}

/// A tree parsed into the shape the differ compares: every name in one buffer,
/// and one fixed-size entry per name pointing into it.
///
/// The layout keeps a `bucket/` tree at two allocations instead of one per
/// entry, which matters because the seed parses one tree per commit.
struct TreeData {
    names: Vec<u8>,
    entries: Vec<TreeEntry>,
}

/// One entry of a [`TreeData`].
struct TreeEntry {
    /// Byte range of the name within `TreeData::names`.
    name: std::ops::Range<u32>,
    /// Git filemode: `0o100644`, `0o100755`, `0o120000`, `0o160000` or
    /// `0o040000` for a subtree.
    mode: i32,
    oid: git2::Oid,
}

impl TreeData {
    /// The name of entry `index`, as raw bytes (a tree entry name may be any
    /// bytes but `/` and NUL).
    fn name(&self, index: usize) -> &[u8] {
        let name = &self.entries[index].name;
        &self.names[name.start as usize..name.end as usize]
    }

    /// Whether entry `index` is a subtree.
    fn is_tree(&self, index: usize) -> bool {
        self.entries[index].mode & MODE_TYPE_MASK == MODE_TREE
    }
}

/// Read one tree object into [`TreeData`], or `None` when it cannot be read —
/// the same "skip this manifest, never fail" rule [`HistoryRepo::blob_text`]
/// follows for a blob `gc --prune` took away.
fn load_tree(repo: &Repository, oid: git2::Oid) -> Option<TreeData> {
    let tree = repo.find_tree(oid).ok()?;
    // Both buffers get their exact size up front. Growing them instead copies
    // a whole subtree's worth of names again — a ~2,400-entry `bucket/` tree is
    // ~110 KB — and the seed parses one tree per commit on every worker at
    // once, so those copies are what its parallel half is most sensitive to.
    let names_len = tree
        .iter()
        .map(|entry| entry.name_bytes().len())
        .sum::<usize>();
    let mut names = Vec::with_capacity(names_len);
    let mut entries = Vec::with_capacity(tree.len());
    for entry in tree.iter() {
        let start = names.len() as u32;
        names.extend_from_slice(entry.name_bytes());
        entries.push(TreeEntry {
            name: start..names.len() as u32,
            mode: entry.filemode(),
            oid: entry.id(),
        });
    }
    Some(TreeData { names, entries })
}

/// Order two entries of a tree the way git does (`base_name_compare`): by name
/// bytes, with a directory's name compared as if it ended in `/`.
///
/// This is not plain byte order. A file `foo.json` sorts *before* a directory
/// `foo` (`.` is below `/`), while a naive byte comparison puts `foo` first
/// and then misaligns the two sides at every descent into it — the prototype's
/// merge-join counted 3.31 M deltas where the trees hold 97,929 (plan §4.4).
fn base_name_compare(old: &TreeData, oi: usize, new: &TreeData, ni: usize) -> Ordering {
    let a = old.name(oi);
    let b = new.name(ni);
    let len = a.len().min(b.len());

    let ordering = a[..len].cmp(&b[..len]);
    if ordering != Ordering::Equal {
        return ordering;
    }

    // The first byte past the common prefix, with a directory's terminator
    // standing in as `/` — exactly what git compares.
    let tail = |name: &[u8], is_tree: bool| -> u8 {
        match name.get(len) {
            Some(byte) => *byte,
            None if is_tree => b'/',
            None => 0,
        }
    };
    tail(a, old.is_tree(oi)).cmp(&tail(b, new.is_tree(ni)))
}

/// The path prefix of a directory's entries: `prefix`, the directory name, and
/// a `/`.
fn child_prefix(prefix: &str, name: &[u8]) -> String {
    let name = String::from_utf8_lossy(name);
    let mut child = String::with_capacity(prefix.len() + name.len() + 1);
    child.push_str(prefix);
    child.push_str(&name);
    child.push('/');
    child
}

// ---------------------------------------------------------------------------
// Infra trait (git capability)
// ---------------------------------------------------------------------------

/// The git capability of a session (clone/pull/inspect a bucket repo).
///
/// The default implementation ([`RealGit`]) wraps the free functions above
/// verbatim; tests inject an in-memory fake via [`Session::set_git`] to avoid
/// touching real repositories. Lives in the tool layer (no domain-service
/// dependency).
#[async_trait::async_trait]
pub trait Git: Send + Sync {
    /// Clone a remote repository into `path`.
    async fn clone_repo(&self, remote_url: &str, path: &Path, proxy: Option<&str>) -> Fallible<()>;

    /// Check that `remote_url` is a reachable git repository (`git ls-remote`)
    /// without cloning it (see [`ls_remote`]).
    async fn ls_remote(&self, remote_url: &str, proxy: Option<&str>) -> Fallible<()>;

    /// Update a bucket repo with `git pull` semantics. With `autostash` set,
    /// local changes that would block the update are stashed instead of
    /// aborting it (see [`pull`]).
    async fn pull(&self, path: &Path, proxy: Option<&str>, autostash: bool) -> Fallible<()>;

    /// Get the URL of a remote in a repository.
    fn remote_url_of(&self, repo_path: &Path, remote: &str) -> Fallible<Option<String>>;

    /// Get the HEAD commit author time (Unix seconds) of a repository.
    fn head_commit_time(&self, path: &Path) -> Option<i64>;

    /// Get the current HEAD commit id (hex) of a repository, or `None` if it
    /// has no commits.
    fn head_commit_id(&self, path: &Path) -> Option<String>;

    /// List manifest files changed between `from_commit` and HEAD of the repo
    /// at `path` (see [`diff_manifest_changes`]).
    fn diff_manifest_changes(
        &self,
        path: &Path,
        from_commit: &str,
    ) -> Fallible<Vec<ManifestDiffEntry>>;
}

/// Default git implementation wrapping the free functions above.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealGit;

#[async_trait::async_trait]
impl Git for RealGit {
    async fn clone_repo(&self, remote_url: &str, path: &Path, proxy: Option<&str>) -> Fallible<()> {
        // git2 (libgit2) is a blocking library: run the whole clone on the
        // tokio blocking pool so concurrent operations (parallel bucket
        // clones) never occupy a runtime worker thread. The closure needs
        // owned `static + Send` values, so the borrowed args are copied.
        let remote_url = remote_url.to_owned();
        let path = path.to_owned();
        let proxy = proxy.map(|s| s.to_owned());
        tokio::task::spawn_blocking(move || clone_repo(remote_url, path, proxy))
            .await
            .map_err(|e| Error::Custom(format!("git clone task panicked: {}", e)))?
    }

    async fn pull(&self, path: &Path, proxy: Option<&str>, autostash: bool) -> Fallible<()> {
        // Same blocking-pool offload as clone_repo: git2 pull (fetch + merge)
        // is blocking I/O. Each bucket pull runs on the blocking pool.
        let path = path.to_owned();
        let proxy = proxy.map(|s| s.to_owned());
        tokio::task::spawn_blocking(move || pull(path, proxy, autostash))
            .await
            .map_err(|e| Error::Custom(format!("git pull task panicked: {}", e)))?
    }

    async fn ls_remote(&self, remote_url: &str, proxy: Option<&str>) -> Fallible<()> {
        // Blocking network I/O: the same offload as clone_repo.
        let remote_url = remote_url.to_owned();
        let proxy = proxy.map(|s| s.to_owned());
        tokio::task::spawn_blocking(move || ls_remote(remote_url, proxy))
            .await
            .map_err(|e| Error::Custom(format!("git ls-remote task panicked: {}", e)))?
    }

    fn remote_url_of(&self, repo_path: &Path, remote: &str) -> Fallible<Option<String>> {
        remote_url_of(repo_path, remote)
    }

    fn head_commit_time(&self, path: &Path) -> Option<i64> {
        head_commit_time(path)
    }

    fn head_commit_id(&self, path: &Path) -> Option<String> {
        head_commit_id(path)
    }

    fn diff_manifest_changes(
        &self,
        path: &Path,
        from_commit: &str,
    ) -> Fallible<Vec<ManifestDiffEntry>> {
        diff_manifest_changes(path, from_commit)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Delta, Signature, Time};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    /// Create a unique scratch directory for one test.
    fn temp_dir(name: &str) -> PathBuf {
        let seq = DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "hok-git-test-{}-{}-{}",
            name,
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn sig() -> Signature<'static> {
        Signature::new("hok tester", "tester@example.com", &Time::new(0, 0)).unwrap()
    }

    /// A signature with a distinct author name, to tell commits apart.
    fn sig_named(name: &str) -> Signature<'static> {
        Signature::new(name, "tester@example.com", &Time::new(0, 0)).unwrap()
    }

    /// A copy of `base` with `entries` added/replaced, built without touching
    /// the index or the working tree.
    fn tree_with<'r>(
        repo: &'r Repository,
        base: &git2::Tree<'_>,
        entries: &[(&str, &str)],
    ) -> git2::Tree<'r> {
        let mut builder = repo.treebuilder(Some(base)).unwrap();
        for (name, content) in entries {
            let blob = repo.blob(content.as_bytes()).unwrap();
            builder.insert(*name, blob, 0o100644).unwrap();
        }
        repo.find_tree(builder.write().unwrap()).unwrap()
    }

    /// Write a file in the working tree and commit it on HEAD, creating any
    /// missing parent directories (`bucket/…` fixtures are not checked out by
    /// `Repository::init` alone) and materialising the full tree.
    fn commit_file(repo: &Repository, rel: &str, content: &str, msg: &str) {
        let full = repo.workdir().unwrap().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content).unwrap();

        // The existing fixture wrote one flat file per commit into a fresh
        // index; nested fixtures accumulate `bucket/` entries instead, so
        // rebuild the index from the working tree (what `git add` would do)
        // and keep previously committed files present. `index.add_path` needs
        // a repo-relative path with `/` separators on Windows too.
        let workdir = repo.workdir().unwrap().to_owned();
        let mut index = repo.index().unwrap();
        index.clear().unwrap();
        let push = |rel: String, index: &mut git2::Index| {
            let _ = index.add_path(std::path::Path::new(&rel));
        };
        for entry in std::fs::read_dir(&workdir).unwrap().flatten() {
            let path = entry.path();
            if path.is_file() {
                push(entry.file_name().to_string_lossy().into_owned(), &mut index);
            } else if path.is_dir() {
                for nested in std::fs::read_dir(&path).unwrap().flatten() {
                    if nested.path().is_file() {
                        push(
                            format!(
                                "{}/{}",
                                entry.file_name().to_string_lossy(),
                                nested.file_name().to_string_lossy()
                            ),
                            &mut index,
                        );
                    }
                }
            }
        }
        index.write().unwrap();
        let tree_id = index.write_tree_to(repo).unwrap();
        let tree = repo.find_tree(tree_id).unwrap();

        let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig(), &sig(), msg, &tree, &parents)
            .unwrap();
    }

    /// Create an origin repo with one commit and clone it, returning
    /// `(origin_dir, origin, clone_dir, clone)`.
    fn setup() -> (PathBuf, Repository, PathBuf, Repository) {
        let origin_dir = temp_dir("origin");
        let clone_dir = temp_dir("clone");

        let origin = Repository::init(&origin_dir).unwrap();
        commit_file(&origin, "a.txt", "base\n", "initial");

        clone_repo(origin_dir.to_str().unwrap(), &clone_dir, None).unwrap();

        let clone = Repository::open(&clone_dir).unwrap();
        // `pull` needs user.name / user.email to create a merge commit,
        // exactly like `git commit`.
        let mut cfg = clone.config().unwrap();
        cfg.set_str("user.name", "hok tester").unwrap();
        cfg.set_str("user.email", "tester@example.com").unwrap();
        // Pin `core.autocrlf` so the assertions do not depend on the machine's
        // Git configuration. Git for Windows installs it as a *system* setting
        // (`<git>/etc/gitconfig`), which a repo-local value does not override,
        // and a repo-local value here is not enough on its own — see the
        // re-checkout below.
        cfg.set_bool("core.autocrlf", false).unwrap();
        drop(cfg);

        // The clone's initial checkout ran *before* the pin above, so it used
        // whatever `core.autocrlf` the machine provides. With that set to true
        // the worktree copy of `a.txt` is CRLF while its blob is LF, and once
        // the pin takes effect `ensure_updateable` compares the two and
        // correctly reports a local modification — which `pull` then refuses,
        // as `git pull` would. Nothing is wrong with the worktree; it was just
        // materialised under a filter setting that no longer applies, so
        // re-materialise it under the pinned one.
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        clone.checkout_head(Some(&mut checkout)).unwrap();

        (origin_dir, origin, clone_dir, clone)
    }

    #[test]
    fn pull_fast_forward() {
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");

        pull(&clone_dir, None::<&str>, false).unwrap();

        let repo = Repository::open(&clone_dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let origin_head = origin.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id(), origin_head.id());
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("a.txt")).unwrap(),
            "base\nremote\n"
        );
        // fast-forward must NOT create a merge commit
        assert_eq!(head.parent_count(), 1);
        // worktree and index are clean after the fast-forward
        assert!(
            repo.statuses(None)
                .unwrap()
                .iter()
                .all(|e| e.status().is_empty())
        );

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    #[test]
    fn pull_fast_forward_new_files() {
        // Remote-only additions: the files do not exist locally at all.
        // `git_status_file` reports GIT_ENOTFOUND for them, which must be
        // treated as "no local modification", not as a pull failure.
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");
        commit_file(&origin, "new.json", "{\"new\": true}\n", "add new file");

        pull(&clone_dir, None::<&str>, false).unwrap();

        let repo = Repository::open(&clone_dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let origin_head = origin.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.id(), origin_head.id());
        assert!(clone_dir.join("new.json").exists());

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    #[test]
    fn pull_fast_forward_dirty() {
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");

        // local modification on a file the update would change
        std::fs::write(clone_dir.join("a.txt"), "base\nlocal\n").unwrap();

        let err = pull(&clone_dir, None::<&str>, false).unwrap_err();
        assert!(matches!(err, Error::BucketUpdateLocalChanges(_)));

        // the update must NOT have overwritten the local change
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("a.txt")).unwrap(),
            "base\nlocal\n"
        );

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    /// With `autostash_on_conflict` on, a local modification that would block
    /// the fast-forward is stashed and the update proceeds. The stash is left
    /// in place — not popped — like upstream; the saved change is intact in it.
    #[test]
    fn pull_fast_forward_dirty_autostash() {
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");
        std::fs::write(clone_dir.join("a.txt"), "base\nlocal\n").unwrap();

        pull(&clone_dir, None::<&str>, true).unwrap();

        // the update went through instead of aborting
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("a.txt")).unwrap(),
            "base\nremote\n"
        );

        let mut repo = Repository::open(&clone_dir).unwrap();
        let mut stash: Vec<(usize, String, git2::Oid)> = Vec::new();
        repo.stash_foreach(|index, message, oid| {
            stash.push((index, message.to_owned(), *oid));
            true
        })
        .unwrap();
        assert_eq!(stash.len(), 1, "stashes: {stash:?}");
        // upstream's message shape: `WIP at <round-trip timestamp>`. git (and
        // libgit2) prefix the recorded message with `On <branch>: `.
        assert!(
            stash[0].1.contains("WIP at "),
            "unexpected stash message: {}",
            stash[0].1
        );

        // the local change is preserved inside the stash (it would be lost if
        // the stash had been popped or dropped)
        let stash_commit = repo.find_commit(stash[0].2).unwrap();
        let stash_tree = stash_commit.tree().unwrap();
        let entry = stash_tree.get_name("a.txt").unwrap();
        let blob = repo.find_blob(entry.id()).unwrap();
        assert_eq!(
            std::str::from_utf8(blob.content()).unwrap(),
            "base\nlocal\n"
        );

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    /// The stash includes untracked files (`-u`): a locally created file on a
    /// path the remote adds would otherwise abort the update.
    #[test]
    fn pull_autostash_includes_untracked() {
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "new.json", "{\"remote\": true}\n", "add new file");
        std::fs::write(clone_dir.join("new.json"), "{\"local\": true}\n").unwrap();

        // without the stash the untracked file blocks the update
        let err = pull(&clone_dir, None::<&str>, false).unwrap_err();
        assert!(matches!(err, Error::BucketUpdateLocalChanges(_)));

        pull(&clone_dir, None::<&str>, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("new.json")).unwrap(),
            "{\"remote\": true}\n"
        );

        let mut repo = Repository::open(&clone_dir).unwrap();
        let mut stash: Vec<git2::Oid> = Vec::new();
        repo.stash_foreach(|_, _, oid| {
            stash.push(*oid);
            true
        })
        .unwrap();
        assert_eq!(stash.len(), 1, "stashes: {stash:?}");
        let stash_commit = repo.find_commit(stash[0]).unwrap();
        // `-u` parks untracked files in the stash's third parent, not in the
        // stash commit's own tree (which holds the tracked changes).
        assert_eq!(stash_commit.parent_count(), 3, "untracked parent missing");
        let untracked_tree = stash_commit.parent(2).unwrap().tree().unwrap();
        let entry = untracked_tree.get_name("new.json").unwrap();
        let blob = repo.find_blob(entry.id()).unwrap();
        assert_eq!(
            std::str::from_utf8(blob.content()).unwrap(),
            "{\"local\": true}\n"
        );

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    #[test]
    fn pull_merge_commit() {
        let (origin_dir, origin, clone_dir, clone) = setup();
        // local commit on a new file
        commit_file(&clone, "b.txt", "local\n", "local work");
        // remote commit on a different file
        commit_file(&origin, "a.txt", "base\nremote\n", "remote work");

        pull(&clone_dir, None::<&str>, false).unwrap();

        let repo = Repository::open(&clone_dir).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        // diverged histories produce a merge commit with two parents
        assert_eq!(head.parent_count(), 2);
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("a.txt")).unwrap(),
            "base\nremote\n"
        );
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("b.txt")).unwrap(),
            "local\n"
        );

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    #[test]
    fn pull_merge_conflict() {
        let (origin_dir, origin, clone_dir, clone) = setup();
        // both sides edit the same file in conflicting ways
        commit_file(&clone, "a.txt", "local\n", "local edit");
        commit_file(&origin, "a.txt", "remote\n", "remote edit");

        let err = pull(&clone_dir, None::<&str>, false).unwrap_err();
        assert!(matches!(err, Error::BucketUpdateMergeConflict(_)));

        // conflict markers are left in the working tree, like `git merge`
        let content = std::fs::read_to_string(clone_dir.join("a.txt")).unwrap();
        assert!(content.contains("<<<<<<<"));
        assert!(content.contains(">>>>>>>"));

        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    // --- spawn_blocking concurrency guardrails ------------------------

    /// Two concurrent pulls (the parallel-bucket-update scenario) both
    /// succeed via the blocking pool. `RealGit::pull` offloads git2 to
    /// `tokio::task::spawn_blocking`, so joining two pulls must not deadlock
    /// and must produce the correct fast-forward result for both repos.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_pulls_succeed_via_blocking_pool() {
        let (origin_dir, origin, clone_dir, _) = setup();
        let clone2_dir = temp_dir("clone2");
        clone_repo(origin_dir.to_str().unwrap(), &clone2_dir, None).unwrap();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");

        let git = RealGit;
        let d1 = clone_dir;
        let d2 = clone2_dir;
        // Isolate clone2 from the user global core.autocrlf (like setup()
        // does for clone1) so the fast-forward checks out LF files as LF.
        // clone2 was cloned while the global autocrlf=true was in effect, so
        // its worktree a.txt is CRLF; flipping the repo config to false would
        // otherwise make libgit2 see that CRLF file as locally modified. A
        // fresh `checkout_head` re-checks out the worktree under the new
        // config (LF), leaving it clean for the fast-forward status check.
        // FORCE is required: the default (SAFE) strategy would refuse to
        // overwrite the CRLF worktree file (it looks locally modified).
        let repo = Repository::open(&d2).unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        let mut co = git2::build::CheckoutBuilder::new();
        co.force();
        repo.checkout_head(Some(&mut co)).unwrap();

        let (r1, r2) = tokio::join!(git.pull(&d1, None, false), git.pull(&d2, None, false));
        r1.unwrap();
        r2.unwrap();

        for dir in [&d1, &d2] {
            let repo = Repository::open(dir).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            let origin_head = origin.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(head.id(), origin_head.id());
            assert_eq!(
                std::fs::read_to_string(dir.join("a.txt")).unwrap(),
                "base\nremote\n"
            );
            cleanup(dir);
        }
        cleanup(&origin_dir);
    }

    /// A slow blocking pull (git2 on the blocking pool) must not occupy the
    /// runtime worker: a concurrent short async timer fires on schedule.
    /// Before the spawn_blocking offload, the pull blocked the worker and the
    /// timer was delayed until the pull finished.
    #[tokio::test]
    async fn blocking_pull_does_not_starve_runtime() {
        let (origin_dir, origin, clone_dir, _) = setup();
        commit_file(&origin, "a.txt", "base\nremote\n", "remote update");
        let git = RealGit;

        let pull_task = {
            let dir = clone_dir.clone();
            tokio::spawn(async move { git.pull(&dir, None, false).await.unwrap() })
        };

        let start = std::time::Instant::now();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "runtime worker was starved by blocking pull (elapsed {:?})",
            elapsed
        );

        pull_task.await.unwrap();
        cleanup(&origin_dir);
        cleanup(&clone_dir);
    }

    /// `git log -1 -- <path>` semantics: the newest commit that actually
    /// touched the file, not simply HEAD.
    #[test]
    fn last_commit_for_file_skips_commits_that_did_not_touch_it() {
        let dir = temp_dir("last-commit");
        let repo = Repository::init(&dir).unwrap();
        commit_file(&repo, "touched.json", "{}\n", "add touched");
        commit_file(&repo, "other.json", "{}\n", "add other");

        let (secs, author) =
            last_commit_for_file(dir.join("touched.json")).expect("the touching commit");
        assert_eq!(author, "hok tester");
        // The test signature pins the author time to the Unix epoch.
        assert_eq!(secs, 0);

        // A file no commit ever carried has no history…
        std::fs::write(dir.join("untracked.json"), "{}\n").unwrap();
        assert!(last_commit_for_file(dir.join("untracked.json")).is_none());
        // …and neither has a file outside any repository.
        let plain = temp_dir("last-commit-plain");
        std::fs::write(plain.join("x.json"), "{}\n").unwrap();
        assert!(last_commit_for_file(plain.join("x.json")).is_none());

        cleanup(&dir);
        cleanup(&plain);
    }

    /// `git log -1 -- <path>` skips a merge that merely carried the change into
    /// HEAD (it is "treesame" with that parent) and reports the commit that
    /// actually made it.
    #[test]
    fn last_commit_for_file_simplifies_treesame_merges() {
        let dir = temp_dir("last-commit-merge");
        let repo = Repository::init(&dir).unwrap();
        commit_file(&repo, "manifest.json", "{\"v\":1}\n", "base");
        let base = repo.head().unwrap().peel_to_commit().unwrap();

        // The side branch changes the manifest…
        let changed = tree_with(
            &repo,
            &base.tree().unwrap(),
            &[("manifest.json", "{\"v\":2}\n")],
        );
        let side = repo
            .commit(
                None,
                &sig_named("side author"),
                &sig_named("side author"),
                "side change",
                &changed,
                &[&base],
            )
            .unwrap();
        // …while main moves on without touching it, then merges the side
        // branch: the merge's tree for the manifest is the side parent's, so
        // git does not count the merge itself.
        let elsewhere = tree_with(&repo, &base.tree().unwrap(), &[("other.txt", "x\n")]);
        let main = repo
            .commit(
                // main advances HEAD: the merge's first parent must be the tip.
                Some("HEAD"),
                &sig_named("main author"),
                &sig_named("main author"),
                "main change",
                &elsewhere,
                &[&base],
            )
            .unwrap();
        // `Repository::commit` returns the new commit id; load both parents.
        let (main, side) = (
            repo.find_commit(main).unwrap(),
            repo.find_commit(side).unwrap(),
        );
        let merge = repo
            .commit(
                Some("HEAD"),
                &sig_named("merge author"),
                &sig_named("merge author"),
                "merge",
                &changed,
                &[&main, &side],
            )
            .unwrap();
        assert_eq!(repo.head().unwrap().target().unwrap(), merge);

        let (_, author) =
            last_commit_for_file(dir.join("manifest.json")).expect("the treesame merge is skipped");
        assert_eq!(author, "side author");

        cleanup(&dir);
    }

    /// A file that is not in the repository has no history at all, even though
    /// the root commit has no parent to compare against.
    #[test]
    fn last_commit_for_file_ignores_untracked_paths() {
        let dir = temp_dir("last-commit-untracked");
        let repo = Repository::init(&dir).unwrap();
        commit_file(&repo, "tracked.json", "{}\n", "base");

        let untracked = dir.join("untracked.json");
        std::fs::write(&untracked, "{}\n").unwrap();
        assert!(last_commit_for_file(&untracked).is_none());

        cleanup(&dir);
    }

    // --- manifest-history walk primitives --------------------------------

    /// A fixture matching the seed scan's real shape: manifests live under
    /// `bucket/`, root-level JSON (`package.json`) must stay invisible, and
    /// each commit bumps exactly one manifest.
    fn history_fixture(name: &str) -> (PathBuf, Repository) {
        let dir = temp_dir(name);
        let repo = Repository::init(&dir).unwrap();
        commit_file(&repo, "bucket/foo.json", r#"{"version": "1.0"}"#, "add foo");
        commit_file(&repo, "bucket/bar.json", r#"{"version": "1.0"}"#, "add bar");
        commit_file(
            &repo,
            "bucket/foo.json",
            r#"{"version": "2.0"}"#,
            "bump foo",
        );
        commit_file(&repo, "bucket/notes.txt", "notes\n", "add non-manifest");
        commit_file(&repo, "package.json", "{}\n", "add root package");
        (dir, repo)
    }

    /// The chain with each entry's deltas: the shape the seed indexes, now
    /// that the product walks the chain and diffs per chunk.
    fn walk_with_deltas(
        history: &HistoryRepo,
        head: &str,
    ) -> Vec<(HistoryCommit, Vec<HistoryDelta>)> {
        history
            .walk_chain(head, None, None)
            .map(|entry| {
                let deltas = history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref());
                (entry, deltas)
            })
            .collect()
    }

    /// The walk yields every commit from HEAD to the root exactly once, in
    /// first-parent order, with manifest deltas on every step.
    #[test]
    fn history_walk_covers_the_whole_first_parent_chain() {
        let (dir, _repo) = history_fixture("history-walk");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let chain: Vec<HistoryCommit> = history.walk_chain(&head, None, None).collect();

        // Walk order is newest-first: root package, notes, bump, add-bar,
        // add-foo. The bump is entry 2; the root commit is the last entry.
        assert_eq!(chain.len(), 5, "one entry per commit, root included");
        assert_eq!(chain[0].commit, head);
        // The chain walk reads no trees, and hands the parallel diff exactly
        // the two tree OIDs it needs: the commit's and its first parent's.
        assert!(!chain[0].tree.is_empty());
        assert_eq!(
            chain[0].parent_tree.as_deref(),
            Some(chain[1].tree.as_str())
        );
        assert!(
            chain.last().unwrap().parent_tree.is_none(),
            "a root commit has no parent tree — the seed's end-of-chain signal"
        );

        let steps = walk_with_deltas(&history, &head);
        let deltas: usize = steps.iter().map(|(_, deltas)| deltas.len()).sum();
        assert_eq!(deltas, 3, "three manifest-touching commits");
        assert_eq!(steps[2].1.len(), 1, "the bump commit only changed foo");
        assert_eq!(steps[2].1[0].path, "bucket/foo.json");
        assert!(!steps[2].1[0].deleted);

        // The root commit is `add foo` (bar came later): diffed against the
        // empty tree it carries exactly the one manifest added there, and
        // `package.json` never appears at all.
        let root = &steps[4].1;
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].path, "bucket/foo.json");
        assert!(
            steps
                .iter()
                .flat_map(|(_, deltas)| deltas)
                .all(|d| d.path.ends_with(".json") && !d.path.ends_with("package.json")),
            "the manifest filter must match diff_manifest_changes"
        );

        // Every non-deleted delta carries a real blob OID, and the recorded
        // author time is the commit's.
        assert!(
            steps
                .iter()
                .flat_map(|(_, deltas)| deltas)
                .filter(|d| !d.deleted)
                .all(|d| d.blob_oid.is_some())
        );
        assert!(chain.iter().all(|c| c.committed_at == 0), "pinned sig time");

        cleanup(&dir);
    }

    /// `blob_text` reads exactly the bytes the index records, and answers
    /// `None` for an object it cannot read — the read path's invalidation
    /// signal (`gc --prune` removes the blob).
    #[test]
    fn history_blob_text_reads_the_indexed_manifest() {
        let (dir, _repo) = history_fixture("history-blob");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();

        let bumped = walk_with_deltas(&history, &head)
            .into_iter()
            .flat_map(|(_, deltas)| deltas)
            .find(|d| d.path == "bucket/foo.json")
            .and_then(|d| d.blob_oid)
            .unwrap();
        assert_eq!(
            history.blob_text(&bumped).as_deref(),
            Some(r#"{"version": "2.0"}"#),
            "the newest foo blob is the bumped version"
        );

        // An OID the repository does not hold is unreadable, not an error.
        assert!(
            history
                .blob_text("4444444444444444444444444444444444444444")
                .is_none()
        );

        cleanup(&dir);
    }

    /// Resume semantics: `start` is inclusive (the first entry is the stored
    /// baseline itself), `until` stops after the named commit, and an
    /// unreachable `start` falls back to HEAD rather than yielding nothing.
    #[test]
    fn history_walk_resume_bounds_are_inclusive() {
        let (dir, _repo) = history_fixture("history-resume");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let chain: Vec<String> = history
            .walk_chain(&head, None, None)
            .map(|c| c.commit)
            .collect();
        let base = &chain[3];

        // Inclusive start: the baseline commit itself is yielded first, then
        // every older commit down to the root.
        let tail: Vec<String> = history
            .walk_chain(&head, Some(base), None)
            .map(|c| c.commit)
            .collect();
        assert_eq!(tail, chain[3..]);

        // Inclusive stop: everything from HEAD down to (and including) the
        // baseline — this is what bounds an incremental scan.
        let bounded: Vec<String> = history
            .walk_chain(&head, None, Some(base))
            .map(|c| c.commit)
            .collect();
        assert_eq!(bounded, chain[..4]);

        // An unreachable stored baseline degrades to a full walk from HEAD.
        let ghost = "1111111111111111111111111111111111111111";
        assert_eq!(history.walk_chain(&head, Some(ghost), None).count(), 5);
        assert!(!history.is_ancestor(&head, ghost));
        // Reachability is directional: HEAD is not reachable from its parent.
        assert!(history.is_ancestor(&head, &chain[1]));
        assert!(!history.is_ancestor(&chain[1], &head));

        cleanup(&dir);
    }

    /// An unreachable stop commit yields the full walk instead of stopping at
    /// nothing — the caller still detects "baseline not advanced" by checking
    /// whether the recorded commit was reached.
    #[test]
    fn history_walk_unknown_stop_yields_the_full_walk() {
        let (dir, _repo) = history_fixture("history-stop");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let ghost = "2222222222222222222222222222222222222222";
        assert_eq!(history.walk_chain(&head, None, Some(ghost)).count(), 5);

        cleanup(&dir);
    }

    /// A deleted manifest appears with `deleted: true` and no blob OID —
    /// the history index must not drop its row for a deletion.
    #[test]
    fn history_walk_reports_deletions_without_a_blob() {
        let dir = temp_dir("history-delete");
        let repo = Repository::init(&dir).unwrap();
        commit_file(&repo, "bucket/gone.json", r#"{"version": "1.0"}"#, "add");
        // Remove the file from the working tree, then commit the deletion.
        std::fs::remove_file(dir.join("bucket").join("gone.json")).unwrap();
        let mut index = repo.index().unwrap();
        index
            .remove_path(std::path::Path::new("bucket/gone.json"))
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree_to(&repo).unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig(), &sig(), "remove", &tree, &[&parent])
            .unwrap();

        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let deleted = walk_with_deltas(&history, &head)
            .into_iter()
            .flat_map(|(_, deltas)| deltas)
            .find(|d| d.path == "bucket/gone.json")
            .unwrap();
        assert!(deleted.deleted);
        assert!(deleted.blob_oid.is_none());

        cleanup(&dir);
    }

    /// Baseline validation walks the same chain as the indexer: a stored
    /// baseline that is still reachable answers true, one from a rewritten
    /// history answers false.
    #[test]
    fn history_is_ancestor_matches_the_walk_chain() {
        let (dir, _repo) = history_fixture("history-ancestor");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let chain: Vec<String> = history
            .walk_chain(&head, None, None)
            .map(|c| c.commit)
            .collect();

        // Every commit the walk yields is reachable from HEAD, and so is HEAD.
        for commit in &chain {
            assert!(
                history.is_ancestor(&head, commit),
                "walk entry {commit} must be reachable from {head}"
            );
        }
        assert!(history.is_ancestor(&head, &head));
        // A commit that does not exist must force a re-seed.
        assert!(!history.is_ancestor(&head, "3333333333333333333333333333333333333333"));

        cleanup(&dir);
    }

    /// A plain directory bucket has no repository to index — `open` is the
    /// single place that decides this, so every caller can skip uniformly.
    #[test]
    fn history_repo_open_rejects_a_plain_directory() {
        let plain = temp_dir("history-plain");
        std::fs::create_dir_all(plain.join("bucket")).unwrap();
        assert!(HistoryRepo::open(&plain).is_none());

        // An initialised repository with no commits has no HEAD to walk.
        let empty = temp_dir("history-empty");
        Repository::init(&empty).unwrap();
        assert!(HistoryRepo::open(&empty).unwrap().head().is_none());

        cleanup(&plain);
        cleanup(&empty);
    }

    /// The normalization table was evaluated against upstream's
    /// `Convert-RepositoryUri` (`lib/buckets.ps1:83-102`) in Windows
    /// PowerShell 5.1 before the port, so hok accepts exactly the URLs Scoop
    /// accepts and derives the same `provider/user/repo` key from them.
    #[test]
    fn normalize_git_uri_matches_upstream() {
        let cases: [(&str, Option<&str>); 16] = [
            (
                "https://github.com/ScoopInstaller/Extras.git",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "https://github.com/ScoopInstaller/Extras",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "git@github.com:ScoopInstaller/Extras.git",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "ssh://git@github.com/ScoopInstaller/Extras.git",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "https://github.com:443/ScoopInstaller/Extras.git",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "https://user:pass@github.com/ScoopInstaller/Extras.git",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            (
                "git://github.com/owner/repo.git",
                Some("github.com/owner/repo"),
            ),
            (
                "https://gitee.com/owner/repo.git",
                Some("gitee.com/owner/repo"),
            ),
            ("file:///D:/buckets/local", Some("D//buckets/local")),
            ("D:\\local\\bucket", None),
            (
                "https://github.com/owner/repo/tree/main",
                Some("github.com/owner/repo/tree/main"),
            ),
            ("https://github.com/owner", None),
            ("not-a-url", None),
            ("", None),
            (
                "https://github.com/ScoopInstaller/Extras/",
                Some("github.com/ScoopInstaller/Extras"),
            ),
            ("https://github.com/a/b/c/d", Some("github.com/a/b/c/d")),
        ];

        for (input, expected) in cases {
            assert_eq!(
                normalize_git_uri(input).as_deref(),
                expected,
                "on `{input}`"
            );
        }
    }

    /// Two spellings of one repository normalize to the same key; the
    /// duplicate scan then compares case-insensitively (PowerShell's `-eq`).
    #[test]
    fn normalize_git_uri_folds_remote_spellings() {
        let https = normalize_git_uri("https://github.com/ScoopInstaller/Extras.git").unwrap();
        let ssh = normalize_git_uri("git@github.com:ScoopInstaller/Extras.git").unwrap();
        assert_eq!(https, ssh);
        assert!(https.eq_ignore_ascii_case("GITHUB.COM/scoopinstaller/extras"));
    }

    /// `ls_remote` reads a repository over the local transport and fails on a
    /// path that is not one — no clone, no temporary directory left behind.
    #[test]
    fn ls_remote_reads_a_local_repository() {
        let origin = temp_dir("ls-remote-origin");
        let repo = Repository::init(&origin).unwrap();
        commit_file(&repo, "a.txt", "a\n", "base");

        assert!(ls_remote(origin.to_str().unwrap(), None).is_ok());

        let not_a_repo = origin.join("nested");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        assert!(ls_remote(not_a_repo.to_str().unwrap(), None).is_err());

        cleanup(&origin);
    }

    /// A repository that advertises no refs is still reachable — and, unlike
    /// `RemoteConnection::list()`, must not abort the process.
    #[test]
    fn ls_remote_accepts_an_empty_repository() {
        let origin = temp_dir("ls-remote-empty");
        Repository::init_bare(&origin).unwrap();

        assert!(ls_remote(origin.to_str().unwrap(), None).is_ok());

        cleanup(&origin);
    }

    /// `remote_url_containing` resolves the remote from a nested directory the
    /// way `git remote get-url origin` does, while `remote_url_of` — the
    /// bucket-facing lookup — only accepts the repository root, so an
    /// unrelated enclosing repository is never picked up.
    #[test]
    fn remote_url_containing_searches_parent_directories() {
        let root = temp_dir("remote-url-nested");
        let repo = Repository::init(&root).unwrap();
        let nested = root.join("buckets").join("extras");
        std::fs::create_dir_all(&nested).unwrap();
        repo.remote("origin", "https://github.com/ScoopInstaller/Extras.git")
            .unwrap();

        assert_eq!(
            remote_url_containing(&nested, "origin").unwrap().as_deref(),
            Some("https://github.com/ScoopInstaller/Extras.git")
        );
        assert!(remote_url_of(&root, "origin").unwrap().is_some());
        assert!(remote_url_of(&nested, "origin").is_err());

        cleanup(&root);
    }

    // --- the memoized manifest diff ---------------------------------------

    /// A blob object holding `content`.
    fn blob_of(repo: &Repository, content: &str) -> git2::Oid {
        repo.blob(content.as_bytes()).unwrap()
    }

    /// A tree object built from `(name, filemode, id)` entries.
    ///
    /// libgit2 sorts the builder's entries into git's tree order when it is
    /// written, so a fixture built this way is as well-formed as a bucket's
    /// own trees — which is what the differ's merge assumes.
    fn tree_of(repo: &Repository, entries: &[(&str, i32, git2::Oid)]) -> git2::Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, mode, oid) in entries {
            builder.insert(*name, *oid, *mode).unwrap();
        }
        builder.write().unwrap()
    }

    /// Commit `tree` on HEAD, as the root commit when HEAD is unborn.
    fn commit_tree(repo: &Repository, tree: git2::Oid, msg: &str) -> String {
        let tree = repo.find_tree(tree).unwrap();
        let parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig(), &sig(), msg, &tree, &parents)
            .unwrap()
            .to_string()
    }

    /// The deltas libgit2's own tree diff produces for one step — the oracle
    /// [`HistoryRepo::manifest_deltas`] must agree with now that it compares
    /// parsed trees itself.
    ///
    /// Deliberately independent of the production walk: it asks
    /// `diff_tree_to_tree` for the deltas and applies the manifest filter.
    fn libgit2_deltas(
        repo: &Repository,
        new_tree: &str,
        old_tree: Option<&str>,
    ) -> Vec<HistoryDelta> {
        let new_tree = repo
            .find_tree(git2::Oid::from_str(new_tree).unwrap())
            .unwrap();
        let old_tree =
            old_tree.map(|oid| repo.find_tree(git2::Oid::from_str(oid).unwrap()).unwrap());
        let diff = repo
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)
            .unwrap();

        let mut out = Vec::new();
        for delta in diff.deltas() {
            let deleted = matches!(delta.status(), Delta::Deleted);
            let candidate = if deleted {
                delta.old_file().path()
            } else {
                delta.new_file().path()
            };
            let Some(path) = candidate else {
                continue;
            };
            let path = path.to_string_lossy().into_owned();
            if !is_manifest_path(&path) {
                continue;
            }
            let oid = delta.new_file().id();
            out.push(HistoryDelta {
                path,
                deleted,
                blob_oid: (!deleted && oid != git2::Oid::zero()).then(|| oid.to_string()),
            });
        }
        out
    }

    /// A history that exercises every shape the memoized differ must
    /// reproduce: a directory and a file colliding as `foo/` and `foo.json`,
    /// a nested directory, additions, deletions, a mode-only change, a
    /// file/directory swap whose sibling orders differently under git's tree
    /// rule than by path, a submodule entry, a root commit, and the
    /// `package.json` the filter drops.
    fn rich_history_fixture(name: &str) -> (PathBuf, Repository) {
        let dir = temp_dir(name);
        let repo = Repository::init(&dir).unwrap();

        let one = blob_of(&repo, r#"{"version": "1.0"}"#);
        let two = blob_of(&repo, r#"{"version": "2.0"}"#);
        let bar = blob_of(&repo, r#"{"version": "1.0"}"#);
        let foo_json = blob_of(&repo, r#"{"version": "1.0"}"#);
        let deep = blob_of(&repo, r#"{"version": "1.0"}"#);
        let package = blob_of(&repo, r#"{"name": "not-a-manifest"}"#);
        let plain = blob_of(&repo, "plain\n");

        let foo_dir = tree_of(&repo, &[("bar.json", 0o100644, bar)]);
        let sub_dir = tree_of(&repo, &[("deep.json", 0o100644, deep)]);

        // The root commit carries everything the later steps delete or move.
        let root = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[
                    ("a.json", 0o100644, one),
                    ("foo", 0o040000, foo_dir),
                    ("foo.json", 0o100644, foo_json),
                    ("package.json", 0o100644, package),
                    ("sub", 0o040000, sub_dir),
                ],
            ),
            "root",
        );

        // A bump, a deletion, a change inside the colliding directory and a
        // filemode change, all in one step.
        let bar2 = blob_of(&repo, r#"{"version": "2.0"}"#);
        commit_tree(
            &repo,
            tree_of(
                &repo,
                &[
                    ("a.json", 0o100755, two),
                    (
                        "foo",
                        0o040000,
                        tree_of(&repo, &[("bar.json", 0o100644, bar2)]),
                    ),
                    ("package.json", 0o100644, package),
                    ("sub", 0o040000, sub_dir),
                ],
            ),
            "bump a.json, drop foo.json, set the exec bit",
        );

        // `a.json` becomes a directory while its sibling `a.json.bak` appears:
        // a directory's name takes part in the tree comparison as `a.json/`,
        // which orders those two differently from their paths.
        let backup = blob_of(&repo, r#"{"version": "1.0"}"#);
        let with_submodule = tree_of(
            &repo,
            &[
                (
                    "a.json",
                    0o040000,
                    tree_of(&repo, &[("inner.json", 0o100644, two)]),
                ),
                ("a.json.bak", 0o100644, backup),
                (
                    "foo",
                    0o040000,
                    tree_of(&repo, &[("bar.json", 0o100644, bar2)]),
                ),
                ("mod.json", 0o160000, git2::Oid::from_str(&root).unwrap()),
                ("package.json", 0o100644, package),
                ("sub", 0o040000, sub_dir),
            ],
        );
        let swapped = commit_tree(&repo, with_submodule, "a.json becomes a directory");

        // …and back: the directory is a plain file again, the submodule moves,
        // `foo` becomes a file and `sub/` disappears.
        commit_tree(
            &repo,
            tree_of(
                &repo,
                &[
                    ("a.json", 0o100644, two),
                    ("a.json.bak", 0o100644, backup),
                    ("foo", 0o100644, plain),
                    ("mod.json", 0o160000, git2::Oid::from_str(&swapped).unwrap()),
                    ("package.json", 0o100644, package),
                ],
            ),
            "swaps back and moves the submodule",
        );

        (dir, repo)
    }

    /// The memoized differ produces exactly what libgit2's tree diff produces
    /// — the same deltas in the same order — on every step of a history that
    /// covers the shapes a real bucket has.
    #[test]
    fn history_deltas_match_libgit2_on_every_commit() {
        let (dir, repo) = rich_history_fixture("history-matches-libgit2");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();

        let steps: Vec<HistoryCommit> = history.walk_chain(&head, None, None).collect();
        assert_eq!(steps.len(), 4, "four commits, the root included");
        for entry in &steps {
            assert_eq!(
                history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref()),
                libgit2_deltas(&repo, &entry.tree, entry.parent_tree.as_deref()),
                "commit {}",
                entry.commit
            );
        }

        cleanup(&dir);
    }

    /// The seed diffs commit N against N+1 and then N+1 against N+2, so the
    /// tree of N+1 is the old side of one step and the new side of the next:
    /// the memo must hold it rather than read it again. (The timing gate is
    /// what proves the saving; this pins that the entry is really there.)
    #[test]
    fn history_diff_keeps_the_parent_tree_memoized() {
        let (dir, _repo) = rich_history_fixture("history-memo");
        let history = HistoryRepo::open(&dir).unwrap();
        let head = history.head().unwrap();
        let steps: Vec<HistoryCommit> = history.walk_chain(&head, None, None).collect();

        for pair in steps.windows(2) {
            let shared = pair[1].tree.clone();
            history.manifest_deltas(&pair[0].tree, pair[0].parent_tree.as_deref());
            history.manifest_deltas(&pair[1].tree, pair[1].parent_tree.as_deref());
            assert!(
                history.is_cached(&shared),
                "the tree shared by two steps must stay memoized: {shared}"
            );
        }

        cleanup(&dir);
    }

    /// A directory's name takes part in the tree comparison as `name/`, so the
    /// file `foo.json` sorts before the directory `foo`. Comparing raw bytes
    /// instead puts `foo` first, misaligns every descent into it and reports
    /// the untouched subtree as changed — the 34x delta count of plan §4.4.
    #[test]
    fn history_diff_orders_a_file_against_its_colliding_directory() {
        let dir = temp_dir("history-base-name");
        let repo = Repository::init(&dir).unwrap();
        let one = blob_of(&repo, r#"{"version": "1.0"}"#);
        let two = blob_of(&repo, r#"{"version": "2.0"}"#);
        let child = tree_of(&repo, &[("bar.json", 0o100644, one)]);

        commit_tree(
            &repo,
            tree_of(
                &repo,
                &[("foo", 0o040000, child), ("foo.json", 0o100644, one)],
            ),
            "root",
        );
        // Only `foo.json` changes; the directory is untouched.
        let head = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[("foo", 0o040000, child), ("foo.json", 0o100644, two)],
            ),
            "bump foo.json",
        );

        let history = HistoryRepo::open(&dir).unwrap();
        let entry = history.walk_chain(&head, None, None).next().unwrap();
        let deltas = history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref());
        assert_eq!(deltas.len(), 1, "only foo.json changed, got {deltas:?}");
        assert_eq!(deltas[0].path, "foo.json");
        assert_eq!(
            deltas[0].blob_oid.as_deref(),
            Some(two.to_string().as_str())
        );

        cleanup(&dir);
    }

    /// A change of a file's basic type is a delete and an add, exactly as
    /// libgit2 splits it (it reports a type-change delta only when the caller
    /// asks for one), while a mode-only change is a single modified delta that
    /// still carries the blob.
    #[test]
    fn history_diff_splits_file_type_changes() {
        let dir = temp_dir("history-typechange");
        let repo = Repository::init(&dir).unwrap();
        let one = blob_of(&repo, r#"{"version": "1.0"}"#);
        let target = blob_of(&repo, "x.json");

        commit_tree(&repo, tree_of(&repo, &[("x.json", 0o100644, one)]), "root");
        let mode = commit_tree(
            &repo,
            tree_of(&repo, &[("x.json", 0o100755, one)]),
            "exec bit",
        );
        let symlink = commit_tree(
            &repo,
            tree_of(&repo, &[("x.json", 0o120000, target)]),
            "symlink",
        );

        let history = HistoryRepo::open(&dir).unwrap();
        let steps: Vec<HistoryCommit> = history.walk_chain(&symlink, None, None).collect();
        let deltas_of = |commit: &str| {
            let entry = steps
                .iter()
                .find(|entry| entry.commit == commit)
                .expect("the commit is on the chain");
            history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref())
        };

        assert_eq!(
            deltas_of(&mode),
            [HistoryDelta {
                path: "x.json".to_owned(),
                deleted: false,
                blob_oid: Some(one.to_string()),
            }],
            "a mode-only change keeps the blob"
        );
        assert_eq!(
            deltas_of(&symlink),
            [
                HistoryDelta {
                    path: "x.json".to_owned(),
                    deleted: true,
                    blob_oid: None,
                },
                HistoryDelta {
                    path: "x.json".to_owned(),
                    deleted: false,
                    blob_oid: Some(target.to_string()),
                },
            ],
            "a type change is a delete and an add"
        );

        cleanup(&dir);
    }

    /// A directory that appears or disappears on one side, and a file/directory
    /// swap, are expanded to one delta per manifest under them — never to a
    /// delta for the directory. A submodule (gitlink) entry is a leaf like any
    /// other and its id is the commit it records.
    #[test]
    fn history_diff_expands_directories_and_submodule_entries() {
        let dir = temp_dir("history-directories");
        let repo = Repository::init(&dir).unwrap();
        let x = blob_of(&repo, r#"{"version": "1.0"}"#);
        let z = blob_of(&repo, r#"{"version": "2.0"}"#);

        let root = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[("d", 0o040000, tree_of(&repo, &[("x.json", 0o100644, x)]))],
            ),
            "root",
        );
        // The directory gains a manifest and a file the filter drops, and a
        // submodule entry appears beside it.
        let grown_dir = tree_of(
            &repo,
            &[
                ("x.json", 0o100644, x),
                ("notes.txt", 0o100644, x),
                ("z.json", 0o100644, z),
            ],
        );
        let grown = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[
                    ("d", 0o040000, grown_dir),
                    ("m.json", 0o160000, git2::Oid::from_str(&root).unwrap()),
                ],
            ),
            "grow the directory, add a submodule",
        );
        let moved = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[
                    ("d", 0o040000, grown_dir),
                    ("m.json", 0o160000, git2::Oid::from_str(&grown).unwrap()),
                ],
            ),
            "move the submodule",
        );
        let gone = commit_tree(
            &repo,
            tree_of(
                &repo,
                &[("m.json", 0o160000, git2::Oid::from_str(&grown).unwrap())],
            ),
            "drop the directory",
        );

        let history = HistoryRepo::open(&dir).unwrap();
        let steps: Vec<HistoryCommit> = history.walk_chain(&gone, None, None).collect();
        let deltas_of = |commit: &str| {
            let entry = steps
                .iter()
                .find(|entry| entry.commit == commit)
                .expect("the commit is on the chain");
            history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref())
        };
        let added = |path: &str, oid: git2::Oid| HistoryDelta {
            path: path.to_owned(),
            deleted: false,
            blob_oid: Some(oid.to_string()),
        };
        let deleted = |path: &str| HistoryDelta {
            path: path.to_owned(),
            deleted: true,
            blob_oid: None,
        };

        assert_eq!(
            deltas_of(&grown),
            [
                added("d/z.json", z),
                added("m.json", git2::Oid::from_str(&root).unwrap()),
            ],
            "the new manifest and the submodule, not the directory"
        );
        assert_eq!(
            deltas_of(&moved),
            [added("m.json", git2::Oid::from_str(&grown).unwrap())],
            "a moved submodule is one modified delta"
        );
        assert_eq!(
            deltas_of(&gone),
            [deleted("d/x.json"), deleted("d/z.json")],
            "a dropped directory is one delete per manifest"
        );

        cleanup(&dir);
    }
}
