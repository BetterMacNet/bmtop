//! 指标页渲染（概览摘要 / CPU / 内存 / 磁盘 / 网络 / GPU / 传感器）。
//! 框架层（标题栏、模式条、进程表、帮助层）在 render.rs。

use crate::state::UiState;
use crate::widgets::*;
use bmtop_core::{CpuTopology, Strings, SystemSnapshot};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// CPU 页。有 SoC 数据（Apple Silicon）时是「集群摘要 + 每核网格」双块；
/// 没有（Intel / IOReport 失败）时保持旧版单块平铺。
pub(crate) fn render_cpu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let text = state.strings();
    let soc = snapshot.soc.as_ref().filter(|soc| !soc.clusters.is_empty());
    let Some(soc) = soc else {
        render_cpu_legacy(frame, area, snapshot, text);
        return;
    };

    let cpu = &snapshot.cpu;
    let mut lines = Vec::new();
    for cluster in &soc.clusters {
        let label = match cluster.name.as_str() {
            "E" => text.cpu_cluster_e,
            "P" => text.cpu_cluster_p,
            _ => text.cpu_cluster_s,
        };
        lines.push(gauge_line(
            label,
            Some(cluster.active_percent),
            &format_ghz(cluster.freq_mhz),
        ));
    }
    lines.push(gauge_line(
        text.cpu_total,
        cpu.total_percent,
        &text
            .cpu_breakdown
            .replace("{user}", &percent(cpu.user_percent))
            .replace("{system}", &percent(cpu.system_percent))
            .replace("{idle}", &percent(cpu.idle_percent)),
    ));
    let mut power_parts = Vec::new();
    if let Some(watts) = soc.power.cpu_watts {
        power_parts.push(format!("CPU {}", format_watts(watts)));
    }
    if let Some(celsius) = soc.temps.cpu_celsius {
        power_parts.push(format!("{} {}", text.label_temp, format_celsius(celsius)));
    }
    lines.push(text_line(
        text.label_power,
        format!(
            "{}  {}",
            power_parts.join(" · "),
            sparkline(state.cpu_history(), GPU_SPARKLINE_WIDTH, Some(100.0))
        ),
    ));

    let top_height = lines.len() as u16 + 2;
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(top_height), Constraint::Min(3)])
        .split(area);

    let title = match &snapshot.topology {
        Some(topology) => format!("{} · {}", text.mode_cpu, topology_summary(topology)),
        None => text.mode_cpu.to_string(),
    };
    let mut secondary = format!("{} {}", text.load_prefix, format_load(&cpu.load_average));
    if let Some(uptime) = snapshot.uptime_seconds {
        secondary = format!(
            "{} · {}",
            secondary,
            text.title_uptime
                .replace("{uptime}", &format_duration(uptime))
        );
    }
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(&title, &secondary)),
        split[0],
    );

    let labels = core_labels(cpu.per_core_percent.len(), snapshot.topology.as_ref());
    let desired = if cpu.per_core_percent.len() > 16 {
        8
    } else {
        4
    };
    let columns =
        (usize::from(split[1].width.saturating_sub(2)) / CORE_CELL_COLUMNS).clamp(1, desired);
    frame.render_widget(
        Paragraph::new(core_grid(&cpu.per_core_percent, &labels, columns))
            .block(titled_block(text.cpu_per_core, "")),
        split[1],
    );
}

/// 旧版 CPU 页（Intel / 无 SoC 数据）。
fn render_cpu_legacy(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &SystemSnapshot,
    text: &'static Strings,
) {
    let cpu = &snapshot.cpu;
    let mut lines = vec![
        gauge_line(text.cpu_total, cpu.total_percent, ""),
        gauge_line(text.cpu_user, cpu.user_percent, ""),
        gauge_line(text.cpu_system, cpu.system_percent, ""),
        gauge_line(text.cpu_idle, cpu.idle_percent, ""),
        Line::from(""),
        text_line(text.load_prefix, format_load(&cpu.load_average)),
    ];
    if !cpu.per_core_percent.is_empty() {
        lines.push(Line::from(""));
        lines.extend(
            cpu.per_core_percent
                .iter()
                .enumerate()
                .map(|(index, percent)| gauge_line(&format!("#{index}"), Some(*percent), "")),
        );
    }
    frame.render_widget(panel(lines, text.mode_cpu), area);
}

