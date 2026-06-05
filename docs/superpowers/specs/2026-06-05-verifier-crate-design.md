# amux-verify Crate — Design Spec (MVP items 1–2)

## Overview

A standalone workspace crate `amux-verify` that runs a per-repository
verification contract: an ordered list of gates (one command each) executed
in a session's worktree, producing a machine-readable verdict. This covers
items 1–2 of the verification MVP (`verification_feature.md` §8): contract
format + discovery, and the verifier crate (parser, cascade runner,
`Verdict`/`GateResult`, CLI).

amux itself does **not** link the crate yet — TUI integration
(`Action::Verify`, `verify.rs`, status badges) is items 3–6, a separate
milestone.

## Motivation

See `verification_feature.md` §1: the definition of "done" must live in a
machine-checkable contract next to the code, not in the operator's head.
The verifier is deliberately a separate crate (§5.3): it must be runnable
from CI and by hand, survive any amux UI refactor, and know nothing about
amux. The contract + cascade is the long-lived artifact.

## Decisions (approved 2026-06-05)

| Decision | Choice | Rationale |
|---|---|---|
| Workspace layout | Root package stays; add `[workspace] members = ["crates/amux-verify"]` | Zero file moves; CI/release/binstall untouched |
| `cmd` execution | argv, no shell; shell operators are a parse error | Spec §5.2 discipline; deterministic; complex logic goes into repo scripts (reusable in CI) |
| Library API | Synchronous core + event callback | Threading policy stays with the caller; amux wraps it in a thread + mpsc later, exactly as spec §5.2 describes |

## Scope

**In:** contract TOML format + validation, contract discovery, argv
splitter, cascade runner (timeout, kill, cancel, fail-fast), verdict types,
JSON serialization, `amux-verify` CLI, tests, dogfood contract for the amux
repo itself.

**Out (future milestones):** amux TUI integration (items 3–6), per-task
contracts (`.amux/tasks/<id>.toml`), gate defaults from amux `config.toml`,
test-adequacy gates (coverage, mutation, TDD guard, LLM judge), verdict
persistence, contract auto-scaffold.

---

## Crate layout

```
crates/amux-verify/
├── Cargo.toml        # serde (derive), toml, serde_json, libc (unix)
├── src/
│   ├── lib.rs        # re-exports, crate docs
│   ├── contract.rs   # Contract, Gate, ContractError, parse/load, find_contract
│   ├── argv.rs       # cmd string → argv splitter
│   ├── runner.rs     # run(), RunOptions, timeout/kill/cancel
│   └── main.rs       # amux-verify CLI
└── tests/
    └── cli.rs        # integration tests against the built binary
```

- Package `amux-verify`, lib `amux_verify`, bin `amux-verify`.
- `edition = "2021"`, `rust-version = "1.82"`, `license = "MIT OR Apache-2.0"`,
  `publish = false` (distribution story is amux's GitHub releases; revisit if
  ever published).
- `libc` is already in the dependency tree transitively; it is needed for
  process-group kill and the CLI SIGINT handler.

Root `Cargo.toml` gains only:

```toml
[workspace]
members = ["crates/amux-verify"]
```

**CI note:** `ci.yml` already runs `cargo fmt --all` and `cargo test --all`
(workspace-wide), but `cargo clippy --all-targets` lints the root package
only — it must become `cargo clippy --workspace --all-targets`.

---

## Contract format

Location: `.amux/verify.toml` at the worktree root.

```toml
[[gate]]
name      = "build"                 # required, unique, non-empty
cmd       = "cargo build --locked"  # required, non-empty, split into argv
timeout_s = 120                     # optional, >= 1; default: RunOptions.default_timeout_s
optional  = false                   # optional, default false; failures don't sink the verdict
```

### Validation (all at parse time, before anything runs)

Unlike `config.rs` (silent fallback to defaults), contract errors are
**loud** — a broken contract must not silently become "no gates, all
green". `Contract::parse` returns `Result<Contract, ContractError>`:

- unknown fields → error (`deny_unknown_fields`; catches `timout_s` typos)
- empty gate list → `NoGates`
- duplicate gate `name` → `DuplicateGateName`
- empty `name` or `cmd` → error
- `timeout_s = 0` → `InvalidTimeout`
- `cmd` is split into argv immediately; shell operators → `ShellOperator`
  error naming the token, with the hint *"shell operators are not
  supported; wrap the command in a script"*

