//! Git operations.
//!
//! Currently uses `git2` (libgit2 bindings). A pure-Rust `gix` version was
//! attempted but abandoned because gix's dependency tree is prohibitively
//! large (aws-lc-rs, quinn-proto, h2, icu — causing 20+ min first build).

use git2::{CredentialType, FetchOptions, Repository};
use regex::Regex;
use std::{collections::HashMap, path::Path, result::Result, sync::LazyLock};

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
// (matching `git pull`); otherwise they are preserved.
pub fn pull<P, S>(path: P, proxy: Option<S>) -> Fallible<()>
where
    P: AsRef<Path>,
    S: AsRef<str>,
{
    let proxy = proxy.as_ref().map(|s| s.as_ref());
    let repo = Repository::open(path.as_ref())?;
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

    let fetch_commit = repo.find_reference("FETCH_HEAD")?.peel_to_commit()?;

    if fetch_commit.id() == head_id {
        return Ok(()); // already up to date
    }

    let fetch_annotated = repo.find_annotated_commit(fetch_commit.id())?;
    let (analysis, _preference) = repo.merge_analysis(&[&fetch_annotated])?;

    // `git pull` refuses to overwrite local changes: fail before touching the
    // working tree if any file the update would change is locally modified.
    ensure_updateable(&repo, head_id, &fetch_commit)?;

    if analysis.is_fast_forward() {
        // Fast-forward: hard-reset the branch to the fetched commit (updates
        // the ref, index and working tree in one step). Safe because
        // ensure_updateable() above guarantees no local modification would
        // be overwritten.
        let reset_target: git2::Object =
            repo.find_object(fetch_commit.id(), Some(git2::ObjectType::Commit))?;
        repo.reset(&reset_target, git2::ResetType::Hard, None)?;

        Ok(())
    } else if analysis.is_normal() {
        // Diverged: perform a 3-way merge like `git pull` (merge mode).
        // SAFE checkout aborts if the merge would overwrite local changes.
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
        let remote_commit = repo.find_commit(fetch_commit.id())?;
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
fn ensure_updateable(repo: &Repository, head_id: git2::Oid, target: &git2::Commit) -> Fallible<()> {
    let old_tree = repo.find_commit(head_id)?.tree()?;
    let new_tree = target.tree()?;
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

pub fn remote_url_of<S>(repo_path: &Path, remote: S) -> Fallible<Option<String>>
where
    S: AsRef<str>,
{
    let repo = Repository::open(repo_path)?;
    let remote = repo.find_remote(remote.as_ref())?;
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

    /// Update a bucket repo with `git pull` semantics.
    async fn pull(&self, path: &Path, proxy: Option<&str>) -> Fallible<()>;

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

    async fn pull(&self, path: &Path, proxy: Option<&str>) -> Fallible<()> {
        // Same blocking-pool offload as clone_repo: git2 pull (fetch + merge)
        // is blocking I/O. Each bucket pull runs on the blocking pool.
        let path = path.to_owned();
        let proxy = proxy.map(|s| s.to_owned());
        tokio::task::spawn_blocking(move || pull(path, proxy))
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
    use git2::{Signature, Time};
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

    /// Write a file in the working tree and commit it on HEAD.
    fn commit_file(repo: &Repository, rel: &str, content: &str, msg: &str) {
        let full = repo.workdir().unwrap().join(rel);
        std::fs::write(&full, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(rel)).unwrap();
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

        pull(&clone_dir, None::<&str>).unwrap();

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

        pull(&clone_dir, None::<&str>).unwrap();

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

        let err = pull(&clone_dir, None::<&str>).unwrap_err();
        assert!(matches!(err, Error::BucketUpdateLocalChanges(_)));

        // the update must NOT have overwritten the local change
        assert_eq!(
            std::fs::read_to_string(clone_dir.join("a.txt")).unwrap(),
            "base\nlocal\n"
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

        pull(&clone_dir, None::<&str>).unwrap();

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

        let err = pull(&clone_dir, None::<&str>).unwrap_err();
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

        let (r1, r2) = tokio::join!(git.pull(&d1, None), git.pull(&d2, None));
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
            tokio::spawn(async move { git.pull(&dir, None).await.unwrap() })
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
}
