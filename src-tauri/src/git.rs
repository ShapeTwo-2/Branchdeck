//! Read-only git queries via the git CLI (never libgit2), so the user's
//! config, aliases and safe.directory rules apply.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `git <args>` in `cwd`; returns trimmed stdout on exit code 0.
pub fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_string())
}

/// Facts about the repo that recipes can reference as built-in variables.
#[derive(Debug, Clone, Default)]
pub struct RepoContext {
    pub root: Option<PathBuf>,
    /// None on a detached HEAD.
    pub current_branch: Option<String>,
    pub default_branch: Option<String>,
}

pub fn repo_root(cwd: &Path) -> Option<PathBuf> {
    git(cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

pub fn context(cwd: &Path) -> RepoContext {
    let Some(root) = repo_root(cwd) else { return RepoContext::default() };
    let current_branch = git(&root, &["symbolic-ref", "--short", "-q", "HEAD"]).filter(|b| !b.is_empty());
    let default_branch = default_branch(&root);
    RepoContext { root: Some(root), current_branch, default_branch }
}

/// origin/HEAD if the remote advertises it, else the first of main/master that exists locally.
fn default_branch(root: &Path) -> Option<String> {
    if let Some(r) = git(root, &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]) {
        if let Some(b) = r.strip_prefix("origin/") {
            return Some(b.to_string());
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| git(root, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_some())
        .map(String::from)
}

/// A multi-step git operation that is waiting for the user (conflicts, edits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

pub fn operation_in_progress(cwd: &Path) -> Option<Operation> {
    let git_dir = PathBuf::from(git(cwd, &["rev-parse", "--absolute-git-dir"])?);
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some(Operation::Rebase)
    } else if git_dir.join("MERGE_HEAD").is_file() {
        Some(Operation::Merge)
    } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
        Some(Operation::CherryPick)
    } else if git_dir.join("REVERT_HEAD").is_file() {
        Some(Operation::Revert)
    } else {
        None
    }
}

#[cfg(test)]
pub mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A throwaway repo under the OS temp dir, with a local bare repo as "origin".
    /// Never touches a real remote. Removed on drop.
    pub struct TempRepo {
        pub base: PathBuf,
        pub work: PathBuf,
        pub origin: PathBuf,
    }

    impl TempRepo {
        pub fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "branchdeck-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            let work = base.join("work");
            let origin = base.join("origin.git");
            std::fs::create_dir_all(&work).unwrap();
            run(&base, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
            run(&work, &["init", "-q", "-b", "main"]);
            run(&work, &["config", "user.email", "test@example.invalid"]);
            run(&work, &["config", "user.name", "Branchdeck Test"]);
            run(&work, &["config", "core.autocrlf", "false"]);
            run(&work, &["remote", "add", "origin", origin.to_str().unwrap()]);
            std::fs::write(work.join("README.md"), "hello\n").unwrap();
            run(&work, &["add", "-A"]);
            run(&work, &["commit", "-q", "-m", "init"]);
            run(&work, &["push", "-q", "-u", "origin", "main"]);
            Self { base, work, origin }
        }

        pub fn git(&self, args: &[&str]) {
            run(&self.work, args)
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    pub fn run(cwd: &Path, args: &[&str]) {
        let out = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempRepo;
    use super::*;

    #[test]
    fn context_in_repo() {
        let r = TempRepo::new("ctx");
        r.git(&["checkout", "-q", "-b", "feature/x"]);
        let ctx = context(&r.work);
        assert_eq!(ctx.current_branch.as_deref(), Some("feature/x"));
        assert_eq!(ctx.default_branch.as_deref(), Some("main"));
        assert!(ctx.root.is_some());
    }

    #[test]
    fn context_outside_repo() {
        let dir = std::env::temp_dir().join(format!("branchdeck-norepo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // GIT_CEILING_DIRECTORIES isn't needed: temp dirs aren't inside repos on CI or dev machines.
        assert!(context(&dir).root.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_merge_in_progress() {
        let r = TempRepo::new("merge");
        r.git(&["checkout", "-q", "-b", "a"]);
        std::fs::write(r.work.join("README.md"), "from a\n").unwrap();
        r.git(&["commit", "-q", "-am", "a"]);
        r.git(&["checkout", "-q", "main"]);
        std::fs::write(r.work.join("README.md"), "from main\n").unwrap();
        r.git(&["commit", "-q", "-am", "main"]);
        assert_eq!(operation_in_progress(&r.work), None);
        let status = Command::new("git").args(["merge", "a"]).current_dir(&r.work).output().unwrap().status;
        assert!(!status.success());
        assert_eq!(operation_in_progress(&r.work), Some(Operation::Merge));
    }
}
