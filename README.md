# amux — agent multiplexer

A terminal cockpit for **AI coding agents**. `amux` manages parallel
[tmux](https://github.com/tmux/tmux) sessions — each running an agent like
**Claude Code**, **Codex**, **Gemini**, or **Aider** — and gives you a live
dashboard to watch them, answer their prompts, attach in one keystroke, spin up
git worktrees, and keep per‑session markdown to‑do notes.

> Think of it as `tmux` + a session list + live previews + notes, purpose‑built
> for juggling several coding agents at once.

<!-- Add a real screenshot / gif here. -->
<!-- ![amux screenshot](docs/screenshot.png) -->

---

## Features

- **Session dashboard** — every managed agent session in one list, grouped by
  project, with a live‑updating status (running / idle / waiting on you).
- **Live preview** — the selected session's terminal output, rendered with ANSI
  colors, without attaching. Scroll history with `Ctrl‑k/j` or `PgUp/PgDn`.
- **One‑key attach** — jump into a session with `Enter`, detach back with
  `Ctrl‑q`. No nested‑tmux headaches (a private socket keeps things isolated).
- **Answer prompts inline** — when an agent shows a numbered choice, press
  `1`–`9` from the dashboard; reply with free text via `i`.
- **Branch picker & git worktrees** — pick a branch right in the new‑session
  form: the current branch opens in place, any other branch opens in its
  worktree (reused or created on the fly), and an unmatched name becomes a
  fresh branch + worktree. The branch marker (`⎇` repo / `⧉` worktree) is
  color‑coded.
- **Per‑session notes & to‑do** — an Obsidian‑style markdown note per session,
  plus a persistent **project note**, shown in place of the preview. Toggle
  checkboxes, select tasks vim‑style and copy them as a numbered list. Progress
  (`3/5`) shows right on the card and on the project header.
- **Claude usage limits** — if you use Claude Code, the header shows your 5h / 7d
  limit windows (read from your local Claude credentials).
- **Keyboard‑first & layout‑independent** — hotkeys work on non‑Latin keyboard
  layouts (e.g. Russian ЙЦУКЕН) too.

## Requirements

`amux` is **macOS‑only** for now (see [Platform support](#platform-support)).

- **macOS** (Apple Silicon or Intel)
- **[tmux](https://github.com/tmux/tmux) ≥ 3.0** — `brew install tmux`
- **git** — for the worktree/branch features
- **At least one agent CLI** on your `PATH` — e.g. `claude`, `codex`, `gemini`,
  or `aider` (the default is `claude`)
- *(optional)* **curl** — for the Claude usage header (degrades gracefully if absent)

`amux` must **not** be launched from inside an existing tmux client.

## Install

### Quick install (script) — recommended

```sh
curl -fsSL https://raw.githubusercontent.com/Kashikuroni/amux/main/install.sh | sh
```

Detects your Mac's architecture, downloads the latest release binary, clears the
Gatekeeper quarantine, and puts `amux` on your PATH. Override the version or
location with `AMUX_VERSION=v0.1.0` / `AMUX_BIN_DIR=~/bin`.

### Prebuilt binary

Download the archive for your arch from the
[Releases](https://github.com/kashikuroni/amux/releases) page, then:

```sh
tar xzf amux-*-apple-darwin.tar.gz
xattr -d com.apple.quarantine amux   # unsigned binary: clear Gatekeeper
sudo mv amux /usr/local/bin/
```

### Prebuilt via cargo-binstall (no compile)

```sh
cargo binstall --git https://github.com/Kashikuroni/amux amux
```

Fetches the prebuilt binary straight from GitHub Releases — needs
[`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall), but no crates.io
and no compilation.

### From source (`cargo install`)

```sh
cargo install --git https://github.com/kashikuroni/amux
```

### Build locally

```sh
git clone https://github.com/kashikuroni/amux
cd amux
cargo build --release   # binary at target/release/amux
```

Prefer the short name? `alias am=amux`.

## Usage

```sh
amux
```

Press `n` to create your first session (name it, pick a directory and an agent),
then watch it run in the list. `?` opens the full keybinding help at any time.

### Keybindings

**Dashboard**

| Key | Action |
|-----|--------|
| `↑`/`↓`, `k`/`j` | Move selection |
| `s` then `1`–`9` | Jump to session N |
| `g` / `G` | First session / jump preview to latest |
| `n` / `N` | New session / new session in the selected project |
| `Enter` / `o` | Attach to the selected session (`Ctrl‑q` detaches) |
| `i` | Reply to the agent with free text (drafts persist per session; `Ctrl‑y` copy all, `Ctrl‑x` clear all) |
| `1`–`9` | Answer a numbered prompt the agent is showing |
| `d` | Kill the selected session |
| `e` | Open (or jump back into) an nvim session where the agent works |
| `v` | Verify the selected session (again to cancel) |
| `space` | Leader menu — everything rarer lives here (see below) |
| `J` / `K` | Reorder the selected session |
| `t` / `T` / `Tab` | Session note / Project note / focus the note |
| `Ctrl‑k`/`Ctrl‑j`, `PgUp`/`PgDn` | Scroll the preview |
| `[` `]` `{` `}`, `Ctrl‑←/→` | Resize the split |
| `/` | Filter sessions · `?` Help · `q` Quit (sessions keep running) |

**Leader menu** (`space`, which‑key style — the panel shows every option as you go)

| Chord | Action |
|-------|--------|
| `space g i` | Create a GitHub issue for the project (`gh` CLI) |
| `space g p` | Promote the worktree session to the repo root |
| `space g b` | Delete the session's branch |
| `space g c` | Clean up merged branches |
| `space s r` / `space s R` | Rename session / rename project |
| `space s v` / `space s V` | Verify / verification details |
| `space s e` | nvim in the agent's directory (also bare `e`) |
| `space a l` | Claude usage log |
| `space a o` | Other tmux sessions (not managed by am) |
| `space a u` | Restart all Claude sessions (re-`--resume` after a CLI update) |

**Notes — render mode** (after `Tab` into a note)

| Key | Action |
|-----|--------|
| `j`/`k` | Move between tasks |
| `space` | Toggle the checkbox |
| `V` then `j`/`k` | Select a range of tasks |
| `y` | Copy selected tasks to the clipboard as a numbered list |
| `d` | Delete the task (or the selected range) |
| `e` | Edit the raw markdown |
| `c` | Clear the note (asks `y`/`n`) |
| `Tab` | Defocus (keep the note shown, switch to another) |
| `Esc` | Exit notes back to the preview |

**Notes — edit mode**: type markdown freely (`- [ ] task`), `Enter` for a new
line, `Ctrl‑w`/`Ctrl‑u` to delete word/line, `Esc` to save & return to render.

## Configuration

Optional, at `~/.agent-multiplexer/config.toml`:

```toml
# Agent launched by default in the new-session form.
default_agent = "claude"

# Quick-pick agents offered in the new-session form.
agent_presets = ["claude", "aider", "codex"]

# How often (ms) the dashboard re-polls tmux for status/preview.
refresh_interval_ms = 1500
```

Persisted UI state (split width, session order, project names, and your notes)
lives at `~/.agent-multiplexer/state.toml` and is saved automatically.

## How it works

- Sessions run under a **dedicated tmux socket** (`-L cm`), isolated from your
  normal tmux server, and are tagged with `@cm_*` user options so `amux` only
  ever touches its own sessions.
- The UI core (`App`) is **pure / IO‑free**; all side effects are modeled as an
  `Action` enum performed by the single orchestrator in `main.rs`. This keeps the
  logic testable (180+ unit tests) and the rendering predictable.
- The Claude usage header reads your OAuth token from `~/.claude/.credentials.json`
  (or the macOS keychain) and calls `api.anthropic.com` read‑only. The token is
  never logged, written, or displayed.

## Platform support

**macOS only**, today. The app shells out to a few macOS‑specific tools:
`pbcopy` (clipboard) and `security` (keychain). One time helper still uses BSD
`date -r`. Linux support is feasible — it mostly needs a `date` portability fix
and an `xclip`/`wl-copy` clipboard path — and is welcome as a contribution.
Windows is not supported (POSIX shell + tmux required).

## Contributing

Issues and PRs welcome. See [ARCHITECTURE.md](ARCHITECTURE.md) for how the code
is organized (pure `App` + `Action` effects + a private tmux socket). Before
submitting:

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Some tests in `tests/tmux_integration.rs` require a real `tmux` on `PATH`.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache‑2.0](LICENSE-APACHE) at
your option. Unless you state otherwise, any contribution you submit shall be
dual‑licensed as above, without additional terms.
