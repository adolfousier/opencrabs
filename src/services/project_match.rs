//! Which project a working directory belongs to.
//!
//! Sessions get an auto-generated title on their first turn but were never
//! linked to a project at that moment. The only path that created the link was
//! the `/cd` handler, so a session carried a correct `working_directory` and a
//! `project_id` of NULL unless someone happened to change directory by hand,
//! and every per-project view and cost rollup under-reported by however many
//! sessions that was (#1445).
//!
//! The rule started as the one `/cd` has always used: slugify the directory's
//! basename and compare it against each project's slugified name. It lives
//! here rather than inside the `/cd` handler so the two call sites cannot
//! answer the same question differently.
//!
//! A basename alone conflates unrelated directories that share a name
//! (#1510): a benchmark checkout of one project living under another
//! project's name, or any clone whose directory was never the project's name.
//! So the comparison now carries two identities, applied in this order:
//!
//! 1. `repo_remote`: a session whose repository origin matches a project's
//!    adopted remote links to it, whatever the directories are called. This
//!    makes worktrees, subdirectories and renamed clones reachable.
//! 2. The basename, kept as the fallback for plain directories and for
//!    projects nobody has linked yet. A project that has adopted a remote,
//!    though, no longer accepts a basename match from a session that cannot
//!    prove it shares the repository: "no remote" is not evidence of a
//!    shared identity, and unrelated checkouts sharing a name is exactly
//!    the collision the remote was added to refuse.

use std::path::Path;

use crate::db::models::Project;
use crate::services::file::slugify_project_name;

/// The git facts about a directory that a project can be identified by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryIdentity {
    /// The directory whose basename names the project: the repository
    /// toplevel when this path is inside one (a subdirectory or worktree
    /// checkout should resolve to the repository, not the folder it lives
    /// in), and the path itself otherwise.
    pub match_directory: String,
    /// The normalized origin remote of that repository, when there is one.
    pub remote: Option<String>,
}

impl DirectoryIdentity {
    /// A directory classified without git: the basename rule only, no remote
    /// to prove or refuse anything with. Tests and deliberately non-git
    /// paths use this; a live repository should go through
    /// [`resolve_directory_identity`].
    pub fn path_only(working_directory: &str) -> Self {
        Self {
            match_directory: working_directory.to_string(),
            remote: None,
        }
    }
}

/// Ask git what repository `working_directory` lives in.
///
/// Best-effort by design: no git binary, a non-repository path, or a repo
/// without an `origin` each degrade to a weaker identity rather than failing
/// the caller. A session that cannot prove a remote falls back to the
/// basename rule, which is exactly the pre-#1510 behavior.
pub fn resolve_directory_identity(working_directory: &str) -> DirectoryIdentity {
    match git_stdout(&["-C", working_directory, "rev-parse", "--show-toplevel"]) {
        Some(toplevel) => {
            let origin = git_stdout(&["-C", &toplevel, "remote", "get-url", "origin"]);
            DirectoryIdentity {
                remote: origin.as_deref().and_then(normalize_remote),
                match_directory: toplevel,
            }
        }
        None => DirectoryIdentity::path_only(working_directory),
    }
}

fn git_stdout(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Reduce a remote URL to `host/owner/repo` so the same repository agrees on
/// its identity whichever way it was cloned.
///
/// https, ssh and the scp shorthand all normalize to one form; the `.git`
/// suffix, trailing slashes and the cloning user are dropped; the host is
/// case-insensitive while the path keeps its case (self-hosted forges can be
/// case-sensitive, and two paths differing only in case are not worth
/// conflating). A remote with no host or no path to compare — an absolute
/// local path, for instance — returns `None` and falls through to the
/// basename rule.
pub fn normalize_remote(url: &str) -> Option<String> {
    let mut s = url.trim().to_string();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    for scheme in ["git+ssh://", "https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            // Byte slicing is safe: the schemes are pure ASCII, so prefix
            // lengths agree between `s` and `lower`.
            s = rest.to_string();
            break;
        }
    }
    // The cloning user says who fetched, not what the repository is.
    if let Some(at) = s.find('@')
        && !s[..at].contains('/')
    {
        s = s[at + 1..].to_string();
    }
    // scp shorthand `host:owner/repo` is the same address as `host/owner/repo`.
    if let Some(colon) = s.find(':')
        && !s[..colon].contains('/')
    {
        s = format!("{}/{}", &s[..colon], &s[colon + 1..]);
    }
    while let Some(stripped) = s.strip_suffix('/') {
        s = stripped.to_string();
    }
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_string();
    }
    let (host, path) = s.split_once('/')?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host.to_ascii_lowercase(), path))
}

/// The basename of `working_directory`, slugified for comparison.
///
/// A trailing separator is ignored, so `~/src/thing/` and `~/src/thing` are
/// the same directory, which is how a user types it about half the time.
fn directory_slug(working_directory: &str) -> Option<String> {
    let trimmed = working_directory.trim_end_matches(['/', '\\']);
    let name = Path::new(trimmed).file_name()?.to_str()?;
    let slug = slugify_project_name(name);
    (!slug.is_empty()).then_some(slug)
}

/// The project this directory belongs to, if any.
///
/// Remote first: a session inside a checkout of the repository a project has
/// adopted links to it whatever either directory is called. Then the
/// basename, with the #1510 guard: once a project has recorded a remote, a
/// session can match it by name only by carrying that same remote, because
/// an unrelated same-named checkout is precisely what this used to hide.
///
/// Ties go to the first project in the list, as before: two projects
/// slugging to the same name is already ambiguous at creation time, and
/// picking arbitrarily is no worse than leaving the session unlinked.
pub fn match_by_directory<'a>(
    identity: &DirectoryIdentity,
    projects: &'a [Project],
) -> Option<&'a Project> {
    if let Some(remote) = identity.remote.as_deref()
        && let Some(hit) = projects
            .iter()
            .find(|p| p.repo_remote.as_deref() == Some(remote))
    {
        return Some(hit);
    }
    let dir = directory_slug(&identity.match_directory)?;
    projects.iter().find(|p| {
        slugify_project_name(&p.name) == dir
            && match (identity.remote.as_deref(), p.repo_remote.as_deref()) {
                (Some(session), Some(project)) => session == project,
                (None, Some(_)) => false,
                _ => true,
            }
    })
}