/// `Apple M3 Max (4E+12P · 40 GPU)`，GPU 核数未知时省略。
pub(crate) fn topology_summary(topology: &CpuTopology) -> String {
    let cores = format!("{}E+{}P", topology.e_cores, topology.p_cores);
    match topology.gpu_cores {
        Some(gpu) => format!("{} ({cores} · {gpu} GPU)", topology.brand),
        None => format!("{} ({cores})", topology.brand),
    }
}

/// 每核网格一个单元的显示列宽：标签 4 + 条 8 + 空格 + 百分比 4 + 列距 3。
pub(crate) const CORE_CELL_COLUMNS: usize = 20;
const CORE_GAUGE_WIDTH: usize = 8;

/// 每核标签：拓扑核数与实际核数吻合时用 `E1..En P1..Pm`（Apple Silicon
/// 的逻辑核枚举顺序是 E 簇在前），否则退回 `#0..#N`。
pub(crate) fn core_labels(count: usize, topology: Option<&CpuTopology>) -> Vec<String> {
    if let Some(topology) = topology {
        let e_cores = topology.e_cores as usize;
        let p_cores = topology.p_cores as usize;
        if e_cores + p_cores == count && count > 0 {
            return (1..=e_cores)
                .map(|index| format!("E{index}"))
                .chain((1..=p_cores).map(|index| format!("P{index}")))
                .collect();
        }
    }
    (0..count).map(|index| format!("#{index}")).collect()
}

/// 每核使用率网格，行优先排列。标签与百分比按列对齐，条带颜色阈值。
pub(crate) fn core_grid(per_core: &[f64], labels: &[String], columns: usize) -> Vec<Line<'static>> {
    let columns = columns.max(1);
    per_core
        .chunks(columns)
        .enumerate()
        .map(|(row, chunk)| {
            let mut spans = Vec::new();
            for (offset, value) in chunk.iter().enumerate() {
                let index = row * columns + offset;
                let label = labels.get(index).map(String::as_str).unwrap_or("?");
                spans.push(Span::styled(
                    pad_label(label, 4),
                    Style::default().fg(Color::Gray),
                ));
                spans.push(gauge(Some(*value), CORE_GAUGE_WIDTH));
                spans.push(Span::raw(format!(" {:>3.0}%   ", value.clamp(0.0, 100.0))));
            }
            Line::from(spans)
        })
        .collect()
}

