use bmtop_core::{
    AppMode, CapabilityState, GpuSnapshot, JsonEnvelope, MetricQuality, RefreshInterval,
};

#[test]
fn gpu_failure_is_not_renderable_and_clears_history() {
    let mut gpu = GpuSnapshot::new(41.0, 59.0);
    gpu.push_history(41.0);
    gpu.mark_failed("ioaccelerator");

    assert_eq!(gpu.quality(), MetricQuality::Unavailable);
    assert!(!gpu.is_renderable());
    assert!(gpu.history().is_empty());
}

#[test]
fn refresh_interval_is_bounded() {
    assert!(RefreshInterval::from_millis(250).is_ok());
    assert!(RefreshInterval::from_millis(60_000).is_ok());
    assert!(RefreshInterval::from_millis(249).is_err());
    assert!(RefreshInterval::from_millis(60_001).is_err());
}

#[test]
fn mode_numbers_are_stable() {
    assert_eq!(AppMode::from_number(1), Some(AppMode::Overview));
    assert_eq!(AppMode::from_number(7), Some(AppMode::Gpu));
    assert_eq!(AppMode::from_number(9), Some(AppMode::Sensors));
    assert_eq!(AppMode::from_number(0), None);
    assert_eq!(AppMode::from_number(10), None);
}

#[test]
fn json_envelope_has_stable_v2_shape() {
    let envelope = JsonEnvelope::new(
        "memory",
        CapabilityState::Available,
        vec!["cpu".into(), "memory".into()],
        serde_json::json!({"used": 10}),
    );
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["kind"], "memory");
    assert_eq!(value["data"]["used"], 10);
    // v2 起 capabilities 是真实能力表，不再是单元素状态串。
    assert_eq!(value["capabilities"][1], "memory");
    // v2 起 captured_at 是 RFC 3339，不再是 epoch 秒拼 Z。
    let captured_at = value["captured_at"].as_str().unwrap();
    assert!(
        captured_at.len() == 24
            && captured_at.ends_with('Z')
            && captured_at.as_bytes()[4] == b'-'
            && captured_at.as_bytes()[10] == b'T',
        "not RFC 3339: {captured_at}"
    );
}
