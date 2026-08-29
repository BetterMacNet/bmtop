//! 概览页：图标卡片仪表盘（mactop 紧凑布局风格）。
//!
//! 三行网格——CPU/GPU/内存、功耗/网络/磁盘、风扇/温度。标题即读数，
//! 进程信息不在这里（模式 2 专责）。窄屏退化为单列堆叠。

use crate::pages::{
    compact_ghz, fan_lines, sensor_group_lines, temp_gauge_line, thermal_level_label,
};
use crate::state::UiState;
use crate::widgets::*;
use bmtop_core::{Strings, SystemSnapshot};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 卡片走势图统一宽度。
const CARD_SPARKLINE_WIDTH: usize = 18;
/// 卡片内的紧凑百分比条：一栏 40 列，16 格放不下尾缀。
const CARD_GAUGE_WIDTH: usize = 11;

fn card_gauge(label: &str, value: Option<f64>, trailing: &str) -> Line<'static> {
    gauge_line_sized(label, value, trailing, CARD_GAUGE_WIDTH)
}

struct Card {
    title: String,
    secondary: String,
    lines: Vec<Line<'static>>,
}

pub(crate) fn render_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let text = state.strings();
    let cards = [
        cpu_card(text, state, snapshot),
        gpu_card(text, state, snapshot),
        memory_card(text, state, snapshot),
        power_card(text, state, snapshot),
        network_card(text, state, snapshot),
        disk_card(text, state, snapshot),
        fans_card(text, snapshot),
        temps_card(text, snapshot),
    ];

    if area.width < NARROW_TERMINAL_COLUMNS {
        // 窄屏单列堆叠，按内容高度手动排布：Layout 在放不下时会把每个
        // Length 一起压扁成空壳，这里要的是「前面的完整、后面的裁掉」。
        let mut y = area.y;
        for card in cards {
            let remaining = area.bottom().saturating_sub(y);
            if remaining < 3 {
                break;
            }
            let height = (card.lines.len() as u16 + 2).min(remaining);
            render_card(frame, Rect::new(area.x, y, area.width, height), card);
            y += height;
        }
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(8),
        ])
        .split(area);
    let mut cards = cards.into_iter();
    for (row, columns) in rows.iter().zip([3usize, 3, 2]) {
        let constraints = vec![Constraint::Ratio(1, columns as u32); columns];
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(*row);
        for cell in cells.iter() {
            if let Some(card) = cards.next() {
                render_card(frame, *cell, card);
            }
        }
    }
}

fn render_card(frame: &mut Frame<'_>, area: Rect, card: Card) {
    frame.render_widget(
        Paragraph::new(card.lines).block(titled_block(&card.title, &card.secondary)),
        area,
    );
}

/// ⚙ CPU：总计 + 集群 gauge + 功耗温度 + 走势；无 SoC 退回用户/系统/空闲。
fn cpu_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let cpu = &snapshot.cpu;
    let title = format!("⚙ {} {}", text.mode_cpu, percent(cpu.total_percent));
    let mut lines = vec![card_gauge(text.cpu_total, cpu.total_percent, "")];
    match snapshot.soc.as_ref().filter(|soc| !soc.clusters.is_empty()) {
        Some(soc) => {
            for cluster in &soc.clusters {
                let label = match cluster.name.as_str() {
                    "E" => text.cpu_cluster_e,
                    "P" => text.cpu_cluster_p,
                    _ => text.cpu_cluster_s,
                };
                lines.push(card_gauge(
                    label,
                    Some(cluster.active_percent),
                    &compact_ghz(cluster.freq_mhz),
                ));
            }
            let mut parts = Vec::new();
            if let Some(watts) = soc.power.cpu_watts {
                parts.push(format!("CPU {}", format_watts(watts)));
            }
            if let Some(celsius) = soc.temps.cpu_celsius {
                parts.push(format!("{} {}", text.label_temp, format_celsius(celsius)));
            }
            lines.push(text_line(text.label_power, parts.join(" · ")));
        }
        None => {
            lines.push(card_gauge(text.cpu_user, cpu.user_percent, ""));
            lines.push(card_gauge(text.cpu_system, cpu.system_percent, ""));
            lines.push(card_gauge(text.cpu_idle, cpu.idle_percent, ""));
        }
    }
    lines.push(text_line(
        text.gpu_trend,
        sparkline(state.cpu_history(), CARD_SPARKLINE_WIDTH, Some(100.0)),
    ));
    Card {
        title,
        secondary: format!("{} {}", text.load_prefix, format_load(&cpu.load_average)),
        lines,
    }
}

