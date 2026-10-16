//! Recipes: YAML workflow definitions, rendered into shell commands and run
//! step by step in the terminal.

pub mod loader;
pub mod model;
pub mod plan;
pub mod template;

use crate::git;
use crate::pty::PtyState;
use model::{InputDef, Recipe, Source};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeView {
    id: String,
    name: String,
    description: Option<String>,
    source: Source,
    confirm: bool,
    inputs: Vec<InputDef>,
}

impl From<&Recipe> for RecipeView {
    fn from(r: &Recipe) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            description: r.description.clone(),
            source: r.source,
            confirm: r.confirm,
            inputs: r.inputs.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeList {
    cwd: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    recipes: Vec<RecipeView>,
    errors: Vec<loader::LoadError>,
    global_file: Option<PathBuf>,
}

fn find(cwd: Option<&Path>, id: &str) -> Result<(Recipe, git::RepoContext), String> {
    let ctx = cwd.map(git::context).unwrap_or_default();
    let loaded = loader::load(ctx.root.as_deref());
    let recipe = loaded
        .recipes
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("Recipe {id:?} no longer exists. Reload the list."))?;
    Ok((recipe, ctx))
}

/// The recipes that apply to the terminal's current directory.
#[tauri::command]
pub async fn recipes_list(state: State<'_, PtyState>) -> Result<RecipeList, String> {
    let cwd = state.cwd();
    let repo_root = cwd.as_deref().and_then(git::repo_root);
    let loaded = loader::load(repo_root.as_deref());
    Ok(RecipeList {
        cwd,
        repo_root,
        recipes: loaded.recipes.iter().map(RecipeView::from).collect(),
        errors: loaded.errors,
        global_file: loader::global_file(),
    })
}

/// Renders the commands for a preview. Always succeeds unless the recipe is gone;
/// problems are reported in `errors`.
#[tauri::command]
pub async fn recipe_plan(
    state: State<'_, PtyState>,
    id: String,
    values: HashMap<String, serde_json::Value>,
) -> Result<plan::Plan, String> {
    let (recipe, ctx) = find(state.cwd().as_deref(), &id)?;
    Ok(plan::plan(&recipe, &values, &ctx))
}

/// Re-renders the plan and runs it, but only if it still matches what the
/// user saw in the preview (`expected`). Never runs anything unreviewed.
#[tauri::command]
pub async fn recipe_run(
    app: AppHandle,
    state: State<'_, PtyState>,
    id: String,
    values: HashMap<String, serde_json::Value>,
    expected: Vec<String>,
) -> Result<(), String> {
    let (recipe, ctx) = find(state.cwd().as_deref(), &id)?;
    let p = plan::plan(&recipe, &values, &ctx);
    if let Some(e) = p.errors.first() {
        return Err(e.message.clone());
    }
    let commands: Vec<&str> = p.steps.iter().map(|s| s.command.as_str()).collect();
    if commands != expected.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err("The commands changed since the preview (branch or recipe file changed). Review them again.".into());
    }
    crate::runner::start_in_app(&app, &state, recipe.name.clone(), p.steps)
}

/// Runs every built-in recipe's real commands with bash in throwaway repos
/// whose "origin" is a local bare repo.
#[cfg(test)]
mod builtin_tests {
    use super::*;
    use crate::git::testutil::TempRepo;
    use serde_json::json;

    fn bash() -> PathBuf {
        #[cfg(windows)]
        return crate::shell::find_git_bash().unwrap();
        #[cfg(not(windows))]
        return PathBuf::from("/bin/bash");
    }

    /// Runs the recipe like the runner does: stop at the first failure, then only `always` steps.
    fn run(repo: &TempRepo, id: &str, values: &[(&str, serde_json::Value)]) -> Vec<(String, i32)> {
        let ctx = git::context(&repo.work);
        let loaded = loader::load(None);
        let recipe = loaded.recipes.iter().find(|r| r.id == id).unwrap();
        let values = values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let p = plan::plan(recipe, &values, &ctx);
        assert!(p.errors.is_empty(), "{id}: {:?}", p.errors);
        let mut failed = false;
        let mut out = vec![];
        for s in p.steps {
            if failed && !s.always {
                continue;
            }
            let o = std::process::Command::new(bash())
                .args(["--norc", "--noprofile", "-c", &s.command])
                .current_dir(&repo.work)
                .output()
                .unwrap();
            let code = o.status.code().unwrap_or(-1);
            if code != 0 {
                eprintln!("{id}: `{}` -> {code}\n{}", s.command, String::from_utf8_lossy(&o.stderr));
                failed = true;
            }
            out.push((s.command, code));
        }
        out
    }

    fn all_ok(results: &[(String, i32)]) -> bool {
        results.iter().all(|(_, c)| *c == 0)
    }

    fn head(repo: &TempRepo) -> String {
        git::git(&repo.work, &["symbolic-ref", "--short", "HEAD"]).unwrap()
    }

    fn commit_file(repo: &TempRepo, name: &str, body: &str) {
        std::fs::write(repo.work.join(name), body).unwrap();
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", name]);
    }

