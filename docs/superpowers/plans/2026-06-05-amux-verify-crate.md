# amux-verify Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Standalone workspace crate `amux-verify` (lib + CLI) that parses a `.amux/verify.toml` contract and runs its gates as a fail-fast cascade in a worktree, producing a `Verdict`.

**Architecture:** Synchronous, single-threaded core (`run()` + event callback) per the approved spec `docs/superpowers/specs/2026-06-05-verifier-crate-design.md`. Gates execute as argv without a shell, each in its own process group; timeout/cancel kill the whole group. amux does NOT link the crate yet — that is the next milestone.

**Tech Stack:** Rust 2021 (rust-version 1.82), serde + toml + serde_json, libc (unix only, for group kill + SIGINT). No clap, no tempfile — hand-rolled arg parsing and test TempDir, matching the repo's dependency-light style.

**Worktree:** All work happens in the existing worktree `/Users/kashikuroni/projects/pets/agents_multiplexer/.worktrees/feat/verification` (branch `feat/verification`). All paths below are relative to its root.

**Conventions for every task:** run `cargo fmt --all` before each commit; tests for a module live in `#[cfg(test)] mod tests` inside that module (repo style). The runner/CLI tests use real binaries (`true`, `false`, `sh`, `sleep`, `seq`, `echo`) — present on macOS and Linux. Invoking `sh -c '...'` *explicitly* in a test gate is legitimate: the runner just doesn't wrap commands in a shell implicitly.

---

## File structure

| File | Responsibility |
|---|---|
| `Cargo.toml` (root, modify) | becomes workspace root (3 lines) |
| `.github/workflows/ci.yml` (modify) | clippy over the whole workspace |
| `crates/amux-verify/Cargo.toml` (create) | crate manifest |
| `crates/amux-verify/src/lib.rs` (create) | crate docs, module decls, re-exports |
| `crates/amux-verify/src/argv.rs` (create) | no-shell cmd → argv splitter |
| `crates/amux-verify/src/contract.rs` (create) | `Contract`/`Gate`/`ContractError`, parse/load/find |
| `crates/amux-verify/src/runner.rs` (create) | verdict types + cascade runner |
| `crates/amux-verify/src/testutil.rs` (create) | `#[cfg(test)]` TempDir helper |
| `crates/amux-verify/src/main.rs` (create) | `amux-verify` CLI |
| `crates/amux-verify/tests/cli.rs` (create) | integration tests against the built binary |
| `.amux/verify.toml` (create) | dogfood contract for amux itself |
| `CHANGELOG.md` (modify) | Unreleased → Added entry |

---

### Task 1: Workspace scaffold + CI clippy fix

**Files:**
- Modify: `Cargo.toml` (root)
- Modify: `.github/workflows/ci.yml:34`
- Create: `crates/amux-verify/Cargo.toml`
- Create: `crates/amux-verify/src/lib.rs`
- Create: `crates/amux-verify/src/main.rs`

- [ ] **Step 1: Add `[workspace]` to the root `Cargo.toml`**

Append at the end of the file (after the `[dependencies]` block):

```toml

[workspace]
members = ["crates/amux-verify"]
```

(The root package stays a package and becomes the workspace root; nothing moves.)

- [ ] **Step 2: Create `crates/amux-verify/Cargo.toml`**

```toml
[package]
name = "amux-verify"
version = "0.1.0"
edition = "2021"
rust-version = "1.82"
description = "Contract-based verification runner: executes gates from .amux/verify.toml and reports a verdict"
license = "MIT OR Apache-2.0"
repository = "https://github.com/kashikuroni/amux"
# Distribution story is amux's GitHub releases; revisit if ever published.
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

[target.'cfg(unix)'.dependencies]
libc = "0.2"
```

- [ ] **Step 3: Create minimal `crates/amux-verify/src/lib.rs`**

```rust
//! Contract-based verification runner for amux sessions.
//!
//! Parses `.amux/verify.toml` (the *contract*: an ordered list of *gates*,
//! one command each), runs the gates in a worktree without a shell, and
//! reports a *verdict*. Knows nothing about amux itself — also usable from
//! CI or by hand via the `amux-verify` binary.
```

- [ ] **Step 4: Create stub `crates/amux-verify/src/main.rs`**

```rust
fn main() {}
```

- [ ] **Step 5: Fix CI clippy scope**

In `.github/workflows/ci.yml` line 34, change:

```yaml
        run: cargo clippy --all-targets -- -D warnings
```

to:

```yaml
        run: cargo clippy --workspace --all-targets -- -D warnings
```

(Without `--workspace`, clippy lints only the root package and would never see the new crate. `fmt --all` and `test --all` on lines 31/37 are already workspace-wide.)

- [ ] **Step 6: Verify the workspace builds**

Run: `cargo check --workspace`
Expected: compiles both `amux` and `amux-verify` with no errors (Cargo.lock gains an `amux-verify` entry).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates .github/workflows/ci.yml
git commit -m "feat(verify): scaffold amux-verify workspace crate; lint whole workspace in CI

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: argv splitter

**Files:**
- Create: `crates/amux-verify/src/argv.rs`
- Modify: `crates/amux-verify/src/lib.rs`

- [ ] **Step 1: Declare the module**

In `lib.rs`, append after the doc comment:

```rust
pub mod argv;
```

- [ ] **Step 2: Write the failing tests**

