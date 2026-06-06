//! The cascade runner: executes a contract's gates in order inside a
//! worktree, reporting progress through events and returning a verdict.

use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::contract::{Contract, Gate};

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
    on_event(VerdictMsg::Started {
        total_gates: contract.gates.len(),
    });

    let mut results: Vec<GateResult> = Vec::with_capacity(contract.gates.len());
    let mut halted = false;

    for (index, gate) in contract.gates.iter().enumerate() {
        if halted {
            let result = skipped(gate);
            on_event(VerdictMsg::GateFinished {
                index,
                result: result.clone(),
            });
            results.push(result);
            continue;
        }
        on_event(VerdictMsg::GateStarted {
            index,
            name: gate.name.clone(),
        });
        let result = run_gate(dir, gate, opts, cancel);
        if result.status != GateStatus::Passed && !gate.optional {
            halted = true; // fail-fast: no point running `test` if `build` broke
        }
        on_event(VerdictMsg::GateFinished {
            index,
            result: result.clone(),
        });
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
    on_event(VerdictMsg::Finished {
        verdict: verdict.clone(),
    });
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

    let stdout = drain(
        child.stdout.take().expect("stdout is piped"),
        opts.stream_output,
    );
    let stderr = drain(
        child.stderr.take().expect("stderr is piped"),
        opts.stream_output,
    );

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

#[cfg(test)]
mod tests {
    use super::*;

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
        let (verdict, events) = run_collect(
            vec![gate("a", "true"), gate("b", "true")],
            &AtomicBool::new(false),
        );
        assert!(verdict.passed);
        assert_eq!(verdict.gates[0].status, GateStatus::Passed);
        assert_eq!(verdict.gates[0].exit_code, Some(0));
        assert_eq!(verdict.gates[0].repro, "true");
        assert_eq!(
            events,
            vec![
                "started:2",
                "gs:0",
                "gf:0:Passed",
                "gs:1",
                "gf:1:Passed",
                "finished"
            ]
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
            vec![
                "started:2",
                "gs:0",
                "gf:0:Failed",
                "gf:1:Skipped",
                "finished"
            ]
        );
    }

    #[test]
    fn optional_failure_does_not_sink_the_verdict() {
        let mut soft = gate("soft", "false");
        soft.optional = true;
        let (verdict, _) = run_collect(vec![soft, gate("hard", "true")], &AtomicBool::new(false));
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

        let tagged = Verdict {
            task_id: Some("s1".into()),
            ..verdict
        };
        assert_eq!(serde_json::to_value(&tagged).unwrap()["task_id"], "s1");
    }
}