pub(crate) fn render_memory(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &SystemSnapshot,
    text: &'static Strings,
) {
    let memory = &snapshot.memory;
    let total = memory.total_bytes;
    // 采集端的 used = 总量 - 空闲 - 非活跃，所以「可用」正是空闲加非活跃。
    let available = memory.free_bytes.saturating_add(memory.inactive_bytes);
    let mut lines = vec![
        gauge_line(
            text.memory_used,
            ratio_percent(memory.used_bytes, total),
            &format!(
                "{} / {}",
                format_bytes(memory.used_bytes),
                format_bytes(total)
            ),
        ),
        gauge_line(
            text.memory_available,
            ratio_percent(available, total),
            &format_bytes(available),
        ),
        gauge_line(text.memory_pressure, memory.pressure_percent, ""),
        Line::from(""),
        text_line(text.memory_wired, format_bytes(memory.wired_bytes)),
        text_line(
            text.memory_compressed,
            format_bytes(memory.compressed_bytes),
        ),
        text_line(text.memory_active, format_bytes(memory.active_bytes)),
        text_line(text.memory_inactive, format_bytes(memory.inactive_bytes)),
        text_line(text.memory_free, format_bytes(memory.free_bytes)),
        text_line(text.memory_purgeable, format_bytes(memory.purgeable_bytes)),
        // swap 有总量时升级成百分比条；字节读不到（sysctl 失败）退回计数文本。
        if memory.swap_total_bytes > 0 {
            gauge_line(
                text.memory_swap,
                ratio_percent(memory.swap_used_bytes, memory.swap_total_bytes),
                &text
                    .memory_swap_bytes
                    .replace("{used}", &format_bytes(memory.swap_used_bytes))
                    .replace("{total}", &format_bytes(memory.swap_total_bytes))
                    .replace("{in}", &memory.swapins.to_string())
                    .replace("{out}", &memory.swapouts.to_string()),
            )
        } else {
            text_line(
                text.memory_swap,
                text.memory_swap_value
                    .replace("{in}", &memory.swapins.to_string())
                    .replace("{out}", &memory.swapouts.to_string()),
            )
        },
    ];
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
    // 副标题带实时占用，标题即读数。
    let secondary = format!(
        "{} {}",
        text.memory_used,
        percent(ratio_percent(memory.used_bytes, total))
    );
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(text.mode_memory, &secondary)),
        area,
    );
}

pub(crate) fn render_disk(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    if state.disks.is_empty() {
        let message = state
            .detail_error
            .clone()
            .unwrap_or_else(|| text.loading.to_string());
        frame.render_widget(panel(vec![Line::from(message)], text.mode_disk), area);
        return;
    }
    let lines: Vec<Line<'static>> = state
        .disks
        .iter()
        .map(|volume| {
            gauge_line(
                // 截到比栏宽少一列，保证挂载点和百分比条之间总有空格。
                &pad_label(
                    &truncate_columns(&volume.mountpoint, DISK_LABEL_COLUMNS - 1),
                    DISK_LABEL_COLUMNS,
                ),
                volume.used_percent,
                &text
                    .disk_usage
                    .replace("{used}", &format_bytes_decimal(volume.used_bytes))
                    .replace("{total}", &format_bytes_decimal(volume.total_bytes)),
            )
        })
        .collect();
    let mut secondary = text
        .disk_volume_count
        .replace("{count}", &state.disks.len().to_string());
    if let Some(io) = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.disk_io.as_ref())
    {
        secondary = format!(
            "{} · {}",
            text.disk_io_value
                .replace("{read}", &rate_decimal(Some(io.read_bytes_per_second)))
                .replace("{write}", &rate_decimal(Some(io.write_bytes_per_second))),
            secondary
        );
    }
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(text.mode_disk, &secondary)),
        area,
    );
}

