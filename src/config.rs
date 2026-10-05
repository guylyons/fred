//! `~/.config/fred/config.toml`.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Take over the terminal (alternate screen) instead of an inline
    /// window: true (the default), false, or `None` = "auto" (only when
    /// the file has more lines than the inline window shows).
    #[serde(deserialize_with = "fullscreen")]
    pub fullscreen: Option<bool>,
    #[serde(deserialize_with = "height")]
    pub height: usize,
    pub wrap: bool,
    pub numbers: bool,
    pub relative_numbers: bool,
    /// Highlight the current buffer line across the text area.
    pub hl_line: bool,
    pub theme: String,
    pub tabstop: usize,
    pub autocomplete: bool,
    /// File-type icons in the file pickers (needs a Nerd Font).
    pub icons: bool,
    /// Share yanks and puts with the system clipboard.
    pub clipboard: bool,
    /// What `:ai` runs, prompt on stdin.
    pub ai_command: String,
    /// Extra instructions added to every `:ai` prompt.
    pub ai_rules: String,
    /// What `:explain` (Visual `K`) runs, prompt on stdin.
    pub explain_command: String,
    /// `[magit]`: Magit's customization options by their upstream names.
    pub magit: MagitOptions,
}

/// The `[magit]` table (see `magit::options`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct MagitOptions(pub toml::Table);
impl Eq for MagitOptions {}

impl Default for Config {
    fn default() -> Self {
        Config {
            fullscreen: Some(true),
            height: 12,
            wrap: false,
            numbers: true,
            relative_numbers: false,
            hl_line: false,
            theme: "ansi".into(),
            tabstop: 8,
            autocomplete: true,
            icons: false,
            clipboard: true,
            // --safe-mode skips your plugins, hooks, MCP servers and
            // CLAUDE.md: much faster, ~100x fewer tokens per call.
            ai_command: "claude -p --tools '' --safe-mode".into(),
            ai_rules: String::new(),
            explain_command: "claude -p --tools '' --safe-mode --model sonnet".into(),
            magit: MagitOptions::default(),
        }
    }
}

/// `fullscreen` is true, false or "auto".
fn fullscreen<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum F {
        B(bool),
        S(String),
    }
    match F::deserialize(d)? {
        F::B(b) => Ok(Some(b)),
        F::S(s) if s == "auto" => Ok(None),
        F::S(s) => Err(serde::de::Error::custom(format!(
            "invalid fullscreen: {s:?}"
        ))),
    }
}

/// `height` is a line count or "max" (as tall as the terminal allows).
fn height<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum H {
        N(usize),
        S(String),
    }
    match H::deserialize(d)? {
        H::N(n) => Ok(n),
        H::S(s) if s == "max" => Ok(usize::MAX),
        H::S(s) => Err(serde::de::Error::custom(format!("invalid height: {s:?}"))),
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("fred/config.toml")
}

impl Config {
    /// Parse config text; on any error, return defaults and the message.
    pub fn parse(text: &str) -> (Config, Option<String>) {
        match toml::from_str::<Config>(text) {
            Ok(c) if c.height == 0 => (
                Config::default(),
                Some("config: height must be at least 1".into()),
            ),
            Ok(c) if c.tabstop == 0 || c.tabstop > 32 => (
                Config::default(),
                Some("config: tabstop must be 1-32".into()),
            ),
            Ok(c) => (c, None),
            Err(e) => {
                let msg = e
                    .message()
                    .lines()
                    .next()
                    .unwrap_or("invalid config")
                    .to_string();
                (Config::default(), Some(format!("config: {msg}")))
            }
        }
    }

    /// Load the user's config file (missing file = defaults).
    pub fn load() -> (Config, Option<String>) {
        match std::fs::read_to_string(config_path()) {
            Ok(t) => Config::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (Config::default(), Some(format!("config: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let (c, err) = Config::parse("");
        assert!(err.is_none());
        assert_eq!(c, Config::default());
        assert_eq!(c.height, 12);
        assert_eq!(c.fullscreen, Some(true));
        assert_eq!(
            Config::parse("fullscreen = false").0.fullscreen,
            Some(false)
        );
        assert_eq!(Config::parse("fullscreen = true").0.fullscreen, Some(true));
        assert_eq!(Config::parse("fullscreen = \"auto\"").0.fullscreen, None);
        assert!(Config::parse("fullscreen = \"big\"").1.is_some());
        assert_eq!(c.tabstop, 8);
        assert_eq!(c.theme, "ansi");
        assert!(
            c.numbers
                && !c.relative_numbers
                && !c.hl_line
                && !c.wrap
                && c.autocomplete
                && !c.icons
                && c.clipboard
        );
    }

    #[test]
    fn hl_line_accepts_boolean_config() {
        let (c, err) = Config::parse("hl_line = true");
        assert!(err.is_none());
        assert!(c.hl_line);
        let (c, err) = Config::parse("hl_line = false");
        assert!(err.is_none());
        assert!(!c.hl_line);
        assert!(Config::parse("hl_line = \"yes\"").1.is_some());
    }

    #[test]
    fn values() {
        let (c, err) = Config::parse("height = 20\nwrap = true\ntheme = \"Nord\"\n");
        assert!(err.is_none());
        assert_eq!((c.height, c.wrap, c.theme.as_str()), (20, true, "Nord"));
        let c = Config::parse("ai_command = \"cat\"\nai_rules = \"terse\"").0;
        assert_eq!(
            (c.ai_command.as_str(), c.ai_rules.as_str()),
            ("cat", "terse")
        );
    }

    #[test]
    fn unknown_key_reports_error_and_uses_defaults() {
        let (c, err) = Config::parse("colour = 1\n");
        assert_eq!(c, Config::default());
        assert!(err.unwrap().contains("colour"));
    }

    #[test]
    fn magit_table_preserves_upstream_names_and_values() {
        let (cfg, err) = Config::parse(
            "[magit]\nmagit-no-confirm = false\nmagit-log-section-commit-count = 20\n",
        );
        assert!(err.is_none(), "{err:?}");
        assert_eq!(cfg.magit.0["magit-no-confirm"].as_bool(), Some(false));
        assert_eq!(
            cfg.magit.0["magit-log-section-commit-count"].as_integer(),
            Some(20)
        );
    }

    #[test]
    fn invalid_values_are_errors() {
        assert!(Config::parse("height = 0").1.is_some());
        assert_eq!(Config::parse("height = \"max\"").0.height, usize::MAX);
        assert!(Config::parse("tabstop = 0").1.is_some());
        assert!(Config::parse("height = \"x\"").1.is_some());
    }
}
