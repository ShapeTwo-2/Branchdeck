//! Turns a recipe plus form values into the exact commands that will be typed.
//! The frontend only ever displays what this produces.

use super::model::{Action, InputKind, Recipe};
use super::template::{self, sh_quote, Value};
use super::version;
use crate::git::{self, RepoContext};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStep {
    pub command: String,
    pub always: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    /// None for problems not tied to one input (e.g. "not in a git repo").
    pub input: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Steps whose `when` holds, rendered. Missing values show as `<label>`.
    pub steps: Vec<PlannedStep>,
    /// Empty means the plan can run.
    pub errors: Vec<FieldError>,
    /// Worth knowing, but not blocking.
    pub warnings: Vec<String>,
}

enum Var {
    Text(String),
    Words(Vec<String>),
    /// Not available; the string is what the preview shows instead.
    Missing(String),
}

impl Var {
    /// The value `when:` / `requires:` conditions compare against.
    fn as_cond_value(&self) -> &str {
        match self {
            Var::Text(s) => s,
            Var::Words(w) if w.is_empty() => "",
            Var::Words(_) => "true",
            Var::Missing(_) => "",
        }
    }
}

fn opt_var(v: &Option<String>, what: &str) -> Var {
    match v {
        Some(s) => Var::Text(s.clone()),
        None => Var::Missing(format!("<{what}>")),
    }
}

