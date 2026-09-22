//! CLI contracts that do not collect native metrics or request authorization.
//! All test fixtures are local/synthetic; environment overrides are per child.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bmtop"));
    for key in [
        "BMTOP_CONFIG",
        "BMTOP_LANG",
        "BMTOP_INTERVAL",
        "BMTOP_PRIVACY",
        "LC_ALL",
        "LC_MESSAGES",
    ] {
        command.env_remove(key);
    }
    command.env("LANG", "C");
    command
}

fn run(arguments: &[&str]) -> Output {
    binary()
        .arg("--no-config")
        .args(arguments)
        .output()
        .expect("execute bmtop")
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(text: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "bmtop-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("config.toml"), text).unwrap();
        Self(path)
    }
    fn config(&self) -> PathBuf {
        self.0.join("config.toml")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn new_flags_are_discoverable() {
    let result = run(&["--help"]);
    assert!(result.status.success());
    let help = String::from_utf8_lossy(&result.stdout);
    for flag in [
        "--privacy",
        "--config",
        "--no-config",
        "--print-config",
        "--schema-version",
        "--quick",
        "--diagnostics-file",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
}

#[test]
fn schema_version_never_loads_a_broken_config() {
    let result = binary()
        .env("BMTOP_CONFIG", "/does-not-exist/config.toml")
        .arg("--schema-version")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "2");
}

#[test]
fn resolved_config_is_valid_toml_without_a_terminal() {
    let result = run(&["--print-config"]);
    assert!(result.status.success());
    let value: toml::Value = toml::from_str(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
    assert_eq!(value["config_version"].as_integer(), Some(1));
    assert_eq!(value["interval"].as_str(), Some("1s"));
    assert_eq!(value["privacy"].as_str(), Some("default"));
    assert_eq!(value["process"]["tree"].as_bool(), Some(false));
}

#[test]
fn explicit_cli_beats_environment_beats_file() {
    let fixture =
        Fixture::new("interval='5s'\nprivacy='strict'\nlanguage='zh'\n[process]\ntree=true\n");
    let result = binary()
        .arg("--config")
        .arg(fixture.config())
        .env("BMTOP_INTERVAL", "2s")
        .env("BMTOP_PRIVACY", "off")
        .env("BMTOP_LANG", "en")
        .args([
            "--interval",
            "250ms",
            "--privacy",
            "default",
            "--print-config",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: toml::Value = toml::from_str(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
    assert_eq!(value["interval"].as_str(), Some("250ms"));
    assert_eq!(value["privacy"].as_str(), Some("default"));
    assert_eq!(value["language"].as_str(), Some("en"));
    assert_eq!(value["process"]["tree"].as_bool(), Some(true));
}

#[test]
fn environment_is_used_when_cli_is_absent() {
    let result = binary()
        .arg("--no-config")
        .env("BMTOP_INTERVAL", "5s")
        .env("BMTOP_PRIVACY", "strict")
        .arg("--print-config")
        .output()
        .unwrap();
    assert!(result.status.success());
    let value: toml::Value = toml::from_str(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
    assert_eq!(value["interval"].as_str(), Some("5s"));
    assert_eq!(value["privacy"].as_str(), Some("strict"));
}

#[test]
fn no_config_ignores_invalid_environment_path_but_not_other_overrides() {
    let result = binary()
        .env("BMTOP_CONFIG", "/does-not-exist/config.toml")
        .args(["--no-config", "--print-config"])
        .output()
        .unwrap();
    assert!(result.status.success());
}

#[test]
fn malformed_config_is_a_usage_error_not_silently_ignored() {
    for text in [
        "interval='oops'",
        "config_version=99",
        "prviacy='off'",
        "[process]\nsort='bogus'",
    ] {
        let fixture = Fixture::new(text);
        let result = binary()
            .arg("--config")
            .arg(fixture.config())
            .arg("--print-config")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(64));
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn oversized_config_is_rejected() {
    let fixture = Fixture::new(&"# comment\n".repeat(7_000));
    let result = binary()
        .arg("--config")
        .arg(fixture.config())
        .arg("--print-config")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&result.stderr).contains("64 KiB"));
}

#[test]
fn command_mode_override_is_printed() {
    let result = run(&["top", "--mode", "2", "--print-config"]);
    assert!(result.status.success());
    let value: toml::Value = toml::from_str(std::str::from_utf8(&result.stdout).unwrap()).unwrap();
    assert_eq!(value["default_mode"].as_str(), Some("processes"));
}

#[test]
fn count_and_formats_are_validated_before_native_probing() {
    for args in [
        vec!["network", "--connections", "--count", "0"],
        vec![
            "network",
            "--connections",
            "--count",
            "2",
            "--format",
            "json",
        ],
        vec!["hardware", "--count", "2", "--format", "json"],
        vec!["doctor", "--count", "1"],
        vec!["doctor", "--format", "csv"],
        vec!["top", "--format", "json"],
        vec!["top", "--watch"],
        vec!["cpu", "--quick", "--watch"],
        vec!["memory", "--quick"],
        vec!["memory", "--diagnostics-file", "unused.json"],
    ] {
        let result = run(&args);
        assert_eq!(
            result.status.code(),
            Some(64),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn doctor_jsonl_is_exactly_one_document_and_is_share_safe_by_schema() {
    let result = run(&["doctor", "--format", "jsonl"]);
    assert!(result.status.success());
    let text = std::str::from_utf8(&result.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(value["kind"], "doctor");
    assert_eq!(value["privacy"]["hardware_identifiers_collected"], false);
    assert_eq!(value["privacy"]["screen_recording_requested"], false);
    assert!(value["soc"]["initialization"]["state"].is_string());
    // The diagnostic schema intentionally excludes process lists and identifiers.
    assert!(value.get("processes").is_none());
    assert!(value.get("serial_number").is_none());
}

#[test]
fn non_tty_top_fails_before_entering_raw_mode() {
    let result = run(&["top"]);
    assert_eq!(result.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&result.stderr).contains("TTY"));
}

#[test]
#[cfg(not(target_os = "macos"))]
fn unsupported_platform_is_not_fabricated_metrics() {
    let result = run(&["memory", "--format", "json"]);
    assert_eq!(result.status.code(), Some(69));
    assert!(result.stdout.is_empty());
}