/// ◇ GPU：使用率 + 频率/功耗/温度 + 走势。
fn gpu_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let Some(gpu) = &snapshot.gpu else {
        return Card {
            title: format!("◇ {}", text.mode_gpu),
            secondary: String::new(),
            lines: vec![Line::from(text.gpu_unavailable)],
        };
    };
    let mut title = format!("◇ {} {:.1}%", text.mode_gpu, gpu.utilization_percent);
    let mut lines = vec![card_gauge(
        text.gpu_utilization,
        Some(gpu.utilization_percent),
        "",
    )];
    if let Some(soc) = &snapshot.soc {
        if let Some(mhz) = soc.gpu_freq_mhz {
            title = format!("{title} · {mhz:.0} MHz");
        }
        let mut parts = Vec::new();
        if let Some(watts) = soc.power.gpu_watts {
            parts.push(format_watts(watts));
        }
        if let Some(celsius) = soc.temps.gpu_celsius {
            parts.push(format_celsius(celsius));
        }
        if !parts.is_empty() {
            lines.push(text_line(text.label_power, parts.join(" · ")));
        }
    }
    if let Some(peak) = peak_line(text, snapshot) {
        lines.push(peak);
    }
    if let Some(fps) = fps_line(text, snapshot, state) {
        lines.push(fps);
    }
    lines.push(text_line(
        text.gpu_trend,
        sparkline(gpu.history(), CARD_SPARKLINE_WIDTH, Some(100.0)),
    ));
    Card {
        title,
        secondary: String::new(),
        lines,
    }
}

/// GPU 峰值频率 + 理论算力（频率表读不到就整行省略，不给编造值）。
pub(crate) fn peak_line(
    text: &'static Strings,
    snapshot: &SystemSnapshot,
) -> Option<Line<'static>> {
    let topology = snapshot.topology.as_ref()?;
    let max_mhz = topology.gpu_max_freq_mhz?;
    let tflops = bmtop_core::gpu_tflops_fp32(topology.gpu_cores.unwrap_or(0), max_mhz)?;
    Some(text_line(
        text.gpu_peak,
        text.gpu_peak_value
            .replace("{ghz}", &format!("{:.2}", f64::from(max_mhz) / 1000.0))
            .replace("{tflops}", &format!("{tflops:.1}")),
    ))
}

/// FPS 行：在跑显示读数；开启但未授权显示原因；关闭时不出现。
pub(crate) fn fps_line(
    text: &'static Strings,
    snapshot: &SystemSnapshot,
    state: &UiState,
) -> Option<Line<'static>> {
    if let Some(fps) = &snapshot.fps {
        return Some(text_line(
            text.label_fps,
            text.fps_value
                .replace("{fps}", &fps.fps.to_string())
                .replace("{interval}", &format!("{:.1}", fps.frame_interval_ms)),
        ));
    }
    if state.fps_enabled {
        let denied = snapshot
            .capabilities
            .iter()
            .any(|capability| capability == "fps:permission_denied");
        let hint = if denied {
            text.fps_permission_hint
        } else {
            text.loading
        };
        return Some(text_line(text.label_fps, hint.to_string()));
    }
    None
}