```rust
pub struct Contract { pub gates: Vec<Gate> }

pub struct Gate {
    pub name: String,
    pub cmd: String,            // original string, kept for repro display
    pub argv: Vec<String>,      // parsed at contract load
    pub timeout_s: Option<u64>,
    pub optional: bool,
}
```

### Discovery

`find_contract(dir: &Path) -> Option<PathBuf>` checks exactly
`<dir>/.amux/verify.toml`. No upward walk: amux always passes the worktree
root; the CLI defaults `dir` to cwd and offers `--dir`/`--contract`
overrides.

---

## argv splitting (`argv.rs`)

Hand-rolled (~60 lines), no new dependencies. POSIX-flavoured:

- whitespace separates words; single/double quotes group; backslash
  escapes the next char (bare or inside double quotes)
- **rejected** when unquoted: `&` `|` `;` `<` `>` `$` `` ` `` `(` `)` —
  operators that would change command structure under a shell. Quoted they
  are literal arguments and pass.
- `*` `?` `~` `=` `#` are **allowed** and literal — there is no shell, so
  no globbing, home expansion, or comments. Documented behaviour.
- unterminated quote → error

Examples: `pytest -k 'not slow'` → `["pytest", "-k", "not slow"]`;
`cd ui && npm test` → error suggesting a script.

---

## Types and events

Per `verification_feature.md` §6.2–6.3, all `Serialize` (snake_case
statuses). Two deliberate deviations from the feature doc's sketch:

1. `task_id: Option<String>` (sketch: `String`) — a bare CLI run has no
   task id; `None` is skipped in JSON.
2. `stdout_tail` added next to `stderr_tail` — `cargo test` and `pytest`
   print failure details to **stdout**; stderr alone would blind triage.

```rust
pub enum GateStatus { Passed, Failed, Skipped, TimedOut }

pub struct GateResult {
    pub name: String,
    pub status: GateStatus,
    pub exit_code: Option<i32>,   // None = killed by signal / failed to spawn
    pub duration_ms: u128,
    pub stdout_tail: String,      // last 40 lines (TAIL_LINES const)
    pub stderr_tail: String,
    pub repro: String,            // original cmd string
}

pub struct Verdict {
    pub task_id: Option<String>,
    pub passed: bool,             // all non-optional gates Passed
    pub gates: Vec<GateResult>,
}

pub enum VerdictMsg {
    Started      { total_gates: usize },
    GateStarted  { index: usize, name: String },
    GateFinished { index: usize, result: GateResult },
    Finished     { verdict: Verdict },
}
```

---

## Runner (`runner.rs`)

```rust
pub struct RunOptions {
    pub default_timeout_s: u64,   // 300; per-gate timeout_s wins
    pub task_id: Option<String>,
    pub stream_output: bool,      // CLI -v: mirror gate output to stderr live
}

pub fn run(
    dir: &Path,
    contract: &Contract,
    opts: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(VerdictMsg),
) -> Verdict
```

Blocking, single-caller-thread (only short-lived internal pipe-drain
threads). Emits `Started`, then per gate `GateStarted`/`GateFinished`,
then `Finished` with the verdict (also returned).

Per gate:

1. `Command::new(&argv[0]).args(&argv[1..]).current_dir(dir)`,
   stdin null, stdout/stderr piped, **`process_group(0)`** (unix) so the
   gate and its children live in their own group.
2. Two drain threads read the pipes into ring buffers keeping the last 40
   lines each (a `cargo build` can emit megabytes; never buffer it all).
   With `stream_output`, drains also mirror lines to the process stderr.
3. Main thread polls `try_wait()` every ~25 ms, watching the deadline
   (`Instant`-based) and the `cancel` flag.
4. **Timeout** → `libc::kill(-pgid, SIGKILL)` (whole group — a hung test
   must not outlive its killed cargo parent), reap with `wait()`, status
   `TimedOut`.