pub(crate) fn render_network(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let text = state.strings();
    // 速率没有分母，画不成百分比条，只能用走势图看变化。
    let mut lines = vec![
        text_line(
            text.network_down,
            format!(
                "{}  {:>10}",
                sparkline(state.receive_history(), NETWORK_SPARKLINE_WIDTH, None),
                rate(latest(state.receive_history()))
            ),
        ),
        text_line(
            text.network_up,
            format!(
                "{}  {:>10}",
                sparkline(state.send_history(), NETWORK_SPARKLINE_WIDTH, None),
                rate(latest(state.send_history()))
            ),
        ),
        Line::from(""),
        Line::from(Span::styled(
            interface_row(
                text.network_interface,
                text.network_rx,
                text.network_tx,
                text.network_total_rx,
                text.network_total_tx,
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    // 机器上有二十来个接口，绝大多数常年是 0。按当前速率、其次按累计流量
    // 降序，真正在跑流量的接口才会排在看得见的位置。
    let mut interfaces: Vec<_> = snapshot.interfaces.iter().collect();
    interfaces.sort_by(|left, right| {
        let activity = |interface: &bmtop_core::NetworkInterfaceMetrics| {
            (
                interface.receive_bytes_per_second.unwrap_or(0.0)
                    + interface.send_bytes_per_second.unwrap_or(0.0),
                interface
                    .received_bytes
                    .saturating_add(interface.sent_bytes),
            )
        };
        let (left_rate, left_total) = activity(left);
        let (right_rate, right_total) = activity(right);
        right_rate
            .partial_cmp(&left_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(right_total.cmp(&left_total))
            .then(left.name.cmp(&right.name))
    });
    lines.extend(interfaces.into_iter().map(|interface| {
        Line::from(interface_row(
            &interface.name,
            &rate(interface.receive_bytes_per_second),
            &rate(interface.send_bytes_per_second),
            &format_bytes(interface.received_bytes),
            &format_bytes(interface.sent_bytes),
        ))
    }));
    let (receive_peak, send_peak) = state.network_peaks();
    let mut parts: Vec<String> = Vec::new();
    if let Some(label) = snapshot.link.as_ref().and_then(|link| link.best_label()) {
        parts.push(label);
    }
    if receive_peak > 0.0 || send_peak > 0.0 {
        parts.push(
            text.network_peak
                .replace("{down}", &rate(Some(receive_peak)))
                .replace("{up}", &rate(Some(send_peak))),
        );
    }
    let secondary = parts.join(" · ");
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(text.mode_network, &secondary)),
        area,
    );
}

/// 网络表格的统一列宽，表头和数据行共用，避免中文表头把列挤歪。
fn interface_row(name: &str, receive: &str, send: &str, total_in: &str, total_out: &str) -> String {
    format!(
        "{}{}{}{}{}",
        pad_label(&truncate_columns(name, 13), 14),
        right_align(receive, 11),
        right_align(send, 11),
        right_align(total_in, 12),
        right_align(total_out, 12)
    )
}

pub(crate) fn render_gpu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let text = state.strings();
    let Some(gpu) = &snapshot.gpu else {
        frame.render_widget(
            panel(vec![Line::from(text.gpu_unavailable)], text.mode_gpu),
            area,
        );
        return;
    };
    let mut lines = Vec::new();
    if let Some(name) = &gpu.name {
        lines.push(text_line(text.gpu_name, name.clone()));
    }
    lines.extend([
        gauge_line(text.gpu_utilization, Some(gpu.utilization_percent), ""),
        gauge_line(text.gpu_idle, Some(gpu.idle_percent), ""),
    ]);
    // SoC 增补：频率 / 功耗 / 温度，缺哪个隐藏哪行。
    if let Some(soc) = &snapshot.soc {
        if let Some(mhz) = soc.gpu_freq_mhz {
            lines.push(text_line(text.label_freq, format!("{mhz:.0} MHz")));
        }
        if let Some(watts) = soc.power.gpu_watts {
            lines.push(text_line(text.label_power, format_watts(watts)));
        }
        if let Some(celsius) = soc.temps.gpu_celsius {
            lines.push(text_line(text.label_temp, format_celsius(celsius)));
        }
    }
    if let Some(peak) = crate::overview::peak_line(text, snapshot) {
        lines.push(peak);
    }
    if let Some(fps) = crate::overview::fps_line(text, snapshot, state) {
        lines.push(fps);
    }
    lines.extend([
        Line::from(""),
        text_line(
            text.gpu_trend,
            sparkline(gpu.history(), GPU_SPARKLINE_WIDTH, Some(100.0)),
        ),
    ]);
    frame.render_widget(panel(lines, text.mode_gpu), area);
}

/// 传感器页：SoC 实时数据（关键温度 gauge + 分组均值 + 风扇）。
/// 与其他页不同，SoC 缺失时页面保留（编号模式不该消失），显示原因。
pub(crate) fn render_sensors(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let text = state.strings();
    let Some(soc) = &snapshot.soc else {
        frame.render_widget(
            panel(
                vec![Line::from(text.sensors_unavailable)],
                text.mode_sensors,
            ),
            area,
        );
        return;
    };

    let mut lines = Vec::new();
    let temp_rows = [
        (text.sensor_group_cpu, soc.temps.cpu_celsius),
        (text.sensor_group_gpu, soc.temps.gpu_celsius),
        (text.sensor_group_soc, soc.temps.soc_celsius),
    ];
    for (group, celsius) in temp_rows {
        if let Some(celsius) = celsius {
            lines.push(temp_gauge_line(
                &format!("{} {}", group, text.label_temp),
                celsius,
            ));
        }
    }

    let group_lines = sensor_group_lines(text, &soc.sensors);
    if !group_lines.is_empty() {
        lines.push(Line::from(""));
        lines.extend(group_lines);
    }

    if !soc.fans.is_empty() {
        lines.push(Line::from(""));
        lines.extend(fan_lines(text, &soc.fans));
    }

    if let Some(battery) = &snapshot.battery {
        let state_label = if battery.charging {
            text.battery_charging
        } else if battery.on_ac {
            text.battery_ac
        } else {
            text.battery_on_battery
        };
        let percent = battery
            .percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "--".into());
        lines.push(Line::from(""));
        lines.push(text_line(
            text.label_battery,
            format!("{percent} ({state_label})"),
        ));
    }

    let title = match soc.thermal_level {
        Some(level) => format!(
            "{} · {} {}",
            text.mode_sensors,
            text.thermal_pressure,
            thermal_level_label(text, level)
        ),
        None => text.mode_sensors.to_string(),
    };
    let secondary = if soc.fans.is_empty() {
        String::new()
    } else {
        format!("{} {}", text.sensors_fans, soc.fans.len())
    };
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(&title, &secondary)),
        area,
    );
}

