use std::{env, fs, path::PathBuf};

pub const DEFAULT_RESIZE_REFLOW_MAX_ROWS: usize = 4_000;
pub const RESIZE_REFLOW_MAX_ROWS_ENV: &str = "NABLA_UI_RESIZE_REFLOW_MAX_ROWS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiConfig {
    /// Zero means replay all canonical history.
    pub resize_reflow_max_rows: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            resize_reflow_max_rows: DEFAULT_RESIZE_REFLOW_MAX_ROWS,
        }
    }
}

impl UiConfig {
    pub fn from_env() -> Self {
        let configured_rows = config_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .as_deref()
            .and_then(resize_reflow_rows_from_json);
        let resize_reflow_max_rows = env::var(RESIZE_REFLOW_MAX_ROWS_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .or(configured_rows)
            .unwrap_or(DEFAULT_RESIZE_REFLOW_MAX_ROWS);
        Self {
            resize_reflow_max_rows,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    env::var_os("NABLA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".nabla")))
        .map(|root| root.join("config.json"))
}

fn resize_reflow_rows_from_json(source: &str) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(source).ok()?;
    let ui = value.get("ui")?;
    let rows = ui
        .get("resize_reflow_max_rows")
        .or_else(|| ui.get("resizeReflowMaxRows"))?
        .as_u64()?;
    usize::try_from(rows).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_reflow_defaults_are_conservative() {
        let config = UiConfig::default();
        assert!((2_000..=4_000).contains(&config.resize_reflow_max_rows));
    }

    #[test]
    fn resize_reflow_rows_accept_zero_and_both_config_key_styles() {
        assert_eq!(
            resize_reflow_rows_from_json(r#"{"ui":{"resize_reflow_max_rows":0}}"#),
            Some(0)
        );
        assert_eq!(
            resize_reflow_rows_from_json(r#"{"ui":{"resizeReflowMaxRows":8192}}"#),
            Some(8192)
        );
        assert_eq!(resize_reflow_rows_from_json(r#"{"ui":{}}"#), None);
    }
}
