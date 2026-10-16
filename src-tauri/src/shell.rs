//! Picks which shell to launch: the configured one, else Git Bash on Windows,
//! else `$SHELL` on macOS/Linux.

use crate::config::{expand_home, Config};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub fn resolve(config: &Config) -> Result<ShellSpec, String> {
    if let Some(sc) = &config.shell {
        let program = expand_home(&sc.path);
        if !program.is_file() {
            return Err(format!(
                "Configured shell not found: {} (check ~/.branchdeck/config.yml)",
                program.display()
            ));
        }
        let args = sc.args.clone().unwrap_or_else(|| default_args(&program));
        return Ok(ShellSpec { program, args });
    }
    let program = detect_default()?;
    let args = default_args(&program);
    Ok(ShellSpec { program, args })
}

/// Login + interactive for POSIX shells so the user's profile is loaded.
fn default_args(program: &Path) -> Vec<String> {
    let name = program
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match name.as_str() {
        "bash" => vec!["--login".into(), "-i".into()],
        "zsh" | "fish" => vec!["-l".into(), "-i".into()],
        "pwsh" | "powershell" => vec!["-NoLogo".into()],
        _ => vec![],
    }
}

#[cfg(windows)]
fn detect_default() -> Result<PathBuf, String> {
    if let Some(bash) = find_git_bash() {
        return Ok(bash);
    }
    Err("Git Bash was not found. Install Git for Windows (https://git-scm.com/download/win) \
         or set shell.path in ~/.branchdeck/config.yml."
        .into())
}

/// Looks in the standard install locations, then next to whatever `git.exe` is on PATH.
#[cfg(windows)]
pub fn find_git_bash() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(var) {
            roots.push(PathBuf::from(p).join("Git"));
        }
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(p).join("Programs").join("Git"));
    }
    // git.exe usually lives in <root>\cmd\ or <root>\mingw64\bin\.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("git.exe").is_file() {
                for up in dir.ancestors().skip(1).take(2) {
                    roots.push(up.to_path_buf());
                }
            }
        }
    }
    roots
        .into_iter()
        .map(|r| r.join("bin").join("bash.exe"))
        .find(|p| p.is_file())
}

#[cfg(not(windows))]
fn detect_default() -> Result<PathBuf, String> {
    if let Ok(sh) = std::env::var("SHELL") {
        let p = PathBuf::from(sh);
        if p.is_file() {
            return Ok(p);
        }
    }
    ["/bin/zsh", "/bin/bash", "/bin/sh"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| "No shell found. Set shell.path in ~/.branchdeck/config.yml.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_args_by_name() {
        assert_eq!(default_args(Path::new("/bin/bash")), vec!["--login", "-i"]);
        assert_eq!(default_args(Path::new(r"C:\Git\bin\bash.exe")), vec!["--login", "-i"]);
        assert_eq!(default_args(Path::new("/bin/zsh")), vec!["-l", "-i"]);
        assert!(default_args(Path::new("/bin/sh")).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn finds_git_bash_on_dev_machine() {
        // Git for Windows is a hard requirement for developing Branchdeck on Windows.
        assert!(find_git_bash().is_some());
    }
}
