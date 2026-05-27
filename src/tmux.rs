/// Tab-separated fields requested from `tmux list-sessions -F`.
/// Order: name, path, created, @cm_managed, @cm_agent, attached-client-count.
pub const LIST_FORMAT: &str =
    "#{session_name}\t#{session_path}\t#{session_created}\t#{@cm_managed}\t#{@cm_agent}\t#{session_attached}";

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Running,
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub dir: String,
    pub created: i64,
    pub agent: String,
    pub status: Status,
    pub attached: bool,
}

/// Parses `tmux list-sessions` output, keeping only sessions marked `@cm_managed=1`.
/// `status` defaults to `Waiting`; the app overwrites it via capture-pane diffing.
pub fn parse_sessions(output: &str) -> Vec<Session> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Session> {
    let mut f = line.splitn(6, '\t');
    let name = f.next()?.to_string();
    let dir = f.next()?.to_string();
    let created = f.next()?.trim().parse::<i64>().ok()?;
    let managed = f.next()?;
    let agent = f.next()?.to_string();
    if managed != "1" {
        return None;
    }
    Some(Session {
        name,
        dir,
        created,
        agent,
        status: Status::Waiting,
        attached: f
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|n| n > 0)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_managed_session() {
        let out = "proj-a\t/home/u/proj-a\t1716800000\t1\tclaude\t0";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "proj-a");
        assert_eq!(sessions[0].dir, "/home/u/proj-a");
        assert_eq!(sessions[0].created, 1716800000);
        assert_eq!(sessions[0].agent, "claude");
        assert_eq!(sessions[0].status, Status::Waiting);
        assert!(!sessions[0].attached);
    }

    #[test]
    fn filters_out_unmanaged_sessions() {
        let out = "mine\t/d\t1\t1\tclaude\t0\nother\t/d\t1\t\t\t1";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "mine");
    }

    #[test]
    fn marks_attached_when_client_count_positive() {
        let out = "live\t/d\t1\t1\tclaude\t1";
        let sessions = parse_sessions(out);
        assert!(sessions[0].attached);
    }

    #[test]
    fn attached_count_greater_than_one_is_attached() {
        let out = "multi\t/d\t1\t1\tclaude\t2";
        let sessions = parse_sessions(out);
        assert!(sessions[0].attached);
    }

    #[test]
    fn empty_input_yields_no_sessions() {
        assert!(parse_sessions("").is_empty());
    }

    #[test]
    fn trailing_newline_does_not_add_empty_session() {
        let out = "solo\t/d\t1\t1\tclaude\t0\n";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "solo");
    }
}
