//! `powermetrics -f plist` 输出解析。
//!
//! 输出结构没有公开契约、随机型和系统版本变化，所以解析一律防御式：
//! 字段缺失返回 `None` 而不是报错（对齐调研结论「私有来源失败即 N/A」）。

use crate::CollectorError;
use bmtop_core::RefreshInterval;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PowerMetricsSample {
    /// GPU 当前频率。powermetrics 的键叫 `freq_hz`，实际单位是 MHz。
    pub gpu_frequency_mhz: Option<f64>,
    pub gpu_active_percent: Option<f64>,
    pub gpu_power_milliwatts: Option<f64>,
    /// Nominal / Moderate / Heavy / Trapping / Sleeping。
    pub thermal_pressure: Option<String>,
}

pub fn parse_powermetrics_plist(bytes: &[u8]) -> Result<PowerMetricsSample, CollectorError> {
    // powermetrics 每个样本以 NUL 收尾，plist 解析器不吃尾部杂讯。
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let value =
        plist::Value::from_reader(std::io::Cursor::new(&bytes[..end])).map_err(|error| {
            CollectorError::Parse {
                kind: "powermetrics".into(),
                message: error.to_string(),
            }
        })?;
    let root = value.as_dictionary().ok_or_else(|| CollectorError::Parse {
        kind: "powermetrics".into(),
        message: "root is not a dictionary".into(),
    })?;
    let gpu = root.get("gpu").and_then(plist::Value::as_dictionary);
    let elapsed_seconds = root
        .get("elapsed_ns")
        .and_then(number)
        .map(|nanoseconds| nanoseconds / 1e9);
    let gpu_power_milliwatts = root
        .get("processor")
        .and_then(plist::Value::as_dictionary)
        .and_then(|processor| processor.get("gpu_power"))
        .and_then(number)
        .or_else(|| {
            // 没有 processor sampler 时用 gpu_energy(mJ) / 时长(s) = mW 兜底。
            let energy = gpu.and_then(|gpu| gpu.get("gpu_energy")).and_then(number)?;
            let seconds = elapsed_seconds.filter(|seconds| *seconds > 0.0)?;
            Some(energy / seconds)
        });
    Ok(PowerMetricsSample {
        gpu_frequency_mhz: gpu.and_then(|gpu| gpu.get("freq_hz")).and_then(number),
        gpu_active_percent: gpu
            .and_then(|gpu| gpu.get("idle_ratio"))
            .and_then(number)
            .map(|idle| ((1.0 - idle) * 100.0).clamp(0.0, 100.0)),
        gpu_power_milliwatts,
        thermal_pressure: root
            .get("thermal_pressure")
            .and_then(plist::Value::as_string)
            .map(str::to_string),
    })
}

fn number(value: &plist::Value) -> Option<f64> {
    value
        .as_real()
        .or_else(|| value.as_signed_integer().map(|number| number as f64))
        .or_else(|| value.as_unsigned_integer().map(|number| number as f64))
}

/// 起一次 powermetrics 并解析。需要 sudo 授权，只应在 `--enhanced` 下调用。
pub fn sample_powermetrics(
    interval: RefreshInterval,
) -> Result<PowerMetricsSample, CollectorError> {
    let output = crate::run_powermetrics_once(interval)?;
    parse_powermetrics_plist(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 Apple Silicon 上 `--samplers gpu_power,thermal` 的已知键构造的样本。
    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>is_delta</key><true/>
    <key>elapsed_ns</key><integer>1000000000</integer>
    <key>hw_model</key><string>Mac15,9</string>
    <key>thermal_pressure</key><string>Nominal</string>
    <key>gpu</key>
    <dict>
        <key>freq_hz</key><real>1398.22</real>
        <key>idle_ratio</key><real>0.372</real>
        <key>gpu_energy</key><integer>18443</integer>
    </dict>
</dict>
</plist>"#;

    #[test]
    fn parses_gpu_and_thermal_fields() {
        let sample = parse_powermetrics_plist(FIXTURE.as_bytes()).unwrap();
        assert_eq!(sample.gpu_frequency_mhz, Some(1398.22));
        let active = sample.gpu_active_percent.unwrap();
        assert!((active - 62.8).abs() < 1e-9, "got {active}");
        // 无 processor sampler 时按 gpu_energy / 时长兜底：18443mJ / 1s。
        assert_eq!(sample.gpu_power_milliwatts, Some(18_443.0));
        assert_eq!(sample.thermal_pressure.as_deref(), Some("Nominal"));
    }

    #[test]
    fn missing_fields_become_none_not_errors() {
        let minimal = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>hw_model</key><string>Mac14,2</string></dict></plist>"#;
        let sample = parse_powermetrics_plist(minimal.as_bytes()).unwrap();
        assert_eq!(sample, PowerMetricsSample::default());
    }

    #[test]
    fn trailing_nul_separator_is_stripped() {
        let mut bytes = FIXTURE.as_bytes().to_vec();
        bytes.extend_from_slice(b"\0garbage after the sample separator");
        assert!(parse_powermetrics_plist(&bytes).is_ok());
    }

    #[test]
    fn garbage_is_a_parse_error() {
        assert!(matches!(
            parse_powermetrics_plist(b"not a plist"),
            Err(CollectorError::Parse { .. })
        ));
    }
}
