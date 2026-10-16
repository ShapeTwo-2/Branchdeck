//! Loads recipes from three layers, later ones overriding earlier ones by id:
//! built-in defaults, `~/.branchdeck/recipes.yml`, then `<repo>/.branchdeck.yml`.
//!
//! A file is either a list of recipes, or a map with a `recipes:` list (which
//! leaves room for repo-level settings).

use super::model::{Recipe, RecipeDef, Source};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const BUILTIN: &str = include_str!("../../recipes/builtin.yml");
pub const REPO_FILE: &str = ".branchdeck.yml";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
    pub file: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub recipes: Vec<Recipe>,
    pub errors: Vec<LoadError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileDoc {
    #[serde(default)]
    recipes: Vec<RecipeDef>,
    /// Read in M5; accepted now so repo files can already declare it.
    #[serde(default)]
    #[allow(dead_code)]
    protected_branches: Option<Vec<String>>,
}

pub fn global_file() -> Option<PathBuf> {
    crate::config::app_dir().map(|d| d.join("recipes.yml"))
}

pub fn load(repo_root: Option<&Path>) -> Loaded {
    let mut loaded = Loaded::default();
    add_source(&mut loaded, "built-in recipes", BUILTIN, Source::Builtin);
    if let Some(path) = global_file() {
        add_file(&mut loaded, &path, Source::Global);
    }
    if let Some(root) = repo_root {
        add_file(&mut loaded, &root.join(REPO_FILE), Source::Repo);
    }
    loaded
}

fn add_file(loaded: &mut Loaded, path: &Path, source: Source) {
    match std::fs::read_to_string(path) {
        Ok(text) => add_source(loaded, &path.display().to_string(), &text, source),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => loaded.errors.push(LoadError { file: path.display().to_string(), message: e.to_string() }),
    }
}

fn parse_defs(text: &str) -> Result<Vec<RecipeDef>, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    match value {
        serde_yaml::Value::Null => Ok(vec![]),
        serde_yaml::Value::Sequence(_) => serde_yaml::from_value(value).map_err(|e| e.to_string()),
        serde_yaml::Value::Mapping(_) => {
            serde_yaml::from_value::<FileDoc>(value).map(|d| d.recipes).map_err(|e| e.to_string())
        }
        _ => Err("expected a list of recipes, or a map with a `recipes:` list".into()),
    }
}

fn add_source(loaded: &mut Loaded, file: &str, text: &str, source: Source) {
    let defs = match parse_defs(text) {
        Ok(d) => d,
        Err(message) => {
            loaded.errors.push(LoadError { file: file.into(), message });
            return;
        }
    };
    for def in defs {
        let label = def.name.clone().or(def.id.clone()).unwrap_or_else(|| "(unnamed)".into());
        let Some(key) = def.key() else {
            loaded.errors.push(LoadError { file: file.into(), message: format!("recipe {label:?}: needs a name or an id") });
            continue;
        };
        let existing = loaded.recipes.iter().position(|r| r.id == key);
        if def.disabled {
            if let Some(i) = existing {
                loaded.recipes.remove(i);
            }
            continue;
        }
        match def.validate(source) {
            Ok(recipe) => match existing {
                Some(i) => loaded.recipes[i] = recipe, // keep its place in the list
                None => loaded.recipes.push(recipe),
            },
            Err(e) => loaded.errors.push(LoadError { file: file.into(), message: format!("recipe {label:?}: {e}") }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid() {
        let mut l = Loaded::default();
        add_source(&mut l, "builtin", BUILTIN, Source::Builtin);
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert!(l.recipes.iter().any(|r| r.id == "bcp"));
    }

    #[test]
    fn later_sources_override_and_disable() {
        let mut l = Loaded::default();
        add_source(&mut l, "a", "- {id: one, name: One, steps: [a]}\n- {id: two, name: Two, steps: [b]}", Source::Builtin);
        add_source(
            &mut l,
            "b",
            "recipes:\n  - {id: one, name: One v2, steps: [c]}\n  - {id: two, disabled: true}\n  - {name: Three, steps: [d]}",
            Source::Repo,
        );
        let names: Vec<_> = l.recipes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["One v2", "Three"]);
        assert_eq!(l.recipes[0].source, Source::Repo);
        assert!(l.errors.is_empty());
    }

    #[test]
    fn one_bad_recipe_does_not_hide_the_rest() {
        let mut l = Loaded::default();
        add_source(&mut l, "f", "- {name: Good, steps: [a]}\n- {name: Bad, steps: []}", Source::Global);
        assert_eq!(l.recipes.len(), 1);
        assert_eq!(l.errors.len(), 1);
        assert!(l.errors[0].message.contains("Bad"));
    }

    #[test]
    fn yaml_syntax_error_is_reported_with_position() {
        let mut l = Loaded::default();
        add_source(&mut l, "f", "- name: X\n  steps: [a\n", Source::Global);
        assert_eq!(l.errors.len(), 1);
        assert!(l.errors[0].message.contains("line"), "{}", l.errors[0].message);
    }
}
