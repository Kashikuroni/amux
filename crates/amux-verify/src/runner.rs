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

        let tagged = Verdict {
            task_id: Some("s1".into()),
            ..verdict
        };
        assert_eq!(serde_json::to_value(&tagged).unwrap()["task_id"], "s1");
    }
}
