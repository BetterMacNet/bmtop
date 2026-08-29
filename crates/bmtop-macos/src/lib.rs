//! Native macOS collectors. Platform-specific `unsafe` code is isolated in the
//! C shim and exposed here through owned Rust values.

mod powermetrics;

pub use powermetrics::{parse_powermetrics_plist, sample_powermetrics, PowerMetricsSample};

use bmtop_core::{
    CpuMetrics, DiskVolume, GpuSnapshot, MemoryMetrics, NetworkInterfaceMetrics, ProcessCpuHistory,
    ProcessRow, RefreshInterval, SystemSnapshot, ThreadRow,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub mod link;

/// doctor 探测：磁盘 I/O 计数器是否可读。
#[cfg(target_os = "macos")]
pub fn disk_io_available() -> bool {
    ffi::disk_io_counters().is_some()
}

pub mod rdma;
pub mod soc;
pub mod thunderbolt;

#[cfg(target_os = "macos")]
mod ffi {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct CpuTicks {
        pub user: u64,
        pub system: u64,
        pub idle: u64,
        pub nice: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct MemoryRaw {
        pub total_bytes: u64,
        pub free_pages: u64,
        pub active_pages: u64,
        pub inactive_pages: u64,
        pub wired_pages: u64,
        pub compressed_pages: u64,
        pub purgeable_pages: u64,
        pub swapins: u64,
        pub swapouts: u64,
        pub swap_total_bytes: u64,
        pub swap_used_bytes: u64,
        pub page_size: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct ProcessRaw {
        pub pid: i32,
        pub parent_pid: i32,
        pub uid: u32,
        pub status: u32,
        pub thread_count: i32,
        pub running_threads: i32,
        pub resident_bytes: u64,
        pub virtual_bytes: u64,
        pub user_ticks: u64,
        pub system_ticks: u64,
        pub start_seconds: u64,
        pub start_microseconds: u64,
        pub name: [u8; 64],
        pub path: [u8; 1024],
    }

    impl Default for ProcessRaw {
        fn default() -> Self {
            Self {
                pid: 0,
                parent_pid: 0,
                uid: 0,
                status: 0,
                thread_count: 0,
                running_threads: 0,
                resident_bytes: 0,
                virtual_bytes: 0,
                user_ticks: 0,
                system_ticks: 0,
                start_seconds: 0,
                start_microseconds: 0,
                name: [0; 64],
                path: [0; 1024],
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct InterfaceRaw {
        pub name: [u8; 64],
        pub received_bytes: u64,
        pub sent_bytes: u64,
    }

    impl Default for InterfaceRaw {
        fn default() -> Self {
            Self {
                name: [0; 64],
                received_bytes: 0,
                sent_bytes: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct GpuRaw {
        pub utilization_percent: f64,
        pub idle_percent: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct ThreadRaw {
        pub thread_id: u64,
        pub run_state: i32,
        pub cpu_percent: f64,
        pub name: [u8; 64],
    }

    impl Default for ThreadRaw {
        fn default() -> Self {
            Self {
                thread_id: 0,
                run_state: 0,
                cpu_percent: 0.0,
                name: [0; 64],
            }
        }
    }

    unsafe extern "C" {
        pub fn bmtop_read_cpu_ticks(out: *mut CpuTicks) -> i32;
        pub fn bmtop_read_core_ticks(out: *mut CpuTicks, capacity: usize) -> usize;
        pub fn bmtop_read_memory(out: *mut MemoryRaw) -> i32;
        pub fn bmtop_read_processes(out: *mut ProcessRaw, capacity: usize) -> usize;
        pub fn bmtop_read_interfaces(out: *mut InterfaceRaw, capacity: usize) -> usize;
        pub fn bmtop_read_gpu(out: *mut GpuRaw) -> i32;
        pub fn bmtop_read_fd_count(pid: i32) -> i32;
        pub fn bmtop_read_process_io(
            pid: i32,
            disk_read_bytes: *mut u64,
            disk_written_bytes: *mut u64,
        ) -> i32;
        pub fn bmtop_read_threads(pid: i32, out: *mut ThreadRaw, capacity: usize) -> usize;
        pub fn bmtop_read_disk_io(
            read_bytes: *mut u64,
            write_bytes: *mut u64,
            read_ops: *mut u64,
            write_ops: *mut u64,
        ) -> i32;
        pub fn bmtop_read_gpu_process_times(out: *mut GpuTimeRaw, capacity: usize) -> usize;
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct GpuTimeRaw {
        pub pid: i32,
        pub gpu_time_ns: u64,
    }

    fn c_string(value: &[u8]) -> Option<String> {
        let bytes = value
            .iter()
            .take_while(|byte| **byte != 0)
            .copied()
            .collect::<Vec<_>>();
        (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn cpu() -> Option<CpuTicks> {
        let mut value = CpuTicks::default();
        (unsafe { bmtop_read_cpu_ticks(&mut value) } == 0).then_some(value)
    }

    pub fn cores() -> Vec<CpuTicks> {
        let count = unsafe { bmtop_read_core_ticks(std::ptr::null_mut(), 0) };
        if count == 0 {
            return Vec::new();
        }
        let mut values = vec![CpuTicks::default(); count.saturating_add(4)];
        let written = unsafe { bmtop_read_core_ticks(values.as_mut_ptr(), values.len()) };
        values.truncate(written.min(values.len()));
        values
    }

    pub fn file_descriptor_count(pid: i32) -> Option<u32> {
        let count = unsafe { bmtop_read_fd_count(pid) };
        (count >= 0).then_some(count as u32)
    }

    pub fn threads(pid: i32) -> Vec<ThreadRaw> {
        let count = unsafe { bmtop_read_threads(pid, std::ptr::null_mut(), 0) };
        if count == 0 {
            return Vec::new();
        }
        let mut values = vec![ThreadRaw::default(); count.saturating_add(8)];
        let written = unsafe { bmtop_read_threads(pid, values.as_mut_ptr(), values.len()) };
        values.truncate(written.min(values.len()));
        values
    }

    pub fn thread_name(value: &ThreadRaw) -> Option<String> {
        c_string(&value.name)
    }

    pub fn process_io(pid: i32) -> Option<(u64, u64)> {
        let mut read_bytes = 0u64;
        let mut written_bytes = 0u64;
        (unsafe { bmtop_read_process_io(pid, &mut read_bytes, &mut written_bytes) } == 0)
            .then_some((read_bytes, written_bytes))
    }

    pub fn memory() -> Option<MemoryRaw> {
        let mut value = MemoryRaw::default();
        (unsafe { bmtop_read_memory(&mut value) } == 0).then_some(value)
    }

    pub fn processes() -> Vec<ProcessRaw> {
        let count = unsafe { bmtop_read_processes(std::ptr::null_mut(), 0) };
        if count == 0 {
            return Vec::new();
        }
        let mut values = vec![ProcessRaw::default(); count.saturating_add(64)];
        let written = unsafe { bmtop_read_processes(values.as_mut_ptr(), values.len()) };
        values.truncate(written.min(values.len()));
        values
    }

    pub fn interfaces() -> Vec<InterfaceRaw> {
        let count = unsafe { bmtop_read_interfaces(std::ptr::null_mut(), 0) };
        let mut values = vec![InterfaceRaw::default(); count.saturating_add(8)];
        let written = unsafe { bmtop_read_interfaces(values.as_mut_ptr(), values.len()) };
        values.truncate(written.min(values.len()));
        values
    }

    pub fn gpu() -> Option<GpuRaw> {
        let mut value = GpuRaw::default();
        (unsafe { bmtop_read_gpu(&mut value) } == 0).then_some(value)
    }

    /// 系统级磁盘 I/O 累计计数（开机以来）：读字节/写字节/读次数/写次数。
    pub fn disk_io_counters() -> Option<[u64; 4]> {
        let mut counters = [0u64; 4];
        let rc = unsafe {
            bmtop_read_disk_io(
                &mut counters[0],
                &mut counters[1],
                &mut counters[2],
                &mut counters[3],
            )
        };
        (rc == 0).then_some(counters)
    }

    /// 每进程累计 GPU 时间（纳秒），上限 256 项。
    pub fn gpu_process_times() -> Vec<GpuTimeRaw> {
        let mut buffer = vec![GpuTimeRaw::default(); 256];
        let written = unsafe { bmtop_read_gpu_process_times(buffer.as_mut_ptr(), buffer.len()) };
        buffer.truncate(written.min(256));
        buffer
    }

    pub fn process_name(value: &ProcessRaw) -> String {
        c_string(&value.name).unwrap_or_else(|| format!("PID {}", value.pid))
    }

    pub fn process_path(value: &ProcessRaw) -> Option<String> {
        c_string(&value.path)
    }

    pub fn interface_name(value: &InterfaceRaw) -> Option<String> {
        c_string(&value.name)
    }
}

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("native collector is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("failed to execute {program}: {message}")]
    Command { program: String, message: String },
    #[error("failed to parse {kind}: {message}")]
    Parse { kind: String, message: String },
    #[error("refusing to signal protected or stale process")]
    UnsafeProcessTarget,
    #[error("administrator authorization was not granted")]
    AuthorizationDenied,
}

#[derive(Debug, Clone)]
pub struct CollectorConfig {
    pub include_system_processes: bool,
    pub show_sensitive: bool,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            include_system_processes: true,
            show_sensitive: false,
        }
    }
}

pub struct MacCollector {
    config: CollectorConfig,
    previous_cpu: Option<ffi::CpuTicks>,
    previous_cpu_at: Option<Instant>,
    previous_cores: Vec<ffi::CpuTicks>,
    previous_interfaces: HashMap<String, (u64, u64, Instant)>,
    process_history: ProcessCpuHistory,
    last_gpu: Option<GpuSnapshot>,
    /// uid → 用户名。以前每行每 uid fork 一次 `/usr/bin/id`，
    /// 1s 间隔 × 上千进程是真实的开销；uid 映射基本不变，缓存整个进程生命周期。
    user_names: HashMap<u32, String>,
    /// GPU 型号名只取一次（`system_profiler` 是子进程，硬件元数据不会变）。
    gpu_name: Option<Option<String>>,
    /// SoC 采集器：外层 None = 未尝试，内层 None = 初始化失败（Intel 等），不再重试。
    soc: Option<Option<soc::SocCollector>>,
    /// 上次磁盘 I/O 累计值（速率差分用）。
    previous_disk_io: Option<(Instant, [u64; 4])>,
    /// 上次每进程 GPU 累计时间（pid → ns）。
    previous_gpu_times: HashMap<i32, u64>,
    previous_gpu_at: Option<Instant>,
    /// 链路探测缓存（getifaddrs/ioctl/CoreWLAN 比 1s 采样贵，5s 刷一次）。
    link_cache: Option<(Instant, bmtop_core::LinkInfo)>,
    /// 屏幕 FPS 开关（TUI `f` 键）；开启但未授权时只在 capabilities 里说明。
    fps_enabled: bool,
    fps_running: bool,
    /// CPU/GPU 静态拓扑，进程生命周期内不变，只读一次。
    topology: Option<Option<bmtop_core::CpuTopology>>,
}

impl MacCollector {
    pub fn new(config: CollectorConfig) -> Self {
        Self {
            config,
            previous_cpu: None,
            previous_cpu_at: None,
            previous_cores: Vec::new(),
            previous_interfaces: HashMap::new(),
            process_history: ProcessCpuHistory::default(),
            last_gpu: None,
            user_names: HashMap::new(),
            gpu_name: None,
            soc: None,
            topology: None,
            previous_disk_io: None,
            previous_gpu_times: HashMap::new(),
            previous_gpu_at: None,
            link_cache: None,
            fps_enabled: false,
            fps_running: false,
        }
    }

    /// TUI `f` 键：开关屏幕 FPS 采集。实际启停在下一次 `snapshot` 里做
    /// （与 CGDisplayStream 的生命周期同线程）。
    pub fn set_fps_enabled(&mut self, enabled: bool) {
        self.fps_enabled = enabled;
    }

    /// `detail_pid`：为这个进程额外补齐 fd 数、磁盘 I/O、完整命令行。
    /// 这些查询单次便宜、乘以全部进程数就贵，所以只对选中的一个做。
    pub fn snapshot(&mut self, detail_pid: Option<i32>) -> Result<SystemSnapshot, CollectorError> {
        #[cfg(target_os = "macos")]
        {
            let now = Instant::now();
            let cpu = self.read_cpu(now);
            let memory = self.read_memory();
            let mut processes = self.read_processes(now);
            enrich_process_detail(&mut processes, detail_pid);
            let interfaces = self.read_interfaces(now);
            let gpu = if let Some(value) = ffi::gpu() {
                let name = self.gpu_name.get_or_insert_with(gpu_model_name).clone();
                let snapshot = self.last_gpu.get_or_insert_with(|| {
                    GpuSnapshot::new(value.utilization_percent, value.idle_percent)
                });
                snapshot.update(value.utilization_percent, value.idle_percent);
                snapshot.name = name;
                Some(snapshot.clone())
            } else {
                self.last_gpu = None;
                None
            };
            let mut capabilities = vec![
                "cpu".to_string(),
                "memory".to_string(),
                "network".to_string(),
            ];
            capabilities.push(if processes.is_empty() {
                "processes:permission_denied".to_string()
            } else {
                "processes".to_string()
            });
            if gpu.is_some() {
                capabilities.push("gpu".to_string());
            }
            let soc_collector = self.soc.get_or_insert_with(soc::SocCollector::new);
            let soc_metrics = soc_collector
                .as_mut()
                .and_then(|collector| collector.sample());
            capabilities.push(if soc_collector.is_some() {
                "soc".to_string()
            } else {
                "soc:unavailable".to_string()
            });
            let topology = self.topology.get_or_insert_with(soc::read_topology).clone();
            let disk_io = self.read_disk_io_rates(now);
            self.apply_gpu_percent(&mut processes, &soc_metrics, now);
            let link = self.read_link_cached(now);
            let fps = self.manage_fps(&mut capabilities);
            Ok(SystemSnapshot {
                captured_at: bmtop_core::rfc3339_now(),
                captured_at_display: local_clock(),
                cpu,
                memory,
                processes,
                interfaces,
                gpu,
                capabilities,
                uptime_seconds: uptime_seconds(),
                soc: soc_metrics,
                topology,
                battery: soc::read_battery(),
                disk_io,
                link,
                fps,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = detail_pid;
            Err(CollectorError::UnsupportedPlatform)
        }
    }

    /// 链路信息 5s 缓存。
    #[cfg(target_os = "macos")]
    fn read_link_cached(&mut self, now: Instant) -> Option<bmtop_core::LinkInfo> {
        const LINK_CACHE_SECONDS: u64 = 5;
        let stale = self.link_cache.as_ref().is_none_or(|(at, _)| {
            now.duration_since(*at) >= Duration::from_secs(LINK_CACHE_SECONDS)
        });
        if stale {
            self.link_cache = Some((now, link::read_link_info()));
        }
        self.link_cache.as_ref().map(|(_, info)| info.clone())
    }

    /// FPS 生命周期：开关变化时启停流；开启但未授权时用 capability 说明原因。
    #[cfg(target_os = "macos")]
    fn manage_fps(&mut self, capabilities: &mut Vec<String>) -> Option<bmtop_core::DisplayFps> {
        if self.fps_enabled && !self.fps_running {
            match link::fps_start() {
                link::FpsStart::Started => self.fps_running = true,
                link::FpsStart::PermissionDenied => {
                    capabilities.push("fps:permission_denied".to_string());
                    return None;
                }
                link::FpsStart::Unavailable => {
                    capabilities.push("fps:unavailable".to_string());
                    return None;
                }
            }
        } else if !self.fps_enabled && self.fps_running {
            link::fps_stop();
            self.fps_running = false;
        }
        if self.fps_running {
            capabilities.push("fps".to_string());
            return link::fps_read();
        }
        None
    }

    /// 磁盘 I/O 速率：累计计数差分。首个采样、计数回退（卸载卷）时为 None。
    #[cfg(target_os = "macos")]
    fn read_disk_io_rates(&mut self, now: Instant) -> Option<bmtop_core::DiskIoRates> {
        let counters = ffi::disk_io_counters()?;
        let previous = self.previous_disk_io.replace((now, counters));
        let (previous_at, previous_counters) = previous?;
        let elapsed = now.duration_since(previous_at).as_secs_f64();
        if elapsed <= 0.0 {
            return None;
        }
        // 卸载卷会让聚合值倒退，这一拍丢弃而不是算出天文数字。
        if counters
            .iter()
            .zip(&previous_counters)
            .any(|(cur, prev)| cur < prev)
        {
            return None;
        }
        let rate = |index: usize| (counters[index] - previous_counters[index]) as f64 / elapsed;
        Some(bmtop_core::DiskIoRates {
            read_bytes_per_second: rate(0),
            write_bytes_per_second: rate(1),
            read_ops_per_second: rate(2),
            write_ops_per_second: rate(3),
        })
    }

    /// 每进程 GPU%：AGX 累计 GPU 时间差分，按系统 GPU 活跃度归一。
    /// mactop 的同名逻辑少除了一次 10（ms/s → %），这里是修正后的版本。
    #[cfg(target_os = "macos")]
    fn apply_gpu_percent(
        &mut self,
        processes: &mut [bmtop_core::ProcessRow],
        soc_metrics: &Option<bmtop_core::SocMetrics>,
        now: Instant,
    ) {
        let current: HashMap<i32, u64> = ffi::gpu_process_times()
            .into_iter()
            .map(|entry| (entry.pid, entry.gpu_time_ns))
            .collect();
        let previous = std::mem::replace(&mut self.previous_gpu_times, current.clone());
        let previous_at = self.previous_gpu_at.replace(now);
        let Some(previous_at) = previous_at else {
            return; // 首个采样没有基线
        };
        let elapsed = now.duration_since(previous_at).as_secs_f64();
        if elapsed <= 0.0 || previous.is_empty() {
            return;
        }
        // pid → 每秒 GPU 毫秒数；新出现的 pid 跳一拍，只防倒退。
        let mut raw_percent_total = 0.0;
        let per_pid: HashMap<i32, f64> = current
            .iter()
            .filter_map(|(pid, cur)| {
                let prev = previous.get(pid)?;
                (cur >= prev).then(|| {
                    let gpu_ms = (cur - prev) as f64 / elapsed / 1e6;
                    let percent = gpu_ms / 10.0; // 1000 ms/s == 100%
                    raw_percent_total += percent;
                    (*pid, percent)
                })
            })
            .collect();
        // 按 IOReport 的系统 GPU 活跃度归一，让列合计与 GPU 卡一致。
        let system_percent = soc_metrics
            .as_ref()
            .and_then(|soc| soc.gpu_active_percent)
            .unwrap_or(0.0);
        let scale = if raw_percent_total > 0.01 && system_percent > 0.01 {
            system_percent / raw_percent_total
        } else {
            1.0
        };
        for process in processes.iter_mut() {
            if let Some(percent) = per_pid.get(&process.pid) {
                process.gpu_percent = Some((percent * scale).clamp(0.0, 100.0));
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn read_cpu(&mut self, now: Instant) -> CpuMetrics {
        let Some(current) = ffi::cpu() else {
            return CpuMetrics::default();
        };
        let mut result = self
            .previous_cpu
            .zip(self.previous_cpu_at)
            .and_then(|(old, old_at)| {
                let elapsed = now.duration_since(old_at).as_secs_f64();
                let user = current.user.saturating_sub(old.user);
                let system = current.system.saturating_sub(old.system);
                let nice = current.nice.saturating_sub(old.nice);
                let idle = current.idle.saturating_sub(old.idle);
                let total = user + system + nice + idle;
                (elapsed > 0.0 && total > 0).then(|| {
                    let percent = |ticks: u64| ticks as f64 / total as f64 * 100.0;
                    CpuMetrics {
                        total_percent: Some(percent(user + system + nice)),
                        user_percent: Some(percent(user + nice)),
                        system_percent: Some(percent(system)),
                        idle_percent: Some(percent(idle)),
                        load_average: load_average(),
                        per_core_percent: Vec::new(),
                    }
                })
            })
            .unwrap_or_else(|| CpuMetrics {
                load_average: load_average(),
                ..CpuMetrics::default()
            });
        self.previous_cpu = Some(current);
        self.previous_cpu_at = Some(now);
        let cores = ffi::cores();
        result.per_core_percent = per_core_percentages(&self.previous_cores, &cores);
        self.previous_cores = cores;
        result
    }

    #[cfg(target_os = "macos")]
    fn read_memory(&self) -> MemoryMetrics {
        let Some(raw) = ffi::memory() else {
            return MemoryMetrics::default();
        };
        let page = raw.page_size;
        let free = raw.free_pages.saturating_mul(page);
        let active = raw.active_pages.saturating_mul(page);
        let inactive = raw.inactive_pages.saturating_mul(page);
        let wired = raw.wired_pages.saturating_mul(page);
        let compressed = raw.compressed_pages.saturating_mul(page);
        let used = raw
            .total_bytes
            .saturating_sub(free)
            .saturating_sub(inactive);
        let pressure = (raw.total_bytes > 0)
            .then(|| (wired.saturating_add(compressed)) as f64 / raw.total_bytes as f64 * 100.0)
            .map(|value| value.min(100.0));
        MemoryMetrics {
            total_bytes: raw.total_bytes,
            used_bytes: used,
            free_bytes: free,
            active_bytes: active,
            inactive_bytes: inactive,
            wired_bytes: wired,
            compressed_bytes: compressed,
            purgeable_bytes: raw.purgeable_pages.saturating_mul(page),
            swapins: raw.swapins,
            swapouts: raw.swapouts,
            swap_total_bytes: raw.swap_total_bytes,
            swap_used_bytes: raw.swap_used_bytes,
            pressure_percent: pressure,
        }
    }

    #[cfg(target_os = "macos")]
    fn read_processes(&mut self, now: Instant) -> Vec<ProcessRow> {
        let current_uid = unsafe { libc::getuid() };
        let native = ffi::processes();
        let mut rows: Vec<ProcessRow> = native
            .into_iter()
            .filter_map(|value| {
                if !self.config.include_system_processes && value.uid != current_uid {
                    return None;
                }
                let cpu_percent = self.process_history.cpu_percent(
                    value.pid,
                    value.start_seconds,
                    value.start_microseconds,
                    value.user_ticks,
                    value.system_ticks,
                    now,
                );
                let user = user_name(value.uid, &mut self.user_names);
                Some(ProcessRow {
                    pid: value.pid,
                    parent_pid: value.parent_pid,
                    uid: value.uid,
                    user,
                    name: ffi::process_name(&value),
                    path: ffi::process_path(&value),
                    state: process_state(value.status, value.running_threads).to_string(),
                    resident_bytes: Some(value.resident_bytes),
                    virtual_bytes: Some(value.virtual_bytes),
                    thread_count: (value.thread_count >= 0).then_some(value.thread_count as u32),
                    file_descriptor_count: None,
                    cpu_percent,
                    gpu_percent: None,
                    cpu_time_seconds: Some(
                        (value.user_ticks.saturating_add(value.system_ticks)) as f64 / 1e9,
                    ),
                    start_time_seconds: value.start_seconds,
                    start_time_microseconds: value.start_microseconds,
                    disk_read_bytes: None,
                    disk_written_bytes: None,
                    arguments: None,
                    threads: None,
                })
            })
            .collect();
        if rows.is_empty() {
            rows = ps_fallback(
                self.config.include_system_processes,
                current_uid,
                &mut self.user_names,
            );
        }
        rows
    }

    #[cfg(target_os = "macos")]
    fn read_interfaces(&mut self, now: Instant) -> Vec<NetworkInterfaceMetrics> {
        ffi::interfaces()
            .into_iter()
            .filter_map(|value| {
                let name = ffi::interface_name(&value)?;
                let previous = self
                    .previous_interfaces
                    .insert(name.clone(), (value.received_bytes, value.sent_bytes, now));
                let rates = previous.and_then(|(old_in, old_out, old_at)| {
                    let seconds = now.duration_since(old_at).as_secs_f64();
                    Some((
                        counter_rate(old_in, value.received_bytes, seconds)?,
                        counter_rate(old_out, value.sent_bytes, seconds)?,
                    ))
                });
                Some(NetworkInterfaceMetrics {
                    name,
                    received_bytes: value.received_bytes,
                    sent_bytes: value.sent_bytes,
                    receive_bytes_per_second: rates.map(|rate| rate.0),
                    send_bytes_per_second: rates.map(|rate| rate.1),
                })
            })
            .collect()
    }
}

/// 累计计数器 → 速率。计数器倒退（接口重建 / 系统重置计数）或时间没有
/// 前进时返回 `None`，让这一拍显示 `--` 而不是负数或天文数字。
fn counter_rate(old: u64, new: u64, seconds: f64) -> Option<f64> {
    (seconds > 0.0 && new >= old).then(|| (new - old) as f64 / seconds)
}

/// 两轮每核 tick → 每核百分比。首个样本或核数变化（不可能但防御）时为空。
fn per_core_percentages(previous: &[ffi::CpuTicks], current: &[ffi::CpuTicks]) -> Vec<f64> {
    if previous.len() != current.len() || current.is_empty() {
        return Vec::new();
    }
    current
        .iter()
        .zip(previous)
        .map(|(new, old)| {
            let user = new.user.saturating_sub(old.user);
            let system = new.system.saturating_sub(old.system);
            let nice = new.nice.saturating_sub(old.nice);
            let idle = new.idle.saturating_sub(old.idle);
            let total = user + system + nice + idle;
            if total == 0 {
                0.0
            } else {
                (user + system + nice) as f64 / total as f64 * 100.0
            }
        })
        .collect()
}

/// 为选中的进程补齐详情字段（fd 数、磁盘 I/O、完整命令行）。
#[cfg(target_os = "macos")]
fn enrich_process_detail(rows: &mut [ProcessRow], detail_pid: Option<i32>) {
    let Some(pid) = detail_pid else { return };
    let Some(row) = rows.iter_mut().find(|row| row.pid == pid) else {
        return;
    };
    row.file_descriptor_count = ffi::file_descriptor_count(pid);
    if let Some((read_bytes, written_bytes)) = ffi::process_io(pid) {
        row.disk_read_bytes = Some(read_bytes);
        row.disk_written_bytes = Some(written_bytes);
    }
    row.arguments = process_arguments(pid);
    let mut threads: Vec<ThreadRow> = ffi::threads(pid)
        .iter()
        .map(|thread| ThreadRow {
            thread_id: thread.thread_id,
            name: ffi::thread_name(thread),
            state: thread_state(thread.run_state).to_string(),
            cpu_percent: thread.cpu_percent.clamp(0.0, 100.0),
        })
        .collect();
    threads.sort_by(|left, right| {
        right
            .cpu_percent
            .partial_cmp(&left.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    row.threads = (!threads.is_empty()).then_some(threads);
}

/// `pth_run_state`（thread_basic_info 的 TH_STATE_*）→ 可读状态。
fn thread_state(run_state: i32) -> &'static str {
    match run_state {
        1 => "run",
        2 => "stop",
        3 => "sleep",
        4 => "uwait",
        5 => "halt",
        _ => "other",
    }
}

#[cfg(not(target_os = "macos"))]
fn enrich_process_detail(_rows: &mut [ProcessRow], _detail_pid: Option<i32>) {}

#[cfg(target_os = "macos")]
fn ps_fallback(
    include_system_processes: bool,
    current_uid: u32,
    user_names: &mut HashMap<u32, String>,
) -> Vec<ProcessRow> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,uid=,state=,%cpu=,rss=,comm="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 7 {
                return None;
            }
            let pid = fields[0].parse::<i32>().ok()?;
            let parent_pid = fields[1].parse::<i32>().ok()?;
            let uid = fields[2].parse::<u32>().ok()?;
            if !include_system_processes && uid != current_uid {
                return None;
            }
            let cpu_percent = fields[4].parse::<f64>().ok();
            let resident_bytes = fields[5]
                .parse::<u64>()
                .ok()
                .map(|kb| kb.saturating_mul(1024));
            let name = fields[6..].join(" ");
            Some(ProcessRow {
                pid,
                parent_pid,
                uid,
                user: user_name(uid, user_names),
                name,
                path: None,
                state: fields[3].to_string(),
                resident_bytes,
                virtual_bytes: None,
                thread_count: None,
                file_descriptor_count: None,
                cpu_percent,
                gpu_percent: None,
                cpu_time_seconds: None,
                start_time_seconds: 0,
                start_time_microseconds: 0,
                disk_read_bytes: None,
                disk_written_bytes: None,
                arguments: None,
                threads: None,
            })
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn ps_fallback(
    _include_system_processes: bool,
    _current_uid: u32,
    _user_names: &mut HashMap<u32, String>,
) -> Vec<ProcessRow> {
    Vec::new()
}

/// 本地挂钟 `HH:MM:SS`。`localtime_r` 失败时退回 UTC 时刻，宁可差个时区
/// 也不要在标题栏留空。
fn local_clock() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let raw = seconds as libc::time_t;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    let resolved = unsafe { !libc::localtime_r(&raw, &mut parts).is_null() };
    if resolved {
        format!(
            "{:02}:{:02}:{:02}",
            parts.tm_hour, parts.tm_min, parts.tm_sec
        )
    } else {
        let day = seconds % 86_400;
        format!("{:02}:{:02}:{:02}", day / 3_600, day % 3_600 / 60, day % 60)
    }
}

#[cfg(target_os = "macos")]
fn load_average() -> Vec<f64> {
    let mut values = [0.0; 3];
    let count = unsafe { libc::getloadavg(values.as_mut_ptr(), 3) };
    values[..count.max(0) as usize].to_vec()
}

#[cfg(not(target_os = "macos"))]
fn load_average() -> Vec<f64> {
    Vec::new()
}

/// uid → 用户名，`getpwuid_r` + 缓存。查不到（已删除的用户等）就落回数字。
fn user_name(uid: u32, cache: &mut HashMap<u32, String>) -> String {
    cache
        .entry(uid)
        .or_insert_with(|| lookup_user_name(uid).unwrap_or_else(|| uid.to_string()))
        .clone()
}

#[cfg(target_os = "macos")]
fn lookup_user_name(uid: u32) -> Option<String> {
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = [0u8; 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || record.pw_name.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(record.pw_name) }
        .to_string_lossy()
        .into_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(target_os = "macos"))]
fn lookup_user_name(_uid: u32) -> Option<String> {
    None
}

/// 开机至今的秒数（`kern.boottime`）。
#[cfg(target_os = "macos")]
pub fn uptime_seconds() -> Option<u64> {
    let mut boot = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut size = std::mem::size_of::<libc::timeval>();
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            &mut boot as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 || boot.tv_sec <= 0 {
        return None;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    now.checked_sub(boot.tv_sec as u64)
}

#[cfg(not(target_os = "macos"))]
pub fn uptime_seconds() -> Option<u64> {
    None
}

/// GPU 型号名：`system_profiler SPDisplaysDataType` 的 `sppci_model`。
/// 子进程较慢，调用方（collector）只取一次并缓存。
#[cfg(target_os = "macos")]
fn gpu_model_name() -> Option<String> {
    let output = Command::new("/usr/sbin/system_profiler")
        .args(["-json", "SPDisplaysDataType"])
        .output()
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    value["SPDisplaysDataType"][0]["sppci_model"]
        .as_str()
        .map(str::to_string)
}

#[cfg(not(target_os = "macos"))]
fn gpu_model_name() -> Option<String> {
    None
}

/// 完整命令行（`KERN_PROCARGS2`）。受保护进程读不到时返回 `None`。
#[cfg(target_os = "macos")]
fn process_arguments(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let probed = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if probed != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size];
    let read = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return None;
    }
    buffer.truncate(size);
    parse_procargs2(&buffer)
}

#[cfg(not(target_os = "macos"))]
fn process_arguments(_pid: i32) -> Option<Vec<String>> {
    None
}

/// `KERN_PROCARGS2` 的内存布局：`argc(i32) | exec_path\0 | 若干填充 \0 |
/// argv[0]\0 argv[1]\0 …`。纯函数便于用 fixture 测试。
fn parse_procargs2(raw: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(raw.get(..4)?.try_into().ok()?);
    if argc <= 0 {
        return None;
    }
    let rest = raw.get(4..)?;
    let path_end = rest.iter().position(|byte| *byte == 0)?;
    let mut cursor = path_end;
    while rest.get(cursor) == Some(&0) {
        cursor += 1;
    }
    let mut arguments = Vec::with_capacity(argc as usize);
    let mut start = cursor;
    for index in cursor..rest.len() {
        if arguments.len() == argc as usize {
            break;
        }
        if rest[index] == 0 {
            arguments.push(String::from_utf8_lossy(&rest[start..index]).into_owned());
            start = index + 1;
        }
    }
    (!arguments.is_empty()).then_some(arguments)
}

/// `pbi_status` 只有 SIDL/SRUN/SSLEEP/SSTOP/SZOMB 五种，而 BSD 会把进程
/// 一直标成 SRUN，哪怕它所有线程都在睡。`ps` 的 R/S 实际取自线程状态，
/// 所以这里也用正在运行的线程数来区分 run 和 sleep。
fn process_state(status: u32, running_threads: i32) -> &'static str {
    match status {
        4 => "stop",
        5 => "zombie",
        1 => "idle",
        2 | 3 if running_threads > 0 => "run",
        2 | 3 => "sleep",
        _ => "other",
    }
}

#[derive(Debug, Deserialize)]
pub struct HardwareReport {
    #[serde(flatten)]
    pub sections: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NetworkConnection {
    pub pid: i32,
    pub command: Option<String>,
    pub protocol: Option<String>,
    pub endpoint: Option<String>,
    pub state: Option<String>,
}

pub fn network_connections() -> Result<Vec<NetworkConnection>, CollectorError> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/sbin/lsof")
            .args(["-nP", "-i", "-F0pcnPT"])
            .output()
            .map_err(|error| CollectorError::Command {
                program: "lsof".into(),
                message: error.to_string(),
            })?;
        if !output.status.success() && output.stdout.is_empty() {
            return Err(CollectorError::Command {
                program: "lsof".into(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let mut pid = None;
        let mut command = None;
        let mut protocol = None;
        let mut endpoint = None;
        let mut state = None;
        let mut rows = Vec::new();
        for field in String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|value| !value.is_empty())
        {
            let (tag, value) = field.split_at(1);
            match tag {
                "p" => {
                    if let Some(pid_value) = pid.take() {
                        rows.push(NetworkConnection {
                            pid: pid_value,
                            command: command.take(),
                            protocol: protocol.take(),
                            endpoint: endpoint.take(),
                            state: state.take(),
                        });
                    }
                    pid = value.parse().ok();
                }
                "c" => command = Some(value.to_string()),
                "P" => protocol = Some(value.to_string()),
                "n" => endpoint = Some(value.to_string()),
                "T" if value.starts_with("ST=") => {
                    state = Some(value.trim_start_matches("ST=").to_string())
                }
                _ => {}
            }
        }
        if let Some(pid_value) = pid {
            rows.push(NetworkConnection {
                pid: pid_value,
                command,
                protocol,
                endpoint,
                state,
            });
        }
        Ok(rows)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(CollectorError::UnsupportedPlatform)
    }
}

pub fn hardware_report(show_sensitive: bool) -> Result<HardwareReport, CollectorError> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/sbin/system_profiler")
            .args([
                "-json",
                "SPHardwareDataType",
                "SPDisplaysDataType",
                "SPMemoryDataType",
                "SPStorageDataType",
                "SPPowerDataType",
                "SPNetworkDataType",
                "SPUSBDataType",
                "SPThunderboltDataType",
                "SPBluetoothDataType",
                "SPAudioDataType",
                "SPCameraDataType",
            ])
            .output()
            .map_err(|error| CollectorError::Command {
                program: "system_profiler".into(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(CollectorError::Command {
                program: "system_profiler".into(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let mut report: HardwareReport =
            serde_json::from_slice(&output.stdout).map_err(|error| CollectorError::Parse {
                kind: "system_profiler".into(),
                message: error.to_string(),
            })?;
        if !show_sensitive {
            redact_json(&mut report.sections);
        }
        Ok(report)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = show_sensitive;
        Err(CollectorError::UnsupportedPlatform)
    }
}

/// 小于此容量的卷不上报（恢复分区、EFI、iSCPreboot 之类的噪音）。
const DISK_MIN_BYTES: u64 = 1 << 30;
const KIB: u64 = 1024;

/// 这些挂载点是 APFS 容器的内部卷，和根卷共用同一份容量，逐个列出没有意义。
fn is_internal_mountpoint(mountpoint: &str) -> bool {
    mountpoint == "/dev"
        || mountpoint.starts_with("/System/Volumes/")
        || mountpoint.starts_with("/private/")
}

/// `/dev/disk3s1s1` → `disk3`。同一物理设备只保留第一个（也就是最主要的）卷。
fn base_device_name(filesystem: &str) -> Option<String> {
    let name = filesystem.strip_prefix("/dev/")?;
    let digits = name.strip_prefix("disk")?;
    let index: String = digits.chars().take_while(char::is_ascii_digit).collect();
    (!index.is_empty()).then(|| format!("disk{index}"))
}

/// 解析一行 `df -kP` 输出。
///
/// 列是 `<文件系统> <总块数> <已用> <可用> <容量%> <挂载点>`，但文件系统名和
/// 挂载点都可能含空格（`map auto_home`、`/Volumes/My Disk`），所以不能按固定
/// 下标取字段。改为定位唯一以 `%` 结尾的容量列，再向两侧切。
fn parse_df_row(line: &str) -> Option<DiskVolume> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let capacity_index = fields.iter().position(|field| {
        field.ends_with('%') && field.trim_end_matches('%').parse::<u64>().is_ok()
    })?;
    if capacity_index < 4 || capacity_index + 1 >= fields.len() {
        return None;
    }
    let blocks: u64 = fields[capacity_index - 3].parse().ok()?;
    let _used: u64 = fields[capacity_index - 2].parse().ok()?;
    let available: u64 = fields[capacity_index - 1].parse().ok()?;
    Some(DiskVolume::new(
        fields[..capacity_index - 3].join(" "),
        fields[capacity_index + 1..].join(" "),
        blocks.saturating_mul(KIB),
        available.saturating_mul(KIB),
    ))
}

/// 排序：根卷在前，然后是用户可见的外接卷，其余按容量降序。
fn volume_rank(mountpoint: &str) -> u8 {
    match mountpoint {
        "/" => 0,
        value if value.starts_with("/Volumes/") => 1,
        _ => 2,
    }
}

/// 从 `mount` 输出里挑出带 `nobrowse` 标志的挂载点。
///
/// `nobrowse` 正是访达用来隐藏挂载卷的标志，Xcode 模拟器运行时镜像、
/// Time Machine 本地快照都带它。这些卷是只读镜像，容量按内容量身定做，
/// 「已用 97%」对它们没有意义，所以按访达的口径一并隐藏。
/// 注意不能改用 `read-only` 判断——密封的系统根卷 `/` 也是只读的。
fn parse_nobrowse_mounts(text: &str) -> std::collections::HashSet<String> {
    text.lines()
        .filter_map(|line| {
            // 格式：`<设备> on <挂载点> (<标志,...>)`，挂载点可能含空格。
            let (_, rest) = line.split_once(" on ")?;
            let (mountpoint, flags) = rest.rsplit_once(" (")?;
            flags
                .trim_end_matches(')')
                .split(',')
                .any(|flag| flag.trim() == "nobrowse")
                .then(|| mountpoint.to_string())
        })
        .collect()
}

pub fn parse_df_output(df_text: &str, mount_text: &str) -> Vec<DiskVolume> {
    let hidden = parse_nobrowse_mounts(mount_text);
    let mut seen_devices = std::collections::HashSet::new();
    let mut volumes: Vec<DiskVolume> = df_text
        .lines()
        .skip(1)
        .filter_map(parse_df_row)
        .filter(|volume| {
            volume.total_bytes >= DISK_MIN_BYTES
                && !is_internal_mountpoint(&volume.mountpoint)
                && !hidden.contains(&volume.mountpoint)
        })
        .filter(|volume| match base_device_name(&volume.filesystem) {
            // 非 /dev/ 设备是 devfs / map auto_home / 网络挂载，一律丢弃。
            None => false,
            Some(device) => seen_devices.insert(device),
        })
        .collect();
    volumes.sort_by(|left, right| {
        volume_rank(&left.mountpoint)
            .cmp(&volume_rank(&right.mountpoint))
            .then(right.total_bytes.cmp(&left.total_bytes))
    });
    volumes
}

pub fn disk_report() -> Result<Vec<DiskVolume>, CollectorError> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/bin/df")
            .args(["-kP"])
            .output()
            .map_err(|error| CollectorError::Command {
                program: "df".into(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(CollectorError::Command {
                program: "df".into(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        // `mount` 拿不到时按空输出处理：宁可多列几个卷，也不能让磁盘页整个失败。
        let mounts = Command::new("/sbin/mount")
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        let volumes = parse_df_output(&String::from_utf8_lossy(&output.stdout), &mounts);
        if volumes.is_empty() {
            return Err(CollectorError::Parse {
                kind: "df".into(),
                message: "no reportable volume".into(),
            });
        }
        Ok(volumes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(CollectorError::UnsupportedPlatform)
    }
}

pub fn sensor_report(show_sensitive: bool) -> Result<serde_json::Value, CollectorError> {
    let report = hardware_report(show_sensitive)?;
    Ok(report
        .sections
        .get("SPPowerDataType")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({})))
}

fn redact_json(value: &mut serde_json::Map<String, serde_json::Value>) {
    for (key, item) in value.iter_mut() {
        let lower_key = key.to_ascii_lowercase();
        // UDID 和 UUID 只差一个字母，`provisioning_UDID` 之前整串明文漏了出来。
        let sensitive = lower_key.contains("serial")
            || lower_key.contains("uuid")
            || lower_key.contains("udid")
            || lower_key.contains("address");
        if sensitive {
            *item = serde_json::Value::String("••••••".to_string());
        } else if let serde_json::Value::Object(map) = item {
            redact_json(map);
        } else if let serde_json::Value::Array(items) = item {
            for child in items {
                if let serde_json::Value::Object(map) = child {
                    redact_json(map);
                }
            }
        }
    }
}

pub fn run_fixed_command(program: &OsStr, args: &[&str]) -> Result<Vec<u8>, CollectorError> {
    let output =
        Command::new(program)
            .args(args)
            .output()
            .map_err(|error| CollectorError::Command {
                program: program.to_string_lossy().into_owned(),
                message: error.to_string(),
            })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(CollectorError::Command {
            program: program.to_string_lossy().into_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSignal {
    Terminate,
    Kill,
}

impl ProcessSignal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Terminate => "TERM",
            Self::Kill => "KILL",
        }
    }
}

#[cfg(target_os = "macos")]
pub fn authorize_sudo() -> Result<(), CollectorError> {
    let status = Command::new("/usr/bin/sudo")
        .args(["-v"])
        .status()
        .map_err(|error| CollectorError::Command {
            program: "sudo".into(),
            message: error.to_string(),
        })?;
    status
        .success()
        .then_some(())
        .ok_or(CollectorError::AuthorizationDenied)
}

#[cfg(not(target_os = "macos"))]
pub fn authorize_sudo() -> Result<(), CollectorError> {
    Err(CollectorError::UnsupportedPlatform)
}

#[cfg(target_os = "macos")]
pub fn run_powermetrics_once(interval: RefreshInterval) -> Result<Vec<u8>, CollectorError> {
    authorize_sudo()?;
    let interval_ms = interval.as_millis().to_string();
    let output = Command::new("/usr/bin/sudo")
        .args([
            "-n",
            "--",
            "/usr/bin/powermetrics",
            "-n",
            "1",
            "-i",
            &interval_ms,
            "--samplers",
            "gpu_power,thermal",
            "-f",
            "plist",
        ])
        .output()
        .map_err(|error| CollectorError::Command {
            program: "powermetrics".into(),
            message: error.to_string(),
        })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(CollectorError::Command {
            program: "powermetrics".into(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn run_powermetrics_once(_interval: RefreshInterval) -> Result<Vec<u8>, CollectorError> {
    Err(CollectorError::UnsupportedPlatform)
}

#[cfg(target_os = "macos")]
pub fn send_signal_if_identity(
    pid: i32,
    start_seconds: u64,
    start_microseconds: u64,
    signal: ProcessSignal,
) -> Result<(), CollectorError> {
    if pid <= 1 || pid == unsafe { libc::getpid() } {
        return Err(CollectorError::UnsafeProcessTarget);
    }
    let process = ffi::processes()
        .into_iter()
        .find(|process| process.pid == pid)
        .ok_or(CollectorError::UnsafeProcessTarget)?;
    if process.start_seconds != start_seconds || process.start_microseconds != start_microseconds {
        return Err(CollectorError::UnsafeProcessTarget);
    }
    let own_uid = unsafe { libc::getuid() } as u32;
    let pid_text = pid.to_string();
    let status = if process.uid == own_uid {
        Command::new("/bin/kill")
            .args(["-s", signal.as_str(), &pid_text])
            .status()
    } else {
        authorize_sudo()?;
        Command::new("/usr/bin/sudo")
            .args(["-n", "--", "/bin/kill", "-s", signal.as_str(), &pid_text])
            .status()
    }
    .map_err(|error| CollectorError::Command {
        program: "kill".into(),
        message: error.to_string(),
    })?;
    status
        .success()
        .then_some(())
        .ok_or(CollectorError::AuthorizationDenied)
}

#[cfg(not(target_os = "macos"))]
pub fn send_signal_if_identity(
    _pid: i32,
    _start_seconds: u64,
    _start_microseconds: u64,
    _signal: ProcessSignal,
) -> Result<(), CollectorError> {
    Err(CollectorError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取自真机 `df -kP`：APFS 容器的多个内部卷、伪文件系统、含空格的字段。
    const DF_FIXTURE: &str = "\
Filesystem     1024-blocks       Used  Available Capacity  Mounted on
/dev/disk3s1s1  7811085600   16688940 4116883116     1%    /
devfs                  242        242          0   100%    /dev
/dev/disk3s6    7811085600    1048596 4116883116     1%    /System/Volumes/VM
/dev/disk1s1        512000       5736     492284     2%    /System/Volumes/iSCPreboot
/dev/disk3s5    7811085600 3654617788 4116883116    48%    /System/Volumes/Data
map auto_home            0          0          0   100%    /System/Volumes/Data/home
/dev/disk17s1      1303844     927088     371024    72%    /Volumes/WorkBuddy 5.3.14-arm64
/dev/disk7s1      19380224   18845428     486292    98%    /Library/Developer/CoreSimulator/Volumes/iOS_22D8075
";

    /// 取自真机 `mount`：模拟器运行时镜像带 nobrowse，根卷和已挂载 DMG 不带。
    const MOUNT_FIXTURE: &str = "\
/dev/disk3s1s1 on / (apfs, sealed, local, read-only, journaled)
/dev/disk17s1 on /Volumes/WorkBuddy 5.3.14-arm64 (apfs, local, nodev, nosuid, read-only, journaled, noowners, quarantine, mounted by mac)
/dev/disk7s1 on /Library/Developer/CoreSimulator/Volumes/iOS_22D8075 (apfs, sealed, local, nodev, nosuid, read-only, journaled, noatime, nobrowse)
";

    #[test]
    fn df_parsing_keeps_one_volume_per_device_and_drops_pseudo_filesystems() {
        let volumes = parse_df_output(DF_FIXTURE, MOUNT_FIXTURE);
        let mounts: Vec<&str> = volumes.iter().map(|v| v.mountpoint.as_str()).collect();
        assert_eq!(
            mounts,
            vec!["/", "/Volumes/WorkBuddy 5.3.14-arm64"],
            "根卷优先，外接卷其次；devfs / map auto_home / APFS 内部卷 / nobrowse 卷全部丢弃"
        );
    }

    #[test]
    fn nobrowse_volumes_are_hidden_like_finder_hides_them() {
        let hidden = parse_nobrowse_mounts(MOUNT_FIXTURE);
        assert!(hidden.contains("/Library/Developer/CoreSimulator/Volumes/iOS_22D8075"));
        // 根卷同样是 read-only，但没有 nobrowse，必须保留。
        assert!(!hidden.contains("/"));
        assert!(!hidden.contains("/Volumes/WorkBuddy 5.3.14-arm64"));
        assert!(parse_nobrowse_mounts("").is_empty());
    }

    #[test]
    fn mount_unavailable_falls_back_to_showing_every_volume() {
        // `mount` 失败时传空串，磁盘页不能整个塌掉。
        let volumes = parse_df_output(DF_FIXTURE, "");
        assert_eq!(volumes.len(), 3);
    }

    #[test]
    fn df_parsing_reports_container_usage_for_the_root_volume() {
        let root = &parse_df_output(DF_FIXTURE, MOUNT_FIXTURE)[0];
        // df 自报 1%，但容器真实占用是 总量-可用 ≈ 47%。
        let percent = root.used_percent.unwrap();
        assert!((percent - 47.29).abs() < 0.01, "got {percent}");
        assert_eq!(root.total_bytes, 7_811_085_600 * 1024);
    }

    #[test]
    fn df_parsing_handles_spaces_in_both_variable_width_columns() {
        // 挂载点含空格
        let volume =
            parse_df_row("/dev/disk17s1  1303844  927088  371024  72%  /Volumes/My Big Disk")
                .expect("row should parse");
        assert_eq!(volume.mountpoint, "/Volumes/My Big Disk");
        assert_eq!(volume.filesystem, "/dev/disk17s1");
        // 文件系统名含空格
        let mapped = parse_df_row("map auto_home  0  0  0  100%  /System/Volumes/Data/home")
            .expect("row should parse");
        assert_eq!(mapped.filesystem, "map auto_home");
    }

    #[test]
    fn df_parsing_rejects_malformed_rows() {
        assert!(parse_df_row("").is_none());
        assert!(
            parse_df_row("Filesystem 1024-blocks Used Available Capacity Mounted on").is_none()
        );
        assert!(parse_df_row("/dev/disk1 100 50 50").is_none());
        assert!(parse_df_row("/dev/disk1 abc def ghi 50% /").is_none());
        assert_eq!(parse_df_output("only a header line\n", ""), Vec::new());
    }

    #[test]
    fn base_device_name_collapses_apfs_slices() {
        assert_eq!(base_device_name("/dev/disk3s1s1").as_deref(), Some("disk3"));
        assert_eq!(base_device_name("/dev/disk17s1").as_deref(), Some("disk17"));
        assert_eq!(base_device_name("devfs"), None);
        assert_eq!(base_device_name("map auto_home"), None);
        assert_eq!(base_device_name("//user@server/share"), None);
    }

    #[test]
    fn redaction_masks_serial_uuid_and_addresses() {
        let mut value = serde_json::json!({
            "device_serialNumber": "secret",
            "controller_address": "AA:BB:CC:DD:EE:FF",
            "platform_UUID": "0000-1111",
            "provisioning_UDID": "00006031-000C21960A23001C",
            "machine_model": "Mac15,9",
            "model": "Mac"
        });
        let object = value.as_object_mut().unwrap();
        redact_json(object);
        assert_eq!(value["device_serialNumber"], "••••••");
        assert_eq!(value["controller_address"], "••••••");
        assert_eq!(value["platform_UUID"], "••••••");
        // UDID 和 UUID 只差一个字母，以前整串明文漏了出来。
        assert_eq!(value["provisioning_UDID"], "••••••");
        assert_eq!(value["machine_model"], "Mac15,9");
        assert_eq!(value["model"], "Mac");
    }

    #[test]
    fn process_state_labels_match_bsd_values() {
        assert_eq!(process_state(4, 0), "stop");
        assert_eq!(process_state(5, 0), "zombie");
        assert_eq!(process_state(1, 0), "idle");
        assert_eq!(process_state(99, 0), "other");
    }

    #[test]
    fn running_threads_separate_run_from_sleep() {
        // BSD 把几乎所有进程标成 SRUN(2)，只有线程状态能区分死活。
        assert_eq!(process_state(2, 0), "sleep");
        assert_eq!(process_state(2, 1), "run");
        assert_eq!(process_state(3, 0), "sleep");
        assert_eq!(process_state(3, 2), "run");
    }

    #[test]
    fn thread_states_map_th_state_values() {
        assert_eq!(thread_state(1), "run");
        assert_eq!(thread_state(3), "sleep");
        assert_eq!(thread_state(4), "uwait");
        assert_eq!(thread_state(99), "other");
    }

    #[test]
    fn counter_rate_handles_growth_reset_and_frozen_clock() {
        assert_eq!(counter_rate(100, 300, 2.0), Some(100.0));
        // 计数器倒退（接口重建 / 计数重置）→ 丢样本。
        assert_eq!(counter_rate(300, 100, 2.0), None);
        // 时间没有前进 → 丢样本，不能除零。
        assert_eq!(counter_rate(100, 300, 0.0), None);
        assert_eq!(counter_rate(100, 100, 1.0), Some(0.0));
    }

    #[test]
    fn per_core_percentages_require_matching_samples() {
        let ticks = |user, idle| ffi::CpuTicks {
            user,
            system: 0,
            idle,
            nice: 0,
        };
        // 首个样本（无历史）→ 空。
        assert!(per_core_percentages(&[], &[ticks(1, 1)]).is_empty());
        // 核数变化 → 空，绝不错位配对。
        assert!(per_core_percentages(&[ticks(0, 0)], &[ticks(1, 1), ticks(1, 1)]).is_empty());
        let previous = [ticks(100, 100), ticks(0, 0)];
        let current = [ticks(150, 150), ticks(0, 100)];
        let percents = per_core_percentages(&previous, &current);
        assert_eq!(percents, vec![50.0, 0.0]);
    }

    #[test]
    fn procargs2_layout_is_parsed_into_argv() {
        // argc=3 | exec_path\0 | 填充\0 | argv...；argv 之后还有环境变量，不能读进来。
        let mut raw = 3i32.to_ne_bytes().to_vec();
        raw.extend_from_slice(b"/usr/bin/tool\0\0\0");
        raw.extend_from_slice(b"tool\0--flag\0value with space\0HOME=/Users/mac\0");
        assert_eq!(
            parse_procargs2(&raw),
            Some(vec![
                "tool".to_string(),
                "--flag".to_string(),
                "value with space".to_string()
            ])
        );
    }

    #[test]
    fn procargs2_rejects_truncated_or_empty_buffers() {
        assert_eq!(parse_procargs2(&[]), None);
        assert_eq!(parse_procargs2(&[1, 0]), None);
        assert_eq!(parse_procargs2(&0i32.to_ne_bytes()), None);
        // 只有 exec path、没有以 NUL 收尾的 argv。
        let mut raw = 1i32.to_ne_bytes().to_vec();
        raw.extend_from_slice(b"/bin/x");
        assert_eq!(parse_procargs2(&raw), None);
    }
}