Create `argv.rs` containing ONLY the tests module for now (the `use super::*;` items don't exist yet — that's the point):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_words() {
        assert_eq!(
            split("cargo build --locked").unwrap(),
            vec!["cargo", "build", "--locked"]
        );
    }

    #[test]
    fn collapses_repeated_whitespace() {
        assert_eq!(split("a  \t b").unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn single_quotes_group_and_keep_backslashes() {
        assert_eq!(
            split("pytest -k 'not slow'").unwrap(),
            vec!["pytest", "-k", "not slow"]
        );
        assert_eq!(split(r"echo 'a\b'").unwrap(), vec!["echo", r"a\b"]);
    }

    #[test]
    fn double_quotes_group_and_unescape() {
        assert_eq!(split(r#"echo "a b""#).unwrap(), vec!["echo", "a b"]);
        assert_eq!(split(r#"echo "a\"b""#).unwrap(), vec!["echo", r#"a"b"#]);
    }

    #[test]
    fn bare_backslash_escapes_next_char() {
        assert_eq!(split(r"echo a\ b").unwrap(), vec!["echo", "a b"]);
    }

    #[test]
    fn empty_quotes_make_empty_arg() {
        assert_eq!(split("run ''").unwrap(), vec!["run", ""]);
    }

    #[test]
    fn quoted_operators_are_literal() {
        assert_eq!(split("grep '&&' src").unwrap(), vec!["grep", "&&", "src"]);
        assert_eq!(split(r#"echo "$HOME""#).unwrap(), vec!["echo", "$HOME"]);
    }

    #[test]
    fn glob_tilde_equals_hash_are_literal_words() {
        assert_eq!(
            split("pytest tests/* -x? ~/x FOO=bar #tag").unwrap(),
            vec!["pytest", "tests/*", "-x?", "~/x", "FOO=bar", "#tag"]
        );
    }

    #[test]
    fn unicode_survives() {
        assert_eq!(split("echo 'тест юникода'").unwrap(), vec!["echo", "тест юникода"]);
    }

    #[test]
    fn rejects_unquoted_operators() {
        for (cmd, tok) in [
            ("a && b", "&&"),
            ("a || b", "||"),
            ("a | b", "|"),
            ("a ; b", ";"),
            ("a > f", ">"),
            ("a < f", "<"),
            ("echo $HOME", "$"),
            ("echo `id`", "`"),
            ("(a)", "("),
            ("a)", ")"),
            ("a & b", "&"),
        ] {
            assert_eq!(
                split(cmd).unwrap_err(),
                SplitError::ShellOperator(tok.to_string()),
                "cmd: {cmd}"
            );
        }
    }

    #[test]
    fn rejects_unterminated_quotes_and_trailing_backslash() {
        assert_eq!(split("echo 'abc").unwrap_err(), SplitError::UnterminatedQuote);
        assert_eq!(split(r#"echo "abc"#).unwrap_err(), SplitError::UnterminatedQuote);
        assert_eq!(split(r"echo abc\").unwrap_err(), SplitError::TrailingBackslash);
    }

    #[test]
    fn empty_input_splits_to_no_words() {
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split("   ").unwrap(), Vec::<String>::new());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p amux-verify`
Expected: compile error — `split` and `SplitError` not found.

- [ ] **Step 4: Implement the splitter**

Add ABOVE the tests module in `argv.rs`:

```rust
//! Splits a gate `cmd` string into argv without invoking a shell.
//!
//! POSIX-flavoured quoting: whitespace separates words, single/double
//! quotes group, backslash escapes the next character (bare or inside
//! double quotes; literal inside single quotes). Unquoted shell operators
//! are rejected: there is no shell at run time, so `&&` or `$VAR` would
//! reach the program as literal arguments — never what the author meant.
//! `*`, `?`, `~`, `=`, `#` are allowed and stay literal (no globbing, no
//! expansion, no comments).

/// Characters that would change command structure under a shell. Unquoted
/// occurrences are errors; quoted they are literal arguments.
const OPERATORS: &[char] = &['&', '|', ';', '<', '>', '$', '`', '(', ')'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    UnterminatedQuote,
    TrailingBackslash,
    /// An unquoted shell operator (the offending token).
    ShellOperator(String),
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitError::UnterminatedQuote => write!(f, "unterminated quote"),
            SplitError::TrailingBackslash => write!(f, "trailing backslash"),
            SplitError::ShellOperator(tok) => write!(
                f,
                "shell operators are not supported ({tok}); wrap the command in a script"
            ),
        }
    }
}

impl std::error::Error for SplitError {}

pub fn split(cmd: &str) -> Result<Vec<String>, SplitError> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return Err(SplitError::UnterminatedQuote),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c) => current.push(c),
                            None => return Err(SplitError::UnterminatedQuote),
                        },
                        Some(c) => current.push(c),
                        None => return Err(SplitError::UnterminatedQuote),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(c) => {
                    in_word = true;
                    current.push(c);
                }
                None => return Err(SplitError::TrailingBackslash),
            },
            c if OPERATORS.contains(&c) => {
                let token = if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    format!("{c}{c}")
                } else {
                    c.to_string()
                };
                return Err(SplitError::ShellOperator(token));
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all argv tests PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): no-shell argv splitter with loud operator rejection

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Contract parsing with loud validation

**Files:**
- Create: `crates/amux-verify/src/contract.rs`
- Modify: `crates/amux-verify/src/lib.rs`

- [ ] **Step 1: Declare module + re-exports in `lib.rs`**

```rust
pub mod argv;
pub mod contract;

pub use contract::{Contract, ContractError, Gate};
```

- [ ] **Step 2: Write the failing tests**

Create `contract.rs` with only the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[[gate]]
name      = "build"
cmd       = "cargo build --locked"
timeout_s = 120

[[gate]]
name     = "clippy"
cmd      = "cargo clippy -- -D warnings"
optional = true
"#;

    #[test]
    fn parses_valid_contract() {
        let contract = Contract::parse(VALID).unwrap();
        assert_eq!(contract.gates.len(), 2);
        let build = &contract.gates[0];
        assert_eq!(build.name, "build");
        assert_eq!(build.cmd, "cargo build --locked");
        assert_eq!(build.argv, vec!["cargo", "build", "--locked"]);
        assert_eq!(build.timeout_s, Some(120));
        assert!(!build.optional);
        let clippy = &contract.gates[1];
        assert_eq!(clippy.timeout_s, None);
        assert!(clippy.optional);
    }

    #[test]
    fn empty_contract_is_no_gates() {
        assert!(matches!(Contract::parse("").unwrap_err(), ContractError::NoGates));
    }

    #[test]
    fn unknown_field_is_loud() {
        let toml = "[[gate]]\nname = \"a\"\ncmd = \"true\"\ntimout_s = 5\n";
        let err = Contract::parse(toml).unwrap_err();
        assert!(matches!(err, ContractError::Toml(_)));
        assert!(err.to_string().contains("timout_s"), "err: {err}");
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let toml =
            "[[gate]]\nname = \"a\"\ncmd = \"true\"\n[[gate]]\nname = \"a\"\ncmd = \"false\"\n";
        assert!(matches!(
            Contract::parse(toml).unwrap_err(),
            ContractError::DuplicateGateName(name) if name == "a"
        ));
    }

    #[test]
    fn empty_name_and_cmd_are_rejected() {
        let toml = "[[gate]]\nname = \" \"\ncmd = \"true\"\n";
        assert!(matches!(Contract::parse(toml).unwrap_err(), ContractError::EmptyGateName));
        let toml = "[[gate]]\nname = \"a\"\ncmd = \"  \"\n";
        assert!(matches!(
            Contract::parse(toml).unwrap_err(),
            ContractError::EmptyCmd { gate } if gate == "a"
        ));
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let toml = "[[gate]]\nname = \"a\"\ncmd = \"true\"\ntimeout_s = 0\n";
        assert!(matches!(
            Contract::parse(toml).unwrap_err(),
            ContractError::InvalidTimeout { gate } if gate == "a"
        ));
    }

    #[test]
    fn shell_operator_error_names_gate_and_hints_script() {
        let toml = "[[gate]]\nname = \"ui\"\ncmd = \"cd ui && npm test\"\n";
        let err = Contract::parse(toml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("\"ui\""), "msg: {msg}");
        assert!(msg.contains("&&"), "msg: {msg}");
        assert!(msg.contains("wrap the command in a script"), "msg: {msg}");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p amux-verify contract`
Expected: compile error — `Contract`, `ContractError` not found.

- [ ] **Step 4: Implement types, validation, and errors**

Add above the tests in `contract.rs`:

```rust
//! The verification contract: `.amux/verify.toml` parsed into ordered gates.
//!
//! Unlike amux's `Config` (silent fallback to defaults), contract errors
//! are loud: a broken contract must fail verification, not silently
//! degrade to "no gates, all green".

use std::path::PathBuf;

use serde::Deserialize;

use crate::argv::{self, SplitError};

#[derive(Debug, Clone)]
pub struct Contract {
    pub gates: Vec<Gate>,
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub name: String,
    /// Original command string, kept verbatim for repro display.
    pub cmd: String,
    /// `cmd` split into argv at parse time (no shell at run time).
    pub argv: Vec<String>,
    pub timeout_s: Option<u64>,
    pub optional: bool,
}

#[derive(Debug)]
pub enum ContractError {
    Io { path: PathBuf, source: std::io::Error },
    Toml(toml::de::Error),
    NoGates,
    DuplicateGateName(String),
    EmptyGateName,
    EmptyCmd { gate: String },
    InvalidTimeout { gate: String },
    /// The cmd string didn't split (shell operator, unterminated quote…).
    Split { gate: String, source: SplitError },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractError::Io { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
            ContractError::Toml(err) => write!(f, "invalid contract TOML: {err}"),
            ContractError::NoGates => write!(f, "contract has no gates"),
            ContractError::DuplicateGateName(name) => {
                write!(f, "duplicate gate name \"{name}\"")
            }
            ContractError::EmptyGateName => write!(f, "gate with empty name"),
            ContractError::EmptyCmd { gate } => write!(f, "gate \"{gate}\": empty cmd"),
            ContractError::InvalidTimeout { gate } => {
                write!(f, "gate \"{gate}\": timeout_s must be >= 1")
            }
            ContractError::Split { gate, source } => write!(f, "gate \"{gate}\": {source}"),
        }
    }
}

impl std::error::Error for ContractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ContractError::Io { source, .. } => Some(source),
            ContractError::Toml(err) => Some(err),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContract {
    #[serde(default)]
    gate: Vec<RawGate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate {
    name: String,
    cmd: String,
    timeout_s: Option<u64>,
    #[serde(default)]
    optional: bool,
}

impl Contract {
    pub fn parse(toml_str: &str) -> Result<Contract, ContractError> {
        let raw: RawContract = toml::from_str(toml_str).map_err(ContractError::Toml)?;
        if raw.gate.is_empty() {
            return Err(ContractError::NoGates);
        }
        let mut gates: Vec<Gate> = Vec::with_capacity(raw.gate.len());
        for gate in raw.gate {
            if gate.name.trim().is_empty() {
                return Err(ContractError::EmptyGateName);
            }
            if gates.iter().any(|g| g.name == gate.name) {
                return Err(ContractError::DuplicateGateName(gate.name));
            }
            if gate.timeout_s == Some(0) {
                return Err(ContractError::InvalidTimeout { gate: gate.name });
            }
            let argv = argv::split(&gate.cmd).map_err(|source| ContractError::Split {
                gate: gate.name.clone(),
                source,
            })?;
            if argv.is_empty() || argv[0].is_empty() {
                return Err(ContractError::EmptyCmd { gate: gate.name });
            }
            gates.push(Gate {
                name: gate.name,
                cmd: gate.cmd,
                argv,
                timeout_s: gate.timeout_s,
                optional: gate.optional,
            });
        }
        Ok(Contract { gates })
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p amux-verify contract`
Expected: all contract tests PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): contract parsing with loud validation

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: TempDir test helper + `load`/`find_contract`

**Files:**
- Create: `crates/amux-verify/src/testutil.rs`
- Modify: `crates/amux-verify/src/contract.rs`
- Modify: `crates/amux-verify/src/lib.rs`

- [ ] **Step 1: Create `testutil.rs`**

```rust
//! Test-only helpers (the repo avoids dev-dependencies like `tempfile`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// RAII temp dir under the system temp root; removed on drop.
/// (`pub(crate)` everywhere — keeps clippy's `new_without_default` quiet.)
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("amux-verify-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// Writes a file under the dir, creating parent dirs as needed.
    pub(crate) fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
```

- [ ] **Step 2: Update `lib.rs`**

```rust
pub mod argv;
pub mod contract;
#[cfg(test)]
pub(crate) mod testutil;

pub use contract::{find_contract, Contract, ContractError, Gate};
```

- [ ] **Step 3: Write the failing tests**

Append inside `mod tests` in `contract.rs`:

```rust
    #[test]
    fn load_reads_file_and_missing_file_is_io_error() {
        let td = crate::testutil::TempDir::new();
        let path = td.write(".amux/verify.toml", "[[gate]]\nname = \"a\"\ncmd = \"true\"\n");
        let contract = Contract::load(&path).unwrap();
        assert_eq!(contract.gates[0].name, "a");

        let missing = td.path().join("nope.toml");
        assert!(matches!(
            Contract::load(&missing).unwrap_err(),
            ContractError::Io { .. }
        ));
    }

    #[test]
    fn find_contract_checks_exact_location() {
        let td = crate::testutil::TempDir::new();
        assert_eq!(find_contract(td.path()), None);
        let written = td.write(".amux/verify.toml", "[[gate]]\nname = \"a\"\ncmd = \"true\"\n");
        assert_eq!(find_contract(td.path()), Some(written));
    }
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p amux-verify contract`
Expected: compile error — `load`, `find_contract` not found.

- [ ] **Step 5: Implement `load` and `find_contract`**

In `contract.rs`: change the `use std::path::PathBuf;` line to `use std::path::{Path, PathBuf};`, add inside `impl Contract`:

```rust
    pub fn load(path: &Path) -> Result<Contract, ContractError> {
        let contents = std::fs::read_to_string(path).map_err(|source| ContractError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Contract::parse(&contents)
    }
```

and add at module level (below `impl Contract`):

```rust
/// Where a contract lives relative to a worktree root.
pub const CONTRACT_REL_PATH: &str = ".amux/verify.toml";

/// Returns the contract path if `<dir>/.amux/verify.toml` exists. No
/// upward walk: amux always passes the worktree root, and the CLI has
/// `--dir`/`--contract` overrides.
pub fn find_contract(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(CONTRACT_REL_PATH);
    path.is_file().then_some(path)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): contract load + discovery; TempDir test helper

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Verdict types with JSON serialization

**Files:**
- Create: `crates/amux-verify/src/runner.rs`
- Modify: `crates/amux-verify/src/lib.rs`

- [ ] **Step 1: Declare module + re-exports in `lib.rs`**

Final `lib.rs` state:

```rust
pub mod argv;
pub mod contract;
pub mod runner;
#[cfg(test)]
pub(crate) mod testutil;

pub use contract::{find_contract, Contract, ContractError, Gate};
pub use runner::{GateResult, GateStatus, Verdict, VerdictMsg};
```

- [ ] **Step 2: Write the failing test**

Create `runner.rs`:

```rust
//! The cascade runner: executes a contract's gates in order inside a
//! worktree, reporting progress through events and returning a verdict.

use serde::Serialize;

/// Last lines of each output stream kept per gate.
pub const TAIL_LINES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Failed,
    Skipped,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateResult {
    pub name: String,
    pub status: GateStatus,
    /// `None` when the gate was killed by a signal or failed to spawn.
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    /// Last [`TAIL_LINES`] lines of stdout — `cargo test`/`pytest` print
    /// failure details there; stderr alone would blind triage.
    pub stdout_tail: String,
    pub stderr_tail: String,
    /// The original `cmd` string, for re-running by hand.
    pub repro: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// `true` when every non-optional gate passed.
    pub passed: bool,
    pub gates: Vec<GateResult>,
}

/// Progress events emitted while a contract runs (feature doc §6.3).
#[derive(Debug, Clone)]
pub enum VerdictMsg {
    Started { total_gates: usize },
    GateStarted { index: usize, name: String },
    GateFinished { index: usize, result: GateResult },
    Finished { verdict: Verdict },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_json_uses_snake_case_and_omits_missing_task_id() {
        let verdict = Verdict {
            task_id: None,
            passed: false,
            gates: vec![GateResult {
                name: "tests".into(),
                status: GateStatus::TimedOut,
                exit_code: None,
                duration_ms: 1500,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                repro: "cargo test".into(),
            }],
        };
        let json = serde_json::to_value(&verdict).unwrap();
        assert!(json.get("task_id").is_none());
        assert_eq!(json["passed"], false);
        assert_eq!(json["gates"][0]["status"], "timed_out");
        assert_eq!(json["gates"][0]["exit_code"], serde_json::Value::Null);

        let tagged = Verdict { task_id: Some("s1".into()), ..verdict };
        assert_eq!(serde_json::to_value(&tagged).unwrap()["task_id"], "s1");
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p amux-verify runner`
Expected: PASS (types + derives are the implementation here; the test pins the JSON wire shape: snake_case statuses, absent `task_id` omitted, `exit_code: null`).

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): verdict types with pinned JSON wire shape

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Runner core — cascade, exit codes, fail-fast, output capture

**Files:**
- Modify: `crates/amux-verify/src/runner.rs`
- Modify: `crates/amux-verify/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` in `runner.rs`:

```rust
    use crate::contract::{Contract, Gate};
    use crate::testutil::TempDir;
    use std::sync::atomic::AtomicBool;

    fn gate(name: &str, cmd: &str) -> Gate {
        Gate {
            name: name.into(),
            cmd: cmd.into(),
            argv: crate::argv::split(cmd).unwrap(),
            timeout_s: None,
            optional: false,
        }
    }

    /// Runs gates in a fresh temp dir, recording a compact event trace.
    fn run_collect(gates: Vec<Gate>, cancel: &AtomicBool) -> (Verdict, Vec<String>) {
        let td = TempDir::new();
        let contract = Contract { gates };
        let mut events = Vec::new();
        let verdict = run(
            td.path(),
            &contract,
            &RunOptions::default(),
            cancel,
            &mut |msg| {
                events.push(match &msg {
                    VerdictMsg::Started { total_gates } => format!("started:{total_gates}"),
                    VerdictMsg::GateStarted { index, .. } => format!("gs:{index}"),
                    VerdictMsg::GateFinished { index, result } => {
                        format!("gf:{index}:{:?}", result.status)
                    }
                    VerdictMsg::Finished { .. } => "finished".into(),
                });
            },
        );
        (verdict, events)
    }

    #[test]
    fn passing_cascade_emits_ordered_events() {
        let (verdict, events) =
            run_collect(vec![gate("a", "true"), gate("b", "true")], &AtomicBool::new(false));
        assert!(verdict.passed);
        assert_eq!(verdict.gates[0].status, GateStatus::Passed);
        assert_eq!(verdict.gates[0].exit_code, Some(0));
        assert_eq!(verdict.gates[0].repro, "true");
        assert_eq!(
            events,
            vec!["started:2", "gs:0", "gf:0:Passed", "gs:1", "gf:1:Passed", "finished"]
        );
    }

    #[test]
    fn failed_gate_records_code_and_skips_the_rest() {
        // `sh -c '...'` invoked EXPLICITLY is fine — the runner just never
        // wraps commands in a shell implicitly.
        let (verdict, events) = run_collect(
            vec![gate("bad", "sh -c 'exit 3'"), gate("after", "true")],
            &AtomicBool::new(false),
        );
        assert!(!verdict.passed);
        assert_eq!(verdict.gates[0].status, GateStatus::Failed);
        assert_eq!(verdict.gates[0].exit_code, Some(3));
        assert_eq!(verdict.gates[1].status, GateStatus::Skipped);
        assert_eq!(
            events,
            vec!["started:2", "gs:0", "gf:0:Failed", "gf:1:Skipped", "finished"]
        );
    }

    #[test]
    fn optional_failure_does_not_sink_the_verdict() {
        let mut soft = gate("soft", "false");
        soft.optional = true;
        let (verdict, _) =
            run_collect(vec![soft, gate("hard", "true")], &AtomicBool::new(false));
        assert!(verdict.passed);
        assert_eq!(verdict.gates[0].status, GateStatus::Failed);
        assert_eq!(verdict.gates[1].status, GateStatus::Passed);
    }

    #[test]
    fn missing_binary_is_a_failed_gate() {
        let (verdict, _) = run_collect(
            vec![gate("ghost", "amux-verify-no-such-binary")],
            &AtomicBool::new(false),
        );
        assert_eq!(verdict.gates[0].status, GateStatus::Failed);
        assert_eq!(verdict.gates[0].exit_code, None);
        assert!(!verdict.gates[0].stderr_tail.is_empty());
    }

    #[test]
    fn gate_output_is_captured() {
        let (verdict, _) = run_collect(
            vec![gate("noise", "sh -c 'echo out; echo err >&2'")],
            &AtomicBool::new(false),
        );
        assert_eq!(verdict.gates[0].stdout_tail, "out");
        assert_eq!(verdict.gates[0].stderr_tail, "err");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p amux-verify runner`
Expected: compile error — `run`, `RunOptions` not found.

- [ ] **Step 3: Implement the runner**

Add to `runner.rs` below the existing `use serde::Serialize;`:

```rust
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::contract::{Contract, Gate};
```

Add below the `VerdictMsg` enum:

```rust
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// How long to wait for pipe drains after the child is reaped. Normally
/// they finish instantly on EOF; a gate that hands its pipes to a
/// double-forked daemon could hold them open forever — never let that
/// hang the verification.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

pub struct RunOptions {
    /// Timeout for gates without `timeout_s` (per-repo defaults from amux
    /// config come in a later milestone; the crate default is 300 s).
    pub default_timeout_s: u64,
    pub task_id: Option<String>,
    /// Mirror gate output to this process's stderr as it arrives (CLI -v).
    pub stream_output: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            default_timeout_s: 300,
            task_id: None,
            stream_output: false,
        }
    }
}

/// Runs the contract's gates in order inside `dir`. Blocking; emits
/// progress through `on_event` and returns the final verdict (also sent
/// as [`VerdictMsg::Finished`]). The caller owns threading: amux will
/// call this from a background thread with a channel-sending callback.
pub fn run(
    dir: &Path,
    contract: &Contract,
    opts: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(VerdictMsg),
) -> Verdict {
    on_event(VerdictMsg::Started { total_gates: contract.gates.len() });

    let mut results: Vec<GateResult> = Vec::with_capacity(contract.gates.len());
    let mut halted = false;

    for (index, gate) in contract.gates.iter().enumerate() {
        if halted {
            let result = skipped(gate);
            on_event(VerdictMsg::GateFinished { index, result: result.clone() });
            results.push(result);
            continue;
        }
        on_event(VerdictMsg::GateStarted { index, name: gate.name.clone() });
        let result = run_gate(dir, gate, opts, cancel);
        if result.status != GateStatus::Passed && !gate.optional {
            halted = true; // fail-fast: no point running `test` if `build` broke
        }
        on_event(VerdictMsg::GateFinished { index, result: result.clone() });
        results.push(result);
    }

    let passed = contract
        .gates
        .iter()
        .zip(&results)
        .filter(|(gate, _)| !gate.optional)
        .all(|(_, result)| result.status == GateStatus::Passed);
    let verdict = Verdict {
        task_id: opts.task_id.clone(),
        passed,
        gates: results,
    };
    on_event(VerdictMsg::Finished { verdict: verdict.clone() });
    verdict
}

fn run_gate(dir: &Path, gate: &Gate, opts: &RunOptions, _cancel: &AtomicBool) -> GateResult {
    let start = Instant::now();

    let mut command = Command::new(&gate.argv[0]);
    command
        .args(&gate.argv[1..])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The gate gets its own process group so a later kill (timeout or
    // cancel) takes its children down too, not just the direct child.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return GateResult {
                name: gate.name.clone(),
                status: GateStatus::Failed,
                exit_code: None,
                duration_ms: start.elapsed().as_millis(),
                stdout_tail: String::new(),
                stderr_tail: err.to_string(),
                repro: gate.cmd.clone(),
            };
        }
    };

    let stdout = drain(child.stdout.take().expect("stdout is piped"), opts.stream_output);
    let stderr = drain(child.stderr.take().expect("stderr is piped"), opts.stream_output);

    let (status, exit_code) = loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                break match exit.code() {
                    Some(0) => (GateStatus::Passed, Some(0)),
                    Some(code) => (GateStatus::Failed, Some(code)),
                    None => (GateStatus::Failed, None), // killed by a signal
                };
            }
            Ok(None) => {}
            Err(_) => {
                // try_wait failing is exotic; treat it as a gate failure.
                kill_gate(&mut child);
                break (GateStatus::Failed, None);
            }
        }
        thread::sleep(POLL_INTERVAL);
    };

    GateResult {
        name: gate.name.clone(),
        status,
        exit_code,
        duration_ms: start.elapsed().as_millis(),
        stdout_tail: stdout.collect(),
        stderr_tail: stderr.collect(),
        repro: gate.cmd.clone(),
    }
}

fn skipped(gate: &Gate) -> GateResult {
    GateResult {
        name: gate.name.clone(),
        status: GateStatus::Skipped,
        exit_code: None,
        duration_ms: 0,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        repro: gate.cmd.clone(),
    }
}

/// Kills the gate's whole process group (the child and everything it
/// spawned), then reaps the child so no zombie is left behind.
fn kill_gate(child: &mut Child) {
    #[cfg(unix)]
    // SAFETY: plain kill(2). The child was made its own group leader via
    // process_group(0), so its pgid equals its pid and the negative pid
    // addresses the whole group.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.kill(); // non-unix path and belt-and-braces
    let _ = child.wait();
}

/// Reads a pipe to EOF on a thread, collecting lines.
struct Drain {
    lines: Arc<Mutex<Vec<String>>>,
    done: Arc<AtomicBool>,
}

fn drain(reader: impl Read + Send + 'static, mirror: bool) -> Drain {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(AtomicBool::new(false));
    let handle = Drain {
        lines: Arc::clone(&lines),
        done: Arc::clone(&done),
    };
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if mirror {
                eprintln!("{line}");
            }
            lines.lock().unwrap().push(line);
        }
        done.store(true, Ordering::SeqCst);
    });
    handle
}

impl Drain {
    /// Waits briefly for EOF, then snapshots whatever arrived (bounded by
    /// [`DRAIN_GRACE`] so a daemonized grandchild can't hang us).
    fn collect(self) -> String {
        let started = Instant::now();
        while !self.done.load(Ordering::SeqCst) && started.elapsed() < DRAIN_GRACE {
            thread::sleep(Duration::from_millis(5));
        }
        self.lines.lock().unwrap().join("\n")
    }
}
```

Also update the re-export line in `lib.rs` to:

```rust
pub use runner::{run, GateResult, GateStatus, RunOptions, Verdict, VerdictMsg};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS (the `_cancel` underscore is intentional — cancellation lands in Task 9).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): cascade runner — exit codes, fail-fast, output capture

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Bound output tails to the last 40 lines

**Files:**
- Modify: `crates/amux-verify/src/runner.rs`

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` in `runner.rs`:

```rust
    #[test]
    fn output_tails_keep_only_the_last_lines() {
        let (verdict, _) =
            run_collect(vec![gate("noisy", "seq 1 50")], &AtomicBool::new(false));
        let tail = &verdict.gates[0].stdout_tail;
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.first(), Some(&"11"));
        assert_eq!(lines.last(), Some(&"50"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p amux-verify output_tails_keep_only`
Expected: FAIL — 50 lines captured, not 40.

- [ ] **Step 3: Switch `Drain` to a capped ring buffer**

In `runner.rs`: add `use std::collections::VecDeque;` to the imports, then replace the `Drain` struct, `drain` fn, and `impl Drain` entirely with:

```rust
/// Reads a pipe to EOF on a thread, keeping only the last [`TAIL_LINES`]
/// lines — a `cargo build` can emit megabytes; never buffer it all.
struct Drain {
    lines: Arc<Mutex<VecDeque<String>>>,
    done: Arc<AtomicBool>,
}

fn drain(reader: impl Read + Send + 'static, mirror: bool) -> Drain {
    let lines = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_LINES)));
    let done = Arc::new(AtomicBool::new(false));
    let handle = Drain {
        lines: Arc::clone(&lines),
        done: Arc::clone(&done),
    };
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if mirror {
                eprintln!("{line}");
            }
            let mut lines = lines.lock().unwrap();
            if lines.len() == TAIL_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
        }
        done.store(true, Ordering::SeqCst);
    });
    handle
}

impl Drain {
    /// Waits briefly for EOF, then snapshots whatever arrived (bounded by
    /// [`DRAIN_GRACE`] so a daemonized grandchild can't hang us).
    fn collect(self) -> String {
        let started = Instant::now();
        while !self.done.load(Ordering::SeqCst) && started.elapsed() < DRAIN_GRACE {
            thread::sleep(Duration::from_millis(5));
        }
        let lines = self.lines.lock().unwrap();
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): bound gate output tails to the last 40 lines

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: Gate timeout kills the whole process group

**Files:**
- Modify: `crates/amux-verify/src/runner.rs`

- [ ] **Step 1: Write the failing test**

Append inside `mod tests`:

```rust
    #[test]
    fn timeout_kills_the_gate() {
        let mut slow = gate("slow", "sleep 5");
        slow.timeout_s = Some(1);
        let started = Instant::now();
        let (verdict, _) = run_collect(vec![slow], &AtomicBool::new(false));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "runner did not honor the timeout"
        );
        assert_eq!(verdict.gates[0].status, GateStatus::TimedOut);
        assert_eq!(verdict.gates[0].exit_code, None);
        assert!(!verdict.passed);
    }
```

(Also add `use std::time::{Duration, Instant};` to the tests module imports if not already in scope there.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p amux-verify timeout_kills_the_gate`
Expected: FAIL — the gate runs the full 5 s and exits 0 → status `Passed` (and the elapsed assertion trips).

- [ ] **Step 3: Implement the deadline**

In `run_gate`, add after `let start = Instant::now();`:

```rust
    let timeout = Duration::from_secs(gate.timeout_s.unwrap_or(opts.default_timeout_s));
```

and add inside the poll loop, after the `match child.try_wait()` block (before `thread::sleep`):

```rust
        if start.elapsed() >= timeout {
            kill_gate(&mut child);
            break (GateStatus::TimedOut, None);
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS (~1 s for the timeout test).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): gate timeout kills the whole process group

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 9: Cancellation — pre-run and mid-gate

**Files:**
- Modify: `crates/amux-verify/src/runner.rs`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` (also add `Ordering` to the test imports: `use std::sync::atomic::{AtomicBool, Ordering};` replacing the earlier `AtomicBool`-only import, and `use std::sync::Arc;`):

```rust
    #[test]
    fn preset_cancel_skips_every_gate() {
        let (verdict, events) =
            run_collect(vec![gate("a", "true"), gate("b", "true")], &AtomicBool::new(true));
        assert!(!verdict.passed);
        assert!(verdict.gates.iter().all(|g| g.status == GateStatus::Skipped));
        assert_eq!(events, vec!["started:2", "gf:0:Skipped", "gf:1:Skipped", "finished"]);
    }

    #[test]
    fn midgate_cancel_kills_the_gate_promptly() {
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&cancel);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            setter.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let (verdict, _) =
            run_collect(vec![gate("slow", "sleep 5"), gate("after", "true")], &cancel);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "cancel did not interrupt the gate"
        );
        assert_eq!(verdict.gates[0].status, GateStatus::Skipped);
        assert!(verdict.gates[0].duration_ms >= 200);
        assert_eq!(verdict.gates[1].status, GateStatus::Skipped);
        assert!(!verdict.passed);
    }
```

(`use std::thread;` is already imported at module level and visible via `use super::*;`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p amux-verify cancel`
Expected: `preset_cancel_skips_every_gate` FAILS (gates run and pass); `midgate_cancel_kills_the_gate_promptly` FAILS (gate sleeps the full 5 s).

- [ ] **Step 3: Implement cancellation**

In `run()`, change the skip condition from `if halted {` to:

```rust
        if halted || cancel.load(Ordering::SeqCst) {
```

In `run_gate`, rename the `_cancel` parameter to `cancel` and add inside the poll loop, BEFORE the timeout check:

```rust
        if cancel.load(Ordering::SeqCst) {
            kill_gate(&mut child);
            break (GateStatus::Skipped, None);
        }
```

(The mid-gate check is deliberately stronger than the feature doc's "between gates": amux's future `CancelVerify` must never wait out a 600 s timeout. The killed gate records `Skipped` — it was never judged — with its elapsed duration and captured tails.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src
git commit -m "feat(verify): cancellation — pre-run and mid-gate, kills the gate group

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 10: CLI — args, human output, exit codes 0/1/2

**Files:**
- Modify: `crates/amux-verify/src/main.rs` (replace the stub entirely)
- Create: `crates/amux-verify/tests/cli.rs`

- [ ] **Step 1: Write the failing integration tests**

Create `crates/amux-verify/tests/cli.rs`:

```rust
//! Integration tests driving the built amux-verify binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_amux-verify");

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Local copy of the crate's test TempDir: integration tests are a
/// separate crate and cannot see `#[cfg(test)]` items from the lib.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("amux-verify-cli-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn verify(dir: &Path, extra: &[&str]) -> Output {
    Command::new(BIN)
        .arg("--dir")
        .arg(dir)
        .args(extra)
        .output()
        .expect("run amux-verify")
}

#[test]
fn passing_contract_exits_zero() {
    let td = TempDir::new();
    td.write(".amux/verify.toml", "[[gate]]\nname = \"ok\"\ncmd = \"true\"\n");
    let out = verify(td.path(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("verdict: PASSED"), "stdout: {stdout}");
    assert!(stdout.contains("[1/1] ok"), "stdout: {stdout}");
}

#[test]
fn failing_contract_exits_one_with_repro() {
    let td = TempDir::new();
    td.write(".amux/verify.toml", "[[gate]]\nname = \"bad\"\ncmd = \"false\"\n");
    let out = verify(td.path(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout.contains("verdict: FAILED"), "stdout: {stdout}");
    assert!(stdout.contains("repro: false"), "stdout: {stdout}");
}

#[test]
fn missing_contract_exits_two() {
    let td = TempDir::new();
    let out = verify(td.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no contract"));
}

#[test]
fn invalid_contract_exits_two_with_hint() {
    let td = TempDir::new();
    td.write(".amux/verify.toml", "[[gate]]\nname = \"ui\"\ncmd = \"cd ui && npm test\"\n");
    let out = verify(td.path(), &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("shell operators"), "stderr: {stderr}");
}

#[test]
fn unknown_flag_exits_two() {
    let out = Command::new(BIN).arg("--frobnicate").output().expect("run amux-verify");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument"));
}

#[test]
fn help_exits_zero() {
    let out = Command::new(BIN).arg("--help").output().expect("run amux-verify");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("usage: amux-verify"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p amux-verify --test cli`
Expected: tests run (stub binary builds) and FAIL — exits 0 with empty output everywhere.

- [ ] **Step 3: Implement the CLI**

Replace `crates/amux-verify/src/main.rs` entirely:

```rust
//! amux-verify — run a verification contract in a worktree.
//!
//! Exit codes: 0 verdict passed · 1 verdict failed · 2 usage or contract
//! error · 130 interrupted.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use amux_verify::{
    find_contract, run, Contract, GateResult, GateStatus, RunOptions, Verdict, VerdictMsg,
};

const USAGE: &str = "\
usage: amux-verify [--dir <path>] [--contract <file>] [-v|--verbose]
                   [--default-timeout <secs>] [--task-id <id>]

Runs the gates from <dir>/.amux/verify.toml (or --contract) inside <dir>.
Gate commands run without a shell; exit code 0 means the gate passed.

  --dir <path>              worktree to verify (default: current directory)
  --contract <file>         contract path (default: <dir>/.amux/verify.toml)
  -v, --verbose             mirror gate output live to stderr
  --default-timeout <secs>  timeout for gates without timeout_s (default 300)
  --task-id <id>            tag the verdict with a task id
";

struct CliArgs {
    dir: PathBuf,
    contract: Option<PathBuf>,
    verbose: bool,
    default_timeout: u64,
    task_id: Option<String>,
}

impl Default for CliArgs {
    fn default() -> Self {
        CliArgs {
            dir: PathBuf::from("."),
            contract: None,
            verbose: false,
            default_timeout: 300,
            task_id: None,
        }
    }
}

fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    let mut out = CliArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dir" => out.dir = PathBuf::from(value(&mut it, "--dir")?),
            "--contract" => out.contract = Some(PathBuf::from(value(&mut it, "--contract")?)),
            "-v" | "--verbose" => out.verbose = true,
            "--default-timeout" => {
                let raw = value(&mut it, "--default-timeout")?;
                out.default_timeout = raw
                    .parse()
                    .ok()
                    .filter(|&n| n >= 1)
                    .ok_or_else(|| {
                        format!("--default-timeout: expected a positive number, got {raw}")
                    })?;
            }
            "--task-id" => out.task_id = Some(value(&mut it, "--task-id")?),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(out)
}

fn value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, String> {
    it.next()
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn gate_line(index: usize, total: usize, width: usize, result: &GateResult) -> String {
    let secs = result.duration_ms as f64 / 1000.0;
    let outcome = match result.status {
        GateStatus::Passed => format!("ok      ({secs:.1}s)"),
        GateStatus::Failed => match result.exit_code {
            Some(code) => format!("FAILED  (exit {code}, {secs:.1}s)"),
            None => format!("FAILED  ({secs:.1}s)"),
        },
        GateStatus::TimedOut => format!("TIMEOUT ({secs:.1}s)"),
        GateStatus::Skipped => "skipped".into(),
    };
    format!("[{}/{}] {:<width$} … {}", index + 1, total, result.name, outcome)
}

fn failure_details(result: &GateResult) -> Vec<String> {
    let mut lines = vec![format!("      repro: {}", result.repro)];
    for (label, tail) in [("stderr", &result.stderr_tail), ("stdout", &result.stdout_tail)] {
        if !tail.is_empty() {
            lines.push(format!("      ── {label} ──"));
            lines.extend(tail.lines().map(|l| format!("      {l}")));
        }
    }
    lines
}

fn verdict_line(verdict: &Verdict) -> String {
    let count = |status: GateStatus| {
        verdict.gates.iter().filter(|g| g.status == status).count()
    };
    let mut parts = Vec::new();
    for (n, label) in [
        (count(GateStatus::Failed), "failed"),
        (count(GateStatus::TimedOut), "timed out"),
        (count(GateStatus::Passed), "passed"),
        (count(GateStatus::Skipped), "skipped"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {label}"));
        }
    }
    let outcome = if verdict.passed { "PASSED" } else { "FAILED" };
    format!("verdict: {outcome} — {}", parts.join(", "))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(&args) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("amux-verify: {msg}");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let contract_path = match args.contract.clone().or_else(|| find_contract(&args.dir)) {
        Some(path) => path,
        None => {
            eprintln!(
                "amux-verify: no contract at {}",
                args.dir.join(".amux/verify.toml").display()
            );
            return ExitCode::from(2);
        }
    };
    let contract = match Contract::load(&contract_path) {
        Ok(contract) => contract,
        Err(err) => {
            eprintln!("amux-verify: {err}");
            return ExitCode::from(2);
        }
    };

    let opts = RunOptions {
        default_timeout_s: args.default_timeout,
        task_id: args.task_id.clone(),
        stream_output: args.verbose,
    };
    let total = contract.gates.len();
    let width = contract.gates.iter().map(|g| g.name.len()).max().unwrap_or(0);
    println!("amux-verify: {total} gates from {}", contract_path.display());

    let cancel = AtomicBool::new(false);
    let verdict = run(&args.dir, &contract, &opts, &cancel, &mut |msg| {
        if let VerdictMsg::GateFinished { index, result } = &msg {
            println!("{}", gate_line(*index, total, width, result));
            if matches!(result.status, GateStatus::Failed | GateStatus::TimedOut) {
                for line in failure_details(result) {
                    println!("{line}");
                }
            }
        }
    });

    println!("{}", verdict_line(&verdict));
    ExitCode::from(if verdict.passed { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args_defaults_and_flags() {
        let args = parse_args(&strings(&[])).unwrap();
        assert_eq!(args.dir, PathBuf::from("."));
        assert_eq!(args.default_timeout, 300);
        assert!(args.task_id.is_none());
        assert!(!args.verbose);

        let args = parse_args(&strings(&[
            "--dir",
            "/tmp/wt",
            "--contract",
            "c.toml",
            "-v",
            "--default-timeout",
            "60",
            "--task-id",
            "s1",
        ]))
        .unwrap();
        assert_eq!(args.dir, PathBuf::from("/tmp/wt"));
        assert_eq!(args.contract, Some(PathBuf::from("c.toml")));
        assert!(args.verbose);
        assert_eq!(args.default_timeout, 60);
        assert_eq!(args.task_id.as_deref(), Some("s1"));
    }

    #[test]
    fn parse_args_rejects_unknown_and_bad_values() {
        assert!(parse_args(&strings(&["--frobnicate"]))
            .unwrap_err()
            .contains("unknown argument"));
        assert!(parse_args(&strings(&["--dir"])).unwrap_err().contains("requires a value"));
        assert!(parse_args(&strings(&["--default-timeout", "0"]))
            .unwrap_err()
            .contains("positive"));
        assert!(parse_args(&strings(&["--default-timeout", "x"]))
            .unwrap_err()
            .contains("positive"));
    }

    #[test]
    fn gate_line_formats_statuses() {
        let result = GateResult {
            name: "clippy".into(),
            status: GateStatus::Failed,
            exit_code: Some(101),
            duration_ms: 4100,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            repro: "cargo clippy".into(),
        };
        assert_eq!(gate_line(1, 5, 6, &result), "[2/5] clippy … FAILED  (exit 101, 4.1s)");
    }

    #[test]
    fn verdict_line_counts_outcomes() {
        let mk = |status: GateStatus| GateResult {
            name: "g".into(),
            status,
            exit_code: None,
            duration_ms: 0,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            repro: "x".into(),
        };
        let verdict = Verdict {
            task_id: None,
            passed: false,
            gates: vec![mk(GateStatus::Failed), mk(GateStatus::Passed), mk(GateStatus::Skipped)],
        };
        assert_eq!(verdict_line(&verdict), "verdict: FAILED — 1 failed, 1 passed, 1 skipped");
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all unit + integration tests PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src crates/amux-verify/tests
git commit -m "feat(verify): amux-verify CLI — human output, exit codes 0/1/2

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 11: `--json` verdict output

**Files:**
- Modify: `crates/amux-verify/src/main.rs`
- Modify: `crates/amux-verify/tests/cli.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/cli.rs`:

```rust
#[test]
fn json_mode_prints_verdict_on_stdout_and_progress_on_stderr() {
    let td = TempDir::new();
    td.write(".amux/verify.toml", "[[gate]]\nname = \"bad\"\ncmd = \"false\"\n");
    let out = verify(td.path(), &["--json", "--task-id", "s1"]);
    assert_eq!(out.status.code(), Some(1));
    let verdict: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is a JSON verdict");
    assert_eq!(verdict["passed"], false);
    assert_eq!(verdict["task_id"], "s1");
    assert_eq!(verdict["gates"][0]["status"], "failed");
    assert!(!out.stderr.is_empty(), "progress should be on stderr");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p amux-verify --test cli json_mode`
Expected: FAIL — `unknown argument: --json` (exit 2).

- [ ] **Step 3: Implement `--json`**

In `main.rs`:

1. Add field `json: bool` to `CliArgs` (and `json: false` to its `Default`).
2. Add a match arm in `parse_args`: `"--json" => out.json = true,`.
3. Add to `USAGE` after the `--contract` line:

```
  --json                    print the final verdict as JSON on stdout
                            (progress lines move to stderr)
```

and add `[--json]` to the first usage line:

```
usage: amux-verify [--dir <path>] [--contract <file>] [--json] [-v|--verbose]
```

4. Add a routing helper and use it for ALL progress lines in `main` (replace every `println!` for the header, gate lines, failure details, and verdict line with `progress(args.json, ...)`):

```rust
/// Progress goes to stdout normally; with --json stdout is reserved for
/// the verdict, so progress moves to stderr.
fn progress(json: bool, line: &str) {
    if json {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}
```

In `main`, the calls become:

```rust
    progress(args.json, &format!("amux-verify: {total} gates from {}", contract_path.display()));
    // ... inside the run callback:
            progress(args.json, &gate_line(*index, total, width, result));
            // ...
                    progress(args.json, &line);
    // ... after run():
    progress(args.json, &verdict_line(&verdict));
```

5. After the `verdict_line` call, before computing the exit code, add:

```rust
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&verdict).expect("verdict serializes")
        );
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src crates/amux-verify/tests
git commit -m "feat(verify): --json verdict on stdout, progress on stderr

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 12: SIGINT cancels the run, exit 130

**Files:**
- Modify: `crates/amux-verify/src/main.rs`
- Modify: `crates/amux-verify/tests/cli.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/cli.rs`:

```rust
#[cfg(unix)]
#[test]
fn sigint_cancels_the_run_and_exits_130() {
    use std::time::{Duration, Instant};

    let td = TempDir::new();
    td.write(".amux/verify.toml", "[[gate]]\nname = \"slow\"\ncmd = \"sleep 5\"\n");
    let mut child = Command::new(BIN)
        .arg("--dir")
        .arg(td.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn amux-verify");
    std::thread::sleep(Duration::from_millis(400)); // let the gate start
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "amux-verify did not exit after SIGINT"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(status.code(), Some(130));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p amux-verify --test cli sigint`
Expected: FAIL — default SIGINT disposition kills the CLI immediately with signal 2 → `status.code()` is `None`, not `Some(130)`. (This also documents WHY the handler matters: the gate in its own process group would survive an unhandled Ctrl+C.)

- [ ] **Step 3: Implement the handler**

In `main.rs`:

1. Change the atomics import to `use std::sync::atomic::{AtomicBool, Ordering};` and add a static + handler installer below `USAGE`:

```rust
static CANCEL: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn install_sigint_handler() {
    extern "C" fn on_sigint(_: libc::c_int) {
        CANCEL.store(true, Ordering::SeqCst);
    }
    let handler = on_sigint as extern "C" fn(libc::c_int);
    // SAFETY: the handler only performs an atomic store, which is
    // async-signal-safe; replacing the default SIGINT disposition is the
    // whole point — gate children sit in their own process groups, so the
    // terminal's Ctrl+C never reaches them by itself.
    unsafe { libc::signal(libc::SIGINT, handler as libc::sighandler_t) };
}

#[cfg(not(unix))]
fn install_sigint_handler() {}
```

2. In `main`, delete the local `let cancel = AtomicBool::new(false);`, call `install_sigint_handler();` right after the contract loads successfully, and pass `&CANCEL` to `run(...)` instead of `&cancel`.

3. Change the final exit-code computation to:

```rust
    if CANCEL.load(Ordering::SeqCst) {
        return ExitCode::from(130);
    }
    ExitCode::from(if verdict.passed { 0 } else { 1 })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p amux-verify`
Expected: all PASS (~0.5 s for the SIGINT test).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/amux-verify/src crates/amux-verify/tests
git commit -m "feat(verify): SIGINT cancels the run and exits 130

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 13: Dogfood contract, CHANGELOG, full acceptance

**Files:**
- Create: `.amux/verify.toml` (repo root)
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Write the dogfood contract**

Create `.amux/verify.toml` at the repo root:

```toml
# Verification contract for amux itself (dogfood). Run with:
#   cargo run -p amux-verify -- --dir .

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

- [ ] **Step 2: Acceptance run**

Run: `cargo run -p amux-verify -- --dir . && echo "exit: $?"`
Expected output (durations vary; tests gate takes a few minutes — it includes the tmux integration tests and this crate's sleep-based tests):

```
amux-verify: 3 gates from ./.amux/verify.toml
[1/3] fmt    … ok      (0.…s)
[2/3] clippy … ok      (…s)
[3/3] tests  … ok      (…s)
verdict: PASSED — 3 passed
exit: 0
```

If `fmt` or `clippy` fail: fix the reported issues (`cargo fmt --all`, address clippy warnings), re-run.

- [ ] **Step 3: Add the CHANGELOG entry**

In `CHANGELOG.md`, add a bullet at the TOP of the `### Added` list under `## [Unreleased]`:

```markdown
- `amux-verify`: a standalone workspace crate + CLI that runs the repo's
  verification contract (`.amux/verify.toml`) — ordered gates executed
  without a shell in a worktree, fail-fast cascade, per-gate timeout with
  process-group kill, `--json` verdict. Foundation for in-app verification
  (items 1–2 of the verification MVP).
```

- [ ] **Step 4: Final full-suite check**

Run: `cargo test --workspace`
Expected: all green (amux + amux-verify).

- [ ] **Step 5: Commit**

```bash
git add .amux/verify.toml CHANGELOG.md
git commit -m "feat(verify): dogfood contract for amux; changelog entry

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Out of scope (next milestones)

amux TUI integration (`Action::Verify`, `verify.rs` thread+channel wrapper, status enum + badges, detail panel) — items 3–6 of the MVP; per-task contracts; gate defaults from amux `config.toml`; test-adequacy gates; verdict persistence.
