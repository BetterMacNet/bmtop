//! 渲染原语：百分比条、走势图、按显示列宽对齐的文本工具。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

/// 宽度与阈值抄自 mole 的 `plainProgressBar` / `colorizePercent`，保持视觉一致。
pub(crate) const GAUGE_WIDTH: usize = 16;
pub(crate) const GAUGE_WARN_PERCENT: f64 = 60.0;
pub(crate) const GAUGE_DANGER_PERCENT: f64 = 85.0;
pub(crate) const GAUGE_LABEL_COLUMNS: usize = 10;
/// 磁盘页左侧放的是挂载点，比普通标签宽得多。
pub(crate) const DISK_LABEL_COLUMNS: usize = 22;
/// 概览摘要块的高度：边框 2 行 + 最多 5 个域。
pub(crate) const NETWORK_SPARKLINE_WIDTH: usize = 24;
pub(crate) const GPU_SPARKLINE_WIDTH: usize = 32;
/// 网络速率历史保留的采样点数，对齐 mole 的 NetworkHistorySize。
pub(crate) const NETWORK_HISTORY_MAX: usize = 120;
/// PgUp / PgDn 每次滚动的行数。
pub(crate) const DETAIL_SCROLL_STEP: u16 = 10;
/// 低于这个列宽就不再左右分栏，改上下堆叠。
pub(crate) const NARROW_TERMINAL_COLUMNS: u16 = 92;
/// 堆叠模式下分区列表占的行数。
pub(crate) const SECTION_LIST_STACKED_HEIGHT: u16 = 7;
pub(crate) const SPARKLINE_LEVELS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];
/// sparkline 自动缩放时的下限，避免全零窗口除零。
pub(crate) const SPARKLINE_MIN_SCALE: f64 = 0.1;

pub(crate) fn gauge_color(percent: f64) -> Color {
    if percent >= GAUGE_DANGER_PERCENT {
        Color::Red
    } else if percent >= GAUGE_WARN_PERCENT {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// 16 格百分比条。`None` 或非有限值渲染为灰色空条，绝不 panic。
pub(crate) fn gauge(percent: Option<f64>, width: usize) -> Span<'static> {
    let Some(value) = percent.filter(|value| value.is_finite()) else {
        return Span::styled(
            "\u{2591}".repeat(width),
            Style::default().fg(Color::DarkGray),
        );
    };
    let value = value.clamp(0.0, 100.0);
    let filled = ((value / 100.0) * width as f64) as usize;
    let filled = filled.min(width);
    let bar = format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(width - filled)
    );
    Span::styled(bar, Style::default().fg(gauge_color(value)))
}

/// 按显示列宽补齐标签，中文标签占两列，不能用 `{:<8}`。
/// 标签本身就比列宽长时不补空格——表格列要靠这个精确对齐。
pub(crate) fn pad_label(label: &str, columns: usize) -> String {
    let used = UnicodeWidthStr::width(label);
    format!("{label}{}", " ".repeat(columns.saturating_sub(used)))
}

/// 键值行的标签列：补到 `columns`，但标签再长也至少留一个空格。
/// 英文的 `Compressed` 正好 10 列，用 `pad_label` 会渲染成 `Compressed4.8G`。
pub(crate) fn pad_field_label(label: &str, columns: usize) -> String {
    let used = UnicodeWidthStr::width(label);
    format!("{label}{}", " ".repeat(columns.saturating_sub(used).max(1)))
}

/// 统一版式：`标签  [百分比条]  xx.x%   补充文字`。
pub(crate) fn gauge_line(label: &str, value: Option<f64>, trailing: &str) -> Line<'static> {
    gauge_line_sized(label, value, trailing, GAUGE_WIDTH)
}

/// 同 `gauge_line`，但条宽可调——概览卡片一栏只有 40 列，16 格放不下尾缀。
pub(crate) fn gauge_line_sized(
    label: &str,
    value: Option<f64>,
    trailing: &str,
    width: usize,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            pad_field_label(label, GAUGE_LABEL_COLUMNS),
            Style::default().fg(Color::Gray),
        ),
        gauge(value, width),
        Span::raw(format!("  {:>6}", percent(value))),
    ];
    if !trailing.is_empty() {
        spans.push(Span::raw(format!("   {trailing}")));
    }
    Line::from(spans)
}

/// 线框里每个区块标题都是「左主右副」：`系统负载 …… Load 2.18 2.04 1.96`。
pub(crate) fn titled_block(title: &str, secondary: &str) -> Block<'static> {
    let block = Block::default()
        .title(title.to_string())
        .borders(Borders::ALL);
    if secondary.is_empty() {
        block
    } else {
        block.title(Line::from(secondary.to_string()).right_aligned())
    }
}

pub(crate) fn text_line(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            pad_field_label(label, GAUGE_LABEL_COLUMNS),
            Style::default().fg(Color::Gray),
        ),
        Span::raw(value),
    ])
}