/// 概览行里的紧凑频率：`1.2GHz`。
pub(crate) fn compact_ghz(mhz: f64) -> String {
    format!("{:.1}GHz", mhz / 1000.0)
}

/// 传感器分组统计行（传感器页与概览温度卡共用）：
/// `CPU E 核   54.2°C (47.0–55.1) ×3`，按均值上色。
pub(crate) fn sensor_group_lines(
    text: &'static Strings,
    sensors: &[bmtop_core::SensorReading],
) -> Vec<Line<'static>> {
    bmtop_core::group_sensor_stats(sensors)
        .iter()
        .map(|stat| {
            let value = format!(
                "{} ×{}",
                text.sensor_range
                    .replace("{avg}", &format_celsius(stat.average))
                    .replace("{min}", &format!("{:.1}", stat.min))
                    .replace("{max}", &format!("{:.1}", stat.max)),
                stat.count
            );
            Line::from(vec![
                Span::styled(
                    pad_field_label(sensor_group_label(text, &stat.group), GAUGE_LABEL_COLUMNS),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(value, Style::default().fg(temp_color(stat.average))),
            ])
        })
        .collect()
}

/// 风扇行（传感器页与概览风扇卡共用），每风扇两行：
/// `Fan 0     ████░░  2390 / 6898 RPM` + 缩进的目标/范围。
pub(crate) fn fan_lines(
    text: &'static Strings,
    fans: &[bmtop_core::FanReading],
) -> Vec<Line<'static>> {
    fans.iter()
        .flat_map(|fan| {
            [
                Line::from(vec![
                    Span::styled(
                        pad_field_label(&fan.name, GAUGE_LABEL_COLUMNS),
                        Style::default().fg(Color::Gray),
                    ),
                    gauge(fan.percent(), GAUGE_WIDTH),
                    Span::raw(format!(
                        "  {}",
                        text.fan_rpm
                            .replace("{rpm}", &fan.actual_rpm.to_string())
                            .replace("{max}", &fan.max_rpm.to_string())
                    )),
                ]),
                Line::from(vec![
                    Span::raw(" ".repeat(GAUGE_LABEL_COLUMNS)),
                    Span::styled(
                        text.fan_target_range
                            .replace("{target}", &fan.target_rpm.to_string())
                            .replace("{min}", &fan.min_rpm.to_string())
                            .replace("{max}", &fan.max_rpm.to_string()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            ]
        })
        .collect()
}

/// 温度 gauge：0–110°C 固定量程，右侧显示实际摄氏度。
pub(crate) fn temp_gauge_line(label: &str, celsius: f64) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            pad_field_label(label, GAUGE_LABEL_COLUMNS),
            Style::default().fg(Color::Gray),
        ),
        gauge(
            Some((celsius / TEMP_GAUGE_MAX_CELSIUS * 100.0).clamp(0.0, 100.0)),
            GAUGE_WIDTH,
        ),
        Span::styled(
            format!("  {:>7}", format_celsius(celsius)),
            Style::default().fg(temp_color(celsius)),
        ),
    ])
}

