//! The verification contract: `.amux/verify.toml` parsed into ordered gates.
//!
//! Unlike amux's `Config` (silent fallback to defaults), contract errors
//! are loud: a broken contract must fail verification, not silently
//! degrade to "no gates, all green".

use std::path::{Path, PathBuf};

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
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Toml(toml::de::Error),
    NoGates,
    DuplicateGateName(String),
    EmptyGateName,
    EmptyCmd {
        gate: String,
    },
    InvalidTimeout {
        gate: String,
    },
    /// The cmd string didn't split (shell operator, unterminated quote…).
    Split {
        gate: String,
        source: SplitError,
    },
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

    pub fn load(path: &Path) -> Result<Contract, ContractError> {
        let contents = std::fs::read_to_string(path).map_err(|source| ContractError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Contract::parse(&contents)
    }
}

/// Where a contract lives relative to a worktree root.
pub const CONTRACT_REL_PATH: &str = ".amux/verify.toml";

/// Returns the contract path if `<dir>/.amux/verify.toml` exists. No
/// upward walk: amux always passes the worktree root, and the CLI has
/// `--dir`/`--contract` overrides.
pub fn find_contract(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(CONTRACT_REL_PATH);
    path.is_file().then_some(path)
}

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
        assert!(matches!(
            Contract::parse("").unwrap_err(),
            ContractError::NoGates
        ));
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
        assert!(matches!(
            Contract::parse(toml).unwrap_err(),
            ContractError::EmptyGateName
        ));
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

    #[test]
    fn load_reads_file_and_missing_file_is_io_error() {
        let td = crate::testutil::TempDir::new();
        let path = td.write(
            ".amux/verify.toml",
            "[[gate]]\nname = \"a\"\ncmd = \"true\"\n",
        );
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
        let written = td.write(
            ".amux/verify.toml",
            "[[gate]]\nname = \"a\"\ncmd = \"true\"\n",
        );
        assert_eq!(find_contract(td.path()), Some(written));
    }
}
