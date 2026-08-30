//! 「能耗影响」模型——对齐活动监视器的能耗列。
//!
//! macOS 把这套系数明文放在 `/usr/share/pmenergy/*.plist` 的 `energy_constants` 里，
//! 活动监视器就照它加权。这里只放纯模型（系数 + 差分公式），
//! 读盘取系数是平台相关的，留在 `bmtop-macos`。

use std::collections::HashMap;
use std::time::Instant;

/// 计数器保留时长，与 [`crate::ProcessCpuHistory`] 对齐。
const COUNTER_TTL_SECONDS: u64 = 120;
/// 与 CPU% 同款上限，挡住时钟回跳导致的荒唐数值。
const IMPACT_CEILING: f64 = 10_000.0;

/// QoS 分档数量，顺序与 `bmtop_sys.h` 的 `qos_ns[]` 严格一致。
pub const QOS_BUCKETS: usize = 7;

/// `energy_constants` 的加权系数。
///
/// [`Default`] 是 `/usr/share/pmenergy/default.plist` 的实测值——Apple Silicon 没有
/// `board-id`，必然落到这一份，所以它同时是兜底值和实际值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyCoefficients {
    pub cpu_time: f64,
    pub cpu_wakeups: f64,
    pub diskio_bytesread: f64,
    pub diskio_byteswritten: f64,
    pub qos_default: f64,
    pub qos_background: f64,
    pub qos_utility: f64,
    pub qos_legacy: f64,
    pub qos_user_initiated: f64,
    pub qos_user_interactive: f64,
}

impl Default for EnergyCoefficients {
    fn default() -> Self {
        Self {
            cpu_time: 1.0,
            cpu_wakeups: 0.0002,
            diskio_bytesread: 4.5e-10,
            diskio_byteswritten: 2.4e-10,
            qos_default: 1.0,
            qos_background: 0.8,
            qos_utility: 1.0,
            qos_legacy: 1.0,
            qos_user_initiated: 1.0,
            qos_user_interactive: 1.0,
        }
    }
}

impl EnergyCoefficients {
    /// 按 `qos_ns[]` 的下标顺序展开权重。
    ///
    /// plist 里没有 `kqos_maintenance`（只有六档），而 rusage 有七个桶；
    /// maintenance 语义上是最低优先级的后台工作，这里并到 background 的权重。
    fn qos_weights(&self) -> [f64; QOS_BUCKETS] {
        [
            self.qos_default,
            self.qos_background,
            self.qos_background,
            self.qos_utility,
            self.qos_legacy,
            self.qos_user_initiated,
            self.qos_user_interactive,
        ]
    }
}

/// 一次采样里某进程的累计计数（全部单调递增）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProcessEnergyCounters {
    /// `ri_cpu_time_qos_*`，纳秒。
    pub qos_nanoseconds: [u64; QOS_BUCKETS],
    /// user + system，纳秒；QoS 桶全为 0 时的兜底口径。
    pub cpu_nanoseconds: u64,
    /// `ri_pkg_idle_wkups`：把整个封装从空闲唤醒的次数，最贵的那种唤醒。
    pub idle_wakeups: u64,
    /// `ri_interrupt_wkups`。Apple Silicon 实测 `ri_pkg_idle_wkups` 恒为 0，
    /// 而这个计数器每秒上百，是那些机器上唯一能用的唤醒信号。
    pub interrupt_wakeups: u64,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
}

/// 能耗影响的差分器。
///
/// 刻意与 [`crate::ProcessCpuHistory`] 分开：合并要改掉那边一个 `pub` 方法的签名，
/// 重复的只有 key 与过期清理几行，不值当。
#[derive(Debug, Clone, Default)]
pub struct ProcessEnergyHistory {
    counters: HashMap<(i32, u64, u64), (ProcessEnergyCounters, Instant)>,
}