pub fn plan(recipe: &Recipe, raw: &HashMap<String, serde_json::Value>, ctx: &RepoContext) -> Plan {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut vars: HashMap<&str, Var> = HashMap::new();
    let error = |input: Option<&str>, message: String| FieldError { input: input.map(String::from), message };

    if ctx.root.is_none() {
        errors.push(error(None, "The terminal is not inside a git repository. cd into one first.".into()));
    }
    vars.insert("current_branch", opt_var(&ctx.current_branch, "current branch"));
    vars.insert("default_branch", opt_var(&ctx.default_branch, "default branch"));
    vars.insert("repo_root", opt_var(&ctx.root.as_ref().map(|p| p.display().to_string()), "repo root"));
    vars.insert("operation", Var::Text(ctx.operation.map(|o| o.as_str()).unwrap_or("").into()));

    for input in &recipe.inputs {
        let label = input.label.clone().unwrap_or_else(|| input.id.clone());
        let given = raw.get(&input.id).filter(|v| !v.is_null()).or(input.default.as_ref());
        let mut value = match input.kind {
            InputKind::Bool => Ok(given.and_then(|v| v.as_bool()).unwrap_or(false).to_string()),
            _ => {
                let text = given.and_then(|v| v.as_str()).unwrap_or("");
                let text = if input.kind == InputKind::Branch { text.trim() } else { text };
                check_text(input.kind, text, input.required, &input.options)
            }
        };
        if let (Ok(name), Some(want), Some(root)) = (&value, input.exists, &ctx.root) {
            if !name.is_empty() {
                let local = git::local_branch_exists(root, name);
                if want && !local && !git::remote_branch_exists(root, name) {
                    value = Err(format!("No branch named {name:?} (locally or on origin)"));
                } else if !want && local {
                    value = Err(format!("Branch {name:?} already exists"));
                }
            }
        }
        match value {
            Ok(v) => {
                vars.insert(&input.id, Var::Text(v));
            }
            Err(message) => {
                errors.push(error(Some(&input.id), message));
                vars.insert(&input.id, Var::Missing(format!("<{label}>")));
            }
        }
    }

    if ctx.root.is_some() {
        for r in &recipe.requires {
            let v = vars.get(r.cond.var()).map(Var::as_cond_value).unwrap_or("");
            if !r.cond.eval(v) {
                errors.push(error(None, r.message.clone()));
            }
        }
    }

    // Version bump: computed from the manifests, expanded into visible commands.
    let mut bump_commands = Vec::new();
    if let Some(vdef) = &recipe.version {
        let set_missing =|vars: &mut HashMap<&str, Var>| {
            vars.insert("version", Var::Missing("<version>".into()));
            vars.insert("new_version", Var::Missing("<new version>".into()));
            vars.insert("version_files", Var::Missing("<version files>".into()));
        };
        match (&ctx.root, vars.get(vdef.bump.as_str())) {
            (Some(root), Some(Var::Text(how))) => match version::plan_bump(root, &vdef.files, how) {
                Ok(b) => {
                    vars.insert("version", Var::Text(b.current.to_string()));
                    vars.insert("new_version", Var::Text(b.new.to_string()));
                    vars.insert("version_files", Var::Words(b.files));
                    bump_commands = b.commands;
                    warnings.extend(b.warnings);
                }
                Err(e) => {
                    errors.push(error(Some(&vdef.bump), e));
                    set_missing(&mut vars);
                }
            },
            _ => set_missing(&mut vars),
        }
    }

    let mut steps = Vec::new();
    // Version files are repo-relative; run from the root if the terminal is in a subfolder.
    if recipe.version.is_some() && ctx.cwd_is_not_root() {
        if let Some(root) = &ctx.root {
            let root = root.to_string_lossy().replace('\\', "/");
            steps.push(PlannedStep { command: format!("cd {}", sh_quote(&root)), always: false });
        }
    }

    let mut needs_branch = false;
    let mut used_display = false;
    for step in &recipe.steps {
        if let Some(cond) = &step.when {
            // A condition on a value the user hasn't filled in yet counts as false.
            let v = vars.get(cond.var()).map(Var::as_cond_value).unwrap_or("");
            if !cond.eval(v) {
                continue;
            }
        }
        let run = match &step.action {
            Action::UpdateVersionFiles => {
                steps.extend(bump_commands.iter().map(|c| PlannedStep { command: c.clone(), always: step.always }));
                continue;
            }
            Action::Run(run) => run,
        };
        // Steps that operate on an empty file list (tag-only projects) have nothing to do.
        if matches!(vars.get("version_files"), Some(Var::Words(w)) if w.is_empty())
            && template::placeholders(run).is_ok_and(|p| p.iter().any(|n| n == "version_files"))
        {
            continue;
        }
        let rendered = template::render(run, |name| match vars.get(name) {
            Some(Var::Text(v)) => Ok(Value::Quoted(v.clone())),
            Some(Var::Words(w)) => Ok(Value::Words(w.clone())),
            Some(Var::Missing(display)) => {
                used_display = true;
                if name == "current_branch" {
                    needs_branch = true;
                }
                Ok(Value::Display(display.clone()))
            }
            None => Err(format!("unknown variable {name}")),
        });
        match rendered {
            Ok(command) => steps.push(PlannedStep { command, always: step.always }),
            // Validated at load time; only reachable if that check is bypassed.
            Err(e) => errors.push(error(None, e)),
        }
    }
    if needs_branch && ctx.root.is_some() {
        errors.push(error(None, "HEAD is detached; this recipe needs you to be on a branch.".into()));
    }
    let uses_default = recipe.steps.iter().any(|s| matches!(&s.action, Action::Run(r) if r.contains("default_branch")));
    if ctx.root.is_some() && ctx.default_branch.is_none() && uses_default {
        errors.push(error(None, "Could not find the default branch (no origin/HEAD, main or master).".into()));
    }
    // Invariant: a `<placeholder>` is only for display and must never be typed.
    if used_display && errors.is_empty() {
        errors.push(error(None, "Some values are missing.".into()));
    }
    Plan { steps, errors, warnings }
}