    #[test]
    fn bcp() {
        let r = TempRepo::new("bcp");
        std::fs::write(r.work.join("a.txt"), "a").unwrap();
        let res = run(&r, "bcp", &[("branch", json!("feat/a")), ("msg", json!("feat: it's \"done\" $(x)"))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "feat/a");
        let msg = git::git(&r.origin, &["log", "-1", "--format=%s", "feat/a"]).unwrap();
        assert_eq!(msg, "feat: it's \"done\" $(x)");
    }

    #[test]
    fn merge_into_returns_even_when_push_fails() {
        let r = TempRepo::new("merge");
        r.git(&["checkout", "-q", "-b", "dev"]);
        r.git(&["push", "-q", "-u", "origin", "dev"]);
        r.git(&["checkout", "-q", "-b", "feat/m"]);
        commit_file(&r, "m.txt", "m");
        let res = run(&r, "merge-into", &[("target", json!("dev"))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "feat/m");
        let parents = git::git(&r.origin, &["log", "-1", "--format=%p", "dev"]).unwrap();
        assert_eq!(parents.split(' ').count(), 2, "--no-ff should create a merge commit");

        // Make the push fail: the always-step must still bring us back.
        commit_file(&r, "m2.txt", "m2");
        std::fs::write(r.origin.join("hooks").join("pre-receive"), "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(r.origin.join("hooks/pre-receive"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let res = run(&r, "merge-into", &[("target", json!("dev")), ("no_ff", json!(false))]);
        assert!(!all_ok(&res), "push should have failed: {res:?}");
        assert_eq!(res.last().unwrap().0, "git checkout 'feat/m'");
        assert_eq!(head(&r), "feat/m");
    }

    #[test]
    fn sync_main_rebase_and_merge() {
        let r = TempRepo::new("sync");
        r.git(&["checkout", "-q", "-b", "feat/s"]);
        commit_file(&r, "s.txt", "s");
        r.git(&["checkout", "-q", "main"]);
        commit_file(&r, "main2.txt", "x");
        r.git(&["push", "-q"]);
        r.git(&["checkout", "-q", "feat/s"]);
        assert!(all_ok(&run(&r, "sync-main", &[])));
        assert!(r.work.join("main2.txt").exists());

        r.git(&["checkout", "-q", "main"]);
        commit_file(&r, "main3.txt", "y");
        r.git(&["push", "-q"]);
        r.git(&["checkout", "-q", "feat/s"]);
        assert!(all_ok(&run(&r, "sync-main", &[("strategy", json!("merge"))])));
        assert!(r.work.join("main3.txt").exists());
    }

    #[test]
    fn stash_switch_pops_only_its_own_stash() {
        let r = TempRepo::new("stash");
        r.git(&["branch", "other"]);
        // An older, unrelated stash that must survive.
        std::fs::write(r.work.join("old.txt"), "old").unwrap();
        r.git(&["stash", "push", "-q", "-u", "-m", "older work"]);

        // Clean tree + pop: nothing stashed, so nothing popped.
        let res = run(&r, "stash-switch", &[("branch", json!("other")), ("pop", json!(true))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "other");
        assert!(!r.work.join("old.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 1);

        // Dirty tree + pop: changes come along, older stash still there.
        std::fs::write(r.work.join("wip.txt"), "wip").unwrap();
        let res = run(&r, "stash-switch", &[("branch", json!("main")), ("pop", json!(true))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "main");
        assert!(r.work.join("wip.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 1);

        // Without pop the changes stay stashed.
        let res = run(&r, "stash-switch", &[("branch", json!("other"))]);
        assert!(all_ok(&res), "{res:?}");
        assert!(!r.work.join("wip.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 2);
    }

    #[test]
    fn undo_commit_keeps_changes_staged() {
        let r = TempRepo::new("undo");
        commit_file(&r, "u.txt", "u");
        assert!(all_ok(&run(&r, "undo-commit", &[])));
        assert_eq!(git::git(&r.work, &["log", "-1", "--format=%s"]).unwrap(), "init");
        assert_eq!(git::git(&r.work, &["diff", "--cached", "--name-only"]).unwrap(), "u.txt");
    }

    #[test]
    fn clean_merged_keeps_protected_and_unmerged() {
        let r = TempRepo::new("clean");
        for b in ["merged-1", "dev", "release/1.0"] {
            r.git(&["branch", b]);
        }
        r.git(&["checkout", "-q", "-b", "unmerged"]);
        commit_file(&r, "x.txt", "x");
        r.git(&["checkout", "-q", "main"]);
        let res = run(&r, "clean-merged", &[]);
        assert!(all_ok(&res), "{res:?}");
        let branches = git::git(&r.work, &["branch", "--format=%(refname:short)"]).unwrap();
        let mut names: Vec<_> = branches.lines().collect();
        names.sort();
        assert_eq!(names, vec!["dev", "main", "release/1.0", "unmerged"]);

        // Nothing left to delete is still a success.
        assert!(all_ok(&run(&r, "clean-merged", &[])));
    }
}
