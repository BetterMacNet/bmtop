//! Apple Silicon SoC 指标（IOReport / SMC 采集）的平台无关模型。
//!
//! 采集器在 Intel 机器或 IOReport 初始化失败时给出 `None`，
//! 界面按 GPU 的先例隐藏对应行，CLI 契约按增量字段处理。

use serde::{Deserialize, Serialize};

/// ANE 满载功耗估算基准（瓦）。mactop 同款：利用率 ≈ 功耗 / 8W。
pub const ANE_MAX_POWER_WATTS: f64 = 8.0;

/// 单个 CPU 集群（E/P/S）的活跃度与频率。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterMetrics {
    /// 集群类型："E"、"P" 或 "S"。
    pub name: String,
    pub active_percent: f64,
    pub freq_mhz: f64,
}

/// 各域功耗（瓦）。`None` 表示对应能量通道不存在。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SocPower {
    pub cpu_watts: Option<f64>,
    pub gpu_watts: Option<f64>,
    pub ane_watts: Option<f64>,
    pub dram_watts: Option<f64>,
    /// 整机输入功率（SMC `PSTR`，墙上功率）；不计入 [`Self::total_watts`]。
    #[serde(default)]
    pub system_watts: Option<f64>,
}

impl SocPower {
    /// 已知各域之和；全部缺失时为 `None`。
    pub fn total_watts(&self) -> Option<f64> {
        let parts = [
            self.cpu_watts,
            self.gpu_watts,
            self.ane_watts,
            self.dram_watts,
        ];
        parts
            .iter()
            .flatten()
            .copied()
            .fold(None, |sum, value| Some(sum.unwrap_or(0.0) + value))
    }

    /// ANE 利用率估算：功耗 / 8W，钳位 0–100。
    pub fn ane_active_percent(&self) -> Option<f64> {
        self.ane_watts
            .map(|watts| (watts / ANE_MAX_POWER_WATTS * 100.0).clamp(0.0, 100.0))
    }
}

/// 关键温度（摄氏度）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SocTemps {
    pub cpu_celsius: Option<f64>,
    pub gpu_celsius: Option<f64>,
    pub soc_celsius: Option<f64>,
}

/// 一把风扇的读数（RPM）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanReading {
    pub name: String,
    pub actual_rpm: u32,
    pub min_rpm: u32,
    pub max_rpm: u32,
    pub target_rpm: u32,
}

impl FanReading {
    /// 实际转速在 min–max 区间内的百分比；区间退化（max<=min）时为 `None`。
    pub fn percent(&self) -> Option<f64> {
        (self.max_rpm > self.min_rpm).then(|| {
            let span = f64::from(self.max_rpm - self.min_rpm);
            let above = f64::from(self.actual_rpm.saturating_sub(self.min_rpm));
            (above / span * 100.0).clamp(0.0, 100.0)
        })
    }
}

/// 单个 SMC 温度传感器读数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorReading {
    /// 4 字符 SMC key（如 `Tp01`）。
    pub key: String,
    /// 分组标识，取值见 [`sensor_group_for_key`]。
    pub group: String,
    pub celsius: f64,
}

/// 一次 SoC 采样的全部结果。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocMetrics {
    pub clusters: Vec<ClusterMetrics>,
    pub power: SocPower,
    pub temps: SocTemps,
    pub gpu_freq_mhz: Option<f64>,
    pub gpu_active_percent: Option<f64>,
    /// 系统热压力等级 0–4（正常/偏暖/严重/临界/休眠）。
    pub thermal_level: Option<u8>,
    /// 空表示无风扇机型或 SMC key 缺失，界面直接省略段落。
    pub fans: Vec<FanReading>,
    /// 原始传感器列表（上限由采集器控制），供传感器页分组。
    pub sensors: Vec<SensorReading>,
    /// DRAM 带宽（GB/s，十进制；AMC 字节计数器）。无源时为 `None`。
    #[serde(default)]
    pub dram_read_gbs: Option<f64>,
    #[serde(default)]
    pub dram_write_gbs: Option<f64>,
    /// ANE 带宽（GB/s）。
    #[serde(default)]
    pub ane_read_gbs: Option<f64>,
    #[serde(default)]
    pub ane_write_gbs: Option<f64>,
}

