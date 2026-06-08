//! `amux doctor`: surface & clean tmux/agent detritus the cm-only view hides.

/// How a tmux socket found in the user's socket dir is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketClass {
    /// `cm` — the live amux socket. Protected.
    LiveManaged,
    /// `default` — the user's own tmux server. Protected.
    UserDefault,
    /// A non-`cm` live server whose sessions carry `@cm_*` tags → an amux server
    /// we created and leaked. Cleanable (`kill-server` + remove the file).
    LeakedAmux,
    /// A live server we don't recognise (no `@cm_*` tags). Protected; list only.
    OtherLive,
    /// A socket file whose server is dead — junk. Cleanable (remove the file).
    StaleFile,
}

impl SocketClass {
    /// Whether `--clean` may remove this socket.
    pub fn cleanable(self) -> bool {
        matches!(self, SocketClass::LeakedAmux | SocketClass::StaleFile)
    }
}

/// Classify a socket by name, whether its server is alive (`None`/`Some(false)`
/// = dead), and whether its sessions look amux-made (any `@cm_managed=1`).
pub fn classify(name: &str, alive: Option<bool>, has_cm_tags: bool) -> SocketClass {
    match name {
        "cm" => SocketClass::LiveManaged,
        "default" => SocketClass::UserDefault,
        _ => match alive {
            Some(true) if has_cm_tags => SocketClass::LeakedAmux,
            Some(true) => SocketClass::OtherLive,
            _ => SocketClass::StaleFile, // dead server → just a file to remove
        },
    }
}

/// Parsed `tmux list-panes -a` output for one server.
#[derive(Debug, Default, PartialEq)]
pub struct Panes {
    pub sessions: usize,
    pub dead_panes: usize,
    pub has_cm_tags: bool,
    pub commands: Vec<String>,
}

/// Parse the tab-separated `list-panes -a` output produced with the format
/// `#{session_name}\t#{@cm_managed}\t#{pane_dead}\t#{pane_current_command}`.
pub fn parse_panes(out: &str) -> Panes {
    use std::collections::HashSet;
    let mut names: HashSet<&str> = HashSet::new();
    let mut p = Panes::default();
    for line in out.lines() {
        let mut f = line.splitn(4, '\t');
        let name = f.next().unwrap_or("");
        let managed = f.next().unwrap_or("");
        let dead = f.next().unwrap_or("");
        let cmd = f.next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        names.insert(name);
        if dead.trim() == "1" {
            p.dead_panes += 1;
        }
        if managed.trim() == "1" {
            p.has_cm_tags = true;
        }
        if !cmd.trim().is_empty() {
            p.commands.push(cmd.trim().to_string());
        }
    }
    p.sessions = names.len();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_protects_cm_and_default() {
        assert_eq!(classify("cm", Some(true), true), SocketClass::LiveManaged);
        assert_eq!(classify("default", Some(true), false), SocketClass::UserDefault);
        // even a dead cm/default file stays protected:
        assert_eq!(classify("cm", None, false), SocketClass::LiveManaged);
    }

    #[test]
    fn classify_leaked_amux_is_live_server_with_cm_tags() {
        assert_eq!(classify("cmtest", Some(true), true), SocketClass::LeakedAmux);
        assert_eq!(classify("am_test_42247", Some(true), true), SocketClass::LeakedAmux);
    }

    #[test]
    fn classify_other_live_server_is_protected() {
        // a live non-cm server with no @cm_* tags is the user's own — list only:
        assert_eq!(classify("mywork", Some(true), false), SocketClass::OtherLive);
    }

    #[test]
    fn classify_dead_nonprotected_is_stale_file() {
        assert_eq!(classify("cmtest", None, false), SocketClass::StaleFile);
        assert_eq!(classify("amtest", Some(false), false), SocketClass::StaleFile);
    }

    #[test]
    fn parse_panes_counts_sessions_dead_and_cm_tags() {
        // fmt: session_name \t @cm_managed \t pane_dead \t pane_current_command
        let out = "work\t1\t0\tclaude\nwork\t1\t0\tnode\nshell\t\t1\tbash\n";
        let p = parse_panes(out);
        assert_eq!(p.sessions, 2, "two distinct sessions (work, shell)");
        assert_eq!(p.dead_panes, 1, "one dead pane");
        assert!(p.has_cm_tags, "at least one @cm_managed=1");
        assert!(p.commands.iter().any(|c| c == "claude"));
    }

    #[test]
    fn parse_panes_no_cm_tags_when_all_blank() {
        let out = "a\t\t0\tvim\nb\t\t0\tzsh\n";
        let p = parse_panes(out);
        assert_eq!(p.sessions, 2);
        assert!(!p.has_cm_tags);
    }
}
