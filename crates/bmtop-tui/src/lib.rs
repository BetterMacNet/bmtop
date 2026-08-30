//! Ratatui application and interaction state for bmtop.

mod overview;
mod pages;
mod render;
mod state;
mod widgets;

pub use render::render;
pub use state::{DetailSection, InputMode, ModeDetail, ProcessSignalKind, SortKey, UiState};

use anyhow::Result;
use bmtop_core::{AppMode, Language, RefreshInterval, SystemSnapshot};
use crossterm::event::{
    self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use std::io::IsTerminal;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 终端是否支持增强键盘协议。⌘1…⌘9 依赖它把 SUPER 修饰键上报给应用；
/// 默认 macOS Terminal 不支持，kitty / WezTerm / Ghostty 等支持。
pub fn keyboard_enhancement_supported() -> bool {
    use crossterm::terminal;
    if !std::io::stdout().is_terminal() {
        return false;
    }
    // 探测要跟终端来回对话，必须在 raw mode 下进行，用完恢复原状。
    let raw_was_on = terminal::is_raw_mode_enabled().unwrap_or(false);
    if !raw_was_on && terminal::enable_raw_mode().is_err() {
        return false;
    }
    let supported = terminal::supports_keyboard_enhancement().unwrap_or(false);
    if !raw_was_on {
        let _ = terminal::disable_raw_mode();
    }
    supported
}

fn push_keyboard_enhancement() -> bool {
    if !crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        return false;
    }
    crossterm::execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok()
}

fn pop_keyboard_enhancement() {
    let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
}

/// 发给采样线程的请求。`detail_pid` 是当前选中的进程，
/// 采集端只为它补齐 fd 数 / 磁盘 I/O / 完整命令行。
enum WorkerRequest {
    Sample {
        detail_pid: Option<i32>,
        fps_enabled: bool,
    },
    Detail(AppMode),
}

enum WorkerResponse {
    // Box 掉大快照，别让每个 channel 消息都按最大变体分配。
    Sample(std::result::Result<Box<SystemSnapshot>, String>),
    Detail(std::result::Result<ModeDetail, String>),
}

/// 采样与按模式详情都移到一个 worker 线程执行，主循环只收结果。
/// `system_profiler` / `lsof` 这类慢子进程从此不会冻结界面。
pub fn run_with_details<F, A, D>(
    interval: RefreshInterval,
    initial_mode: AppMode,
    language: Language,
    mut sample: F,
    mut details: D,
    mut action: A,
) -> Result<()>
where
    F: FnMut(Option<i32>, bool) -> std::result::Result<SystemSnapshot, String> + Send + 'static,
    A: FnMut(ProcessSignalKind, i32, u64, u64) -> std::result::Result<String, String>,
    D: FnMut(AppMode) -> std::result::Result<ModeDetail, String> + Send + 'static,
{
    let (request_sender, request_receiver) = mpsc::channel::<WorkerRequest>();
    let (response_sender, response_receiver) = mpsc::channel::<WorkerResponse>();
    // 不 join：退出时 request_sender 一断开，worker 自然走完退出；
    // 若它正卡在慢子进程上，也不该拖住终端恢复。
    std::thread::spawn(move || {
        for request in request_receiver {
            let response = match request {
                WorkerRequest::Sample {
                    detail_pid,
                    fps_enabled,
                } => WorkerResponse::Sample(sample(detail_pid, fps_enabled).map(Box::new)),
                WorkerRequest::Detail(mode) => WorkerResponse::Detail(details(mode)),
            };
            if response_sender.send(response).is_err() {
                break;
            }
        }
    });
    let mut terminal = ratatui::init();
    let keyboard_enhanced = push_keyboard_enhancement();
    let mut state = UiState::with_mode(initial_mode);
    state.interval_millis = interval.as_millis();
    state.set_language(language);
    let result = run_loop(
        &mut terminal,
        &mut state,
        interval,
        &request_sender,
        &response_receiver,
        &mut action,
        keyboard_enhanced,
    );
    if keyboard_enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();
    result
}

