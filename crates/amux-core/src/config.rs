use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_agent: String,
    pub refresh_interval_ms: u64,
    pub agent_presets: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_agent: "claude".to_string(),
            refresh_interval_ms: 1500,
            agent_presets: vec!["claude".into(), "codex".into(), "opencode".into()],
        }
    }
}

impl Config {
    pub fn parse(toml_str: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Loads `~/.agent-multiplexer/config.toml`, falling back to the legacy
    /// `~/.claude-manager/config.toml` if the new path is absent (so existing
    /// users keep their config after the rename). Missing/invalid → defaults.
    pub fn load() -> Config {
        let Ok(home) = std::env::var("HOME") else {
            return Config::default();
        };
        let home = std::path::Path::new(&home);
        let new_path = home.join(".agent-multiplexer").join("config.toml");
        let legacy_path = home.join(".claude-manager").join("config.toml");
        let contents =
            std::fs::read_to_string(&new_path).or_else(|_| std::fs::read_to_string(&legacy_path));
        match contents {
            Ok(contents) => Config::parse(&contents).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let cfg = Config::parse(
            "default_agent = \"aider\"\nrefresh_interval_ms = 500\nagent_presets = [\"aider\"]",
        )
        .unwrap();
        assert_eq!(cfg.default_agent, "aider");
        assert_eq!(cfg.refresh_interval_ms, 500);
        assert_eq!(cfg.agent_presets, vec!["aider".to_string()]);
    }

    #[test]
    fn defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.default_agent, "claude");
        assert_eq!(
            cfg.agent_presets,
            vec![
                "claude".to_string(),
                "codex".to_string(),
                "opencode".to_string()
            ]
        );
    }

    #[test]
    fn empty_config_uses_defaults() {
        let cfg = Config::parse("").unwrap();
        assert_eq!(cfg.default_agent, "claude");
        assert_eq!(cfg.refresh_interval_ms, 1500);
    }

    #[test]
    fn partial_config_fills_missing_with_defaults() {
        let cfg = Config::parse("default_agent = \"codex\"").unwrap();
        assert_eq!(cfg.default_agent, "codex");
        assert_eq!(cfg.refresh_interval_ms, 1500);
    }
}
