# amux — agent multiplexer

A terminal cockpit for **AI coding agents**. `amux` manages parallel
[tmux](https://github.com/tmux/tmux) sessions — each running an agent like
**Claude Code**, **Codex**, **Gemini**, or **Aider** — and gives you a live
dashboard to watch them, answer their prompts, attach in one keystroke, spin up
git worktrees, and keep per‑session markdown to‑do notes.

> Think of it as `tmux` + a session list + live previews + notes, purpose‑built
> for juggling several coding agents at once.

<!-- Replace OWNER with your GitHub user/org, and add a real screenshot/gif. -->
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
- **Git worktrees** — create a session on a fresh worktree + branch from the new‑
  session form; the branch marker (`⎇` repo / `⧉` worktree) is color‑coded.
- **Per‑session notes & to‑do** — an Obsidian‑style markdown note per session,
  plus a global **Inbox**, shown in place of the preview. Toggle checkboxes,
  select tasks vim‑style and copy them as a numbered list. Progress (`3/5`)
  shows right on the card.
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

> Replace `OWNER` with the GitHub owner once the repo is published.

### Homebrew (recommended on macOS)

```sh
brew install OWNER/tap/amux
```

### Prebuilt binary

Download the archive for your arch from the
[Releases](https://github.com/OWNER/amux/releases) page, then:

```sh
tar xzf amux-*-apple-darwin.tar.gz
xattr -d com.apple.quarantine amux   # unsigned binary: clear Gatekeeper
sudo mv amux /usr/local/bin/
```

### From source (`cargo install`)

```sh
cargo install --git https://github.com/OWNER/amux
```

### Build locally

```sh
git clone https://github.com/OWNER/amux
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
| `i` | Reply to the agent with free text |
| `1`–`9` | Answer a numbered prompt the agent is showing |
| `d` / `r` / `R` | Kill / rename session / rename project |
| `J` / `K` | Reorder the selected session |
| `t` / `T` / `Tab` | Session note / Inbox note / focus the note |
| `Ctrl‑k`/`Ctrl‑j`, `PgUp`/`PgDn` | Scroll the preview |
| `[` `]` `{` `}`, `Ctrl‑←/→` | Resize the split |
| `/` | Filter sessions · `?` Help · `q` Quit (sessions keep running) |

**Notes — render mode** (after `Tab` into a note)

| Key | Action |
|-----|--------|
| `j`/`k` | Move between tasks |
| `space` | Toggle the checkbox |
| `V` then `j`/`k` | Select a range of tasks |
| `y` | Copy selected tasks to the clipboard as a numbered list |
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

Issues and PRs welcome. Before submitting:

```sh
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Some tests in `tests/tmux_integration.rs` require a real `tmux` on `PATH`.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache‑2.0](LICENSE-APACHE) at
your option. Unless you state otherwise, any contribution you submit shall be
dual‑licensed as above, without additional terms.
