//! Git operations.
//!
//! Currently uses `git2` (libgit2 bindings). A pure-Rust `gix` version was
//! attempted but abandoned because gix's dependency tree is prohibitively
//! large (aws-lc-rs, quinn-proto, h2, icu — causing 20+ min first build).

use git2::{CredentialType, FetchOptions, Repository};
use std::{collections::HashMap, path::Path, result::Result};

use crate::error::{Error, Fallible};

fn fetch_options(proxy: Option<&str>) -> FetchOptions<'static> {
    let mut fo = FetchOptions::new();
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

    fo.remote_callbacks(cb);

    if let Some(proxy) = proxy {
        let mut proxy = proxy.to_owned();

        if !(proxy.starts_with("http") || proxy.starts_with("socks")) {
            proxy.insert_str(0, "http://");
        }

        let mut po = git2::ProxyOptions::new();
        po.url(proxy.as_str());
        fo.proxy_options(po);
    }

    fo
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
                && !status.is_empty() {
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

    /// Update a bucket repo with `git pull` semantics.
    async fn pull(&self, path: &Path, proxy: Option<&str>) -> Fallible<()>;

    /// Get the URL of a remote in a repository.
    fn remote_url_of(&self, repo_path: &Path, remote: &str) -> Fallible<Option<String>>;

    /// Get the HEAD commit author time (Unix seconds) of a repository.
    fn head_commit_time(&self, path: &Path) -> Option<i64>;
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

    fn remote_url_of(&self, repo_path: &Path, remote: &str) -> Fallible<Option<String>> {
        remote_url_of(repo_path, remote)
    }

    fn head_commit_time(&self, path: &Path) -> Option<i64> {
        head_commit_time(path)
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
        // Isolate the test from the user's global `core.autocrlf`: the temp
        // repo has no .gitattributes, so a global autocrlf=true would make
        // libgit2 check out LF files as CRLF and break content assertions.
        cfg.set_bool("core.autocrlf", false).unwrap();

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
}