/// ▦ 内存：已用 + Swap + 压力 + 走势。
fn memory_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let memory = &snapshot.memory;
    let used = ratio_percent(memory.used_bytes, memory.total_bytes);
    let mut lines = vec![card_gauge(text.memory_used, used, "")];
    if memory.swap_total_bytes > 0 {
        lines.push(card_gauge(
            text.memory_swap,
            ratio_percent(memory.swap_used_bytes, memory.swap_total_bytes),
            &format_bytes(memory.swap_used_bytes),
        ));
    }
    lines.push(card_gauge(
        text.memory_pressure,
        memory.pressure_percent,
        "",
    ));
    if let Some((read, write)) = snapshot
        .soc
        .as_ref()
        .and_then(|soc| soc.dram_read_gbs.zip(soc.dram_write_gbs))
    {
        lines.push(text_line(
            text.memory_bandwidth,
            text.memory_bandwidth_value
                .replace("{read}", &format!("{read:.1}"))
                .replace("{write}", &format!("{write:.1}")),
        ));
    }
    lines.push(text_line(
        text.gpu_trend,
        sparkline(state.memory_history(), CARD_SPARKLINE_WIDTH, Some(100.0)),
    ));
    Card {
        title: format!(
            "▦ {} {} / {}",
            text.mode_memory,
            format_bytes(memory.used_bytes),
            format_bytes(memory.total_bytes)
        ),
        secondary: String::new(),
        lines,
    }
}

/// ϟ 功耗：各域瓦数 + 走势；无 SoC 显示不可用。
fn power_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let power = snapshot
        .soc
        .as_ref()
        .map(|soc| &soc.power)
        .filter(|power| power.total_watts().is_some());
    let Some(power) = power else {
        return Card {
            title: format!("ϟ {}", text.label_power),
            secondary: String::new(),
            lines: vec![Line::from(text.unavailable)],
        };
    };
    let watts = |value: Option<f64>| value.map(format_watts).unwrap_or_else(|| "--".into());
    let soc = snapshot.soc.as_ref();
    let mut ane_line = format!(
        "ANE {} · DRAM {}",
        watts(power.ane_watts),
        watts(power.dram_watts)
    );
    // ANE 带宽只有 AMC 计数器可用的机器才有（本机 macOS 26.5 内核拒绝该组）。
    if let Some((read, write)) = soc.and_then(|soc| soc.ane_read_gbs.zip(soc.ane_write_gbs)) {
        ane_line = format!("{ane_line} · {:.1} GB/s", read + write);
    }
    let mut lines = vec![
        Line::from(format!(
            "CPU {} · GPU {}",
            watts(power.cpu_watts),
            watts(power.gpu_watts)
        )),
        Line::from(ane_line),
    ];
    if let Some(system) = power.system_watts {
        lines.push(text_line(text.label_system_power, format_watts(system)));
    }
    if let Some(battery) = &snapshot.battery {
        let state_label = if battery.charging {
            text.battery_charging
        } else if battery.on_ac {
            text.battery_ac
        } else {
            text.battery_on_battery
        };
        // 电量未知（容量键缺失）时是灰色空条 + "--"，不冒充 0%。
        lines.push(card_gauge(
            text.label_battery,
            battery.percent.map(f64::from),
            state_label,
        ));
    }
    lines.push(text_line(
        text.gpu_trend,
        sparkline(state.power_history(), CARD_SPARKLINE_WIDTH, None),
    ));
    Card {
        title: format!(
            "ϟ {} {}",
            text.label_power,
            text.card_power_total
                .replace("{total}", &watts(power.total_watts()))
        ),
        secondary: String::new(),
        lines,
    }
}

