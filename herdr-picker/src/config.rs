//! Optional user config at `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//!
//! ```toml
//! [content]
//! enabled = true                       # false: search names and paths only
//! screen_lines = 500                   # scrollback rows indexed per pane
//! transcript_messages = 200            # newest chat messages kept per session
//! transcript_bytes = 1048576           # max chat text kept per session
//! agents = ["claude", "codex", "opencode"]
//! ```

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub content: Content,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Content {
    pub enabled: bool,
    pub screen_lines: u32,
    pub transcript_messages: usize,
    /// Upper bound on transcript text kept per session.
    pub transcript_bytes: usize,
    pub agents: Vec<String>,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            enabled: true,
            screen_lines: 500,
            transcript_messages: 200,
            transcript_bytes: 1 << 20,
            agents: vec!["claude".into(), "codex".into(), "opencode".into()],
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")
            .map(|dir| PathBuf::from(dir).join("config.toml"))
    }

    /// Loads the config, falling back to defaults when the file is absent.
    /// A malformed file is an error so typos don't silently do nothing.
    pub fn load() -> anyhow::Result<Self> {
        let Some(path) = Self::path().filter(|p| p.is_file()) else {
            return Ok(Self::default());
        };
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    pub fn indexes_transcripts_of(&self, agent: &str) -> bool {
        self.content.enabled && self.content.agents.iter().any(|a| a == agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_keeps_defaults() {
        let config: Config =
            toml::from_str("[content]\nscreen_lines = 100\nagents = [\"claude\"]").unwrap();
        assert_eq!(config.content.screen_lines, 100);
        assert_eq!(config.content.transcript_messages, 200);
        assert!(config.indexes_transcripts_of("claude"));
        assert!(!config.indexes_transcripts_of("codex"));
    }

    #[test]
    fn typos_are_rejected() {
        assert!(toml::from_str::<Config>("[content]\nscren_lines = 1").is_err());
    }
}