/// Ok(value) or Err(message). Optional fields may be empty.
fn check_text(kind: InputKind, text: &str, required: bool, options: &[String]) -> Result<String, String> {
    if text.trim().is_empty() {
        return if required { Err("Required".into()) } else { Ok(String::new()) };
    }
    if text.chars().any(|c| c.is_control()) {
        return Err("Must be a single line without control characters".into());
    }
    match kind {
        InputKind::Branch => check_branch_name(text).map(|_| text.to_string()),
        InputKind::Select if !options.iter().any(|o| o == text) => Err(format!("Must be one of: {}", options.join(", "))),
        _ => Ok(text.to_string()),
    }
}

/// Mirrors `git check-ref-format --branch` for the rules that matter here,
/// and also refuses a leading `-` so a name can never be read as an option.
pub fn check_branch_name(name: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("Invalid branch name: {why}"));
    if name.starts_with('-') {
        return bad("can't start with -");
    }
    if name.chars().any(|c| c.is_whitespace() || "~^:?*[\\".contains(c)) {
        return bad("no spaces or any of ~ ^ : ? * [ \\");
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return bad("no .., @{ or //");
    }
    if name == "@" || name.starts_with('/') || name.ends_with('/') || name.ends_with('.') {
        return bad("can't be @, or start/end with / or end with .");
    }
    if name.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) {
        return bad("no part may start with . or end with .lock");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipes::model::{RecipeDef, Source};
    use serde_json::json;
    use std::path::PathBuf;

    fn recipe(yaml: &str) -> Recipe {
        serde_yaml::from_str::<RecipeDef>(yaml).unwrap().validate(Source::Builtin).unwrap()
    }

    fn ctx() -> RepoContext {
        RepoContext {
            root: Some(PathBuf::from("/repo")),
            current_branch: Some("feature/a".into()),
            default_branch: Some("main".into()),
            ..Default::default()
        }
    }

    fn vals(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    const BCP: &str = r#"
name: bcp
inputs:
  - { id: branch, label: Branch name, type: branch, required: true }
  - { id: msg, label: Commit message, required: true }
steps:
  - git checkout -b "{{branch}}"
  - git commit -m "{{msg}}"
"#;

    #[test]
    fn renders_safely_quoted_commands() {
        let p = plan(&recipe(BCP), &vals(&[("branch", json!("feat/x")), ("msg", json!("it's $(done)"))]), &ctx());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(cmds, vec!["git checkout -b 'feat/x'", r"git commit -m 'it'\''s $(done)'"]);
    }

    #[test]
    fn missing_values_show_placeholders_and_errors() {
        let p = plan(&recipe(BCP), &vals(&[]), &ctx());
        assert_eq!(p.steps[0].command, "git checkout -b <Branch name>");
        assert_eq!(p.errors.len(), 2);
    }

    #[test]
    fn rejects_option_injection_and_newlines() {
        let p = plan(&recipe(BCP), &vals(&[("branch", json!("--force")), ("msg", json!("a\nb"))]), &ctx());
        let inputs: Vec<_> = p.errors.iter().filter_map(|e| e.input.as_deref()).collect();
        assert_eq!(inputs, vec!["branch", "msg"]);
    }

    #[test]
    fn when_and_defaults() {
        let r = recipe(
            r#"
name: m
inputs:
  - { id: no_ff, type: bool, default: true }
  - { id: target, type: select, options: [dev, prod], default: dev }
steps:
  - git checkout "{{target}}"
  - run: git merge --no-ff "{{current_branch}}"
    when: no_ff
  - run: git merge "{{current_branch}}"
    when: "!no_ff"
  - run: git checkout "{{current_branch}}"
    always: true
"#,
        );
        let p = plan(&r, &vals(&[]), &ctx());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(
            cmds,
            vec!["git checkout 'dev'", "git merge --no-ff 'feature/a'", "git checkout 'feature/a'"]
        );
        assert!(p.steps[2].always);

        let p = plan(&r, &vals(&[("no_ff", json!(false)), ("target", json!("prod"))]), &ctx());
        assert_eq!(p.steps[1].command, "git merge 'feature/a'");
        let p = plan(&r, &vals(&[("target", json!("nope"))]), &ctx());
        assert_eq!(p.errors.len(), 1);
    }

    #[test]
    fn outside_repo_or_detached() {
        let r = recipe("name: x\nsteps:\n  - git checkout \"{{current_branch}}\"");
        assert!(!plan(&r, &vals(&[]), &RepoContext::default()).errors.is_empty());
        let detached = RepoContext { current_branch: None, ..ctx() };
        let p = plan(&r, &vals(&[]), &detached);
        assert!(p.errors[0].message.contains("detached"));
    }

    #[test]
    fn requires_blocks_with_its_message() {
        let r = recipe(
            "name: abort\nrequires:\n  - { when: operation, message: Nothing to abort }\nsteps:\n  - git merge --abort",
        );
        let p = plan(&r, &vals(&[]), &ctx());
        assert_eq!(p.errors[0].message, "Nothing to abort");
        let merging = RepoContext { operation: Some(crate::git::Operation::Merge), ..ctx() };
        assert!(plan(&r, &vals(&[]), &merging).errors.is_empty());
    }

    #[test]
    fn branch_existence_checks() {
        let repo = crate::git::testutil::TempRepo::new("exists");
        repo.git(&["branch", "taken"]);
        let r = recipe(
            "name: x\ninputs:\n  - { id: new, type: branch, exists: false }\n  - { id: old, type: branch, exists: true }\nsteps:\n  - git switch -c {{new}} {{old}}",
        );
        let c = crate::git::context(&repo.work);
        let p = plan(&r, &vals(&[("new", json!("fresh")), ("old", json!("taken"))]), &c);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let p = plan(&r, &vals(&[("new", json!("taken")), ("old", json!("ghost"))]), &c);
        let msgs: Vec<_> = p.errors.iter().map(|e| e.message.as_str()).collect();
        assert!(msgs[0].contains("already exists") && msgs[1].contains("No branch"), "{msgs:?}");
    }

    #[test]
    fn version_bump_plan_from_subfolder() {
        let repo = crate::git::testutil::TempRepo::new("bumpplan");
        std::fs::create_dir_all(repo.work.join("sub")).unwrap();
        std::fs::write(repo.work.join("Cargo.toml"), "[package]\nname = \"a\"\nversion = \"0.4.2\"\n").unwrap();
        std::fs::write(repo.work.join("sub/keep.txt"), "x").unwrap();
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "c"]);
        let r = recipe(
            r#"
name: bump
inputs: [{ id: bump, type: select, options: [patch, minor, major], default: patch }]
version: { bump: bump }
steps:
  - update_version_files: true
  - git add -- {{version_files}}
  - 'git commit -m "chore: release v{{new_version}}" -- {{version_files}}'
  - git tag -a "v{{new_version}}" -m "v{{new_version}}"
"#,
        );
        let c = crate::git::context(&repo.work.join("sub"));
        let p = plan(&r, &vals(&[("bump", json!("minor"))]), &c);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert!(cmds[0].starts_with("cd '"), "{cmds:?}");
        assert!(cmds[1].starts_with("perl ") && cmds[1].ends_with("'Cargo.toml'"));
        assert_eq!(&cmds[2..], ["git add -- 'Cargo.toml'", r#"git commit -m "chore: release v"'0.5.0' -- 'Cargo.toml'"#, r#"git tag -a "v"'0.5.0' -m "v"'0.5.0'"#]);
    }

    #[test]
    fn branch_names() {
        for ok in ["main", "feature/login", "fix-123", "user/a.b"] {
            assert!(check_branch_name(ok).is_ok(), "{ok}");
        }
        for bad in ["-f", "a b", "a..b", "a~1", "x.lock", "/a", "a/", "a.", ".a", "a//b", "@", "a@{1}", "a:b"] {
            assert!(check_branch_name(bad).is_err(), "{bad}");
        }
    }
}
