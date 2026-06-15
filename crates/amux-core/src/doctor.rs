//! `amux doctor`: surface & clean tmux/agent detritus the cm-only view hides.

use std::process::Command;

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

/// One classified socket, with a short summary of what it holds.
pub struct SocketInfo {
    pub name: String,
    pub path: String,
    pub class: SocketClass,
    pub panes: Panes,
}

/// The directory tmux keeps its `-L` sockets in (e.g. `/private/tmp/tmux-501`).
/// Resolved by starting/asking the default server, then taking the dirname.
pub fn socket_dir() -> Option<std::path::PathBuf> {
    let out = Command::new("tmux")
        .args([
            "start-server",
            ";",
            "display-message",
            "-p",
            "#{socket_path}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    std::path::Path::new(&p).parent().map(|d| d.to_path_buf())
}

/// Query one socket by path: `Some(Panes)` if a server answers, `None` if dead.
fn query(path: &std::path::Path) -> Option<Panes> {
    const FMT: &str = "#{session_name}\t#{@cm_managed}\t#{pane_dead}\t#{pane_current_command}";
    let out = Command::new("tmux")
        .arg("-S")
        .arg(path)
        .args(["list-panes", "-a", "-F", FMT])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // "no server running" → dead socket file
    }
    Some(parse_panes(&String::from_utf8_lossy(&out.stdout)))
}

/// Enumerate and classify every socket in the socket dir.
pub fn scan() -> Vec<SocketInfo> {
    let Some(dir) = socket_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let panes = query(&path);
        // `query` returns `Some(_)` only for a live server, `None` for a dead
        // socket file — so `alive` here is only ever `Some(true)` or `None`
        // (classify also accepts `Some(false)`, but this caller never sends it).
        let alive = panes.as_ref().map(|_| true);
        let has_cm = panes.as_ref().map(|p| p.has_cm_tags).unwrap_or(false);
        let class = classify(&name, alive, has_cm);
        out.push(SocketInfo {
            name,
            path: path.to_string_lossy().into_owned(),
            class,
            panes: panes.unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Human-readable report of all sockets, grouped by class.
pub fn report(infos: &[SocketInfo]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let label = |c: SocketClass| match c {
        SocketClass::LiveManaged => "cm (live amux)   ",
        SocketClass::UserDefault => "default (yours)  ",
        SocketClass::LeakedAmux => "LEAKED amux      ",
        SocketClass::OtherLive => "other (yours)    ",
        SocketClass::StaleFile => "stale socket file",
    };
    for i in infos {
        let _ = writeln!(
            s,
            "  {}  {:20}  {} sessions, {} dead{}",
            label(i.class),
            i.name,
            i.panes.sessions,
            i.panes.dead_panes,
            if i.class.cleanable() {
                "  [cleanable]"
            } else {
                ""
            },
        );
    }
    let n = infos.iter().filter(|i| i.class.cleanable()).count();
    if n > 0 {
        let _ = writeln!(
            s,
            "\n{n} cleanable — run `amux doctor --clean` to remove them."
        );
    } else {
        let _ = writeln!(s, "\nNothing to clean.");
    }
    s
}

/// Remove cleanable sockets: `kill-server` for leaked amux servers, then unlink
/// the socket file (tmux's kill-server doesn't reliably remove it). Returns the
/// names removed. Never touches protected sockets.
pub fn clean(infos: &[SocketInfo]) -> Vec<String> {
    let mut removed = Vec::new();
    for i in infos.iter().filter(|i| i.class.cleanable()) {
        if i.class == SocketClass::LeakedAmux {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&i.path)
                .arg("kill-server")
                .status();
        }
        let _ = std::fs::remove_file(&i.path);
        removed.push(i.name.clone());
    }
    removed
}

/// `amux doctor [--clean]` entry point. Prints the report; with `--clean`,
/// removes cleanable sockets and prints what it did.
pub fn run(args: &[String]) -> std::io::Result<()> {
    let do_clean = args.iter().any(|a| a == "--clean");
    let infos = scan();
    print!("{}", report(&infos));
    if do_clean {
        let removed = clean(&infos);
        if removed.is_empty() {
            println!("Cleaned nothing (no cleanable sockets).");
        } else {
            println!("Cleaned {}: {}", removed.len(), removed.join(", "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_protects_cm_and_default() {
        assert_eq!(classify("cm", Some(true), true), SocketClass::LiveManaged);
        assert_eq!(
            classify("default", Some(true), false),
            SocketClass::UserDefault
        );
        // even a dead cm/default file stays protected:
        assert_eq!(classify("cm", None, false), SocketClass::LiveManaged);
    }

    #[test]
    fn classify_leaked_amux_is_live_server_with_cm_tags() {
        assert_eq!(
            classify("cmtest", Some(true), true),
            SocketClass::LeakedAmux
        );
        assert_eq!(
            classify("am_test_42247", Some(true), true),
            SocketClass::LeakedAmux
        );
    }

    #[test]
    fn classify_other_live_server_is_protected() {
        // a live non-cm server with no @cm_* tags is the user's own — list only:
        assert_eq!(
            classify("mywork", Some(true), false),
            SocketClass::OtherLive
        );
    }

    #[test]
    fn classify_dead_nonprotected_is_stale_file() {
        assert_eq!(classify("cmtest", None, false), SocketClass::StaleFile);
        assert_eq!(
            classify("amtest", Some(false), false),
            SocketClass::StaleFile
        );
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
