//! Versioned preferences. CLI > BMTOP_* environment > file > defaults.
//! Configuration is read-only: terminal key presses never modify dotfiles.
use anyhow::{Context, Result};
use bmtop_core::{AppMode, Language, PrivacyMode};
use bmtop_tui::{SortKey, TuiOptions};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub config_version: u8,
    pub interval: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub default_mode: AppMode,
    pub privacy: PrivacyMode,
    pub process: ProcessConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: 1,
            interval: "1s".into(),
            language: None,
            default_mode: AppMode::Overview,
            privacy: PrivacyMode::Default,
            process: ProcessConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessConfig {
    pub sort: String,
    pub tree: bool,
    pub hide_idle: bool,
    pub show_full_path: bool,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            sort: "cpu".into(),
            tree: false,
            hide_idle: false,
            show_full_path: false,
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("invalid bmtop TOML configuration")?;
        if config.config_version != 1 {
            return Err(super::usage("unsupported config_version; expected 1"));
        }
        super::parse_interval(&config.interval)?;
        config.language()?;
        config.tui_options()?;
        Ok(config)
    }

    pub fn language(&self) -> Result<Language> {
        self.language
            .as_deref()
            .map(str::parse::<Language>)
            .transpose()
            .map_err(|error| super::usage(error.to_string()))
            .map(|language| language.unwrap_or_else(Language::from_environment))
    }

    pub fn tui_options(&self) -> Result<TuiOptions> {
        let sort_key = match self.process.sort.as_str() {
            "cpu" => SortKey::Cpu,
            "gpu" => SortKey::Gpu,
            "memory" | "mem" => SortKey::Memory,
            "pid" => SortKey::Pid,
            "energy" | "nrg" => SortKey::Energy,
            "power" | "watts" => SortKey::Power,
            _ => {
                return Err(super::usage(
                    "process.sort must be cpu/gpu/memory/pid/energy/power",
                ))
            }
        };
        Ok(TuiOptions {
            sort_key,
            tree_view: self.process.tree,
            hide_idle: self.process.hide_idle,
            show_full_path: self.process.show_full_path,
        })
    }
}

pub fn default_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        paths.push(xdg.join("bmtop/config.toml"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        paths.push(home.join(".config/bmtop/config.toml"));
        #[cfg(target_os = "macos")]
        paths.push(home.join("Library/Application Support/bmtop/config.toml"));
    }
    paths
}

pub fn load(explicit: Option<&Path>, disabled: bool) -> Result<Config> {
    if disabled {
        return Ok(Config::default());
    }
    let env_path = std::env::var_os("BMTOP_CONFIG").map(PathBuf::from);
    let required = explicit.or(env_path.as_deref());
    let path = match required {
        Some(path) => Some(path.to_path_buf()),
        None => default_paths().into_iter().find(|path| path.exists()),
    };
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let file =
        File::open(&path).with_context(|| format!("cannot read config {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(65_537).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 65_536, "config exceeds 64 KiB limit");
    let text = std::str::from_utf8(&bytes).context("configuration must be UTF-8")?;
    Config::parse(text).with_context(|| format!("configuration {}", path.display()))
}

/// Exclusive create: do not follow symlinks or overwrite an existing file.
/// Diagnostic output contains only numbers; permissions are private regardless.
pub fn create_private(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).with_context(|| {
        format!(
            "cannot create {}; file must not already exist",
            path.display()
        )
    })
}

pub fn write_diagnostics(
    mut file: File,
    report: &bmtop_core::diagnostics::RuntimeReport,
) -> Result<()> {
    serde_json::to_writer_pretty(&mut file, report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_config_merges_defaults_and_validates() {
        let value = Config::parse("config_version = 1\ninterval = '500ms'\nlanguage = 'zh'\nprivacy = 'strict'\n[process]\ntree = true\n").unwrap();
        assert_eq!(value.interval, "500ms");
        assert_eq!(value.language().unwrap(), Language::Chinese);
        assert_eq!(value.privacy, PrivacyMode::Strict);
        assert!(value.tui_options().unwrap().tree_view);
        assert_eq!(value.process.sort, "cpu");
    }
    #[test]
    fn rejects_unknown_fields_future_versions_and_invalid_values() {
        for text in [
            "config_version=2",
            "interval='10ms'",
            "privacy='public'",
            "typo=true",
            "[process]\nsort='surprise'",
            "[process]\ntreee=true",
        ] {
            assert!(Config::parse(text).is_err(), "unexpectedly accepted {text}");
        }
    }
    #[test]
    fn default_config_roundtrips_without_machine_data() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let value = Config::parse(&text).unwrap();
        assert_eq!(value.interval, "1s");
        assert_eq!(value.privacy, PrivacyMode::Default);
    }
    #[test]
    fn no_config_does_not_read_an_explicit_path() {
        let result = load(Some(Path::new("/this/file/does/not/exist")), true).unwrap();
        assert_eq!(result.interval, "1s");
    }
}
