# Branchdeck

A real terminal with a sidebar of one-click **workflow buttons** for common git tasks.

Every button types its commands into the visible terminal, so you always see exactly
what runs. Branchdeck uses the `git` CLI, which means your aliases, hooks and
credential helpers keep working.

> **Status:** early development. Milestone 1 (terminal) is done; recipes, forms, the
> status bar and safety features are in progress. The full recipe reference lands
> with v1.

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
