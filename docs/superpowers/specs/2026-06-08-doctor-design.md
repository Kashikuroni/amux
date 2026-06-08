# Doctor — Design Spec

## Overview

`amux doctor` is a diagnostic + cleanup surface for tmux/agent detritus that the
normal amux UI **cannot see by design**. The main list only shows sessions on the
private `cm` socket tagged `@cm_managed=1`; everything else — untagged sessions,
dead panes, *other* tmux servers, and orphaned agent processes — is invisible and
accumulates silently. Doctor enumerates all of it and offers safe, explicit
cleanup.

Status: **backlog spec** (not yet implemented). Recommended MVP is a CLI
subcommand; an in-TUI `Mode::Doctor` is a follow-up.

---

## Motivation (real evidence)

While diagnosing "the device runs hot / where are my agents", a user with **5
sessions (2 active)** in amux turned out to have, in the tmux socket directory:

```
cm           ← the live amux socket (5 managed sessions)
default      ← the user's normal tmux
am-cwd-test  am_t_60624  am_test_42247  am_test_45941
amt3  amtest  amtest2  cmtest  cmdbg          ← 9 leaked servers
```

The 9 `am*`/`cmtest`/`cmdbg` sockets are **stale tmux servers/socket files left
over from manual and historical test/debug runs** (dates Jun 2–4) — leftover
runtime artifacts, not anything the *current* code creates (a repo-wide search
found no code referencing those names). Each may still hold sessions running
`claude`/`sh`, plus their multi-process node trees — burning CPU and context
switches that never show under amux's own PID and that the amux list never
reveals. The user could not find them because amux, by construction, looks only
at `cm`.

Two distinct problems surfaced:
1. **No visibility** into anything outside `cm` + `@cm_managed` → ghosts hang
   unseen, and they are runtime artifacts (a code fix can't retroactively remove
   them). (Doctor solves this.)
