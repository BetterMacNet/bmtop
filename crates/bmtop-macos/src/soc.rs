//! Apple Silicon SoC 采集的 Rust 包装（C 实现见 `bmtop_soc.c`）。
//!
//! C 层持有静态状态（IOReport 订阅、上次采样），**非线程安全**：
//! 进程内只允许一个 [`SocCollector`]，且只在创建它的线程上使用
//! （TUI 的 worker 线程 / CLI 的主线程都满足）。

use bmtop_core::{
    sensor_group_for_key, BatteryInfo, ClusterMetrics, CpuTopology, FanReading, SensorReading,
    SocMetrics, SocPower, SocTemps,
};

const MAX_CLUSTERS: usize = 8;
const MAX_FANS: usize = 8;
const MAX_TEMPS: usize = 256;

#[repr(C)]
#[derive(Clone, Copy)]
struct RawCluster {
    name: [u8; 8],
    active_percent: f64,
    freq_mhz: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawFan {
    actual_rpm: u32,
    min_rpm: u32,
    max_rpm: u32,
    target_rpm: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawTemp {
    key: [u8; 5],
    celsius: f32,
}

#[repr(C)]
struct RawSample {
    cluster_count: i32,
    clusters: [RawCluster; MAX_CLUSTERS],
    cpu_watts: f64,
    gpu_watts: f64,
    ane_watts: f64,
    dram_watts: f64,
    gpu_active_percent: f64,
    gpu_freq_mhz: f64,
    cpu_temp_c: f64,
    gpu_temp_c: f64,
    soc_temp_c: f64,
    thermal_level: i32,
    fan_count: i32,
    fans: [RawFan; MAX_FANS],
    temp_count: i32,
    temps: [RawTemp; MAX_TEMPS],
    system_watts: f64,
    dram_read_bytes: i64,
    dram_write_bytes: i64,
    ane_read_bytes: i64,
    ane_write_bytes: i64,
    elapsed_ns: u64,
}

impl Default for RawSample {
    fn default() -> Self {
        // C 侧 memset 后按哨兵填充；Rust 侧仅测试用到，全部置零即可。
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
struct RawTopology {
    brand: [u8; 64],
    e_cores: i32,
    p_cores: i32,
    gpu_cores: i32,
    gpu_max_freq_mhz: i32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn bmtop_soc_init() -> i32;
    fn bmtop_soc_sample(out: *mut RawSample) -> i32;
    fn bmtop_soc_cleanup();
    fn bmtop_soc_read_topology(out: *mut RawTopology) -> i32;
    fn bmtop_soc_smc_available() -> i32;
    fn bmtop_soc_thermal_available() -> i32;
    fn bmtop_soc_read_battery(percent: *mut i32, charging: *mut i32, on_ac: *mut i32) -> i32;
}

/// doctor 用的探测结果。
#[derive(Debug, Clone, Copy, Default)]
pub struct SocProbe {
    pub ioreport: bool,
    pub smc: bool,
    pub thermal: bool,
}

/// IOReport/SMC 采集句柄。`new()` 失败（Intel、系统不支持）即永久 `None`。
pub struct SocCollector {
    _private: (),
}

#[cfg(target_os = "macos")]
impl SocCollector {
    pub fn new() -> Option<Self> {
        // bmtop_soc_init 幂等，重复调用返回 0。
        (unsafe { bmtop_soc_init() } == 0).then_some(Self { _private: () })
    }

    /// 采一次样。首次调用只做预热（返回 `None`），之后每次给出与上次
    /// 调用之间的 delta。
    pub fn sample(&mut self) -> Option<SocMetrics> {
        let mut raw = RawSample::default();
        (unsafe { bmtop_soc_sample(&mut raw) } == 0).then(|| convert_sample(&raw))
    }

    pub fn probe(&self) -> SocProbe {
        SocProbe {
            ioreport: true,
            smc: unsafe { bmtop_soc_smc_available() } != 0,
            thermal: unsafe { bmtop_soc_thermal_available() } != 0,
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for SocCollector {
    fn drop(&mut self) {
        unsafe { bmtop_soc_cleanup() };
    }
}

/// CPU/GPU 静态拓扑；与 IOReport 无关，Intel 上也能拿到品牌串。
#[cfg(target_os = "macos")]
pub fn read_topology() -> Option<CpuTopology> {
    let mut raw = RawTopology {
        brand: [0; 64],
        e_cores: 0,
        p_cores: 0,
        gpu_cores: 0,
        gpu_max_freq_mhz: 0,
    };
    (unsafe { bmtop_soc_read_topology(&mut raw) } == 0).then(|| convert_topology(&raw))
}

/// CLI 一次性采样：预热 + 等待窗口 + 正式采样。
#[cfg(target_os = "macos")]
pub fn sample_soc_once(window: std::time::Duration) -> Option<SocMetrics> {
    let mut collector = SocCollector::new()?;
    let _ = collector.sample(); // 预热
    std::thread::sleep(window);
    collector.sample()
}

fn c_str_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// 哨兵约定：功耗 <0 不可用；温度 <=0 不可用；thermal_level <0 不可用。
fn watts_field(value: f64) -> Option<f64> {
    (value >= 0.0).then_some(value)
}

fn temp_field(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

/// 累计字节 → GB/s（十进制）。负数哨兵或窗口为零时 `None`。
fn bandwidth_gbs(bytes: i64, elapsed_ns: u64) -> Option<f64> {
    (bytes >= 0 && elapsed_ns > 0).then(|| bytes as f64 / (elapsed_ns as f64 / 1e9) / 1e9)
}

fn convert_sample(raw: &RawSample) -> SocMetrics {
    let cluster_count = raw.cluster_count.clamp(0, MAX_CLUSTERS as i32) as usize;
    let clusters = raw.clusters[..cluster_count]
        .iter()
        .map(|cluster| ClusterMetrics {
            name: c_str_field(&cluster.name),
            active_percent: cluster.active_percent.clamp(0.0, 100.0),
            freq_mhz: cluster.freq_mhz.max(0.0),
        })
        .collect();
    let fan_count = raw.fan_count.clamp(0, MAX_FANS as i32) as usize;
    let fans = raw.fans[..fan_count]
        .iter()
        .enumerate()
        .map(|(index, fan)| FanReading {
            name: format!("Fan {index}"),
            actual_rpm: fan.actual_rpm,
            min_rpm: fan.min_rpm,
            max_rpm: fan.max_rpm,
            target_rpm: fan.target_rpm,
        })
        .collect();
    let temp_count = raw.temp_count.clamp(0, MAX_TEMPS as i32) as usize;
    let sensors = raw.temps[..temp_count]
        .iter()
        .map(|temp| {
            let key = c_str_field(&temp.key);
            SensorReading {
                group: sensor_group_for_key(&key).to_string(),
                key,
                celsius: f64::from(temp.celsius),
            }
        })
        .collect();
    SocMetrics {
        clusters,
        power: SocPower {
            cpu_watts: watts_field(raw.cpu_watts),
            gpu_watts: watts_field(raw.gpu_watts),
            ane_watts: watts_field(raw.ane_watts),
            dram_watts: watts_field(raw.dram_watts),
            system_watts: temp_field(raw.system_watts),
        },
        temps: SocTemps {
            cpu_celsius: temp_field(raw.cpu_temp_c),
            gpu_celsius: temp_field(raw.gpu_temp_c),
            soc_celsius: temp_field(raw.soc_temp_c),
        },
        gpu_freq_mhz: watts_field(raw.gpu_freq_mhz).filter(|&freq| freq > 0.0),
        gpu_active_percent: watts_field(raw.gpu_active_percent).map(|pct| pct.clamp(0.0, 100.0)),
        thermal_level: (raw.thermal_level >= 0).then_some(raw.thermal_level as u8),
        fans,
        sensors,
        dram_read_gbs: bandwidth_gbs(raw.dram_read_bytes, raw.elapsed_ns),
        dram_write_gbs: bandwidth_gbs(raw.dram_write_bytes, raw.elapsed_ns),
        ane_read_gbs: bandwidth_gbs(raw.ane_read_bytes, raw.elapsed_ns),
        ane_write_gbs: bandwidth_gbs(raw.ane_write_bytes, raw.elapsed_ns),
    }
}

fn convert_topology(raw: &RawTopology) -> CpuTopology {
    CpuTopology {
        brand: c_str_field(&raw.brand),
        e_cores: raw.e_cores.max(0) as u32,
        p_cores: raw.p_cores.max(0) as u32,
        gpu_cores: (raw.gpu_cores > 0).then_some(raw.gpu_cores as u32),
        gpu_max_freq_mhz: (raw.gpu_max_freq_mhz > 0).then_some(raw.gpu_max_freq_mhz as u32),
    }
}

/// 内置电池状态；无电池机型返回 `None`，电量未知时 `percent` 为 `None`。
#[cfg(target_os = "macos")]
pub fn read_battery() -> Option<BatteryInfo> {
    let mut percent = -1i32;
    let mut charging = 0i32;
    let mut on_ac = 0i32;
    let present = unsafe { bmtop_soc_read_battery(&mut percent, &mut charging, &mut on_ac) };
    (present == 1).then(|| BatteryInfo {
        percent: (0..=100).contains(&percent).then_some(percent as u8),
        charging: charging != 0,
        on_ac: on_ac != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_with_sentinels() -> RawSample {
        RawSample {
            cpu_watts: -1.0,
            gpu_watts: -1.0,
            ane_watts: -1.0,
            dram_watts: -1.0,
            gpu_active_percent: -1.0,
            gpu_freq_mhz: -1.0,
            thermal_level: -1,
            ..RawSample::default()
        }
    }

    #[test]
    fn sentinels_convert_to_none() {
        let metrics = convert_sample(&raw_with_sentinels());
        assert_eq!(metrics.power.cpu_watts, None);
        assert_eq!(metrics.power.total_watts(), None);
        assert_eq!(metrics.temps.cpu_celsius, None);
        assert_eq!(metrics.gpu_freq_mhz, None);
        assert_eq!(metrics.gpu_active_percent, None);
        assert_eq!(metrics.thermal_level, None);
        assert!(metrics.clusters.is_empty());
        assert!(metrics.fans.is_empty());
        assert!(metrics.sensors.is_empty());
    }

    #[test]
    fn populated_sample_converts_fields() {
        let mut raw = raw_with_sentinels();
        raw.cluster_count = 2;
        raw.clusters[0].name[0] = b'E';
        raw.clusters[0].active_percent = 23.4;
        raw.clusters[0].freq_mhz = 1250.0;
        raw.clusters[1].name[0] = b'P';
        raw.clusters[1].active_percent = 144.2; // 越界值应被钳位
        raw.clusters[1].freq_mhz = 3980.0;
        raw.cpu_watts = 4.8;
        raw.cpu_temp_c = 54.3;
        raw.thermal_level = 0;
        raw.fan_count = 1;
        raw.fans[0] = RawFan {
            actual_rpm: 1200,
            min_rpm: 990,
            max_rpm: 3900,
            target_rpm: 1200,
        };
        raw.temp_count = 1;
        raw.temps[0].key = *b"Tp01\0";
        raw.temps[0].celsius = 50.5;

        let metrics = convert_sample(&raw);
        assert_eq!(metrics.clusters.len(), 2);
        assert_eq!(metrics.clusters[0].name, "E");
        assert_eq!(metrics.clusters[1].active_percent, 100.0);
        assert_eq!(metrics.power.cpu_watts, Some(4.8));
        assert_eq!(metrics.temps.cpu_celsius, Some(54.3));
        assert_eq!(metrics.thermal_level, Some(0));
        assert_eq!(metrics.fans[0].name, "Fan 0");
        assert_eq!(metrics.sensors[0].key, "Tp01");
        assert_eq!(metrics.sensors[0].group, "cpu_p");
    }

    #[test]
    fn topology_conversion_handles_missing_gpu() {
        let mut raw = RawTopology {
            brand: [0; 64],
            e_cores: 4,
            p_cores: 12,
            gpu_cores: 0,
            gpu_max_freq_mhz: 0,
        };
        raw.brand[..12].copy_from_slice(b"Apple M3 Max");
        let topology = convert_topology(&raw);
        assert_eq!(topology.brand, "Apple M3 Max");
        assert_eq!(topology.e_cores, 4);
        assert_eq!(topology.p_cores, 12);
        assert_eq!(topology.gpu_cores, None);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod machine_tests {
    use super::*;

    /// 真机冒烟：需要 Apple Silicon。`cargo test -p bmtop-macos -- --ignored`
    #[test]
    #[ignore]
    fn soc_smoke_reports_clusters_and_power() {
        let metrics = sample_soc_once(std::time::Duration::from_millis(600))
            .expect("SocCollector 初始化或采样失败");
        assert!(metrics.clusters.len() >= 2, "应至少有 E/P 两个集群");
        let cpu_watts = metrics.power.cpu_watts.expect("应有 CPU 功耗");
        assert!(cpu_watts > 0.0 && cpu_watts < 200.0);
        if let Some(temp) = metrics.temps.cpu_celsius {
            assert!((10.0..120.0).contains(&temp));
        }
        let topology = read_topology().expect("拓扑读取失败");
        assert!(topology.e_cores > 0 && topology.p_cores > 0);
    }
}
