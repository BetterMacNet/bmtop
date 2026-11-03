//! Opt-in socket accounting; no packet payloads or capture privileges.
use crate::{
    state::UiState,
    widgets::{pad_label, rate, titled_block, truncate_columns},
};
use bmtop_core::{Language, SystemSnapshot};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Paragraph, Wrap},
    Frame,
};

pub(crate) fn render_network(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &UiState,
    snapshot: &SystemSnapshot,
) {
    let english = state.language == Language::English;
    let hint = if english {
        "[v] Socket traffic (not packet capture)"
    } else {
        "[v] 套接字流量（非抓包）"
    };
    let hints = format!(
        "{hint}\n{}\nsudo bmtop --no-config network --flows",
        if english {
            "sudo may show more connections; coverage is not guaranteed:"
        } else {
            "sudo 可查看更多连接，但不保证完整覆盖："
        }
    );
    let hints = Paragraph::new(hints).wrap(Wrap { trim: false });
    let hint_height = hints.line_count(area.width).min(usize::from(area.height)) as u16;
    let areas = Layout::vertical([
        Constraint::Length(hint_height),
        Constraint::Length(if state.network_flows_enabled {
            8.min(area.height.saturating_sub(hint_height) / 2)
        } else {
            area.height.saturating_sub(hint_height)
        }),
        Constraint::Min(0),
    ])
    .split(area);
    frame.render_widget(hints, areas[0]);
    crate::pages::render_network(frame, areas[1], state, snapshot);
    if state.network_flows_enabled {
        render_flows(frame, areas[2], state);
    }
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

fn flow_activity(receive: Option<f64>, send: Option<f64>) -> (bool, f64) {
    [receive, send]
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .fold((false, 0.0), |(_, total), value| (true, total + value))
}

fn render_flows(frame: &mut Frame<'_>, area: Rect, state: &UiState) {
    let english = state.language == Language::English;
    let status = if state.paused {
        if english {
            "Paused · cached"
        } else {
            "已暂停 · 缓存"
        }
    } else if state.detail_error.is_some() {
        if english {
            "Error · stale"
        } else {
            "错误 · 旧数据"
        }
    } else if state.network_flows_loading || state.network_flows.is_none() {
        if english {
            "Loading · cached rows"
        } else {
            "加载中 · 缓存数据"
        }
    } else if english {
        "Latest sample"
    } else {
        "最新采样"
    };
    let mut lines = Vec::new();
    if let Some(error) = &state.detail_error {
        lines.push(Line::from(safe_text(error)));
    }
    if let Some(snapshot) = &state.network_flows {
        if snapshot.truncated {
            lines.push(Line::from(if english {
                "Partial results · limit or ambiguous sockets"
            } else {
                "部分结果 · 达到上限或存在歧义连接"
            }));
        }
        lines.push(Line::from(if english {
            "PID    PROCESS         RX/s       TX/s       PROTO  REMOTE"
        } else {
            "PID    进程            下行       上行       协议   远端"
        }));
        let mut flows: Vec<_> = snapshot.flows.iter().collect();
        let activity = |flow: &bmtop_core::NetworkFlow| {
            flow_activity(flow.receive_bytes_per_second, flow.send_bytes_per_second)
        };
        flows.sort_by(|left, right| {
            activity(right)
                .partial_cmp(&activity(left))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.pid.cmp(&right.pid))
                .then(left.remote_endpoint.cmp(&right.remote_endpoint))
        });
        lines.extend(flows.into_iter().take(area.height as usize).map(|flow| {
            Line::from(format!(
                "{:<7}{}{:>10} {:>10} {} {}",
                flow.pid,
                pad_label(
                    &truncate_columns(&safe_text(flow.command.as_deref().unwrap_or("--")), 15),
                    16
                ),
                rate(flow.receive_bytes_per_second),
                rate(flow.send_bytes_per_second),
                pad_label(&truncate_columns(&safe_text(&flow.protocol), 6), 6),
                safe_text(flow.remote_endpoint.as_deref().unwrap_or("--")),
            ))
        }));
        if snapshot.flows.is_empty() {
            lines.push(Line::from(if english {
                "No visible socket flows in this sample"
            } else {
                "本次采样未见套接字流量"
            }));
        }
    }
    let title = if english {
        "Top socket flows"
    } else {
        "套接字流量排行"
    };
    let title = match state.network_flows.as_ref() {
        Some(snapshot)
            if snapshot.elapsed_seconds.is_finite() && snapshot.elapsed_seconds > 0.0 =>
        {
            format!("{title} · {:.2}s", snapshot.elapsed_seconds)
        }
        _ => title.to_string(),
    };
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(&title, status)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_idle_rates_sort_ahead_of_unknown_and_controls_are_removed() {
        assert!(flow_activity(Some(0.0), None) > flow_activity(None, None));
        assert_eq!(flow_activity(Some(f64::NAN), Some(-1.0)), (false, 0.0));
        assert_eq!(safe_text("app\u{1b}\n\r\t\u{009b}host"), "apphost");
    }
}
