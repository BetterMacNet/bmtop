//! `/usr/share/pmenergy/*.plist` 的 `energy_constants` 读取。
//!
//! 活动监视器的「能耗」列就按这份系数加权。解析一律防御式：缺键回落到
//! [`EnergyCoefficients::default`]，绝不因为某个键消失就整表作废。

use bmtop_core::EnergyCoefficients;

const DEFAULT_PLIST_PATH: &str = "/usr/share/pmenergy/default.plist";

/// 从 plist 字节里取系数；任何一层缺失都退回默认值的对应项。
pub fn parse_energy_constants(bytes: &[u8]) -> EnergyCoefficients {
    let fallback = EnergyCoefficients::default();
    let Ok(value) = plist::Value::from_reader(std::io::Cursor::new(bytes)) else {
        return fallback;
    };
    let Some(constants) = value
        .as_dictionary()
        .and_then(|root| root.get("energy_constants"))
        .and_then(plist::Value::as_dictionary)
    else {
        return fallback;
    };
    let read = |key: &str, default: f64| {
        constants
            .get(key)
            .and_then(number)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(default)
    };
    EnergyCoefficients {
        cpu_time: read("kcpu_time", fallback.cpu_time),
        cpu_wakeups: read("kcpu_wakeups", fallback.cpu_wakeups),
        diskio_bytesread: read("kdiskio_bytesread", fallback.diskio_bytesread),
        diskio_byteswritten: read("kdiskio_byteswritten", fallback.diskio_byteswritten),
        qos_default: read("kqos_default", fallback.qos_default),
        qos_background: read("kqos_background", fallback.qos_background),
        qos_utility: read("kqos_utility", fallback.qos_utility),
        qos_legacy: read("kqos_legacy", fallback.qos_legacy),
        qos_user_initiated: read("kqos_user_initiated", fallback.qos_user_initiated),
        qos_user_interactive: read("kqos_user_interactive", fallback.qos_user_interactive),
    }
}

/// 读机器的系数表。文件读不到（沙箱 / 系统裁剪）就用默认值。
///
/// ponytail: 只读 default.plist。Apple Silicon 的 `IOPlatformExpertDevice` 没有
/// `board-id`，本来就只能落到这一份；Intel 机器的 `Mac-<board-id>.plist` 差异集中在
/// `kgpu_time` 和磁盘系数上，而主导项 `kcpu_time` / `kcpu_wakeups` 三份实测完全一致。
/// 真要在 Intel 上对齐到小数点，再加 board-id 查找。
pub fn load_energy_coefficients() -> EnergyCoefficients {
    std::fs::read(DEFAULT_PLIST_PATH)
        .map(|bytes| parse_energy_constants(&bytes))
        .unwrap_or_default()
}

fn number(value: &plist::Value) -> Option<f64> {
    value
        .as_real()
        .or_else(|| value.as_signed_integer().map(|number| number as f64))
        .or_else(|| value.as_unsigned_integer().map(|number| number as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 `/usr/share/pmenergy/default.plist` 的实际内容（Apple Silicon）。
    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>energy_constants</key>
    <dict>
        <key>kcpu_time</key><real>1.0</real>
        <key>kcpu_wakeups</key><real>0.0002</real>
        <key>kdiskio_bytesread</key><real>4.5e-10</real>
        <key>kdiskio_byteswritten</key><real>2.4e-10</real>
        <key>kgpu_time</key><real>0.0</real>
        <key>kqos_background</key><real>0.8</real>
        <key>kqos_default</key><real>1.0</real>
        <key>kqos_legacy</key><real>1.0</real>
        <key>kqos_user_initiated</key><real>1.0</real>
        <key>kqos_user_interactive</key><real>1.0</real>
        <key>kqos_utility</key><real>1.0</real>
    </dict>
</dict>
</plist>"#;

    #[test]
    fn parses_the_shipped_default_constants() {
        let coefficients = parse_energy_constants(FIXTURE.as_bytes());
        assert_eq!(coefficients, EnergyCoefficients::default());
    }

    #[test]
    fn board_specific_values_override_the_defaults() {
        // Intel 机型的实测差异：磁盘系数大两个数量级，background 权重也不同。
        let intel = FIXTURE
            .replace(
                "<key>kdiskio_bytesread</key><real>4.5e-10</real>",
                "<key>kdiskio_bytesread</key><real>1.4e-08</real>",
            )
            .replace(
                "<key>kqos_background</key><real>0.8</real>",
                "<key>kqos_background</key><real>0.74</real>",
            );
        let coefficients = parse_energy_constants(intel.as_bytes());
        assert_eq!(coefficients.diskio_bytesread, 1.4e-08);
        assert_eq!(coefficients.qos_background, 0.74);
        assert_eq!(coefficients.cpu_wakeups, 0.0002);
    }

    #[test]
    fn missing_keys_fall_back_per_field() {
        let partial = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>energy_constants</key>
<dict><key>kcpu_wakeups</key><real>0.5</real></dict></dict></plist>"#;
        let coefficients = parse_energy_constants(partial.as_bytes());
        assert_eq!(coefficients.cpu_wakeups, 0.5);
        assert_eq!(
            coefficients.cpu_time,
            EnergyCoefficients::default().cpu_time
        );
    }

    #[test]
    fn garbage_falls_back_instead_of_panicking() {
        assert_eq!(
            parse_energy_constants(b"not a plist"),
            EnergyCoefficients::default()
        );
        assert_eq!(parse_energy_constants(b""), EnergyCoefficients::default());
    }
}