impl ProcessEnergyHistory {
    /// 首个样本、计数回退、零间隔一律返回 `None`（显示成 `-`，不冒充 0）。
    pub fn impact(
        &mut self,
        pid: i32,
        start_seconds: u64,
        start_microseconds: u64,
        current: ProcessEnergyCounters,
        coefficients: &EnergyCoefficients,
        now: Instant,
    ) -> Option<f64> {
        // key 带上启动时刻，PID 复用会自然另起一条序列。
        let key = (pid, start_seconds, start_microseconds);
        let previous = self.counters.insert(key, (current, now));
        self.counters.retain(|_, (_, captured)| {
            now.duration_since(*captured).as_secs() < COUNTER_TTL_SECONDS
        });
        let (old, captured) = previous?;
        let elapsed = now.duration_since(captured).as_secs_f64();
        if elapsed <= 0.0 {
            return None;
        }
        let energy = accumulated_energy(&old, &current, coefficients)?;
        Some((energy / elapsed * 100.0).min(IMPACT_CEILING))
    }
}

/// 两次采样之间的加权「能量」。任一计数回退（PID 复用 / 计数器重置）返回 `None`。
fn accumulated_energy(
    old: &ProcessEnergyCounters,
    current: &ProcessEnergyCounters,
    coefficients: &EnergyCoefficients,
) -> Option<f64> {
    // 优先用封装空闲唤醒；它恒为 0 的机器（Apple Silicon）退回中断唤醒。
    // 两者是包含关系，只在前者没读数时替补，不会重复计。
    let package_wakeups = delta(old.idle_wakeups, current.idle_wakeups)?;
    let wakeups = if package_wakeups > 0 {
        package_wakeups
    } else {
        delta(old.interrupt_wakeups, current.interrupt_wakeups)?
    };
    let read = delta(old.disk_read_bytes, current.disk_read_bytes)?;
    let written = delta(old.disk_written_bytes, current.disk_written_bytes)?;
    let cpu_nanoseconds = delta(old.cpu_nanoseconds, current.cpu_nanoseconds)?;

    let weights = coefficients.qos_weights();
    let mut qos_seconds = 0.0;
    for ((previous, current), weight) in old
        .qos_nanoseconds
        .iter()
        .zip(current.qos_nanoseconds.iter())
        .zip(weights)
    {
        qos_seconds += delta(*previous, *current)? as f64 / 1e9 * weight;
    }
    // 有些进程不上报 QoS 分档（桶恒为 0），退回 user+system 并按 default 权重计。
    if qos_seconds == 0.0 {
        qos_seconds = cpu_nanoseconds as f64 / 1e9 * coefficients.qos_default;
    }

    Some(
        coefficients.cpu_time * qos_seconds
            + coefficients.cpu_wakeups * wakeups as f64
            + coefficients.diskio_bytesread * read as f64
            + coefficients.diskio_byteswritten * written as f64,
    )
}

