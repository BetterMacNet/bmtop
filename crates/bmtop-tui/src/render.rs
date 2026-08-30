//! 各模式的绘制。布局按模式选，不是所有模式共用一套分栏。

use crate::state::{InputMode, SortKey, UiState};
use crate::widgets::*;
use bmtop_core::{AppMode, Strings};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame<'_>, state: &UiState) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    render_title(frame, vertical[0], state);
    render_modes(frame, vertical[1], state);
    render_body(frame, vertical[2], state);
    render_status(frame, vertical[3], state);
    if state.input_mode == InputMode::Help {
        render_help(frame, area, state.strings());
    }
}

/// 布局按模式选，不再所有模式共用一个 67/33。
fn render_body(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    // 窄终端下再切 30% 只剩十几列，什么都读不了；线框在窄视图里也是改成上下堆叠。
    let narrow = area.width < NARROW_TERMINAL_COLUMNS;
    let split_two = |first: Constraint, second: Constraint| {
        let (direction, constraints) = if narrow {
            (
                Direction::Vertical,
                [
                    Constraint::Length(SECTION_LIST_STACKED_HEIGHT),
                    Constraint::Min(3),
                ],
            )
        } else {
            (Direction::Horizontal, [first, second])
        };
        Layout::default()
            .direction(direction)
            .constraints(constraints)
            .split(area)
    };
    match state.mode {
        // 硬件：左侧 3 分是分区名，右侧 7 分是详情。
        AppMode::Hardware => {
            let split = split_two(Constraint::Percentage(30), Constraint::Percentage(70));
            render_section_list(frame, split[0], state);
            render_section_detail(frame, split[1], state);
        }
        // 进程页：主表 + 选中进程详情侧栏。
        AppMode::Processes => {
            // 窄终端放不下详情侧栏，主表独占，详情靠 Enter 之外的模式去看。
            if narrow {
                render_process_table(frame, area, state);
                return;
            }
            let split = split_two(
                Constraint::Min(0),
                Constraint::Length(PROCESS_DETAIL_COLUMNS),
            );
            render_process_table(frame, split[0], state);
            render_process_detail(frame, split[1], state);
        }
        // 其余各页是整版的摘要，占满宽度。
        _ => render_primary(frame, area, state),
    }
}