/// 温度量程上限：Apple Silicon 结温红线在 105–110°C 附近。
const TEMP_GAUGE_MAX_CELSIUS: f64 = 110.0;
/// 温度颜色阈值（mactop 同款）：>90 红，>70 黄。
pub(crate) fn temp_color(celsius: f64) -> Color {
    if celsius > 90.0 {
        Color::Red
    } else if celsius > 70.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

pub(crate) fn thermal_level_label(text: &'static Strings, level: u8) -> &'static str {
    match level {
        0 => text.thermal_level_0,
        1 => text.thermal_level_1,
        2 => text.thermal_level_2,
        3 => text.thermal_level_3,
        _ => text.thermal_level_4,
    }
}

pub(crate) fn sensor_group_label(text: &'static Strings, group: &str) -> &'static str {
    match group {
        "cpu" => text.sensor_group_cpu,
        "cpu_e" => text.sensor_group_cpu_e,
        "cpu_p" => text.sensor_group_cpu_p,
        "cpu_die" => text.sensor_group_cpu_die,
        "gpu" => text.sensor_group_gpu,
        "soc" => text.sensor_group_soc,
        "memory" => text.sensor_group_memory,
        "ssd" => text.sensor_group_ssd,
        "ambient" => text.sensor_group_ambient,
        "board" => text.sensor_group_board,
        "vrm" => text.sensor_group_vrm,
        "display" => text.sensor_group_display,
        "wireless" => text.sensor_group_wireless,
        _ => text.sensor_group_other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology(e_cores: u32, p_cores: u32) -> CpuTopology {
        CpuTopology {
            brand: "Apple M3 Max".to_string(),
            e_cores,
            p_cores,
            gpu_cores: Some(40),
            gpu_max_freq_mhz: Some(1380),
        }
    }

    #[test]
    fn core_labels_split_at_e_core_count() {
        let labels = core_labels(6, Some(&topology(2, 4)));
        assert_eq!(labels, ["E1", "E2", "P1", "P2", "P3", "P4"]);
    }

    #[test]
    fn core_labels_fall_back_on_count_mismatch() {
        let labels = core_labels(3, Some(&topology(2, 4)));
        assert_eq!(labels, ["#0", "#1", "#2"]);
        assert_eq!(core_labels(2, None), ["#0", "#1"]);
    }

    #[test]
    fn core_grid_wraps_rows_by_columns() {
        let per_core = [10.0, 20.0, 30.0, 40.0, 50.0];
        let labels = core_labels(5, None);
        let grid = core_grid(&per_core, &labels, 2);
        assert_eq!(grid.len(), 3);
        let first_row: String = grid[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(first_row.contains("#0"));
        assert!(first_row.contains("10%"));
        assert!(first_row.contains("#1"));
        // 单列兜底：columns 0 视为 1。
        assert_eq!(core_grid(&per_core, &labels, 0).len(), 5);
    }

    #[test]
    fn topology_summary_formats_core_config() {
        assert_eq!(
            topology_summary(&topology(4, 12)),
            "Apple M3 Max (4E+12P · 40 GPU)"
        );
        let mut without_gpu = topology(4, 12);
        without_gpu.gpu_cores = None;
        assert_eq!(topology_summary(&without_gpu), "Apple M3 Max (4E+12P)");
    }
}