5. **Cancel** (checked between gates *and* mid-gate — stronger than the
   feature doc, so amux's future `CancelVerify` never waits out a 600 s
   timeout): kill the group; the in-flight gate records `Skipped` with
   elapsed duration and captured tails; remaining gates record `Skipped`
   (duration 0) with `GateFinished` emitted for each.
6. Exit code 0 → `Passed`; non-zero → `Failed` (code recorded); killed by
   signal → `Failed` with `exit_code: None`; spawn failure (binary not
   found) → `Failed`, OS error text in `stderr_tail`.
7. **Fail-fast:** a non-optional gate finishing with any status other than
   `Passed` stops the cascade; remaining gates are emitted as `Skipped`.
   Optional failures never stop the cascade.

`verdict.passed` = every non-optional gate has status `Passed`. (Edge: a
run cancelled during a trailing optional gate can still report
`passed: true` — consistent with the rule; the optional gate could not
have changed it.)

---

## CLI (`main.rs`)

```
amux-verify [--dir <path>] [--contract <file>] [--json] [-v|--verbose]
            [--default-timeout <secs>] [--task-id <id>]
```

Flag parsing is hand-rolled (~30 lines); clap is not in the repo's
character. Defaults: `dir` = cwd, `contract` = `<dir>/.amux/verify.toml`.

Human output (default, stdout):

```
amux-verify: 5 gates from .amux/verify.toml
[1/5] build  … ok      (2.3s)
[2/5] clippy … FAILED  (exit 101, 4.1s)
      repro: cargo clippy --all-targets -- -D warnings
      <indented stderr tail>
[3/5] tests  … skipped
[4/5] mutants … skipped
[5/5] bundle … skipped
verdict: FAILED — 1 failed, 1 passed, 3 skipped
```

- `--json`: progress lines move to **stderr**, final `Verdict` as pretty
  JSON on **stdout** — `amux-verify --json > verdict.json` works in CI
  while progress stays visible.
- **Exit codes:** `0` verdict passed · `1` verdict failed · `2` usage or
  contract error (missing/invalid contract, bad flags) · `130` interrupted.
- **Ctrl+C:** the CLI installs a SIGINT handler (libc) that sets the same
  `cancel` flag; the runner kills the gate's process group and the CLI
  exits 130. Not optional polish: gate children sit in their own process
  groups, so the terminal's SIGINT never reaches them by itself.

---

## Error handling

`ContractError` enum with `Display`: `Io { path, source }`,
`Toml(toml::de::Error)` (carries line/col for unknown fields and syntax),
`NoGates`, `DuplicateGateName(name)`, `EmptyGateName`, `EmptyCmd { gate }`,
`InvalidTimeout { gate }`, `ShellOperator { gate, token }` (message
includes the wrap-it-in-a-script hint). The CLI prints the error to stderr
and exits 2. The runner itself does not error: every gate outcome is a
`GateResult`.

---

## Testing

Repo style: inline `#[cfg(test)]` units + crate-level integration tests.

- **argv.rs** — table tests: quoting, escapes, each rejected operator,
  unterminated quote, unicode.
- **contract.rs** — valid contract, duplicate names, unknown field, empty
  list, zero timeout, shell-operator error; `find_contract` on tempdirs.
- **runner.rs** — real cheap processes (`true`, `false`, `sleep`): pass,
  fail with exit code, fail-fast marks the rest skipped, optional failure
  doesn't sink the verdict, timeout kills (`sleep 5` + `timeout_s = 1`),
  pre-set cancel skips everything, mid-gate cancel, missing binary →
  `Failed`, event sequence order, tails keep only the last 40 lines.
  Unix-only bits behind `#[cfg(unix)]`.
- **tests/cli.rs** — integration via `env!("CARGO_BIN_EXE_amux-verify")`
  against fixture dirs: exit codes 0/1/2, `--json` output parses and
  matches the verdict shape.

### Acceptance (dogfood)

`.amux/verify.toml` lands in the amux repo itself:

```toml
[[gate]]
name = "fmt"
cmd  = "cargo fmt --all --check"
[[gate]]
name = "clippy"
cmd  = "cargo clippy --workspace --all-targets -- -D warnings"
[[gate]]
name = "tests"
cmd  = "cargo test --workspace"
timeout_s = 600
```

From the repo root, `cargo run -p amux-verify -- --dir .` is green and
exits 0.