pub(crate) fn panel(lines: Vec<Line<'static>>, title: &str) -> Paragraph<'static> {
    Paragraph::new(lines).block(titled_block(title, ""))
}

/// MHz → `x.xx GHz`。
pub(crate) fn format_ghz(mhz: f64) -> String {
    format!("{:.2} GHz", mhz / 1000.0)
}

/// 瓦数：小于 10 保留两位小数，否则一位。
pub(crate) fn format_watts(watts: f64) -> String {
    if watts < 10.0 {
        format!("{watts:.2}W")
    } else {
        format!("{watts:.1}W")
    }
}

pub(crate) fn format_celsius(celsius: f64) -> String {
    format!("{celsius:.1}°C")
}

/// 衰减峰值：`peak = max(cur, prev × 0.98)`，标题里的 `(峰值 x)` 用。
/// 不用全窗口 max 是为了让久远的尖峰慢慢让位给当前量级。
pub(crate) fn decaying_peak(previous: f64, current: f64) -> f64 {
    current.max(previous * 0.98)
}

/// 8 级走势图。`scale` 为 `None` 时按窗口峰值自动缩放（速率用），
/// 给定上限时按固定量程（百分比用）。历史既有 `Vec` 也有 `VecDeque`，
/// 所以按迭代器收，不要求连续内存。
pub(crate) fn sparkline<'a, I>(history: I, width: usize, scale: Option<f64>) -> String
where
    I: IntoIterator<Item = &'a f64>,
    I::IntoIter: ExactSizeIterator,
{
    if width == 0 {
        return String::new();
    }
    let iterator = history.into_iter();
    let start = iterator.len().saturating_sub(width);
    let window: Vec<f64> = iterator
        .skip(start)
        .copied()
        .map(|value| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        })
        .collect();
    let peak = match scale {
        Some(value) if value > 0.0 => value,
        _ => window.iter().copied().fold(SPARKLINE_MIN_SCALE, f64::max),
    };
    let leading = width.saturating_sub(window.len());
    let mut output = String::with_capacity(width * 3);
    for _ in 0..leading {
        output.push(SPARKLINE_LEVELS[0]);
    }
    for value in window {
        let level = ((value / peak) * (SPARKLINE_LEVELS.len() - 1) as f64) as usize;
        output.push(SPARKLINE_LEVELS[level.min(SPARKLINE_LEVELS.len() - 1)]);
    }
    output
}

/// 按显示列宽截断，超出部分用省略号收尾。中文占两列，不能按字符数切。
pub(crate) fn truncate_columns(text: &str, columns: usize) -> String {
    if UnicodeWidthStr::width(text) <= columns || columns == 0 {
        return text.to_string();
    }
    let mut output = String::new();
    let mut used = 0;
    for character in text.chars() {
        let width = UnicodeWidthStr::width(character.to_string().as_str());
        if used + width > columns.saturating_sub(1) {
            break;
        }
        output.push(character);
        used += width;
    }
    output.push('…');
    output
}

/// 分母为 0 时返回 `None`，让百分比条渲染成「不可用」而不是 0%。
pub(crate) fn ratio_percent(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0))
}

/// 按显示列宽右对齐。表头有中文，`{:>10}` 按字符数补空格会错位。
pub(crate) fn right_align(text: &str, columns: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    format!("{}{text}", " ".repeat(columns.saturating_sub(used)))
}

pub(crate) fn latest<'a, I>(history: I) -> Option<f64>
where
    I: IntoIterator<Item = &'a f64>,
    I::IntoIter: DoubleEndedIterator,
{
    history.into_iter().next_back().copied()
}

/// 定长环形历史：超过上限就丢掉最旧的一个。
pub(crate) fn push_bounded(history: &mut std::collections::VecDeque<f64>, value: f64) {
    if history.len() >= NETWORK_HISTORY_MAX {
        history.pop_front();
    }
    history.push_back(if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    });
}

pub(crate) fn percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "--".into())
}
pub(crate) fn rate(value: Option<f64>) -> String {
    value
        .map(|value| format_bytes(value as u64) + "/s")
        .unwrap_or_else(|| "--".into())
}
pub(crate) fn format_load(values: &[f64]) -> String {
    values
        .iter()
        .map(|value| format!("{value:.2}"))
        .collect::<Vec<_>>()
        .join(" ")
}
pub(crate) fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut number = value as f64;
    let mut index = 0;
    while number >= 1024.0 && index < UNITS.len() - 1 {
        number /= 1024.0;
        index += 1;
    }
    if index == 0 {
        format!("{}{}", value, UNITS[index])
    } else {
        format!("{number:.1}{}", UNITS[index])
    }
}

/// 采样间隔：`1.0s` / `250ms`。
pub(crate) fn format_interval(millis: u64) -> String {
    if millis.is_multiple_of(1_000) || millis >= 1_000 {
        format!("{:.1}s", millis as f64 / 1_000.0)
    } else {
        format!("{millis}ms")
    }
}

/// 千分位分组：`1042` → `1,042`。进程数动辄四位数，不分组读不出量级。
pub(crate) fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// 时长：`9h 18m` / `18m 04s` / `3d 07h`。系统 uptime 和进程运行时长共用。
pub(crate) fn format_duration(elapsed: u64) -> String {
    let (days, hours) = (elapsed / 86_400, elapsed % 86_400 / 3_600);
    let (minutes, seconds) = (elapsed % 3_600 / 60, elapsed % 60);
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

/// 进程已运行时长（自 epoch 启动时刻起算）。
pub(crate) fn format_uptime(start_seconds: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    // 时钟回拨或进程起始时间异常时不显示负数。
    let Some(elapsed) = now.checked_sub(start_seconds).filter(|_| start_seconds > 0) else {
        return "--".to_string();
    };
    format_duration(elapsed)
}