fn delta(old: u64, current: u64) -> Option<u64> {
    current.checked_sub(old)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn counters(cpu_nanoseconds: u64, idle_wakeups: u64) -> ProcessEnergyCounters {
        ProcessEnergyCounters {
            cpu_nanoseconds,
            idle_wakeups,
            ..ProcessEnergyCounters::default()
        }
    }

    #[test]
    fn first_sample_has_no_impact() {
        let mut history = ProcessEnergyHistory::default();
        let impact = history.impact(
            1,
            10,
            0,
            counters(1_000_000_000, 0),
            &EnergyCoefficients::default(),
            Instant::now(),
        );
        assert_eq!(impact, None);
    }

    #[test]
    fn full_core_second_scores_one_hundred_like_cpu_percent() {
        // kcpu_time=1 时公式退化成 100 × Δcpu / 间隔，也就是 %CPU。
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        history.impact(1, 10, 0, counters(0, 0), &coefficients, start);
        let impact = history
            .impact(
                1,
                10,
                0,
                counters(1_000_000_000, 0),
                &coefficients,
                start + Duration::from_secs(1),
            )
            .expect("second sample yields impact");
        assert!((impact - 100.0).abs() < 1e-6, "got {impact}");
    }

    #[test]
    fn idle_wakeups_add_weighted_points() {
        // 500 次/秒 × 0.0002 × 100 = 10 分。
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        history.impact(1, 10, 0, counters(0, 0), &coefficients, start);
        let impact = history
            .impact(
                1,
                10,
                0,
                counters(0, 500),
                &coefficients,
                start + Duration::from_secs(1),
            )
            .expect("second sample yields impact");
        assert!((impact - 10.0).abs() < 1e-6, "got {impact}");
    }

    #[test]
    fn qos_buckets_win_over_the_cpu_fallback() {
        // background 权重 0.8：整核一秒只记 80 分。
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        let mut after = ProcessEnergyCounters {
            cpu_nanoseconds: 1_000_000_000,
            ..ProcessEnergyCounters::default()
        };
        after.qos_nanoseconds[2] = 1_000_000_000;
        history.impact(
            1,
            10,
            0,
            ProcessEnergyCounters::default(),
            &coefficients,
            start,
        );
        let impact = history
            .impact(
                1,
                10,
                0,
                after,
                &coefficients,
                start + Duration::from_secs(1),
            )
            .expect("second sample yields impact");
        assert!((impact - 80.0).abs() < 1e-6, "got {impact}");
    }

    #[test]
    fn interrupt_wakeups_stand_in_when_package_wakeups_stay_flat() {
        // Apple Silicon 的实际情形：pkg_idle 恒 0，中断唤醒每秒上百。
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        let after = ProcessEnergyCounters {
            interrupt_wakeups: 500,
            ..ProcessEnergyCounters::default()
        };
        history.impact(
            1,
            10,
            0,
            ProcessEnergyCounters::default(),
            &coefficients,
            start,
        );
        let impact = history
            .impact(
                1,
                10,
                0,
                after,
                &coefficients,
                start + Duration::from_secs(1),
            )
            .expect("second sample yields impact");
        assert!((impact - 10.0).abs() < 1e-6, "got {impact}");
    }

    #[test]
    fn package_wakeups_win_over_interrupt_wakeups() {
        // 两个计数器都有值时只认封装唤醒，免得重复计（中断唤醒是它的超集）。
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        let after = ProcessEnergyCounters {
            idle_wakeups: 500,
            interrupt_wakeups: 900,
            ..ProcessEnergyCounters::default()
        };
        history.impact(
            1,
            10,
            0,
            ProcessEnergyCounters::default(),
            &coefficients,
            start,
        );
        let impact = history
            .impact(
                1,
                10,
                0,
                after,
                &coefficients,
                start + Duration::from_secs(1),
            )
            .expect("second sample yields impact");
        assert!((impact - 10.0).abs() < 1e-6, "got {impact}");
    }

    #[test]
    fn counter_regression_yields_none() {
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        history.impact(1, 10, 0, counters(5_000_000_000, 0), &coefficients, start);
        let impact = history.impact(
            1,
            10,
            0,
            counters(1_000_000_000, 0),
            &coefficients,
            start + Duration::from_secs(1),
        );
        assert_eq!(impact, None);
    }

    #[test]
    fn reused_pid_starts_a_fresh_series() {
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        history.impact(1, 10, 0, counters(9_000_000_000, 0), &coefficients, start);
        // 同 PID、不同启动时刻 = 另一个进程，不能拿旧计数差分。
        let impact = history.impact(
            1,
            77,
            0,
            counters(1_000_000, 0),
            &coefficients,
            start + Duration::from_secs(1),
        );
        assert_eq!(impact, None);
    }

    #[test]
    fn stale_entries_expire() {
        let mut history = ProcessEnergyHistory::default();
        let coefficients = EnergyCoefficients::default();
        let start = Instant::now();
        history.impact(1, 10, 0, counters(1_000_000_000, 0), &coefficients, start);
        let much_later = start + Duration::from_secs(COUNTER_TTL_SECONDS + 1);
        history.impact(2, 10, 0, counters(0, 0), &coefficients, much_later);
        assert!(!history.counters.contains_key(&(1, 10, 0)));
    }
}