2. **Test isolation** (since fixed — see [Related](#related-not-doctor)): the
   integration tests ran against the live `cm` socket, so `cargo test` polluted
   the user's real amux. They now isolate onto a throwaway socket.

---

## Goals / Non-goals

**Goals**
- Surface, in one place, every user-owned tmux server/session and agent process
  amux normally hides, classified by safety.
- Let the user selectively kill a session / a whole foreign server / an orphan
  process, and bulk-clean the obviously-safe leftovers — always behind explicit
  confirmation.
- Be safe by default: never touch the live `cm` managed sessions or the user's
  `default` server without per-item confirmation.

**Non-goals**
- Managing healthy `cm` sessions (that's the normal amux UI).
- Killing arbitrary non-agent user processes.
- Fixing the test-teardown leak (related, but a separate change).

---

## What doctor detects (categories)

Enumerated by scanning the tmux socket directory (`#{socket_path}`'s dirname) and
the process table. Each finding is classified:

| Category | How detected | Default safety |
|---|---|---|
| **Untagged cm session** | session on `cm` with `@cm_managed` ≠ `1` | suspicious — show, confirm to kill |
| **Dead pane** | `cm` (or any) pane with `#{pane_dead}=1` | leftover — safe to clean |
| **Foreign amux server** | socket whose name matches amux test/debug patterns (`am*`, `cm*` except `cm`) and/or holds sessions tagged `@cm_*` | **safe to bulk-clean** |
| **Other server** | any other live socket (e.g. `default`, unknown `-L`) | **protected** — list only, per-item confirm |
| **Stale socket file** | socket file whose server is dead (`tmux -S … ls` errors) | safe to `rm` |
| **Orphan agent process** | `claude`/`node`/`codex` process not a descendant of any live tmux pane (esp. `ppid==1`) | suspicious — show, confirm to kill |

Pattern for "amux server" sockets MUST be conservative and centralized (one
constant), since it gates bulk deletion. Proposed: the live socket is exactly
`cm`; bulk-cleanable names match `^(am|cm)` AND are not `cm`/`default`. Unknown
names are never bulk-cleaned.

---

## Detection (how)

All read-only; no mutation during scan.

1. **Socket dir:** `dirname` of `tmux display -p '#{socket_path}'` (fallback to
   `${TMUX_TMPDIR:-/tmp}/tmux-$(uid)`). List socket files there.
2. **Per socket:** `tmux -S <sock> list-panes -a -F '<fmt>'` with
   `session_name`, `@cm_managed`, `pane_dead`, `pane_pid`, `pane_current_command`,
   `@cm_*` tags. A connection error ⇒ dead server ⇒ stale socket file.
3. **Processes:** the process table (e.g. `ps -axo pid,ppid,etime,pcpu,comm`),
   matched for agent commands; an agent pid whose ancestry never reaches a live
   pane pid (collected in step 2) is an orphan.
4. Reuse existing wrappers where possible (`src/tmux.rs` already shells tmux with
   `-L cm`; doctor needs a `-S <path>` / arbitrary-socket variant).

---

## UX

### MVP — CLI subcommand (recommended first)

`amux doctor` — print a classified report and exit (read-only):

```
amux doctor
  cm (managed)            5 sessions, 0 dead
  default (protected)     1 session
  LEAKED amux servers (9):
    amtest        2 sessions  (claude, sh)        ~3d old
    am_test_42247 0 sessions  (stale socket file)
    …
  orphan agents (2):
    pid 64096 node   etime=2-04:11  ppid=1
    pid 64097 node   etime=2-04:11  ppid=1
  → run `amux doctor --clean` to remove leaked amux servers + stale sockets
  → run `amux doctor --clean-orphans` to also kill orphan agent processes
```

- `amux doctor --clean` — `kill-server` each **leaked amux server** + `rm` stale
  socket files. Never touches `cm`/`default`/unknown servers. Prints what it did.
- `amux doctor --clean-orphans` — additionally `kill` orphan agent pids.
- A `--json` mode (mirrors the existing `amux-verify --json` convention) for
  scripting.

Rationale for CLI-first: no new UI state machine, scriptable, and the destructive
actions are explicit flags rather than keystrokes.

### Follow-up — in-TUI `Mode::Doctor`

A panel (entered via a key, e.g. a dedicated chord — `d` is taken by kill) that
renders the same categories as a navigable list, with per-row actions and a typed
confirmation for bulk clean (mirroring `Mode::ConfirmRestart`'s typed-word gate).
Defer until the CLI version proves the detection logic.

---

## Safety / confirmation

- **Never** auto-kill: `cm` sessions, the `default` server, or any unknown
  socket. These are list-only; killing requires an explicit, per-item action.
- Bulk `--clean` is gated to the conservative amux-server name pattern + stale
  files only.
- `--clean-orphans` (process kills) is a separate, opt-in flag — process killing
  is the most dangerous action.
- All destructive actions print exactly what they killed/removed (no silent
  cleanup), consistent with the repo's "no silent truncation" norm.

---

## Out of scope (v1)

- Touching healthy `cm` sessions.
- Cross-user / system-wide tmux (only the current user's socket dir).
- Auto-running on amux startup (doctor is explicit; a startup *hint* like
  "N leaked servers found — run `amux doctor`" is a possible later nicety).

---

## Related (not doctor): test isolation — DONE

Investigation correction: the *current* tests were NOT the source of the leaked
`am*` sockets. The only tmux-touching test, `tests/tmux_integration.rs`, ran
against the production `cm` socket with `Drop` guards that kill the sessions they
create — so it cleaned up its sessions but **polluted the user's live `cm`
server** during `cargo test` (and could leave sessions on `cm` if hard-killed).
The leaked `am*`/`cmtest`/`cmdbg` sockets are historical/manual artifacts no
current code creates.

Fixed independently of doctor: `tmux::isolate_socket(name)` (a `#[doc(hidden)]`
test hook backed by a thread-local socket override, thread-safe under cargo's
parallel tests) routes each integration test onto a throwaway socket and, on
guard drop, `kill-server`s it AND removes the lingering socket file (tmux's
`kill-server` does not reliably unlink it). Result: `cargo test` never touches
`cm` and leaves zero tmux state behind. Doctor still cleans the *historical*
runtime artifacts, which no code fix can retroactively remove.

---

## Open questions

- TUI entry key for the follow-up `Mode::Doctor` (every easy letter is taken;
  candidates: a chord, or `!`/`~`).
- Exact "amux server" socket pattern — confirm `^(am|cm)` minus `cm`/`default`
  is right, or tag-based detection (any socket holding `@cm_*`-tagged sessions)
  is safer than name-based.
- Should `amux doctor` (read-only) run automatically and print a one-line hint on
  normal `amux` startup when leaks are detected?
