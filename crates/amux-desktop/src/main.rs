use amux_core::{config::Config, tmux};
use eframe::egui;
use std::process::Command;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("amux desktop"),
        ..Default::default()
    };
    eframe::run_native(
        "amux",
        options,
        Box::new(|_cc| Ok(Box::new(DesktopApp::new()))),
    )
}

struct DesktopApp {
    sessions: Vec<tmux::Session>,
    selected: Option<usize>,
    error: Option<String>,
    _config: Config,
}

impl DesktopApp {
    fn new() -> Self {
        let config = Config::load();
        let mut app = Self {
            sessions: Vec::new(),
            selected: None,
            error: None,
            _config: config,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let result = Command::new("tmux")
            .args(["list-sessions", "-F", tmux::LIST_FORMAT])
            .output();
        match result {
            Ok(out) if out.status.success() => {
                let raw = String::from_utf8_lossy(&out.stdout);
                self.sessions = tmux::parse_sessions(&raw);
                self.error = None;
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                self.error = Some(stderr.trim().to_string());
            }
            Err(e) => {
                self.error = Some(format!("tmux: {e}"));
            }
        }
    }
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("sessions")
            .min_size(200.0)
            .show_inside(ui, |ui| {
                ui.heading("Sessions");
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
                ui.separator();
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::RED, err);
                    return;
                }
                if self.sessions.is_empty() {
                    ui.label("No managed sessions");
                    return;
                }
                for (i, session) in self.sessions.iter().enumerate() {
                    let label = format!("{}  [{}]", session.name, session.agent);
                    let selected = self.selected == Some(i);
                    if ui.selectable_label(selected, &label).clicked() {
                        self.selected = Some(i);
                    }
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            match self.selected.and_then(|i| self.sessions.get(i)) {
                Some(session) => {
                    ui.heading(&session.name);
                    ui.label(format!("Agent: {}", session.agent));
                    ui.label(format!("Dir: {}", session.dir));
                    if let Some(git) = &session.git {
                        ui.label(format!("Branch: {}", git.branch));
                    }
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.label("Select a session");
                    });
                }
            }
        });
    }
}
