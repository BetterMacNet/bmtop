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
fn connection_help_explains_visibility_and_pid_filter_in_both_languages() {
    for (language, visibility, capture) in [
        ("en", "permissions", "No packet capture"),
        ("zh", "权限", "不抓包"),
    ] {
        let result = run(&["--lang", language, "network", "--help"]);
        assert!(result.status.success());
        let help = String::from_utf8_lossy(&result.stdout);
        for expected in [
            "--pid",
            "sudo bmtop network --connections",
            visibility,
            capture,
        ] {
            assert!(help.contains(expected), "missing {expected}: {help}");
        }
        for command in ["ps", "top"] {
            let result = run(&["--lang", language, command, "--help"]);
            assert!(result.status.success());
            let help = String::from_utf8_lossy(&result.stdout);
            assert!(help.contains("sudo bmtop"));
            assert!(help.contains(visibility));
        }
    }
}

#[test]
fn connection_pid_requires_connections_and_a_positive_pid() {
    for arguments in [
        vec!["network", "--pid", "42"],
        vec!["network", "--connections", "--pid", "0"],
        vec!["network", "--connections", "--pid=-1"],
        vec!["network", "--connections", "--pid", "2147483648"],
    ] {
        let result = run(&arguments);
        assert_eq!(result.status.code(), Some(2), "{arguments:?}");
        assert!(result.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("connection query failed"));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn connection_filter_preserves_machine_output_and_strict_privacy() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let pid = std::process::id().to_string();
    for privacy in ["default", "strict"] {
        let result = run(&[
            "--lang",
            "en",
            "--privacy",
            privacy,
            "network",
            "--connections",
            "--pid",
            &pid,
            "--format",
            "jsonl",
            "--count",
            "2",
            "--interval",
            "250ms",
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(
            stderr.matches("sudo bmtop network --connections").count(),
            1
        );
        let stdout = std::str::from_utf8(&result.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 2);
        for line in stdout.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["kind"], "network_connections");
            let rows = value["data"]["connections"].as_array().unwrap();
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|row| row["pid"] == std::process::id()));
            if privacy == "strict" {
                assert!(rows
                    .iter()
                    .all(|row| row["command"].is_null() && row["endpoint"].is_null()));
            } else {
                assert!(rows.iter().any(|row| row["endpoint"] == endpoint));
            }
        }
    }
}

#[test]
fn flow_options_are_discoverable_and_validated_before_collection() {
    for language in ["en", "zh"] {
        let help = run(&["--lang", language, "network", "--help"]);
        let text = String::from_utf8_lossy(&help.stdout);
        assert!(text.contains("--flows"));
        assert!(text.contains("sudo bmtop --no-config network --flows"));
    }
    let conflict = run(&["network", "--flows", "--connections"]);
    assert_eq!(conflict.status.code(), Some(2));
    for interval in ["250ms", "1500ms", "61s"] {
        let result = run(&["network", "--flows", "--interval", interval]);
        assert_eq!(result.status.code(), Some(64));
        assert!(result.stdout.is_empty());
    }
}

#[test]
#[cfg(target_os = "macos")]
fn live_flows_keep_jsonl_machine_readable_and_strictly_redacted() {
    let result = run(&[
        "--lang",
        "en",
        "--privacy",
        "strict",
        "network",
        "--flows",
        "--format",
        "jsonl",
        "--count",
        "2",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = std::str::from_utf8(&result.stdout).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert_eq!(
        String::from_utf8_lossy(&result.stderr)
            .matches("sudo bmtop --no-config network --flows")
            .count(),
        1
    );
    for line in text.lines() {
        let report: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(report["kind"], "network_flows");
        assert!(report["data"]["elapsed_seconds"].as_f64().unwrap() > 0.0);
        assert!(report["data"]["truncated"].is_boolean());
        let flows = report["data"]["flows"].as_array().unwrap();
        assert!(flows.len() <= 512);
        assert!(flows.iter().all(|f| f["command"].is_null()
            && f["local_endpoint"].is_null()
            && f["remote_endpoint"].is_null()));
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
