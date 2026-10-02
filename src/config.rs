//! `~/.config/fred/config.toml`.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Take over the terminal (alternate screen) instead of an inline
    /// window: always, never, or `None` = "auto" (when the file has more
    /// lines than the inline window shows).
    #[serde(deserialize_with = "fullscreen")]
    pub fullscreen: Option<bool>,
    #[serde(deserialize_with = "height")]
    pub height: usize,
    pub wrap: bool,
    pub numbers: bool,
    pub relative_numbers: bool,
    pub theme: String,
    pub tabstop: usize,
    pub autocomplete: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            fullscreen: None,
            height: 12,
            wrap: false,
            numbers: true,
            relative_numbers: false,
            theme: "ansi".into(),
            tabstop: 8,
            autocomplete: true,
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
        assert_eq!(c.fullscreen, None);
        assert_eq!(
            Config::parse("fullscreen = false").0.fullscreen,
            Some(false)
        );
        assert_eq!(Config::parse("fullscreen = true").0.fullscreen, Some(true));
        assert_eq!(Config::parse("fullscreen = \"auto\"").0.fullscreen, None);
        assert!(Config::parse("fullscreen = \"big\"").1.is_some());
        assert_eq!(c.tabstop, 8);
        assert_eq!(c.theme, "ansi");
        assert!(c.numbers && !c.relative_numbers && !c.wrap && c.autocomplete);
    }

    #[test]
    fn values() {
        let (c, err) = Config::parse("height = 20\nwrap = true\ntheme = \"Nord\"\n");
        assert!(err.is_none());
        assert_eq!((c.height, c.wrap, c.theme.as_str()), (20, true, "Nord"));
    }

    #[test]
    fn unknown_key_reports_error_and_uses_defaults() {
        let (c, err) = Config::parse("colour = 1\n");
        assert_eq!(c, Config::default());
        assert!(err.unwrap().contains("colour"));
    }

    #[test]
    fn invalid_values_are_errors() {
        assert!(Config::parse("height = 0").1.is_some());
        assert_eq!(Config::parse("height = \"max\"").0.height, usize::MAX);
        assert!(Config::parse("tabstop = 0").1.is_some());
        assert!(Config::parse("height = \"x\"").1.is_some());
    }
}
