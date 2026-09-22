use bmtop_core::{privacy::REDACTED, DiskVolume, PrivacyMode, SystemSnapshot};
use serde_json::{json, Value};

fn snapshot() -> SystemSnapshot {
    // A legacy schema-v2 document without any new optional fields.
    serde_json::from_value(json!({
        "captured_at": "2026-01-01T00:00:00.000Z", "captured_at_display": "00:00:00",
        "cpu": { "total_percent": null, "user_percent": null, "system_percent": null, "idle_percent": null, "load_average": [], "per_core_percent": [] },
        "memory": { "total_bytes": 100, "used_bytes": 50, "free_bytes": 50, "wired_bytes": 0, "compressed_bytes": 0, "active_bytes": 0, "inactive_bytes": 0, "purgeable_bytes": 0, "swapins": 0, "swapouts": 0, "swap_used_bytes": 0, "pressure_percent": null },
        "processes": [{"pid": 42, "parent_pid": 1, "uid": 501, "user": "SYNTHETIC_SECRET_USER", "name": "Private application",
          "path": "/Users/SYNTHETIC_SECRET_USER/app", "state": "run", "resident_bytes": 512, "thread_count": 1, "file_descriptor_count": null,
          "cpu_percent": 12.0, "start_time_seconds": 1, "start_time_microseconds": 2,
          "arguments": ["app", "--token=SYNTHETIC_SECRET_TOKEN"],
          "threads": [{"thread_id": 9, "name": "SYNTHETIC_SECRET_THREAD", "state": "run", "cpu_percent": 1.0}]}],
        "interfaces": [], "gpu": null, "capabilities": ["cpu"]
    })).unwrap()
}

#[test]
fn synthetic_profiler_fixture_is_fail_closed_and_still_informative() {
    let input: Value =
        serde_json::from_str(include_str!("fixtures/profiler-synthetic.json")).unwrap();
    for mode in [PrivacyMode::Default, PrivacyMode::Strict] {
        let mut value = input.clone();
        mode.redact_profiler(value.as_object_mut().unwrap());
        assert!(!value.to_string().contains("SYNTHETIC_SECRET"));
        assert_eq!(
            value["SPHardwareDataType"][0]["machine_model"],
            "Mac-SYNTHETIC"
        );
        assert_eq!(
            value["SPPowerDataType"][0]["sppower_battery_health_info"]
                ["sppower_battery_cycle_count"],
            42
        );
    }
}

#[test]
fn strict_removes_selected_process_identifiers_but_preserves_metrics() {
    let mut sample = snapshot();
    PrivacyMode::Strict.redact_snapshot(&mut sample);
    let process = &sample.processes[0];
    assert_eq!(process.pid, 42);
    assert_eq!(process.cpu_percent, Some(12.0));
    assert_eq!(process.name, REDACTED);
    assert!(process.path.is_none());
    assert!(process.arguments.is_none());
    assert!(!serde_json::to_string(&sample)
        .unwrap()
        .contains("SYNTHETIC_SECRET"));
}

#[test]
fn default_and_off_have_distinct_explicit_boundaries() {
    let mut safe = snapshot();
    PrivacyMode::Default.redact_snapshot(&mut safe);
    assert!(safe.processes[0].arguments.is_none());
    assert!(safe.processes[0].path.is_some()); // default is NOT anonymous
    let mut off = snapshot();
    PrivacyMode::Off.redact_snapshot(&mut off);
    assert!(off.processes[0].arguments.is_some());
}

#[test]
fn old_schema_v2_snapshots_deserialize_without_gpu_coverage() {
    let sample = snapshot();
    assert!(sample.gpu_process_coverage.is_none());
    assert!(serde_json::to_value(sample)
        .unwrap()
        .get("gpu_process_coverage")
        .is_none());
}

#[test]
fn strict_disk_reports_hide_remote_shares_and_nonroot_mounts() {
    let mut volumes: Vec<DiskVolume> = serde_json::from_value(json!([
      {"filesystem":"//SYNTHETIC_SECRET_USER@host/share", "mountpoint":"/Volumes/private", "total_bytes":100, "used_bytes":50, "available_bytes":50, "used_percent":50.0},
      {"filesystem":"/dev/disk1s1", "mountpoint":"/", "total_bytes":100, "used_bytes":50, "available_bytes":50, "used_percent":50.0}
    ])).unwrap();
    PrivacyMode::Strict.redact_disks(&mut volumes);
    assert_eq!(volumes[0].filesystem, REDACTED);
    assert_eq!(volumes[0].mountpoint, REDACTED);
    assert_eq!(volumes[1].mountpoint, "/");
    assert_eq!(volumes[1].total_bytes, 100);
}
