//! Turns a recipe plus form values into the exact commands that will be typed.
//! The frontend only ever displays what this produces.

use super::model::{InputKind, Recipe};
use super::template::{self, Value};
use crate::git::RepoContext;
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
}

pub fn plan(recipe: &Recipe, raw: &HashMap<String, serde_json::Value>, ctx: &RepoContext) -> Plan {
    let mut errors = Vec::new();
    // name -> Ok(text) if usable, Err(display placeholder) if not.
    let mut vars: HashMap<&str, Result<String, String>> = HashMap::new();

    if ctx.root.is_none() {
        errors.push(FieldError {
            input: None,
            message: "The terminal is not inside a git repository. cd into one first.".into(),
        });
    }
    let builtin = |v: &Option<String>, what: &str| v.clone().ok_or_else(|| format!("<{what}>"));
    vars.insert("current_branch", builtin(&ctx.current_branch, "current branch"));
    vars.insert("default_branch", builtin(&ctx.default_branch, "default branch"));
    vars.insert(
        "repo_root",
        ctx.root.as_ref().map(|p| p.display().to_string()).ok_or_else(|| "<repo root>".into()),
    );

    for input in &recipe.inputs {
        let label = input.label.clone().unwrap_or_else(|| input.id.clone());
        let given = raw.get(&input.id).filter(|v| !v.is_null()).or(input.default.as_ref());
        let value = match input.kind {
            InputKind::Bool => Ok(given.and_then(|v| v.as_bool()).unwrap_or(false).to_string()),
            _ => {
                let text = given.and_then(|v| v.as_str()).unwrap_or("");
                let text = if input.kind == InputKind::Branch { text.trim() } else { text };
                check_text(input.kind, text, input.required, &input.options)
            }
        };
        match value {
            Ok(v) => {
                vars.insert(&input.id, Ok(v));
            }
            Err(message) => {
                errors.push(FieldError { input: Some(input.id.clone()), message });
                vars.insert(&input.id, Err(format!("<{label}>")));
            }
        }
    }

    let mut steps = Vec::new();
    let mut needs_branch = false;
    let mut used_display = false;
    for step in &recipe.steps {
        if let Some(cond) = &step.when {
            // A condition on a value the user hasn't filled in yet counts as false.
            let v = vars.get(cond.var()).and_then(|v| v.as_ref().ok()).map(String::as_str).unwrap_or("");
            if !cond.eval(v) {
                continue;
            }
        }
        let rendered = template::render(&step.run, |name| match vars.get(name) {
            Some(Ok(v)) => Ok(Value::Quoted(v.clone())),
            Some(Err(display)) => {
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
            Err(e) => errors.push(FieldError { input: None, message: e }),
        }
    }
    if needs_branch && ctx.root.is_some() {
        errors.push(FieldError {
            input: None,
            message: "HEAD is detached; this recipe needs you to be on a branch.".into(),
        });
    }
    if ctx.root.is_some() && ctx.default_branch.is_none() && recipe.steps.iter().any(|s| s.run.contains("default_branch")) {
        errors.push(FieldError {
            input: None,
            message: "Could not find the default branch (no origin/HEAD, main or master).".into(),
        });
    }
    // Invariant: a `<placeholder>` is only for display and must never be typed.
    if used_display && errors.is_empty() {
        errors.push(FieldError { input: None, message: "Some values are missing.".into() });
    }
    Plan { steps, errors }
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
    fn branch_names() {
        for ok in ["main", "feature/login", "fix-123", "user/a.b"] {
            assert!(check_branch_name(ok).is_ok(), "{ok}");
        }
        for bad in ["-f", "a b", "a..b", "a~1", "x.lock", "/a", "a/", "a.", ".a", "a//b", "@", "a@{1}", "a:b"] {
            assert!(check_branch_name(bad).is_err(), "{bad}");
        }
    }
}