/// ⇅ 网络：收发走势 + 当前速率；副标题带衰减峰值。
fn network_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let lines = vec![
        text_line(
            text.network_down,
            format!(
                "{}  {:>10}",
                sparkline(state.receive_history(), CARD_SPARKLINE_WIDTH, None),
                rate(latest(state.receive_history()))
            ),
        ),
        text_line(
            text.network_up,
            format!(
                "{}  {:>10}",
                sparkline(state.send_history(), CARD_SPARKLINE_WIDTH, None),
                rate(latest(state.send_history()))
            ),
        ),
    ];
    // 卡宽 40 列放不下「链路 + 峰值」两段副标题：链路优先（峰值在网络专页有）。
    let (receive_peak, send_peak) = state.network_peaks();
    let secondary = match snapshot.link.as_ref().and_then(|link| link.best_label()) {
        Some(label) => label,
        None if receive_peak > 0.0 || send_peak > 0.0 => text
            .network_peak
            .replace("{down}", &rate(Some(receive_peak)))
            .replace("{up}", &rate(Some(send_peak))),
        None => String::new(),
    };
    Card {
        title: format!("⇅ {}", text.mode_network),
        secondary,
        lines,
    }
}

/// ▥ 磁盘：每卷一条 gauge（懒加载中显示 loading）。
fn disk_card(text: &'static Strings, state: &UiState, snapshot: &SystemSnapshot) -> Card {
    let lines: Vec<Line<'static>> = if state.disks.is_empty() {
        vec![Line::from(text.loading)]
    } else {
        state
            .disks
            .iter()
            .map(|volume| {
                card_gauge(
                    &truncate_columns(&volume.mountpoint, GAUGE_LABEL_COLUMNS - 1),
                    volume.used_percent,
                    "",
                )
            })
            .collect()
    };
    let secondary = if state.disks.is_empty() {
        String::new()
    } else {
        text.disk_volume_count
            .replace("{count}", &state.disks.len().to_string())
    };
    let mut lines = lines;
    if let Some(io) = &snapshot.disk_io {
        lines.push(text_line(
            "I/O",
            text.disk_io_value
                .replace("{read}", &rate_decimal(Some(io.read_bytes_per_second)))
                .replace("{write}", &rate_decimal(Some(io.write_bytes_per_second))),
        ));
    }
    Card {
        title: format!("▥ {}", text.mode_disk),
        secondary,
        lines,
    }
}

/// ⊙ 风扇：每风扇转速条 + 目标/范围；副标题带散热状态。
fn fans_card(text: &'static Strings, snapshot: &SystemSnapshot) -> Card {
    let soc = snapshot.soc.as_ref();
    let lines = match soc {
        Some(soc) if !soc.fans.is_empty() => fan_lines(text, &soc.fans),
        Some(_) => vec![Line::from("--")],
        None => vec![Line::from(text.unavailable)],
    };
    // 散热状态：`正常 (57.9°C)`，级别与 SoC 温度都来自快照。
    let secondary = soc
        .and_then(|soc| {
            soc.thermal_level
                .map(|level| (level, soc.temps.soc_celsius))
        })
        .map(|(level, celsius)| {
            let label = thermal_level_label(text, level);
            match celsius {
                Some(celsius) => format!("{label} ({})", format_celsius(celsius)),
                None => label.to_string(),
            }
        })
        .unwrap_or_default();
    Card {
        title: format!("⊙ {}", text.sensors_fans),
        secondary,
        lines,
    }
}

/// ◉ 温度：关键温度 gauge + 分组均值（高度不够自动裁剪）。
fn temps_card(text: &'static Strings, snapshot: &SystemSnapshot) -> Card {
    let lines = match &snapshot.soc {
        Some(soc) => {
            let mut lines = Vec::new();
            for (group, celsius) in [
                (text.sensor_group_cpu, soc.temps.cpu_celsius),
                (text.sensor_group_gpu, soc.temps.gpu_celsius),
            ] {
                if let Some(celsius) = celsius {
                    lines.push(temp_gauge_line(
                        &format!("{} {}", group, text.label_temp),
                        celsius,
                    ));
                }
            }
            lines.extend(sensor_group_lines(text, &soc.sensors));
            lines
        }
        None => vec![Line::from(text.unavailable)],
    };
    Card {
        title: format!("◉ {}", text.label_temp),
        secondary: String::new(),
        lines,
    }
}