#[allow(clippy::too_many_arguments)]
fn run_loop<A>(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut UiState,
    interval: RefreshInterval,
    requests: &mpsc::Sender<WorkerRequest>,
    responses: &mpsc::Receiver<WorkerResponse>,
    action: &mut A,
    keyboard_enhanced: bool,
) -> Result<()>
where
    A: FnMut(ProcessSignalKind, i32, u64, u64) -> std::result::Result<String, String>,
{
    let mut refresh = Duration::from_millis(interval.as_millis());
    let mut next_sample = Instant::now();
    let mut loaded_detail_mode = None;
    // in-flight 标志防止 worker 变慢时请求越积越多。
    let mut sample_in_flight = false;
    let mut detail_in_flight = false;
    loop {
        // `+`/`-` 改了间隔就跟着走；调小时把已排定的下一拍拉近，
        // 否则从 60s 调回 1s 还要干等最后一个 60s 周期。
        let configured = Duration::from_millis(state.interval_millis);
        if configured != refresh {
            refresh = configured;
            next_sample = next_sample.min(Instant::now() + refresh);
        }
        if loaded_detail_mode != Some(state.mode) && !detail_in_flight {
            loaded_detail_mode = Some(state.mode);
            // 概览页也要磁盘数据，所以它同样触发一次懒加载。
            if matches!(
                state.mode,
                AppMode::Overview | AppMode::Disk | AppMode::Hardware | AppMode::Sensors
            ) && requests.send(WorkerRequest::Detail(state.mode)).is_ok()
            {
                detail_in_flight = true;
            }
        }
        if !state.paused && !sample_in_flight && Instant::now() >= next_sample {
            let detail_pid = state.selected_process_pid();
            let fps_enabled = state.fps_enabled;
            if requests
                .send(WorkerRequest::Sample {
                    detail_pid,
                    fps_enabled,
                })
                .is_ok()
            {
                sample_in_flight = true;
            }
            next_sample = Instant::now() + refresh;
        }
        while let Ok(response) = responses.try_recv() {
            match response {
                WorkerResponse::Sample(Ok(snapshot)) => {
                    state.set_snapshot(*snapshot);
                    sample_in_flight = false;
                }
                WorkerResponse::Sample(Err(error)) => {
                    state.status = state
                        .strings()
                        .status_sample_failed
                        .replace("{error}", &error);
                    sample_in_flight = false;
                }
                WorkerResponse::Detail(result) => {
                    let text = state.strings();
                    state.apply_detail(
                        result.map_err(|error| text.detail_load_failed.replace("{error}", &error)),
                    );
                    detail_in_flight = false;
                }
            }
        }
        terminal
            .draw(|frame| render(frame, state))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                // ^L：终端被外部输出弄花时强制全量重绘（差量渲染修不回来）。
                if key.code == crossterm::event::KeyCode::Char('l')
                    && key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL)
                {
                    terminal.clear()?;
                    continue;
                }
                if state.handle_key(key) {
                    break;
                }
                if state.take_refresh_request() {
                    next_sample = Instant::now();
                    loaded_detail_mode = None;
                }
                if let Some(pending) = state.take_completed_action() {
                    // 进程操作可能要走 sudo 的终端密码提示，先退出 TUI。
                    if keyboard_enhanced {
                        pop_keyboard_enhancement();
                    }
                    ratatui::restore();
                    let outcome = action(
                        pending.signal,
                        pending.pid,
                        pending.start_seconds,
                        pending.start_microseconds,
                    );
                    *terminal = ratatui::init();
                    if keyboard_enhanced {
                        push_keyboard_enhancement();
                    }
                    state.status = match outcome {
                        Ok(message) => message,
                        Err(error) => state
                            .strings()
                            .status_action_failed
                            .replace("{error}", &error),
                    };
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::parse_interval_input;
    use crate::widgets::*;
    use bmtop_core::{
        ClusterMetrics, CpuMetrics, CpuTopology, DiskVolume, Language, MemoryMetrics,
        NetworkInterfaceMetrics, SocMetrics, SocPower, SocTemps,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use unicode_width::UnicodeWidthStr;

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            captured_at: "1787888251.274Z".into(),
            captured_at_display: "14:32:08".into(),
            cpu: CpuMetrics::default(),
            memory: MemoryMetrics::default(),
            processes: Vec::new(),
            interfaces: vec![NetworkInterfaceMetrics {
                name: "en0".into(),
                received_bytes: 10,
                sent_bytes: 20,
                receive_bytes_per_second: None,
                send_bytes_per_second: None,
            }],
            gpu: None,
            capabilities: vec!["cpu".into()],
            uptime_seconds: Some(11 * 3_600),
            soc: None,
            topology: None,
            battery: None,
            disk_io: None,
            link: None,
            fps: None,
        }
    }

    fn process_snapshot() -> SystemSnapshot {
        let mut value = snapshot();
        value.processes.push(bmtop_core::ProcessRow {
            pid: 4242,
            parent_pid: 1,
            uid: 501,
            user: "mac".into(),
            name: "fixture".into(),
            path: None,
            state: "run".into(),
            resident_bytes: Some(1024),
            virtual_bytes: Some(4096),
            thread_count: Some(1),
            file_descriptor_count: None,
            cpu_percent: Some(1.0),
            gpu_percent: None,
            cpu_time_seconds: Some(12.0),
            energy_impact: Some(64.9),
            power_watts: Some(3.42),
            start_time_seconds: 10,
            start_time_microseconds: 20,
            disk_read_bytes: None,
            disk_written_bytes: None,
            arguments: None,
            threads: None,
        });
        value
    }

    fn bar_text(percent: Option<f64>) -> String {
        gauge(percent, 16).content.to_string()
    }

    #[test]
    fn gauge_fills_proportionally_and_clamps() {
        assert_eq!(bar_text(Some(0.0)), "\u{2591}".repeat(16));
        assert_eq!(bar_text(Some(100.0)), "\u{2588}".repeat(16));
        assert_eq!(bar_text(Some(50.0)).matches('\u{2588}').count(), 8);
        assert_eq!(bar_text(Some(-30.0)), "\u{2591}".repeat(16));
        assert_eq!(bar_text(Some(400.0)), "\u{2588}".repeat(16));
    }

    #[test]
    fn gauge_renders_unavailable_metrics_without_panicking() {
        assert_eq!(bar_text(None), "\u{2591}".repeat(16));
        assert_eq!(bar_text(Some(f64::NAN)), "\u{2591}".repeat(16));
        assert_eq!(bar_text(Some(f64::INFINITY)), "\u{2591}".repeat(16));
        assert_eq!(gauge(Some(50.0), 0).content.to_string(), "");
    }

    #[test]
    fn gauge_color_follows_mole_thresholds() {
        assert_eq!(gauge_color(59.9), Color::Green);
        assert_eq!(gauge_color(60.0), Color::Yellow);
        assert_eq!(gauge_color(84.9), Color::Yellow);
        assert_eq!(gauge_color(85.0), Color::Red);
    }

    #[test]
    fn pad_label_counts_display_columns_not_chars() {
        assert_eq!(pad_label("已用", 10).chars().count(), 2 + 6);
        assert_eq!(pad_label("CPU", 10), "CPU       ");
        assert_eq!(pad_label("超长的中文标签文本", 4), "超长的中文标签文本");
    }

    #[test]
    fn sparkline_pads_left_and_uses_fixed_scale_when_given() {
        assert_eq!(
            sparkline(&[100.0], 4, Some(100.0)),
            "\u{2581}\u{2581}\u{2581}\u{2588}"
        );
        assert_eq!(sparkline(&[], 3, None), "\u{2581}\u{2581}\u{2581}");
        assert_eq!(sparkline(&[50.0, 50.0], 2, Some(100.0)), "\u{2584}\u{2584}");
    }

    #[test]
    fn sparkline_auto_scales_to_window_peak_and_keeps_recent_points() {
        // 自动缩放：窗口峰值顶格，且只保留最近 width 个点。
        assert_eq!(
            sparkline(&[1.0, 2.0, 4.0], 3, None),
            "\u{2582}\u{2584}\u{2588}"
        );
        assert_eq!(
            sparkline(&[99.0, 1.0, 2.0, 4.0], 3, None),
            "\u{2582}\u{2584}\u{2588}"
        );
        // 全零窗口不得除零。
        assert_eq!(sparkline(&[0.0, 0.0], 2, None), "\u{2581}\u{2581}");
        // 非有限值按 0 处理。
        assert_eq!(sparkline(&[f64::NAN, 4.0], 2, None), "\u{2581}\u{2588}");
    }

    /// 测试一律显式钉住语言：`UiState::default()` 会读环境变量，
    /// 不钉住的话同一份断言在 `LANG=zh_CN` 和 `LANG=C` 下结果不同。
    fn chinese(mode: AppMode) -> UiState {
        let mut state = UiState::with_mode(mode);
        state.set_language(Language::Chinese);
        state
    }

    fn english(mode: AppMode) -> UiState {
        let mut state = UiState::with_mode(mode);
        state.set_language(Language::English);
        state
    }

    fn rows(state: &UiState, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        // 宽字符占两个单元，第二个单元是占位空格。按显示宽度跳过它，
        // 否则「进程」会被读成「进 程」，断言全都对不上。
        (0..height)
            .map(|y| {
                let mut row = String::new();
                let mut x = 0;
                while x < width {
                    let symbol = buffer[(x, y)].symbol();
                    row.push_str(symbol);
                    x += (UnicodeWidthStr::width(symbol).max(1)) as u16;
                }
                row
            })
            .collect()
    }

    fn screen(state: &UiState, width: u16, height: u16) -> String {
        rows(state, width, height).join("\n")
    }

    fn rich_snapshot() -> SystemSnapshot {
        let mut value = process_snapshot();
        value.cpu = CpuMetrics {
            total_percent: Some(42.0),
            user_percent: Some(30.0),
            system_percent: Some(12.0),
            idle_percent: Some(58.0),
            load_average: vec![2.11, 2.45, 2.30],
            per_core_percent: vec![80.0, 20.0],
        };
        value.memory = MemoryMetrics {
            total_bytes: 32 * 1024 * 1024 * 1024,
            used_bytes: 16 * 1024 * 1024 * 1024,
            free_bytes: 4 * 1024 * 1024 * 1024,
            inactive_bytes: 12 * 1024 * 1024 * 1024,
            active_bytes: 9 * 1024 * 1024 * 1024,
            wired_bytes: 4 * 1024 * 1024 * 1024,
            compressed_bytes: 1024 * 1024 * 1024,
            purgeable_bytes: 512 * 1024 * 1024,
            swapins: 120,
            swapouts: 8,
            swap_total_bytes: 4 * 1024 * 1024 * 1024,
            swap_used_bytes: 1024 * 1024 * 1024,
            pressure_percent: Some(18.0),
        };
        value.interfaces[0].receive_bytes_per_second = Some(12_000_000.0);
        value.interfaces[0].send_bytes_per_second = Some(800_000.0);
        value
    }

    /// Apple Silicon 快照：rich_snapshot 加 SoC 指标与拓扑（4 核 = 2E+2P）。
    fn soc_snapshot() -> SystemSnapshot {
        let mut value = rich_snapshot();
        value.cpu.per_core_percent = vec![12.0, 8.0, 61.0, 48.0];
        value.soc = Some(SocMetrics {
            clusters: vec![
                ClusterMetrics {
                    name: "E".into(),
                    active_percent: 23.4,
                    freq_mhz: 1250.0,
                },
                ClusterMetrics {
                    name: "P".into(),
                    active_percent: 44.2,
                    freq_mhz: 3980.0,
                },
            ],
            power: SocPower {
                cpu_watts: Some(4.8),
                gpu_watts: Some(2.1),
                ane_watts: Some(0.0),
                dram_watts: Some(1.3),
                system_watts: Some(37.5),
            },
            temps: SocTemps {
                cpu_celsius: Some(54.3),
                gpu_celsius: Some(48.1),
                soc_celsius: Some(61.0),
            },
            gpu_freq_mhz: Some(890.0),
            gpu_active_percent: Some(13.0),
            thermal_level: Some(0),
            fans: vec![bmtop_core::FanReading {
                name: "Fan 0".into(),
                actual_rpm: 1240,
                min_rpm: 990,
                max_rpm: 3900,
                target_rpm: 1240,
            }],
            dram_read_gbs: Some(8.1),
            dram_write_gbs: Some(4.3),
            ane_read_gbs: None,
            ane_write_gbs: None,
            sensors: vec![
                bmtop_core::SensorReading {
                    key: "Tp01".into(),
                    group: bmtop_core::sensor_group_for_key("Tp01").into(),
                    celsius: 55.2,
                },
                bmtop_core::SensorReading {
                    key: "Te01".into(),
                    group: bmtop_core::sensor_group_for_key("Te01").into(),
                    celsius: 48.0,
                },
                bmtop_core::SensorReading {
                    key: "TRD0".into(),
                    group: bmtop_core::sensor_group_for_key("TRD0").into(),
                    celsius: 60.5,
                },
            ],
        });
        value.topology = Some(CpuTopology {
            brand: "Apple M3 Max".into(),
            e_cores: 2,
            p_cores: 2,
            gpu_cores: Some(40),
            gpu_max_freq_mhz: Some(1380),
        });
        value.battery = Some(bmtop_core::BatteryInfo {
            percent: Some(93),
            charging: true,
            on_ac: true,
        });
        value.disk_io = Some(bmtop_core::DiskIoRates {
            read_bytes_per_second: 1_300_000.0,
            write_bytes_per_second: 308_000.0,
            read_ops_per_second: 12.0,
            write_ops_per_second: 40.0,
        });
        value.link = Some(bmtop_core::LinkInfo {
            ethernet: vec![],
            wifi: Some(bmtop_core::WifiLink {
                name: "en1".into(),
                generation: "Wi-Fi 6".into(),
                phy_mode: "802.11ax".into(),
                tx_rate_mbps: 866,
                is_connected: true,
            }),
        });
        value
    }

    #[test]
    fn cpu_page_shows_clusters_and_core_grid_with_soc() {
        let mut state = chinese(AppMode::Cpu);
        state.set_snapshot(soc_snapshot());
        let rendered = screen(&state, 120, 30);
        assert!(rendered.contains("E 集群"), "缺 E 集群行:\n{rendered}");
        assert!(rendered.contains("P 集群"), "缺 P 集群行:\n{rendered}");
        assert!(rendered.contains("1.25 GHz"), "缺 E 集群频率:\n{rendered}");
        assert!(rendered.contains("3.98 GHz"), "缺 P 集群频率:\n{rendered}");
        assert!(
            rendered.contains("Apple M3 Max (2E+2P · 40 GPU)"),
            "标题缺芯片拓扑:\n{rendered}"
        );
        assert!(rendered.contains("CPU 4.80W"), "缺功耗:\n{rendered}");
        assert!(rendered.contains("54.3°C"), "缺温度:\n{rendered}");
        assert!(rendered.contains("每核心"), "缺每核块:\n{rendered}");
        assert!(rendered.contains("E1"), "缺 E1 标签:\n{rendered}");
        assert!(rendered.contains("P2"), "缺 P2 标签:\n{rendered}");
        assert!(rendered.contains("用户"), "总计行缺明细:\n{rendered}");
        // 集群行存在时不应再渲染旧版 #0 标签
        assert!(!rendered.contains("#0"), "不应有旧版核标签:\n{rendered}");
    }

    #[test]
    fn cpu_page_falls_back_to_legacy_without_soc() {
        let mut state = chinese(AppMode::Cpu);
        state.set_snapshot(rich_snapshot());
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("总计"));
        assert!(
            rendered.contains("#0"),
            "Intel 路径应保留 #N 标签:\n{rendered}"
        );
        assert!(
            !rendered.contains("集群"),
            "无 soc 不应有集群行:\n{rendered}"
        );
    }

    #[test]
    fn cpu_core_grid_survives_narrow_terminal() {
        let mut state = chinese(AppMode::Cpu);
        state.set_snapshot(soc_snapshot());
        let rendered = screen(&state, 60, 24);
        assert!(rendered.contains("E1"), "窄屏应保留网格:\n{rendered}");
        assert!(rendered.contains("P2"), "窄屏应保留全部核:\n{rendered}");
    }

    #[test]
    fn overview_power_card_totals_and_title_chip() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(soc_snapshot());
        let rendered = screen(&state, 120, 30);
        assert!(rendered.contains("共 8.20W"), "功耗卡缺合计:\n{rendered}");
        assert!(rendered.contains("ANE 0.00W"), "功耗卡缺 ANE:\n{rendered}");
        assert!(rendered.contains("1.2GHz"), "集群行缺频率:\n{rendered}");
        assert!(rendered.contains("54.3°C"), "CPU 卡缺温度:\n{rendered}");
        // 标题栏芯片段
        assert!(
            rendered.contains("Apple M3 Max (2E+2P · 40 GPU)"),
            "标题栏缺芯片段:\n{rendered}"
        );
    }

    #[test]
    fn sensors_page_renders_soc_temps_groups_and_fans() {
        let mut state = chinese(AppMode::Sensors);
        state.set_snapshot(soc_snapshot());
        let rendered = screen(&state, 120, 30);
        assert!(
            rendered.contains("热压力 正常"),
            "标题缺热压力:\n{rendered}"
        );
        assert!(rendered.contains("CPU 温度"), "缺 CPU 温度行:\n{rendered}");
        assert!(rendered.contains("54.3°C"), "缺 CPU 温度值:\n{rendered}");
        assert!(rendered.contains("Fan 0"), "缺风扇行:\n{rendered}");
        assert!(
            rendered.contains("1240 / 3900 RPM"),
            "缺风扇读数:\n{rendered}"
        );
        assert!(
            rendered.contains("目标 1240 · 范围 990–3900"),
            "缺风扇目标行:\n{rendered}"
        );
        assert!(rendered.contains("风扇 1"), "副标题缺风扇数:\n{rendered}");
        assert!(rendered.contains("×1"), "缺分组计数:\n{rendered}");
    }

    #[test]
    fn sensors_page_explains_when_soc_missing() {
        let mut state = chinese(AppMode::Sensors);
        state.set_snapshot(rich_snapshot());
        let rendered = screen(&state, 120, 24);
        assert!(
            rendered.contains("SoC 传感器不可用"),
            "缺不可用提示:\n{rendered}"
        );
    }

    #[test]
    fn memory_page_uses_swap_gauge_and_secondary_title() {
        let mut state = chinese(AppMode::Memory);
        state.set_snapshot(rich_snapshot());
        let rendered = screen(&state, 120, 24);
        // swap 1G/4G = 25%，应有百分比而不是纯文本行
        assert!(rendered.contains("25.0%"), "swap 应为 gauge:\n{rendered}");
        assert!(rendered.contains("已用 50.0%"), "副标题缺占用:\n{rendered}");
    }

    #[test]
    fn gpu_page_shows_soc_freq_power_temp() {
        let mut snapshot = soc_snapshot();
        snapshot.gpu = Some(bmtop_core::GpuSnapshot::new(13.0, 87.0));
        let mut state = chinese(AppMode::Gpu);
        state.set_snapshot(snapshot);
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("890 MHz"), "缺频率:\n{rendered}");
        assert!(rendered.contains("2.10W"), "缺功耗:\n{rendered}");
        assert!(rendered.contains("48.1°C"), "缺温度:\n{rendered}");
    }

    #[test]
    fn network_page_secondary_carries_decaying_peak() {
        let mut state = chinese(AppMode::Network);
        state.set_snapshot(rich_snapshot());
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("峰值"), "副标题缺峰值:\n{rendered}");
        assert!(
            rendered.contains("11.4M/s"),
            "峰值应为当前速率:\n{rendered}"
        );
    }

    #[test]
    fn decaying_peak_ratchets_and_decays() {
        let peak = decaying_peak(0.0, 100.0);
        assert_eq!(peak, 100.0);
        let decayed = decaying_peak(peak, 10.0);
        assert_eq!(decayed, 98.0);
        assert_eq!(decaying_peak(decayed, 200.0), 200.0);
    }

    #[test]
    fn overview_cards_carry_extras() {
        let mut state = chinese(AppMode::Overview);
        let mut snapshot = soc_snapshot();
        snapshot.gpu = Some(bmtop_core::GpuSnapshot::new(13.0, 87.0));
        state.set_snapshot(snapshot);
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        let rendered = screen(&state, 120, 30);
        assert!(rendered.contains("系统"), "功耗卡缺系统功率:\n{rendered}");
        assert!(rendered.contains("37.5W"), "功耗卡缺 PSTR 值:\n{rendered}");
        assert!(rendered.contains("电池"), "功耗卡缺电池行:\n{rendered}");
        assert!(
            rendered.contains("93.0%") && rendered.contains("充电中"),
            "电池应为百分比条+状态:\n{rendered}"
        );
        assert!(
            rendered.contains("R 8.1 · W 4.3 GB/s"),
            "内存卡缺带宽:\n{rendered}"
        );
        assert!(
            rendered.contains("Wi-Fi 6 @ 866Mbps"),
            "网络卡缺链路:\n{rendered}"
        );
        assert!(rendered.contains("读 1.3M/s"), "磁盘卡缺 I/O:\n{rendered}");
        assert!(
            rendered.contains("1.38 GHz · 14.1 TFLOPS"),
            "GPU 卡缺峰值:\n{rendered}"
        );
    }

    #[test]
    fn fps_hint_appears_when_enabled_but_denied() {
        let mut state = chinese(AppMode::Gpu);
        state.fps_enabled = true;
        let mut snapshot = soc_snapshot();
        snapshot.gpu = Some(bmtop_core::GpuSnapshot::new(13.0, 87.0));
        snapshot.capabilities.push("fps:permission_denied".into());
        state.set_snapshot(snapshot);
        let rendered = screen(&state, 120, 24);
        assert!(
            rendered.contains("FPS 需要屏幕录制权限"),
            "缺权限提示:\n{rendered}"
        );
    }

    #[test]
    fn process_table_and_detail_show_gpu_and_vsz() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = rich_snapshot();
        snapshot.processes[0].gpu_percent = Some(12.5);
        state.set_snapshot(snapshot);
        let rendered = screen(&state, 130, 24);
        assert!(rendered.contains("GPU%"), "表头缺 GPU 列:\n{rendered}");
        assert!(rendered.contains("12.5"), "缺 GPU 值:\n{rendered}");
        assert!(rendered.contains("虚拟内存"), "详情缺虚拟内存:\n{rendered}");
        assert!(
            rendered.contains("CPU 时间"),
            "详情缺 CPU 时间:\n{rendered}"
        );
    }

    fn disks() -> Vec<DiskVolume> {
        vec![
            DiskVolume::new("/dev/disk3s1s1", "/", 8_000_000_000_000, 4_200_000_000_000),
            DiskVolume::new(
                "/dev/disk17s1",
                "/Volumes/External",
                2_000_000_000,
                500_000_000,
            ),
        ]
    }

    fn hardware_state() -> UiState {
        let mut state = chinese(AppMode::Hardware);
        state.set_snapshot(rich_snapshot());
        state.apply_detail(Ok(ModeDetail::Sections(vec![
            DetailSection::new("硬件概览", "型号  MacBook Pro\n芯片  Apple M3 Max"),
            DetailSection::new("显示与显卡", "分辨率  3456 x 2234"),
            DetailSection::new("蓝牙", "状态  开启"),
        ])));
        state
    }

    #[test]
    fn overview_renders_card_dashboard_without_processes() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(soc_snapshot());
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        let screen = screen(&state, 120, 30);

        // 八张卡片标题齐全（图标 + 文案）。
        for label in [
            "⚙ CPU",
            "◇ GPU",
            "▦ 内存",
            "ϟ 功耗",
            "⇅ 网络",
            "▥ 磁盘",
            "⊙ 风扇",
            "◉ 温度",
        ] {
            assert!(screen.contains(label), "概览缺少 {label} 卡:\n{screen}");
        }
        assert!(screen.contains("\u{2588}"), "概览应当有百分比条:\n{screen}");
        assert!(
            screen.contains("负载 2.11 2.45 2.30"),
            "CPU 卡副标题应显示负载:\n{screen}"
        );
        // 集群、风扇、温度组内容都在。
        assert!(screen.contains("E 集群"), "缺集群行:\n{screen}");
        assert!(screen.contains("1240 / 3900 RPM"), "缺风扇读数:\n{screen}");
        assert!(
            screen.contains("目标 1240 · 范围 990–3900"),
            "缺风扇目标:\n{screen}"
        );
        assert!(screen.contains("CPU P 核"), "缺温度分组:\n{screen}");
        // 进程表与详情栏已移除。
        assert!(!screen.contains("PID"), "概览不应再有进程表:\n{screen}");
        assert!(
            !screen.contains("父进程"),
            "概览不应再有进程详情:\n{screen}"
        );
    }

    #[test]
    fn overview_degrades_without_soc() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(rich_snapshot());
        let screen = screen(&state, 120, 30);
        // 功耗 / 风扇 / 温度卡显示不可用，CPU 卡退回用户/系统行。
        assert!(screen.contains("不可用"), "无 soc 应显示不可用:\n{screen}");
        assert!(screen.contains("用户"), "CPU 卡应退回用户行:\n{screen}");
        assert!(!screen.contains("集群"), "无 soc 不应有集群行:\n{screen}");
        // rich_snapshot 无电池：整行隐藏，不留空条。
        assert!(
            !screen.contains("电池"),
            "无电池机器不应有电池行:\n{screen}"
        );
    }

    #[test]
    fn overview_narrow_stacks_cards() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(soc_snapshot());
        let screen = screen(&state, 60, 24);
        assert!(screen.contains("⚙ CPU"), "窄屏应保留首卡:\n{screen}");
        assert!(screen.contains("总计"), "窄屏首卡应有内容:\n{screen}");
    }

    #[test]
    fn process_counts_are_grouped_and_include_threads() {
        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(rich_snapshot());
        assert!(screen(&state, 120, 24).contains("1 项 · 1 线程"));
        // 千分位分组本身单独验，渲染层只负责摆位置。
        assert_eq!(format_count(1_042), "1,042");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000_000), "1,000,000");
        assert_eq!(format_count(0), "0");
    }

    #[test]
    fn title_bar_shows_wall_clock_and_refresh_interval() {
        let mut state = chinese(AppMode::Overview);
        state.interval_millis = 1_000;
        state.set_snapshot(rich_snapshot());
        let rendered = screen(&state, 120, 24);
        assert!(
            rendered.contains("14:32:08 · 1.0s · 运行 11h 00m · LIVE"),
            "标题栏应当是 时间 · 间隔 · 运行时长 · 状态:\n{rendered}"
        );
        // 没有快照时不能显示上一次的时间。
        let empty = screen(&chinese(AppMode::Overview), 120, 24);
        assert!(
            empty.contains("--:--:--"),
            "无快照时不应显示旧时间:\n{empty}"
        );
    }

    #[test]
    fn interval_formatting_covers_sub_second_rates() {
        assert_eq!(format_interval(1_000), "1.0s");
        assert_eq!(format_interval(2_500), "2.5s");
        assert_eq!(format_interval(250), "250ms");
        assert_eq!(format_interval(60_000), "60.0s");
    }

    #[test]
    fn summary_rows_carry_trend_sparklines() {
        let mut state = chinese(AppMode::Overview);
        for _ in 0..4 {
            state.set_snapshot(rich_snapshot());
        }
        assert_eq!(state.cpu_history().len(), 4);
        assert_eq!(state.memory_history().len(), 4);
        assert_eq!(latest(state.cpu_history()), Some(42.0));
        assert_eq!(latest(state.memory_history()), Some(50.0));
        // 概览只放得下一条网络走势，用上下行之和。
        assert_eq!(latest(&state.network_history()), Some(12_800_000.0));
        let screen = screen(&state, 120, 24);
        assert!(
            screen.matches('\u{2588}').count() > 0 && screen.contains('\u{2581}'),
            "摘要行应当带走势图:\n{screen}"
        );
    }

    #[test]
    fn narrow_terminals_stack_instead_of_splitting_columns() {
        let state = hardware_state();
        // 宽屏：分区名在左栏，详情在右栏。
        let wide = screen(&state, 120, 20);
        assert!(wide.contains("硬件概览") && wide.contains("MacBook Pro"));

        // 窄屏：同一行里不能既有分区名又有详情，说明已经改成上下堆叠。
        let narrow = rows(&state, 70, 24);
        let side_by_side = narrow
            .iter()
            .any(|row| row.contains("硬件概览") && row.contains("MacBook Pro"));
        assert!(
            !side_by_side,
            "窄终端不应再左右分栏:\n{}",
            narrow.join("\n")
        );
        let joined = narrow.join("\n");
        assert!(
            joined.contains("硬件概览"),
            "窄终端仍要能看到分区列表:\n{joined}"
        );
        assert!(
            joined.contains("MacBook Pro"),
            "窄终端仍要能看到详情:\n{joined}"
        );
    }

    #[test]
    fn process_uptime_is_formatted_by_magnitude() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(format_uptime(now - 9 * 3_600 - 18 * 60), "9h 18m");
        assert_eq!(format_uptime(now - 3 * 86_400 - 7 * 3_600), "3d 07h");
        assert_eq!(format_uptime(now - 64), "1m 04s");
        // 起始时间缺失或时钟回拨都不能显示负数。
        assert_eq!(format_uptime(0), "--");
        assert_eq!(format_uptime(now + 600), "--");
    }

    #[test]
    fn help_overlay_pairs_keys_with_descriptions() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(rich_snapshot());
        state.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        let screen = screen(&state, 120, 24);
        assert!(screen.contains("快捷键"));
        assert!(screen.contains("切换模式") && screen.contains("退出并恢复终端"));
        // 双列：同一行里应当出现两组按键说明。
        // 21 条按键分两列，第 i 条与第 i+11 条同行：切换模式(0) 配 间隔步进(11)。
        let paired = screen
            .lines()
            .any(|line| line.contains("切换模式") && line.contains("±250ms"));
        assert!(paired, "帮助层应当是双列网格:\n{screen}");
    }

    #[test]
    fn sort_key_cycles_and_direction_reverses() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = rich_snapshot();
        snapshot.processes.push({
            let mut row = snapshot.processes[0].clone();
            row.pid = 100;
            row.cpu_percent = Some(90.0);
            row.resident_bytes = Some(1);
            row
        });
        state.set_snapshot(snapshot);
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        let pids = |state: &UiState| {
            state
                .filtered_processes()
                .iter()
                .map(|(_, process)| process.pid)
                .collect::<Vec<_>>()
        };
        // 默认 CPU 降序：90% 的在前。
        assert_eq!(pids(&state), vec![100, 4242]);
        // o → GPU 降序（都无 GPU 值时保持稳定序）。
        press(&mut state, KeyCode::Char('o'));
        assert_eq!(state.sort_key, SortKey::Gpu);
        // o → 能耗 → 功耗，再到内存（活动监视器的两列插在 GPU 之后）。
        press(&mut state, KeyCode::Char('o'));
        assert_eq!(state.sort_key, SortKey::Energy);
        press(&mut state, KeyCode::Char('o'));
        assert_eq!(state.sort_key, SortKey::Power);
        // o → 内存降序：1024 字节的在前（对齐 top 的 o=排序）。
        press(&mut state, KeyCode::Char('o'));
        assert_eq!(state.sort_key, SortKey::Memory);
        assert_eq!(pids(&state), vec![4242, 100]);
        // o → PID 降序，O 反转成升序。
        press(&mut state, KeyCode::Char('o'));
        assert_eq!(pids(&state), vec![4242, 100]);
        press(&mut state, KeyCode::Char('O'));
        assert_eq!(pids(&state), vec![100, 4242]);
        // 表头标注排序列与方向。
        press(&mut state, KeyCode::Char('O'));
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("PID↓"), "表头缺少排序标注:\n{rendered}");
    }

    #[test]
    fn hide_idle_keeps_wakeup_heavy_processes() {
        // 空闲唤醒多的后台进程 CPU 常年 0.0，正是能耗列要暴露的对象，
        // 按 `i` 隐藏空闲时不能把它们一起抹掉。
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = process_snapshot();
        snapshot.processes.push({
            let mut row = snapshot.processes[0].clone();
            row.pid = 1205;
            row.name = "Cursor Helper".into();
            row.cpu_percent = Some(0.0);
            row.energy_impact = Some(3.84);
            row
        });
        state.set_snapshot(snapshot);
        state.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        let pids: Vec<_> = state
            .filtered_processes()
            .iter()
            .map(|(_, process)| process.pid)
            .collect();
        assert!(pids.contains(&1205), "唤醒型进程不该被隐藏: {pids:?}");
    }

    #[test]
    fn energy_sort_keys_reach_both_new_columns() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = rich_snapshot();
        snapshot.processes.push({
            let mut row = snapshot.processes[0].clone();
            row.pid = 100;
            // CPU 低但能耗高（唤醒多），正好证明能耗不是 CPU% 的别名。
            row.cpu_percent = Some(0.5);
            row.energy_impact = Some(115.9);
            row.power_watts = Some(0.2);
            row
        });
        state.set_snapshot(snapshot);
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        let pids = |state: &UiState| {
            state
                .filtered_processes()
                .iter()
                .map(|(_, process)| process.pid)
                .collect::<Vec<_>>()
        };
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(state.sort_key, SortKey::Energy);
        assert_eq!(pids(&state), vec![100, 4242], "能耗降序应把 115.9 排在前");
        press(&mut state, KeyCode::Char('W'));
        assert_eq!(state.sort_key, SortKey::Power);
        assert_eq!(pids(&state), vec![4242, 100], "功耗降序应把 3.42W 排在前");
    }

    #[test]
    fn process_table_shows_energy_columns_only_when_wide() {
        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(process_snapshot());
        // 详情侧栏改成定宽 36 之后，120 列的常见终端也放得下这两列。
        for width in [140, 120] {
            let wide = screen(&state, width, 24);
            assert!(wide.contains("能耗"), "{width} 列应有能耗表头:\n{wide}");
            assert!(wide.contains("功耗"), "{width} 列应有功耗表头:\n{wide}");
            assert!(wide.contains("64.9"), "{width} 列应渲染能耗读数:\n{wide}");
            assert!(wide.contains("3.42W"), "{width} 列应渲染功耗读数:\n{wide}");
        }
        // 再窄下去命令列会被挤到认不出进程，两列让位。
        for width in [110, 100] {
            let narrow = screen(&state, width, 24);
            assert!(
                !narrow.contains("64.9"),
                "{width} 列不应渲染能耗列:\n{narrow}"
            );
            assert!(
                narrow.contains("fixture"),
                "{width} 列仍要看得到进程名:\n{narrow}"
            );
        }
        // 92 列以下没有详情侧栏，主表独占整屏，反而又放得下了。
        let full_width = screen(&state, 91, 24);
        assert!(
            full_width.contains("能耗"),
            "无侧栏时主表够宽，应有能耗列:\n{full_width}"
        );
    }

    #[test]
    fn overview_energy_card_lists_the_top_consumer() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(process_snapshot());
        let rendered = screen(&state, 140, 40);
        assert!(rendered.contains("能耗"), "概览缺少能耗卡:\n{rendered}");
        assert!(
            rendered.contains("fixture"),
            "能耗卡应列出进程名:\n{rendered}"
        );
        assert!(rendered.contains("64.9"), "能耗卡应列出读数:\n{rendered}");
    }

    #[test]
    fn user_filter_narrows_rows_and_esc_clears_it() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = rich_snapshot();
        snapshot.processes.push({
            let mut row = snapshot.processes[0].clone();
            row.pid = 7;
            row.user = "root".into();
            row
        });
        state.set_snapshot(snapshot);
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        press(&mut state, KeyCode::Char('u'));
        assert_eq!(state.input_mode, InputMode::UserFilter);
        for value in "root".chars() {
            press(&mut state, KeyCode::Char(value));
        }
        assert_eq!(state.filtered_processes().len(), 1);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.input_mode, InputMode::Normal);
        // 过滤生效时表头带用户标注。
        assert!(screen(&state, 120, 24).contains("用户 root"));
        // 再进过滤按 Esc = 清除过滤（对齐 top 的 U 空输入语义）。
        press(&mut state, KeyCode::Char('u'));
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.filtered_processes().len(), 2);
    }

    #[test]
    fn interval_keys_step_and_clamp() {
        let mut state = chinese(AppMode::Overview);
        state.interval_millis = 1_000;
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        press(&mut state, KeyCode::Char('+'));
        assert_eq!(state.interval_millis, 1_250);
        press(&mut state, KeyCode::Char('='));
        assert_eq!(state.interval_millis, 1_500);
        // 下限 250ms。
        state.interval_millis = 250;
        press(&mut state, KeyCode::Char('-'));
        assert_eq!(state.interval_millis, 250);
        // 上限 60s。
        state.interval_millis = 60_000;
        press(&mut state, KeyCode::Char('+'));
        assert_eq!(state.interval_millis, 60_000);
    }

    #[test]
    fn interval_prompt_captures_digits_instead_of_switching_modes() {
        let mut state = chinese(AppMode::Overview);
        state.interval_millis = 1_000;
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        // top 语义：s 进提示符，数字进输入缓冲而不是切到模式 2。
        press(&mut state, KeyCode::Char('s'));
        assert_eq!(state.input_mode, InputMode::Interval);
        press(&mut state, KeyCode::Char('2'));
        assert_eq!(state.mode, AppMode::Overview, "数字不得触发模式切换");
        assert_eq!(state.interval_input, "2");
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.interval_millis, 2_000, "裸数字按秒");
        // 支持 ms 后缀，且钳在 250ms 下限。
        press(&mut state, KeyCode::Char('s'));
        for value in "100ms".chars() {
            press(&mut state, KeyCode::Char(value));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.interval_millis, 250);
        // 空输入回车 = 保持现值（top 语义）；Esc = 取消。
        press(&mut state, KeyCode::Char('s'));
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.interval_millis, 250);
        press(&mut state, KeyCode::Char('s'));
        press(&mut state, KeyCode::Char('9'));
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.interval_millis, 250);
        assert_eq!(state.input_mode, InputMode::Normal);
        // 非法输入不改值，状态栏给出提示。
        press(&mut state, KeyCode::Char('s'));
        for value in "abc".chars() {
            press(&mut state, KeyCode::Char(value));
        }
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.interval_millis, 250);
        assert_eq!(state.status, "无效的间隔，示例：2 或 500ms");
        assert_eq!(parse_interval_input("2s"), Some(2_000));
        assert_eq!(parse_interval_input(" 500ms "), Some(500));
        assert_eq!(parse_interval_input("1.5"), None);
    }

    #[test]
    fn direct_sort_keys_and_r_reverse_follow_linux_top() {
        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(rich_snapshot());
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        press(&mut state, KeyCode::Char('M'));
        assert_eq!(state.sort_key, SortKey::Memory);
        press(&mut state, KeyCode::Char('N'));
        assert_eq!(state.sort_key, SortKey::Pid);
        press(&mut state, KeyCode::Char('P'));
        assert_eq!(state.sort_key, SortKey::Cpu);
        assert!(state.sort_descending);
        press(&mut state, KeyCode::Char('R'));
        assert!(!state.sort_descending);
    }

    #[test]
    fn k_and_d_are_top_style_aliases() {
        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(process_snapshot());
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        // k = 杀进程（与 x 同义，TERM + 确认），不再是向上移动。
        press(&mut state, KeyCode::Char('k'));
        assert_eq!(state.input_mode, InputMode::Action);
        assert_eq!(
            state.pending_action.as_ref().map(|action| action.signal),
            Some(ProcessSignalKind::Terminate)
        );
        press(&mut state, KeyCode::Esc);
        // d = 设间隔（与 s 同义，Linux top 双键位）。
        press(&mut state, KeyCode::Char('d'));
        assert_eq!(state.input_mode, InputMode::Interval);
    }

    #[test]
    fn hide_idle_keeps_unknown_cpu_rows_but_drops_zero_rows() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = process_snapshot();
        let mut idle = snapshot.processes[0].clone();
        idle.pid = 8;
        idle.cpu_percent = Some(0.0);
        // 「空闲」现在是 CPU 与能耗双零；只有 CPU 为 0 的唤醒型进程要留下，
        // 见 hide_idle_keeps_wakeup_heavy_processes。
        idle.energy_impact = Some(0.0);
        let mut unknown = snapshot.processes[0].clone();
        unknown.pid = 9;
        unknown.cpu_percent = None;
        snapshot.processes.extend([idle, unknown]);
        state.set_snapshot(snapshot);
        assert_eq!(state.filtered_processes().len(), 3);
        state.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        let pids: Vec<i32> = state
            .filtered_processes()
            .iter()
            .map(|(_, process)| process.pid)
            .collect();
        // CPU 恰为 0 的隐藏；首个样本 CPU 未知的保留，不能开着 i 就白屏。
        assert!(!pids.contains(&8));
        assert!(pids.contains(&9));
        assert!(screen(&state, 120, 24).contains("仅活跃"));
    }

    #[test]
    fn tree_view_orders_children_under_parents_with_depth() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = process_snapshot();
        let base = snapshot.processes[0].clone(); // pid 4242, ppid 1
        let mut child = base.clone();
        child.pid = 5000;
        child.parent_pid = 4242;
        child.cpu_percent = Some(99.0); // 排序上本应在最前
        let mut grandchild = base.clone();
        grandchild.pid = 5001;
        grandchild.parent_pid = 5000;
        grandchild.cpu_percent = Some(50.0);
        let mut self_parent = base.clone();
        self_parent.pid = 0;
        self_parent.parent_pid = 0; // kernel_task 自指，必须成为根而不是死循环
        self_parent.cpu_percent = Some(0.5);
        snapshot.processes.extend([child, grandchild, self_parent]);
        state.set_snapshot(snapshot);
        state.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE));
        let ordered: Vec<(usize, i32)> = state
            .filtered_processes()
            .iter()
            .map(|(depth, process)| (*depth, process.pid))
            .collect();
        // 父不可见（ppid 1 不在列表）→ 4242 是根；子孙按深度递增紧随其后。
        assert_eq!(
            ordered,
            vec![(0, 4242), (1, 5000), (2, 5001), (0, 0)],
            "树序应为父→子→孙，然后下一个根"
        );
        // 渲染层按深度缩进。
        let rendered = screen(&state, 120, 24);
        assert!(
            rendered.contains("└ fixture"),
            "子进程应有缩进:\n{rendered}"
        );
        // 关掉树后深度归零、恢复排序视角（99% 的子进程回到最前）。
        state.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE));
        let first = state.filtered_processes()[0];
        assert_eq!((first.0, first.1.pid), (0, 5000));
    }

    #[test]
    fn thread_view_lists_threads_of_the_selected_process() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = process_snapshot();
        snapshot.processes[0].threads = Some(vec![
            bmtop_core::ThreadRow {
                thread_id: 771,
                name: Some("RenderThread".into()),
                state: "run".into(),
                cpu_percent: 83.5,
            },
            bmtop_core::ThreadRow {
                thread_id: 772,
                name: None,
                state: "sleep".into(),
                cpu_percent: 0.0,
            },
        ]);
        state.set_snapshot(snapshot);
        state.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE));
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("RenderThread"), "缺线程名:\n{rendered}");
        assert!(rendered.contains("83.5%"), "缺线程 CPU:\n{rendered}");
        // 无名线程退回 TID 展示；标题带线程计数。
        assert!(rendered.contains("TID 772"));
        assert!(rendered.contains("线程 · 2"));
        // 再按 H 回到字段详情。
        state.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE));
        assert!(screen(&state, 120, 24).contains("父进程"));
    }

    #[test]
    fn c_toggles_full_command_path_in_the_table() {
        let mut state = chinese(AppMode::Processes);
        let mut snapshot = rich_snapshot();
        snapshot.processes[0].path = Some("/usr/local/bin/fixture".into());
        state.set_snapshot(snapshot);
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        // 右侧详情栏一直显示路径，所以只断言表格行（同时含 PID 和用户列的行）。
        let table_row = |state: &UiState| {
            screen(state, 140, 24)
                .lines()
                .find(|line| line.contains("4242") && line.contains("mac"))
                .expect("进程表应有 fixture 行")
                .to_string()
        };
        assert!(!table_row(&state).contains("/usr/local/bin/fixture"));
        press(&mut state, KeyCode::Char('c'));
        assert!(table_row(&state).contains("/usr/local/bin/fixture"));
        // 再按一次关掉。
        press(&mut state, KeyCode::Char('c'));
        assert!(!table_row(&state).contains("/usr/local/bin/fixture"));
    }

    #[test]
    fn overview_gpu_card_shows_unavailable_when_capability_missing() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(rich_snapshot());
        assert!(state.snapshot.as_ref().unwrap().gpu.is_none());
        let rendered = screen(&state, 110, 24);
        assert!(
            rendered.contains("GPU 不可用"),
            "GPU 卡应显示不可用而不是消失:\n{rendered}"
        );
    }

    #[test]
    fn memory_page_leads_with_percentages_not_raw_bytes() {
        let mut state = chinese(AppMode::Memory);
        state.set_snapshot(rich_snapshot());
        let screen = screen(&state, 110, 22);
        assert!(screen.contains("已用"));
        assert!(
            screen.contains("50.0%"),
            "16G/32G 应当显示 50.0%:\n{screen}"
        );
        assert!(screen.contains("压力"));
        assert!(
            screen.contains("\u{2588}"),
            "内存页应当有百分比条:\n{screen}"
        );
    }

    #[test]
    fn disk_page_shows_one_percentage_bar_per_volume() {
        let mut state = chinese(AppMode::Disk);
        state.set_snapshot(rich_snapshot());
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        let screen = screen(&state, 110, 12);
        assert!(screen.contains("47.5%"), "根卷百分比缺失:\n{screen}");
        assert!(screen.contains("75.0%"), "外接卷百分比缺失:\n{screen}");
        assert!(screen.contains("/Volumes/External"));
        // 不再是 JSON dump。
        assert!(
            !screen.contains("\"filesystem\""),
            "磁盘页不应再输出 JSON:\n{screen}"
        );
    }

    /// 磁盘容量按厂商/Finder 惯例用十进制单位：8_000_000_000_000 字节
    /// 就是 8.0T，而不是二进制的 7.3T。内存等其余字节值仍是二进制。
    #[test]
    fn disk_page_uses_decimal_units() {
        let mut state = chinese(AppMode::Disk);
        state.set_snapshot(rich_snapshot());
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        let screen = screen(&state, 110, 12);
        assert!(
            screen.contains("8.0T"),
            "根卷总量应为十进制 8.0T:\n{screen}"
        );
        assert!(
            screen.contains("2.0G"),
            "外接卷总量应为十进制 2.0G:\n{screen}"
        );
    }

    #[test]
    fn hardware_puts_the_section_list_in_the_left_thirty_percent() {
        let state = hardware_state();
        let rows = rows(&state, 100, 16);
        // 100 列宽下左栏是前 30 列。中文占两列，按显示列切而不是按字节切。
        let split_at = |row: &String, columns: usize| -> (String, String) {
            let mut used = 0;
            let mut left = String::new();
            let mut right = String::new();
            for character in row.chars() {
                let width = UnicodeWidthStr::width(character.to_string().as_str());
                if used + width <= columns {
                    left.push(character);
                } else {
                    right.push(character);
                }
                used += width;
            }
            (left, right)
        };
        let (left, right): (Vec<String>, Vec<String>) =
            rows.iter().map(|row| split_at(row, 30)).unzip();
        let left = left.join("\n");
        let right = right.join("\n");

        assert!(
            left.contains("硬件概览"),
            "分区名应当在左侧 3 分栏:\n{left}"
        );
        assert!(left.contains("蓝牙"), "左侧应列出全部分区:\n{left}");
        assert!(
            right.contains("MacBook Pro"),
            "选中分区的详情应当在右侧 7 分栏:\n{right}"
        );
        assert!(!left.contains("MacBook Pro"), "详情不该挤进左栏:\n{left}");
    }

    #[test]
    fn section_cursor_moves_independently_of_the_process_selection() {
        let mut state = hardware_state();
        assert_eq!(state.selected_section().unwrap().name, "硬件概览");

        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(state.selected_section().unwrap().name, "显示与显卡");
        assert_eq!(state.selected, 0, "进程选择不应被分区导航带动");

        state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(state.selected_section().unwrap().name, "蓝牙");
        // 到底后再按下键不得越界。
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(state.selected_section().unwrap().name, "蓝牙");

        state.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(state.section_selected, 0);
    }

    #[test]
    fn selected_section_drives_the_detail_pane() {
        let mut state = hardware_state();
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let screen = screen(&state, 100, 16);
        assert!(screen.contains("3456 x 2234"), "详情未跟随游标:\n{screen}");
        assert!(
            !screen.contains("MacBook Pro"),
            "旧分区详情未清掉:\n{screen}"
        );
    }

    #[test]
    fn page_keys_scroll_the_detail_pane_in_section_modes() {
        let mut state = hardware_state();
        state.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(state.detail_scroll, DETAIL_SCROLL_STEP);
        assert_eq!(state.selected, 0, "翻页不应移动进程选择");
        state.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(state.detail_scroll, 0);
        // 换分区时滚动量归零，否则新分区会从半截开始显示。
        state.detail_scroll = 30;
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(state.detail_scroll, 0);
    }

    #[test]
    fn switching_modes_resets_the_section_cursor() {
        let mut state = hardware_state();
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        state.detail_scroll = 20;
        assert_eq!(state.section_selected, 1);

        state.handle_key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        assert_eq!(state.mode, AppMode::Sensors);
        assert_eq!(state.section_selected, 0, "换模式后游标必须归零");
        assert_eq!(state.detail_scroll, 0);
    }

    #[test]
    fn shrinking_section_list_keeps_the_cursor_in_range() {
        let mut state = hardware_state();
        state.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(state.section_selected, 2);
        state.apply_detail(Ok(ModeDetail::Sections(vec![DetailSection::new(
            "只剩一个",
            "内容",
        )])));
        assert_eq!(state.section_selected, 0);
        assert!(state.selected_section().is_some());
    }

    #[test]
    fn detail_errors_do_not_wipe_previously_loaded_data() {
        let mut state = chinese(AppMode::Disk);
        state.set_snapshot(rich_snapshot());
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        state.apply_detail(Err("df 失败".into()));
        assert_eq!(state.disks.len(), 2, "一次失败不该清空已有磁盘数据");
        assert_eq!(state.detail_error.as_deref(), Some("df 失败"));
    }

    #[test]
    fn network_history_is_bounded_and_tracks_the_interface_total() {
        let mut state = chinese(AppMode::Overview);
        for _ in 0..(NETWORK_HISTORY_MAX + 40) {
            state.set_snapshot(rich_snapshot());
        }
        assert_eq!(state.receive_history().len(), NETWORK_HISTORY_MAX);
        assert_eq!(state.send_history().len(), NETWORK_HISTORY_MAX);
        assert_eq!(latest(state.receive_history()), Some(12_000_000.0));
        assert_eq!(latest(state.send_history()), Some(800_000.0));
    }

    #[test]
    fn network_page_shows_trend_and_per_interface_rates() {
        let mut state = chinese(AppMode::Network);
        state.set_snapshot(rich_snapshot());
        let screen = screen(&state, 110, 14);
        assert!(screen.contains("下行") && screen.contains("上行"));
        assert!(screen.contains("en0"), "缺少按接口的明细:\n{screen}");
        assert!(screen.contains("11.4M/s"), "缺少下行速率:\n{screen}");
    }

    #[test]
    fn process_page_keeps_the_table_and_detail_split() {
        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(rich_snapshot());
        let screen = screen(&state, 110, 24);
        assert!(screen.contains("PID"));
        assert!(screen.contains("fixture"));
        assert!(screen.contains("父进程"), "进程详情栏应当保留:\n{screen}");
        // 详情路径的新字段：未选中补齐时渲染为 --，不能缺行。
        assert!(screen.contains("磁盘读"), "缺少磁盘 I/O 行:\n{screen}");
        assert!(screen.contains("参数"), "缺少命令行参数行:\n{screen}");
    }

    #[test]
    fn english_mode_renders_every_page_without_chinese() {
        let has_chinese = |text: &str| text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
        for mode in [
            AppMode::Overview,
            AppMode::Processes,
            AppMode::Cpu,
            AppMode::Memory,
            AppMode::Network,
            AppMode::Disk,
        ] {
            let mut state = english(mode);
            state.set_snapshot(rich_snapshot());
            state.apply_detail(Ok(ModeDetail::Disks(disks())));
            let rendered = screen(&state, 120, 24);
            assert!(
                !has_chinese(&rendered),
                "{:?} 页在英文模式下仍有中文:\n{rendered}",
                mode
            );
        }
    }

    #[test]
    fn english_mode_uses_english_labels() {
        let mut state = english(AppMode::Overview);
        state.set_snapshot(rich_snapshot());
        state.apply_detail(Ok(ModeDetail::Disks(disks())));
        let rendered = screen(&state, 120, 30);
        assert!(rendered.contains("bmtop · Overview"));
        assert!(rendered.contains("⚙ CPU"));
        assert!(rendered.contains("Total"));
        assert!(rendered.contains("Load 2.11 2.45 2.30"));

        let mut processes = english(AppMode::Processes);
        processes.set_snapshot(rich_snapshot());
        let rendered = screen(&processes, 120, 24);
        assert!(rendered.contains("Processes · by CPU"));
        assert!(rendered.contains("procs") && rendered.contains("threads"));
        assert!(rendered.contains("MEM") && rendered.contains("COMMAND"));
        assert!(
            rendered.contains("Started"),
            "详情栏应有 Started:\n{rendered}"
        );
    }

    #[test]
    fn english_memory_and_network_pages_are_translated() {
        let mut memory = english(AppMode::Memory);
        memory.set_snapshot(rich_snapshot());
        let rendered = screen(&memory, 120, 22);
        for label in [
            "Used",
            "Available",
            "Pressure",
            "Compressed",
            "Inactive",
            "Purgeable",
        ] {
            assert!(rendered.contains(label), "内存页缺少 {label}:\n{rendered}");
        }

        let mut network = english(AppMode::Network);
        network.set_snapshot(rich_snapshot());
        let rendered = screen(&network, 120, 14);
        for label in ["↓ Down", "↑ Up", "Interface", "Total Rx", "Total Tx"] {
            assert!(rendered.contains(label), "网络页缺少 {label}:\n{rendered}");
        }
    }

    #[test]
    fn labels_never_run_into_their_values() {
        // 英文 `Compressed` 正好占满标签列，曾经渲染成 `Compressed4.8G`。
        assert_eq!(pad_field_label("Compressed", 10), "Compressed ");
        assert_eq!(pad_field_label("Capabilities", 10), "Capabilities ");
        assert_eq!(pad_field_label("Used", 10), "Used      ");
        for language in [Language::Chinese, Language::English] {
            let mut state = UiState::with_mode(AppMode::Memory);
            state.set_language(language);
            state.set_snapshot(rich_snapshot());
            let rendered = screen(&state, 120, 22);
            assert!(
                !rendered.contains("Compressed4") && !rendered.contains("压缩4"),
                "标签和数值黏在一起:\n{rendered}"
            );
        }
    }

    #[test]
    fn english_help_overlay_is_translated() {
        let mut state = english(AppMode::Overview);
        state.set_snapshot(rich_snapshot());
        state.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        let rendered = screen(&state, 120, 24);
        assert!(rendered.contains("Shortcuts") && rendered.contains("Switch mode"));
        assert!(rendered.contains("Quit and restore terminal"));
    }

    #[test]
    fn arrow_keys_cycle_modes_and_skip_the_missing_gpu_tab() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(snapshot()); // gpu: None
        let press = |state: &mut UiState, code| {
            state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };
        press(&mut state, KeyCode::Right);
        assert_eq!(state.mode, AppMode::Processes);
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        // 从概览向左回绕到最后一页。
        assert_eq!(state.mode, AppMode::Sensors);
        // 磁盘 → 右：GPU 不可用，跳到硬件；Tab 与 → 同义。
        state.mode = AppMode::Disk;
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.mode, AppMode::Hardware);
        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.mode, AppMode::Disk);
    }

    #[test]
    fn switching_language_also_refreshes_the_status_line() {
        let mut state = chinese(AppMode::Overview);
        assert!(screen(&state, 120, 10).contains("正在采样"));
        state.set_language(Language::English);
        assert!(
            screen(&state, 120, 10).contains("Sampling"),
            "未采样时状态栏未切换"
        );

        state.set_snapshot(rich_snapshot());
        assert!(screen(&state, 120, 10).contains("LIVE"));
        state.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(screen(&state, 120, 10).contains("PAUSED"));
        // 切回中文，暂停状态也要跟着换语言，不能残留 PAUSED。
        state.set_language(Language::Chinese);
        let rendered = screen(&state, 120, 10);
        assert!(
            rendered.contains("已暂停"),
            "切语言后状态栏残留旧语言:\n{rendered}"
        );
    }

    #[test]
    fn confirmation_prompt_follows_the_selected_language() {
        let mut state = english(AppMode::Processes);
        state.set_snapshot(rich_snapshot());
        state.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        let rendered = screen(&state, 120, 14);
        assert!(
            rendered.contains("Type PID 4242 to confirm force kill"),
            "强制结束确认未翻译:\n{rendered}"
        );

        let mut state = chinese(AppMode::Processes);
        state.set_snapshot(rich_snapshot());
        state.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(screen(&state, 120, 14).contains("输入 y 确认结束 PID 4242"));
    }

    #[test]
    fn refresh_request_is_a_flag_not_a_status_string_match() {
        // 原来靠比对状态栏文案判断是否要立即刷新，翻译后必然失效。
        let mut state = english(AppMode::Overview);
        assert!(!state.take_refresh_request());
        state.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(
            state.take_refresh_request(),
            "英文模式下 r 键应当仍能触发刷新"
        );
        assert!(!state.take_refresh_request(), "刷新标志应当只生效一次");
    }

    #[test]
    fn help_key_opens_and_escape_closes() {
        let mut state = chinese(AppMode::Overview);
        assert!(!state.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)));
        assert_eq!(state.input_mode, InputMode::Help);
        assert!(!state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn gpu_mode_is_ignored_when_capability_is_missing() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(snapshot());
        state.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        assert_eq!(state.mode, AppMode::Overview);
    }

    #[test]
    fn terminate_requires_explicit_confirmation() {
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(process_snapshot());
        assert!(!state.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert_eq!(state.input_mode, InputMode::Action);
        state.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(state.take_completed_action().is_some());
    }

    #[test]
    fn render_is_non_empty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = chinese(AppMode::Overview);
        state.set_snapshot(snapshot());
        terminal.draw(|frame| render(frame, &state)).unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(content.contains("bmtop"));
        assert!(!content.trim().is_empty());
    }
}