/// CPU/GPU 静态拓扑，进程存续期间不变，采集一次即可。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuTopology {
    pub brand: String,
    pub e_cores: u32,
    pub p_cores: u32,
    pub gpu_cores: Option<u32>,
    /// GPU 频率表最大档（MHz）；TFLOPS 由 [`crate::gpu_tflops_fp32`] 计算。
    #[serde(default)]
    pub gpu_max_freq_mhz: Option<u32>,
}

/// SMC 温度 key → 分组标识。
///
/// SMC key 大小写敏感：`Tp*`（E/P 核）与 `TPD*`（SoC 封装）是不同前缀。
/// `Tf*` 在 M3 上是 P 核（mactop 实测），归入 CPU。
pub fn sensor_group_for_key(key: &str) -> &'static str {
    // 移植自 mactop tempSensorName：多字符前缀优先，其余按第二字符归类。
    // SMC key 大小写敏感：Tp*（每核）与 TPD*（SoC 封装）是不同前缀。
    const PREFIX_GROUPS: [(&str, &str); 9] = [
        ("TPD", "soc"), // SoC Package Die
        ("TPM", "soc"),
        ("TPS", "soc"),
        ("TRD", "gpu"),     // GPU Render Die（M3 系列）
        ("TCM", "cpu_die"), // CPU Die Max
        ("TCD", "cpu_die"), // CPU Die 聚合
        ("Tp", "cpu_p"),    // P 核每核（M1/M2/M4）
        ("Te", "cpu_e"),    // E 核每核
        ("Tf", "cpu_p"),    // P 核每核（M3）
    ];
    if let Some((_, group)) = PREFIX_GROUPS
        .iter()
        .find(|(prefix, _)| key.starts_with(prefix))
    {
        return group;
    }
    match key.as_bytes().get(1) {
        Some(b'C') | Some(b'c') => "cpu_die",
        Some(b'g') | Some(b'R') => "gpu",
        Some(b'P') => "soc",
        Some(b'm') | Some(b'M') => "memory",
        Some(b's') | Some(b'S') | Some(b'H') | Some(b'N') => "ssd",
        Some(b'a') | Some(b'A') | Some(b'F') => "ambient",
        Some(b'B') | Some(b'b') => "board",
        Some(b'V') => "vrm",
        Some(b'D') | Some(b'd') | Some(b'L') => "display",
        Some(b'w') | Some(b'W') => "wireless",
        _ => "other",
    }
}

/// 传感器分组统计（供传感器页渲染一行 `组名 avg (min–max) ×count`）。
#[derive(Debug, Clone, PartialEq)]
pub struct SensorGroupStat {
    pub group: String,
    pub average: f64,
    pub min: f64,
    pub max: f64,
    pub count: usize,
}

