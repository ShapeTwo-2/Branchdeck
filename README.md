# Branchdeck

A real terminal with a sidebar of one-click **workflow buttons** for common git tasks.

Every button types its commands into the visible terminal, so you always see exactly
what runs. Branchdeck uses the `git` CLI, which means your aliases, hooks and
credential helpers keep working.

> **Status:** early development. The terminal and recipe buttons work; the git status
> bar and safety features are in progress. The full recipe reference lands with v1.

## Recipes

A recipe is a named list of shell steps with optional inputs. Branchdeck loads them from
the built-in set, then `~/.branchdeck/recipes.yml` (just for you), then `.branchdeck.yml`
in the repo root (shared with your team). A later recipe with the same `id` replaces an
earlier one; `- { id: bcp, disabled: true }` hides one.

```yaml
- id: bcp
  name: New branch + commit + push
  description: Create a branch, commit all changes, push and set upstream
  confirm: false              # ask before running
  inputs:
    - { id: branch, label: Branch name, type: branch, required: true }   # text | branch | select | bool
    - { id: msg, label: Commit message, required: true }
  steps:
    - git checkout -b "{{branch}}"
    - git add -A
    - git commit -m "{{msg}}"
    - git push -u origin "{{branch}}"
    - run: git checkout main  # a step can also be an object
      always: true            # runs even if an earlier step failed
      when: "!some_bool"      # flag, !flag, name == value, name != value
```

- Values are always inserted as a single safely-quoted shell word, so nothing typed into a
  form can run extra commands. Placeholders inside single quotes are rejected.
- Built-in variables: `{{current_branch}}`, `{{default_branch}}`, `{{repo_root}}`.
- Steps run one at a time in the terminal; the first failing step stops the recipe.
- YAML gotcha: a step containing `": "` must be wrapped in single quotes,
  e.g. `- 'git commit -m "fix: {{msg}}"'`.

Recipes run in bash or zsh. Branchdeck starts the shell with a small prompt hook (in
`~/.branchdeck/shell/`) that loads your usual startup files and reports each command's exit code.

## Development

Prerequisites: [Node.js](https://nodejs.org) 20+, [Rust](https://rustup.rs) (stable) and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS (WebView2 and
the MSVC Build Tools on Windows). On Windows you also need [Git for Windows](https://git-scm.com/download/win),
which provides Git Bash.

```sh
npm install
npm run tauri dev      # run the app with hot reload
npm run tauri build    # build an installer for your platform
cd src-tauri && cargo test
```

## Configuration

Branchdeck reads optional settings from `~/.branchdeck/config.yml`:

```yaml
shell:
  path: C:\Program Files\Git\bin\bash.exe   # default: Git Bash on Windows, $SHELL elsewhere
  args: [--login, -i]                       # default depends on the shell
startDir: ~/code                            # default: your home directory
```

## License

[MIT](LICENSE)