fn render_title(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let clock = state
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.captured_at_display.as_str())
        .unwrap_or("--:--:--");
    // 开机时长可能读不到（非 macOS / sysctl 失败），读不到就整段省略。
    let uptime = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.uptime_seconds)
        .map(|seconds| {
            format!(
                " · {}",
                state
                    .strings()
                    .title_uptime
                    .replace("{uptime}", &format_duration(seconds))
            )
        })
        .unwrap_or_default();
    // 芯片段是纯锦上添花，窄终端优先保住时钟和状态。
    let chip = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.topology.as_ref())
        .filter(|_| area.width >= NARROW_TERMINAL_COLUMNS)
        .map(|topology| format!(" · {}", crate::pages::topology_summary(topology)))
        .unwrap_or_default();
    let title = Line::from(vec![
        Span::styled(
            format!(" bmtop · {} ", state.mode.label(state.language)),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "{clock}{chip} · {}{uptime} · {}",
            format_interval(state.interval_millis),
            state.status
        )),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_modes(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let gpu_available = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.gpu.as_ref())
        .is_some();
    let spans = AppMode::ALL
        .iter()
        .filter(|mode| **mode != AppMode::Gpu || gpu_available)
        .flat_map(|mode| {
            let style = if *mode == state.mode {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            [Span::styled(
                format!(" {} {} ", mode.number(), mode.label(state.language)),
                style,
            )]
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_primary(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let Some(snapshot) = &state.snapshot else {
        frame.render_widget(
            Paragraph::new(text.waiting_snapshot).block(
                Block::default()
                    .title(text.panel_system_load)
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    };
    match state.mode {
        AppMode::Overview => crate::overview::render_overview(frame, area, state, snapshot),
        AppMode::Cpu => crate::pages::render_cpu(frame, area, state, snapshot),
        AppMode::Memory => crate::pages::render_memory(frame, area, snapshot, state.strings()),
        AppMode::Network => crate::pages::render_network(frame, area, state, snapshot),
        AppMode::Disk => crate::pages::render_disk(frame, area, state),
        AppMode::Gpu => crate::pages::render_gpu(frame, area, state, snapshot),
        AppMode::Sensors => crate::pages::render_sensors(frame, area, state, snapshot),
        // 进程页与硬件页由 render_body 直接分流，走不到这里。
        AppMode::Processes | AppMode::Hardware => render_process_table(frame, area, state),
    }
}

/// 硬件 / 传感器左侧的分区名列表，带最小可视窗口，游标不会滚出屏幕。
fn render_section_list(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let title = format!(
        "{} · {}",
        state.mode.label(state.language),
        text.section_count
            .replace("{count}", &state.sections.len().to_string())
    );
    if state.sections.is_empty() {
        let message = state
            .detail_error
            .clone()
            .unwrap_or_else(|| text.loading.to_string());
        frame.render_widget(panel(vec![Line::from(message)], &title), area);
        return;
    }
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let offset = state
        .section_selected
        .saturating_sub(visible.saturating_sub(1));
    let lines = state
        .sections
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible)
        .map(|(index, section)| {
            let style = if index == state.section_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            Line::from(Span::styled(section.name.clone(), style))
        })
        .collect();
    frame.render_widget(panel(lines, &title), area);
}

fn render_section_detail(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let (title, body) = match state.selected_section() {
        Some(section) => (section.name.clone(), section.body.clone()),
        None => (
            text.detail.to_string(),
            state
                .detail_error
                .clone()
                .unwrap_or_else(|| text.loading.to_string()),
        ),
    };
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((state.detail_scroll, 0))
            .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

pub(crate) fn render_process_table(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let rows = state.filtered_processes();
    // 两列能耗要 14 列（6+6+两个间隔）。进程页表格只占屏宽 67%，
    // 窄终端塞不下就整体收起，命令列宽度维持改动前的手感。
    let show_energy = area.width >= process_energy_min_width();
    let table_rows = rows.iter().enumerate().map(|(index, (depth, process))| {
        let cpu = process
            .cpu_percent
            .map(|value| format!("{value:>5.1}"))
            .unwrap_or_else(|| "  --  ".to_string());
        let memory = format_bytes(process.resident_bytes.unwrap_or_default());
        let mut command = if state.show_full_path {
            process.path.clone().unwrap_or_else(|| process.name.clone())
        } else {
            process.name.clone()
        };
        if *depth > 0 {
            command = format!("{}└ {command}", "  ".repeat(depth - 1));
        }
        let gpu = process
            .gpu_percent
            .map(|value| format!("{value:>5.1}"))
            .unwrap_or_else(|| "   - ".to_string());
        let mut cells = vec![process.pid.to_string(), cpu, gpu];
        if show_energy {
            cells.push(
                process
                    .energy_impact
                    .map(|value| format!("{value:>5.1}"))
                    .unwrap_or_else(|| "    - ".to_string()),
            );
            cells.push(right_align(
                &process
                    .power_watts
                    .map(format_watts)
                    .unwrap_or_else(|| "-".to_string()),
                PROCESS_POWER_COLUMN_WIDTH as usize,
            ));
        }
        cells.extend([
            memory,
            process
                .thread_count
                .map(|value| value.to_string())
                .unwrap_or_else(|| "--".into()),
            process.user.clone(),
            command,
        ]);
        let style = if index == state.selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };
        Row::new(cells).style(style)
    });
    let mut header_cells = vec!["PID", "CPU%", text.column_gpu];
    if show_energy {
        header_cells.push(text.column_energy);
        header_cells.push(text.column_power);
    }
    header_cells.extend([
        text.column_memory,
        text.column_threads,
        text.column_user,
        text.column_command,
    ]);
    let header = Row::new(header_cells).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let threads: u64 = rows
        .iter()
        .filter_map(|(_, process)| process.thread_count)
        .map(u64::from)
        .sum();
    // 名字带 `_column` 后缀：上面已经有个 `threads` 是线程总数，别让列宽把它盖掉。
    let [pid_column, cpu_column, gpu_column, memory_column, threads_column, user_column] =
        PROCESS_COLUMN_WIDTHS;
    let mut widths = vec![
        Constraint::Length(pid_column),
        Constraint::Length(cpu_column),
        Constraint::Length(gpu_column),
    ];
    if show_energy {
        widths.push(Constraint::Length(PROCESS_ENERGY_COLUMN_WIDTH));
        widths.push(Constraint::Length(PROCESS_POWER_COLUMN_WIDTH));
    }
    widths.extend([
        Constraint::Length(memory_column),
        Constraint::Length(threads_column),
        Constraint::Length(user_column),
        Constraint::Min(10),
    ]);
    let table = Table::new(table_rows, widths);
    let sort_label = match state.sort_key {
        SortKey::Cpu => text.mode_cpu,
        SortKey::Gpu => text.field_gpu,
        SortKey::Energy => text.label_energy,
        SortKey::Power => text.label_power,
        SortKey::Memory => text.column_memory,
        SortKey::Pid => "PID",
    };
    let direction = if state.sort_descending { "↓" } else { "↑" };
    let mut title = text
        .process_sorted_by
        .replace("{key}", &format!("{sort_label}{direction}"));
    if !state.user_filter.is_empty() {
        title.push_str(
            &text
                .process_user_filter
                .replace("{user}", &state.user_filter),
        );
    }
    if state.hide_idle {
        title.push_str(text.process_active_only);
    }
    let table = table
        .header(header)
        .block(titled_block(
            &title,
            &text
                .process_count
                .replace("{items}", &format_count(rows.len()))
                .replace("{threads}", &format_count(threads as usize)),
        ))
        .column_spacing(1);
    frame.render_widget(table, area);
}

/// 详情面板的键值行，标签按显示列宽对齐，中英标签宽度差很大。
fn detail_fields(fields: &[(&str, String)]) -> String {
    const LABEL_COLUMNS: usize = 10;
    fields
        .iter()
        .map(|(label, value)| format!("{}{value}", pad_field_label(label, LABEL_COLUMNS)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn optional_count(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_string())
}

fn optional_bytes(value: Option<u64>) -> String {
    value.map(format_bytes).unwrap_or_else(|| "--".to_string())
}

/// 选中进程的线程列表：CPU% / 状态 / 名称，按 CPU 降序（采集端已排）。
fn render_thread_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &'static bmtop_core::Strings,
    process: &bmtop_core::ProcessRow,
) {
    let (title, body) = match &process.threads {
        Some(threads) => (
            format!("{} · {}", text.field_threads, format_count(threads.len())),
            threads
                .iter()
                .map(|thread| {
                    format!(
                        "{:>5.1}% {:<6} {}",
                        thread.cpu_percent,
                        thread.state,
                        thread
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("TID {}", thread.thread_id))
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        None => (text.field_threads.to_string(), text.loading.to_string()),
    };
    frame.render_widget(
        Paragraph::new(format!("{}\n{}", process.name, body))
            .wrap(Wrap { trim: false })
            .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_process_detail(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let Some(snapshot) = &state.snapshot else {
        return;
    };
    let rows = state.filtered_processes();
    // `H`：详情栏切到选中进程的线程视图（数据只为选中进程采集）。
    if state.thread_view {
        if let Some((_, process)) = rows.get(state.selected).copied() {
            render_thread_detail(frame, area, text, process);
            return;
        }
    }
    let body = match rows.get(state.selected).copied() {
        Some((_, process)) => format!(
            "{}\n{}",
            process.name,
            detail_fields(&[
                (text.field_state, process.state.clone()),
                (
                    text.field_started,
                    format_uptime(process.start_time_seconds)
                ),
                ("PID", process.pid.to_string()),
                (text.mode_cpu, percent(process.cpu_percent)),
                (text.field_gpu, percent(process.gpu_percent)),
                (
                    text.field_cpu_time,
                    process
                        .cpu_time_seconds
                        .map(|seconds| format_duration(seconds as u64))
                        .unwrap_or_else(|| "--".into())
                ),
                (
                    text.field_memory,
                    format_bytes(process.resident_bytes.unwrap_or_default())
                ),
                (text.field_virtual, optional_bytes(process.virtual_bytes)),
                (text.field_threads, optional_count(process.thread_count)),
                (
                    text.field_files,
                    optional_count(process.file_descriptor_count)
                ),
                (
                    text.field_disk_read,
                    optional_bytes(process.disk_read_bytes)
                ),
                (
                    text.field_disk_write,
                    optional_bytes(process.disk_written_bytes)
                ),
                (text.field_user, process.user.clone()),
                (text.field_parent, process.parent_pid.to_string()),
                (
                    text.field_path,
                    process.path.as_deref().unwrap_or("--").to_string()
                ),
                (
                    text.field_arguments,
                    process
                        .arguments
                        .as_ref()
                        .map(|arguments| arguments.join(" "))
                        .unwrap_or_else(|| "--".to_string())
                ),
            ])
        ),
        None => detail_fields(&[
            (
                text.field_mode,
                state.mode.label(state.language).to_string(),
            ),
            (text.field_snapshot, snapshot.captured_at.clone()),
            (text.field_capabilities, snapshot.capabilities.join(", ")),
        ]),
    };
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: true })
            .block(Block::default().title(text.detail).borders(Borders::ALL)),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let text = state.strings();
    let hint = match state.input_mode {
        InputMode::Search => format!("/{}  {}", state.input, text.hint_search),
        InputMode::UserFilter => format!("u:{}  {}", state.user_filter, text.hint_search),
        InputMode::Interval => format!("s:{}  {}", state.interval_input, text.hint_interval),
        InputMode::Action => format!(
            "{}: {}  {}",
            state
                .pending_action
                .as_ref()
                .map(|action| action.confirmation.as_str())
                .unwrap_or(text.action_fallback),
            state.action_input,
            text.hint_action
        ),
        _ if state.uses_section_list() => text.hint_sections.to_string(),
        _ => text.hint_normal.to_string(),
    };
    frame.render_widget(Paragraph::new(hint), area);
}

fn render_help(frame: &mut Frame<'_>, area: Rect, text: &'static Strings) {
    // 线框的帮助层是「快捷键 | 说明」双列网格，不是一串平铺文字。
    let bindings: [(&str, &str); 23] = [
        ("1…9 / F1…F9", text.help_modes),
        ("⌘1…⌘9", text.help_enhanced),
        ("↑ / ↓", text.help_move),
        ("← / →", text.help_focus),
        ("PgUp / PgDn", text.help_page),
        ("Home / End", text.help_ends),
        ("/", text.help_search),
        ("u", text.help_user_filter),
        ("o / P / M / N", text.help_sort),
        ("E / W", text.help_sort_energy),
        ("O / R", text.help_sort_direction),
        ("s / d", text.help_set_interval),
        ("+ / -", text.help_interval),
        ("c", text.help_full_path),
        ("i", text.help_hide_idle),
        ("f", text.help_fps),
        ("V", text.help_tree),
        ("H", text.help_threads),
        ("r", text.help_refresh),
        ("Ctrl+L", text.help_redraw),
        ("Space", text.help_pause),
        ("x / k / X", text.help_signal),
        ("q / Ctrl+C", text.help_quit),
    ];
    const KEY_COLUMNS: usize = 14;
    const DESCRIPTION_COLUMNS: usize = 24;

    let rows = bindings.len().div_ceil(2);
    let width = area.width.saturating_sub(8).min(84);
    let height = area.height.saturating_sub(4).min(rows as u16 + 3);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let lines: Vec<Line<'static>> = (0..rows)
        .map(|row| {
            let mut text = String::new();
            for (key, description) in [bindings.get(row), bindings.get(row + rows)]
                .into_iter()
                .flatten()
            {
                text.push_str(&pad_label(key, KEY_COLUMNS));
                text.push_str(&pad_label(description, DESCRIPTION_COLUMNS));
            }
            Line::from(text.trim_end().to_string())
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(titled_block(text.help_title, text.help_close))
            .wrap(Wrap { trim: false }),
        popup,
    );
}
