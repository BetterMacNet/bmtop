//! Platform-independent models and invariants for bmtop.

mod energy;
mod extras;
mod i18n;
mod soc;

pub use energy::{EnergyCoefficients, ProcessEnergyCounters, ProcessEnergyHistory, QOS_BUCKETS};
pub use extras::{
    format_link_speed, gpu_tflops_fp32, BatteryInfo, DiskIoRates, DisplayFps, EthernetLink,
    LinkInfo, RdmaDevice, RdmaStatus, TbBus, TbDevice, WifiLink,
};
pub use i18n::{Language, LanguageParseError, Strings};
pub use soc::{
    group_sensor_stats, sensor_group_for_key, ClusterMetrics, CpuTopology, FanReading,
    SensorGroupStat, SensorReading, SocMetrics, SocPower, SocTemps, ANE_MAX_POWER_WATTS,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// JSON 输出契约版本。v2：`captured_at` 改为 RFC 3339、`capabilities`
/// 携带真实能力表、新增 swap 字节 / per-core CPU / 进程 I/O 等字段。
pub const SCHEMA_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricQuality {
    Fresh,
    Stale,
    Unavailable,
    PermissionDenied,
    Loading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    Unavailable,
    PermissionDenied,
    Loading,
}

impl fmt::Display for CapabilityState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::Loading => "loading",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RefreshIntervalError {
    #[error("refresh interval must be between 250ms and 60s")]
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefreshInterval(u64);

impl RefreshInterval {
    pub const MIN_MILLIS: u64 = 250;
    pub const MAX_MILLIS: u64 = 60_000;

    pub fn from_millis(millis: u64) -> Result<Self, RefreshIntervalError> {
        if (Self::MIN_MILLIS..=Self::MAX_MILLIS).contains(&millis) {
            Ok(Self(millis))
        } else {
            Err(RefreshIntervalError::OutOfRange)
        }
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

impl Default for RefreshInterval {
    fn default() -> Self {
        Self(1_000)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    Overview,
    Processes,
    Cpu,
    Memory,
    Network,
    Disk,
    Gpu,
    Hardware,
    Sensors,
}

impl AppMode {
    pub const ALL: [Self; 9] = [
        Self::Overview,
        Self::Processes,
        Self::Cpu,
        Self::Memory,
        Self::Network,
        Self::Disk,
        Self::Gpu,
        Self::Hardware,
        Self::Sensors,
    ];

    pub const fn number(self) -> u8 {
        match self {
            Self::Overview => 1,
            Self::Processes => 2,
            Self::Cpu => 3,
            Self::Memory => 4,
            Self::Network => 5,
            Self::Disk => 6,
            Self::Gpu => 7,
            Self::Hardware => 8,
            Self::Sensors => 9,
        }
    }

    pub const fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::Overview),
            2 => Some(Self::Processes),
            3 => Some(Self::Cpu),
            4 => Some(Self::Memory),
            5 => Some(Self::Network),
            6 => Some(Self::Disk),
            7 => Some(Self::Gpu),
            8 => Some(Self::Hardware),
            9 => Some(Self::Sensors),
            _ => None,
        }
    }

    pub const fn label(self, language: Language) -> &'static str {
        let strings = language.strings();
        match self {
            Self::Overview => strings.mode_overview,
            Self::Processes => strings.mode_processes,
            Self::Cpu => strings.mode_cpu,
            Self::Memory => strings.mode_memory,
            Self::Network => strings.mode_network,
            Self::Disk => strings.mode_disk,
            Self::Gpu => strings.mode_gpu,
            Self::Hardware => strings.mode_hardware,
            Self::Sensors => strings.mode_sensors,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuSnapshot {
    pub utilization_percent: f64,
    pub idle_percent: f64,
    /// GPU 型号名（来自 SPDisplaysDataType，采集端缓存后填入）。
    #[serde(default)]
    pub name: Option<String>,
    quality: MetricQuality,
    error_kind: Option<String>,
    history: VecDeque<f64>,
}

impl GpuSnapshot {
    const MAX_HISTORY: usize = 3_600;

    pub fn new(utilization_percent: f64, idle_percent: f64) -> Self {
        Self {
            utilization_percent: utilization_percent.clamp(0.0, 100.0),
            idle_percent: idle_percent.clamp(0.0, 100.0),
            name: None,
            quality: MetricQuality::Fresh,
            error_kind: None,
            history: VecDeque::new(),
        }
    }

    pub fn push_history(&mut self, utilization_percent: f64) {
        if !self.is_renderable() || !utilization_percent.is_finite() {
            return;
        }
        if self.history.len() == Self::MAX_HISTORY {
            self.history.pop_front();
        }
        self.history
            .push_back(utilization_percent.clamp(0.0, 100.0));
    }

    pub fn update(&mut self, utilization_percent: f64, idle_percent: f64) {
        self.utilization_percent = utilization_percent.clamp(0.0, 100.0);
        self.idle_percent = idle_percent.clamp(0.0, 100.0);
        self.quality = MetricQuality::Fresh;
        self.error_kind = None;
        self.push_history(self.utilization_percent);
    }

    pub fn mark_failed(&mut self, error_kind: impl Into<String>) {
        self.quality = MetricQuality::Unavailable;
        self.error_kind = Some(error_kind.into());
        self.history.clear();
    }

    pub fn quality(&self) -> MetricQuality {
        self.quality
    }

    pub fn error_kind(&self) -> Option<&str> {
        self.error_kind.as_deref()
    }

    pub fn history(&self) -> &VecDeque<f64> {
        &self.history
    }

    pub fn is_renderable(&self) -> bool {
        matches!(self.quality, MetricQuality::Fresh)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEnvelope {
    pub schema_version: u8,
    pub kind: String,
    pub captured_at: String,
    pub source: String,
    pub capabilities: Vec<String>,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRow {
    pub pid: i32,
    pub parent_pid: i32,
    pub uid: u32,
    pub user: String,
    pub name: String,
    pub path: Option<String>,
    pub state: String,
    pub resident_bytes: Option<u64>,
    /// 虚拟地址空间大小；仅在 TUI 详情栏展示。
    #[serde(default)]
    pub virtual_bytes: Option<u64>,
    pub thread_count: Option<u32>,
    pub file_descriptor_count: Option<u32>,
    pub cpu_percent: Option<f64>,
    /// GPU 使用率（AGX 累计 GPU 时间差分，按系统 GPU% 归一）。
    #[serde(default)]
    pub gpu_percent: Option<f64>,
    /// 进程累计 CPU 时间（秒，user+system）。
    #[serde(default)]
    pub cpu_time_seconds: Option<f64>,
    /// 能耗影响，对齐活动监视器的能耗列（无量纲，见 [`EnergyCoefficients`]）。
    #[serde(default)]
    pub energy_impact: Option<f64>,
    /// 估算功耗：把 IOReport 实测的 CPU/GPU 封装瓦特按占用比摊到进程。
    /// 无 SoC 读数（Intel / IOReport 初始化失败）时为 `None`。
    #[serde(default)]
    pub power_watts: Option<f64>,
    pub start_time_seconds: u64,
    pub start_time_microseconds: u64,
    /// 以下三项只在「详情路径」（选中进程 / `ps --pid`）按需填充，
    /// 全进程热路径一律为 `None`，避免每秒对上千个 PID 做重查询。
    #[serde(default)]
    pub disk_read_bytes: Option<u64>,
    #[serde(default)]
    pub disk_written_bytes: Option<u64>,
    #[serde(default)]
    pub arguments: Option<Vec<String>>,
    #[serde(default)]
    pub threads: Option<Vec<ThreadRow>>,
}

/// 选中进程的单个线程（详情路径按需采集）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadRow {
    pub thread_id: u64,
    pub name: Option<String>,
    pub state: String,
    pub cpu_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuMetrics {
    pub total_percent: Option<f64>,
    pub user_percent: Option<f64>,
    pub system_percent: Option<f64>,
    pub idle_percent: Option<f64>,
    pub load_average: Vec<f64>,
    /// 每逻辑核的占用百分比，首个样本或核数变化时为空。
    #[serde(default)]
    pub per_core_percent: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub active_bytes: u64,
    pub inactive_bytes: u64,
    pub wired_bytes: u64,
    pub compressed_bytes: u64,
    pub purgeable_bytes: u64,
    pub swapins: u64,
    pub swapouts: u64,
    #[serde(default)]
    pub swap_total_bytes: u64,
    #[serde(default)]
    pub swap_used_bytes: u64,
    pub pressure_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceMetrics {
    pub name: String,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub receive_bytes_per_second: Option<f64>,
    pub send_bytes_per_second: Option<f64>,
}

/// 一个已挂载的本地卷。
///
/// APFS 上 `df` 自报的 `capacity` 对根卷是错的：根卷是密封的系统快照，
/// 它的 `used` 只算系统文件，而容器的真实占用要用 `总量 - 可用` 才对得上
/// 访达显示的数字。`used_bytes` 因此统一按 `总量 - 可用` 计算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskVolume {
    pub filesystem: String,
    pub mountpoint: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: Option<f64>,
}

impl DiskVolume {
    pub fn new(
        filesystem: impl Into<String>,
        mountpoint: impl Into<String>,
        total_bytes: u64,
        available_bytes: u64,
    ) -> Self {
        let used_bytes = total_bytes.saturating_sub(available_bytes);
        let used_percent = (total_bytes > 0)
            .then(|| (used_bytes as f64 / total_bytes as f64 * 100.0).clamp(0.0, 100.0));
        Self {
            filesystem: filesystem.into(),
            mountpoint: mountpoint.into(),
            total_bytes,
            used_bytes,
            available_bytes,
            used_percent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    /// 采样时刻，schema v2 起为 UTC RFC 3339（v1 是 epoch 秒拼 `Z` 的非法格式）。
    pub captured_at: String,
    /// 供界面显示的本地挂钟时间 `HH:MM:SS`。`captured_at` 是 epoch 串，
    /// 直接摆在标题栏没人看得懂，所以另存一份给渲染用。
    pub captured_at_display: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub processes: Vec<ProcessRow>,
    pub interfaces: Vec<NetworkInterfaceMetrics>,
    pub gpu: Option<GpuSnapshot>,
    pub capabilities: Vec<String>,
    /// 开机至今的秒数（`kern.boottime`），读不到时为 `None`。
    #[serde(default)]
    pub uptime_seconds: Option<u64>,
    /// Apple Silicon SoC 指标（IOReport/SMC）；Intel 或初始化失败时为 `None`。
    #[serde(default)]
    pub soc: Option<SocMetrics>,
    /// CPU/GPU 静态拓扑；读不到时为 `None`。
    #[serde(default)]
    pub topology: Option<CpuTopology>,
    /// 内置电池；无电池机型为 `None`。
    #[serde(default)]
    pub battery: Option<BatteryInfo>,
    /// 系统级磁盘 I/O 速率；首个采样或读不到时为 `None`。
    #[serde(default)]
    pub disk_io: Option<DiskIoRates>,
    /// 网络链路（Ethernet 速率 / Wi-Fi 代际），5 秒缓存。
    #[serde(default)]
    pub link: Option<LinkInfo>,
    /// 屏幕合成帧率；默认关闭，`f` 键开启且已授权时才有值。
    #[serde(default)]
    pub fps: Option<DisplayFps>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessCpuHistory {
    counters: HashMap<(i32, u64, u64), (u64, u64, std::time::Instant)>,
}

impl ProcessCpuHistory {
    pub fn cpu_percent(
        &mut self,
        pid: i32,
        start_seconds: u64,
        start_microseconds: u64,
        user_ticks: u64,
        system_ticks: u64,
        now: std::time::Instant,
    ) -> Option<f64> {
        let key = (pid, start_seconds, start_microseconds);
        let current = user_ticks.saturating_add(system_ticks);
        let previous = self.counters.insert(key, (current, 0, now));
        let result = previous.and_then(|(old, _, captured)| {
            let elapsed = now.duration_since(captured).as_secs_f64();
            (elapsed > 0.0 && current >= old)
                .then(|| (current - old) as f64 / 1_000_000_000.0 / elapsed * 100.0)
        });
        self.counters
            .retain(|_, (_, _, captured)| now.duration_since(*captured).as_secs() < 120);
        result.map(|value| value.min(10_000.0))
    }
}

impl JsonEnvelope {
    /// `capabilities` 应传快照的真实能力表；没有快照的报告类输出
    /// 传 `vec![state.to_string()]` 即可。
    pub fn new(
        kind: impl Into<String>,
        state: CapabilityState,
        capabilities: Vec<String>,
        data: Value,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: kind.into(),
            captured_at: rfc3339_now(),
            source: state.to_string(),
            capabilities,
            data,
        }
    }
}

/// 当前时刻的 UTC RFC 3339 时间戳，毫秒精度。
pub fn rfc3339_now() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    rfc3339_utc(duration.as_secs(), duration.subsec_millis())
}

/// epoch 秒 + 毫秒 → `2026-08-28T02:30:14.274Z`。
/// 手写 20 行换掉 chrono 依赖；只做 UTC，不做时区。
pub fn rfc3339_utc(epoch_seconds: u64, millis: u32) -> String {
    let (year, month, day) = civil_from_days((epoch_seconds / 86_400) as i64);
    let seconds = epoch_seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// Howard Hinnant 的 civil_from_days 算法：epoch 天数 → (年, 月, 日)。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_volume_uses_container_usage_not_df_capacity() {
        // 真实根卷数据：df 自报 1%，但容器实际占用接近一半。
        let volume = DiskVolume::new("/dev/disk3s1s1", "/", 7_811_085_600, 4_116_883_116);
        assert_eq!(volume.used_bytes, 3_694_202_484);
        let percent = volume.used_percent.unwrap();
        assert!((percent - 47.29).abs() < 0.01, "got {percent}");
    }

    #[test]
    fn disk_volume_handles_degenerate_sizes() {
        let empty = DiskVolume::new("none", "/nowhere", 0, 0);
        assert_eq!(empty.used_percent, None);
        // 可用大于总量（reserved blocks 的反常上报）不得下溢。
        let odd = DiskVolume::new("/dev/disk1", "/odd", 100, 250);
        assert_eq!(odd.used_bytes, 0);
        assert_eq!(odd.used_percent, Some(0.0));
    }

    #[test]
    fn mode_labels_are_stable() {
        assert_eq!(AppMode::ALL.len(), 9);
        assert_eq!(AppMode::Gpu.label(Language::Chinese), "GPU");
        assert_eq!(AppMode::Overview.label(Language::Chinese), "概览");
        assert_eq!(AppMode::Overview.label(Language::English), "Overview");
        assert_eq!(AppMode::Sensors.label(Language::English), "Sensors");
    }

    #[test]
    fn gpu_history_is_bounded() {
        let mut gpu = GpuSnapshot::new(1.0, 99.0);
        for i in 0..4_000 {
            gpu.push_history(i as f64);
        }
        assert_eq!(gpu.history().len(), 3_600);
        // 淘汰的是最旧的点。
        assert_eq!(gpu.history().front().copied(), Some(100.0));
    }

    #[test]
    fn rfc3339_formats_known_timestamps() {
        assert_eq!(rfc3339_utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(1_700_000_000, 274), "2023-11-14T22:13:20.274Z");
        // 闰日。
        assert_eq!(rfc3339_utc(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339_utc(951_868_800, 999), "2000-03-01T00:00:00.999Z");
    }

    #[test]
    fn process_cpu_first_sample_yields_no_percent() {
        let mut history = ProcessCpuHistory::default();
        let now = std::time::Instant::now();
        assert_eq!(history.cpu_percent(100, 10, 0, 5_000, 5_000, now), None);
    }

    #[test]
    fn process_cpu_percent_is_delta_over_elapsed() {
        let mut history = ProcessCpuHistory::default();
        let start = std::time::Instant::now();
        history.cpu_percent(100, 10, 0, 0, 0, start);
        // 2 秒内累计 1 秒 CPU 时间（纳秒计）→ 50%。
        let later = start + std::time::Duration::from_secs(2);
        let percent = history
            .cpu_percent(100, 10, 0, 600_000_000, 400_000_000, later)
            .unwrap();
        assert!((percent - 50.0).abs() < 1e-9, "got {percent}");
    }

    #[test]
    fn process_cpu_pid_reuse_starts_a_fresh_series() {
        let mut history = ProcessCpuHistory::default();
        let start = std::time::Instant::now();
        history.cpu_percent(100, 10, 0, 9_000_000_000, 0, start);
        // 同一 PID 但启动时间不同 = 新进程，绝不能沿用旧计数器算出天文数字。
        let later = start + std::time::Duration::from_secs(1);
        assert_eq!(history.cpu_percent(100, 77, 0, 1_000_000, 0, later), None);
    }

    #[test]
    fn process_cpu_counter_regression_yields_none_and_percent_is_clamped() {
        let mut history = ProcessCpuHistory::default();
        let start = std::time::Instant::now();
        history.cpu_percent(100, 10, 0, 5_000_000_000, 0, start);
        // 计数器倒退（理论上不该发生）→ 丢样本而不是负数。
        let later = start + std::time::Duration::from_secs(1);
        assert_eq!(
            history.cpu_percent(100, 10, 0, 1_000_000_000, 0, later),
            None
        );
        // 荒谬的暴涨被钳到 10000%。
        let mut clamped = ProcessCpuHistory::default();
        clamped.cpu_percent(200, 10, 0, 0, 0, start);
        let value = clamped
            .cpu_percent(200, 10, 0, u64::MAX / 2, 0, later)
            .unwrap();
        assert_eq!(value, 10_000.0);
    }

    #[test]
    fn process_cpu_entries_expire_after_two_minutes() {
        let mut history = ProcessCpuHistory::default();
        let start = std::time::Instant::now();
        history.cpu_percent(100, 10, 0, 1_000_000_000, 0, start);
        // 121 秒后为别的 PID 采样会触发淘汰……
        let much_later = start + std::time::Duration::from_secs(121);
        history.cpu_percent(300, 10, 0, 0, 0, much_later);
        // ……PID 100 的旧计数器已被清掉，再采样等同首个样本。
        let after = much_later + std::time::Duration::from_secs(1);
        assert_eq!(
            history.cpu_percent(100, 10, 0, 2_000_000_000, 0, after),
            None
        );
    }
}
