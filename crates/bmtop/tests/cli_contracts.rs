use std::process::Command;

fn help_with(language: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["--lang", language, "--help"])
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "C")
        .output()
        .expect("bmtop binary should run");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn help_lists_primary_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .arg("--help")
        .output()
        .expect("bmtop binary should run");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("ps"));
    assert!(text.contains("memory"));
    assert!(text.contains("network"));
    assert!(text.contains("hardware"));
}

#[test]
fn help_text_follows_the_language_flag() {
    let english = help_with("en");
    assert!(
        english.contains("Read-only macOS resource and hardware monitor"),
        "英文帮助未生效:\n{english}"
    );
    assert!(english.contains("Interface language"));

    let chinese = help_with("zh");
    assert!(
        chinese.contains("macOS 资源与硬件只读监控工具"),
        "中文帮助未生效:\n{chinese}"
    );
}

#[test]
fn help_language_falls_back_to_the_locale_environment() {
    let run = |locale: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
            .arg("--help")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", locale)
            .output()
            .expect("bmtop binary should run");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(run("zh_CN.UTF-8").contains("macOS 资源与硬件只读监控工具"));
    assert!(run("en_US.UTF-8").contains("Read-only macOS resource"));
    // 未设置或 C locale 一律按英文。
    assert!(run("C").contains("Read-only macOS resource"));
}

#[test]
fn invalid_language_is_rejected_with_a_clear_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["--lang", "fr", "--help"])
        .output()
        .expect("bmtop binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("zh") && stderr.contains("en"),
        "报错未说明可选值:\n{stderr}"
    );
}

#[test]
fn doctor_output_note_is_translated() {
    let run = |language: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
            .args(["--lang", language, "doctor", "--format", "json"])
            .output()
            .expect("bmtop binary should run");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(run("en").contains("enhanced keyboard protocol"));
    assert!(run("zh").contains("增强键盘协议"));
}

#[test]
fn memory_json_envelope_follows_schema_v2() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["memory", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON document");
    assert_eq!(value["schema_version"], 2);
    // captured_at 必须是 RFC 3339，v1 的 "epoch.millisZ" 是非法时间戳。
    let captured_at = value["captured_at"].as_str().unwrap();
    assert!(
        captured_at.len() == 24 && captured_at.as_bytes()[10] == b'T' && captured_at.ends_with('Z'),
        "not RFC 3339: {captured_at}"
    );
    // capabilities 是真实能力表，至少包含 memory 本身。
    let capabilities = value["capabilities"].as_array().unwrap();
    assert!(capabilities.iter().any(|item| item == "memory"));
    assert!(value["data"]["memory"]["total_bytes"].as_u64().unwrap() > 0);
}

#[test]
fn ps_limit_caps_the_row_count() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["ps", "--limit", "3", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = value["data"]["processes"].as_array().unwrap();
    assert!(rows.len() <= 3, "got {} rows", rows.len());
}

#[test]
fn usage_errors_exit_with_sysexits_64() {
    // CSV 不支持 gpu。
    let csv = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["gpu", "--format", "csv"])
        .output()
        .expect("bmtop binary should run");
    assert_eq!(csv.status.code(), Some(64));
    // --enhanced 只接在 gpu / sensors 上。
    let enhanced = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["memory", "--enhanced"])
        .output()
        .expect("bmtop binary should run");
    assert_eq!(enhanced.status.code(), Some(64));
    // 非法 interval。
    let interval = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["memory", "-i", "1h"])
        .output()
        .expect("bmtop binary should run");
    assert_eq!(interval.status.code(), Some(64));
}

#[test]
fn count_flag_emits_exactly_n_jsonl_samples() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["memory", "-n", "2", "-i", "250ms", "--format", "jsonl"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let lines: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 2, "expected 2 samples: {lines:?}");
    for line in lines {
        serde_json::from_str::<serde_json::Value>(&line).expect("each line is standalone JSON");
    }
}

#[test]
fn cpu_json_carries_soc_and_topology_additively() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["cpu", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON document");
    // schema v2 保持不变：soc/topology 是增量字段。
    assert_eq!(value["schema_version"], 2);
    assert!(
        value["data"]["cpu"]["total_percent"].is_number()
            || value["data"]["cpu"]["total_percent"].is_null()
    );
    // capabilities 必含 soc 或 soc:unavailable（诚实探测，二选一）。
    let capabilities = value["capabilities"].as_array().unwrap();
    assert!(
        capabilities.iter().any(|item| item == "soc")
            ^ capabilities.iter().any(|item| item == "soc:unavailable"),
        "exactly one soc capability expected: {capabilities:?}"
    );
    // soc 要么缺失/null（Intel），要么形状完整。
    let soc = &value["data"]["soc"];
    if soc.is_object() {
        assert!(soc["clusters"].is_array());
        assert!(soc["power"].is_object());
        assert!(soc["temps"].is_object());
    }
    let topology = &value["data"]["topology"];
    if topology.is_object() {
        assert!(topology["brand"].is_string());
        assert!(topology["e_cores"].is_number());
        assert!(topology["p_cores"].is_number());
    }
}

#[test]
fn sensors_json_keeps_legacy_key_and_adds_soc() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["sensors", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON document");
    // 旧的 sensors 键（SPPowerDataType 切片）必须保留，soc 为增量。
    assert!(
        !value["data"]["sensors"].is_null(),
        "legacy sensors key must survive"
    );
    assert!(
        value["data"].get("soc").is_some(),
        "soc key must be present (may be null on Intel)"
    );
}

#[test]
fn doctor_reports_soc_probe() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["doctor", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON document");
    for probe in ["ioreport", "smc", "thermal"] {
        assert!(
            value["soc"][probe].is_boolean(),
            "doctor.soc.{probe} must be a boolean"
        );
    }
}

#[test]
fn memory_and_network_json_carry_additive_extras() {
    let memory = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["memory", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(memory.status.success());
    let value: serde_json::Value = serde_json::from_slice(&memory.stdout).unwrap();
    assert!(
        value["data"].get("bandwidth").is_some(),
        "memory data must carry bandwidth key (may be null)"
    );

    let network = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["network", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(network.status.success());
    let value: serde_json::Value = serde_json::from_slice(&network.stdout).unwrap();
    assert!(
        value["data"].get("link").is_some(),
        "network data must carry link key (may be null)"
    );
}

#[test]
fn ps_rows_carry_gpu_and_vsz_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["ps", "--limit", "1", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = &value["data"]["processes"][0];
    for key in [
        "gpu_percent",
        "virtual_bytes",
        "cpu_time_seconds",
        "energy_impact",
        "power_watts",
    ] {
        assert!(row.get(key).is_some(), "process row must carry {key}");
    }
}

#[test]
fn ps_accepts_the_energy_sort_keys() {
    for key in ["energy", "nrg", "power", "watts"] {
        let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
            .args(["ps", "--sort", key, "--limit", "3", "--format", "json"])
            .output()
            .expect("bmtop binary should run");
        assert!(output.status.success(), "--sort {key} must be accepted");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value["data"]["processes"].is_array());
    }
}

#[test]
fn doctor_reports_extras_probe() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmtop"))
        .args(["doctor", "--format", "json"])
        .output()
        .expect("bmtop binary should run");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for key in ["battery", "disk_io", "wifi", "rdma", "fps_permission"] {
        assert!(
            value["extras"][key].is_boolean() || value["extras"][key].is_number(),
            "doctor.extras.{key} must exist"
        );
    }
}
