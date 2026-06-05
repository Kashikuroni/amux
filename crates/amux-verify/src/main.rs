//! amux-verify — run a verification contract in a worktree.
//!
//! Exit codes: 0 verdict passed · 1 verdict failed · 2 usage or contract
//! error · 130 interrupted.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use amux_verify::contract::CONTRACT_REL_PATH;
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

#[derive(Debug)]
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
                out.default_timeout = raw.parse().ok().filter(|&n| n >= 1).ok_or_else(|| {
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
    format!(
        "[{}/{}] {:<width$} … {}",
        index + 1,
        total,
        result.name,
        outcome
    )
}

fn failure_details(result: &GateResult) -> Vec<String> {
    let mut lines = vec![format!("      repro: {}", result.repro)];
    for (label, tail) in [
        ("stderr", &result.stderr_tail),
        ("stdout", &result.stdout_tail),
    ] {
        if !tail.is_empty() {
            lines.push(format!("      ── {label} ──"));
            lines.extend(tail.lines().map(|l| format!("      {l}")));
        }
    }
    lines
}

fn verdict_line(verdict: &Verdict) -> String {
    let count = |status: GateStatus| verdict.gates.iter().filter(|g| g.status == status).count();
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
                args.dir.join(CONTRACT_REL_PATH).display()
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
    let width = contract
        .gates
        .iter()
        .map(|g| g.name.len())
        .max()
        .unwrap_or(0);
    println!(
        "amux-verify: {total} gates from {}",
        contract_path.display()
    );

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
        assert!(parse_args(&strings(&["--dir"]))
            .unwrap_err()
            .contains("requires a value"));
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
        assert_eq!(
            gate_line(1, 5, 6, &result),
            "[2/5] clippy … FAILED  (exit 101, 4.1s)"
        );
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
            gates: vec![
                mk(GateStatus::Failed),
                mk(GateStatus::Passed),
                mk(GateStatus::Skipped),
            ],
        };
        assert_eq!(
            verdict_line(&verdict),
            "verdict: FAILED — 1 failed, 1 passed, 1 skipped"
        );
    }
}