/// 按分组聚合传感器读数，输出顺序固定（CPU → GPU → SoC → 内存 → SSD → 环境 → 主板 → 其他）。
pub fn group_sensor_stats(sensors: &[SensorReading]) -> Vec<SensorGroupStat> {
    const GROUP_ORDER: [&str; 13] = [
        "cpu_e", "cpu_p", "cpu_die", "gpu", "soc", "memory", "ssd", "vrm", "display", "wireless",
        "ambient", "board", "other",
    ];
    GROUP_ORDER
        .iter()
        .filter_map(|group| {
            let values: Vec<f64> = sensors
                .iter()
                .filter(|sensor| sensor.group == *group)
                .map(|sensor| sensor.celsius)
                .collect();
            (!values.is_empty()).then(|| SensorGroupStat {
                group: (*group).to_string(),
                average: values.iter().sum::<f64>() / values.len() as f64,
                min: values.iter().copied().fold(f64::INFINITY, f64::min),
                max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                count: values.len(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(key: &str, celsius: f64) -> SensorReading {
        SensorReading {
            key: key.to_string(),
            group: sensor_group_for_key(key).to_string(),
            celsius,
        }
    }

    #[test]
    fn total_watts_sums_only_present_domains() {
        let power = SocPower {
            cpu_watts: Some(1.5),
            gpu_watts: None,
            ane_watts: Some(0.5),
            dram_watts: None,
            system_watts: Some(40.0),
        };
        assert_eq!(power.total_watts(), Some(2.0));
    }

    #[test]
    fn total_watts_is_none_when_all_domains_missing() {
        assert_eq!(SocPower::default().total_watts(), None);
    }

    #[test]
    fn ane_active_percent_scales_and_clamps() {
        let half = SocPower {
            ane_watts: Some(4.0),
            ..SocPower::default()
        };
        assert_eq!(half.ane_active_percent(), Some(50.0));
        let over = SocPower {
            ane_watts: Some(100.0),
            ..SocPower::default()
        };
        assert_eq!(over.ane_active_percent(), Some(100.0));
        assert_eq!(SocPower::default().ane_active_percent(), None);
    }

    #[test]
    fn fan_percent_maps_rpm_range() {
        let fan = FanReading {
            name: "Fan 0".to_string(),
            actual_rpm: 2000,
            min_rpm: 1000,
            max_rpm: 3000,
            target_rpm: 2000,
        };
        assert_eq!(fan.percent(), Some(50.0));
    }

    #[test]
    fn fan_percent_is_none_when_range_degenerate() {
        let fan = FanReading {
            name: "Fan 0".to_string(),
            actual_rpm: 2000,
            min_rpm: 3000,
            max_rpm: 3000,
            target_rpm: 2000,
        };
        assert_eq!(fan.percent(), None);
    }

    #[test]
    fn sensor_groups_follow_smc_prefixes() {
        assert_eq!(sensor_group_for_key("Tp01"), "cpu_p");
        assert_eq!(sensor_group_for_key("Te05"), "cpu_e");
        assert_eq!(sensor_group_for_key("Tf04"), "cpu_p");
        assert_eq!(sensor_group_for_key("TC0a"), "cpu_die");
        assert_eq!(sensor_group_for_key("TPD1"), "soc");
        assert_eq!(sensor_group_for_key("Tg0f"), "gpu");
        assert_eq!(sensor_group_for_key("Tm02"), "memory");
        assert_eq!(sensor_group_for_key("TS0a"), "ssd");
        assert_eq!(sensor_group_for_key("TH0x"), "ssd");
        assert_eq!(sensor_group_for_key("Ta01"), "ambient");
        assert_eq!(sensor_group_for_key("TB1T"), "board");
        assert_eq!(sensor_group_for_key("TRD3"), "gpu");
        assert_eq!(sensor_group_for_key("TCMz"), "cpu_die");
        assert_eq!(sensor_group_for_key("TCDX"), "cpu_die");
        assert_eq!(sensor_group_for_key("TV0h"), "vrm");
        assert_eq!(sensor_group_for_key("TD01"), "display");
        assert_eq!(sensor_group_for_key("TW0P"), "wireless");
        assert_eq!(sensor_group_for_key("TPSP"), "soc");
        assert_eq!(sensor_group_for_key("Tz99"), "other");
    }

    #[test]
    fn group_sensor_stats_aggregates_in_fixed_order() {
        let sensors = vec![
            reading("Tg01", 48.0),
            reading("Tp01", 50.0),
            reading("Tp02", 60.0),
            reading("Tm01", 40.0),
        ];
        let stats = group_sensor_stats(&sensors);
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].group, "cpu_p");
        assert_eq!(stats[0].average, 55.0);
        assert_eq!(stats[0].min, 50.0);
        assert_eq!(stats[0].max, 60.0);
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[1].group, "gpu");
        assert_eq!(stats[2].group, "memory");
    }

    #[test]
    fn group_sensor_stats_empty_input_yields_empty() {
        assert!(group_sensor_stats(&[]).is_empty());
    }
}
