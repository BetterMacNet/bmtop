//! 雷雳拓扑：IOKit switch 列表（C 层）→ 总线树（纯函数，可测）。
//! 桥接成员来自 ifconfig/networksetup 子进程，进程内缓存。

use bmtop_core::{TbBus, TbDevice};
use std::process::Command;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RawTbSwitch {
    pub uid: i64,
    pub parent_uid: i64,
    pub depth: i32,
    pub link_speed: i32,
    pub current_speed: i32,
    pub vendor: [u8; 64],
    pub device: [u8; 128],
}

impl Default for RawTbSwitch {
    fn default() -> Self {
        RawTbSwitch {
            uid: 0,
            parent_uid: 0,
            depth: 0,
            link_speed: 0,
            current_speed: 0,
            vendor: [0; 64],
            device: [0; 128],
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn bmtop_read_tb_switches(out: *mut RawTbSwitch, capacity: usize) -> usize;
}

fn c_str_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// 速度档 → 代际标签。阈值语义（≥14→TB5、≥12→TB4、>0→TB3、0→TB4 默认），
/// 外接设备优先用协商档（Current）。
fn tb_mode(link_speed: i32, current_speed: i32, prefer_current: bool) -> &'static str {
    let speed = if prefer_current && current_speed > 0 {
        current_speed
    } else {
        link_speed
    };
    match speed {
        s if s >= 14 => "TB5",
        s if s >= 12 => "TB4",
        s if s > 0 => "TB3",
        _ => "TB4",
    }
}

/// switch 平面列表 → 总线树。纯函数（单测覆盖）。
pub fn build_tb_buses(switches: &[TbSwitchInfo]) -> Vec<TbBus> {
    let mut buses: Vec<(i64, TbBus)> = switches
        .iter()
        .filter(|switch| switch.depth == 0)
        .map(|switch| {
            let mode = tb_mode(switch.link_speed, switch.current_speed, false);
            let generation: u32 = mode[2..].parse().unwrap_or(4);
            let speed = if generation >= 5 {
                "80 Gb/s"
            } else {
                "40 Gb/s"
            };
            (
                switch.uid,
                TbBus {
                    name: format!("{mode} Bus {}", switch.uid & 0xF),
                    is_active: false,
                    speed_label: format!("Up to {speed}"),
                    devices: Vec::new(),
                },
            )
        })
        .collect();
    for switch in switches.iter().filter(|switch| switch.depth > 0) {
        if let Some((_, bus)) = buses.iter_mut().find(|(uid, _)| *uid == switch.parent_uid) {
            bus.is_active = true;
            bus.devices.push(TbDevice {
                name: switch.device.clone(),
                vendor: switch.vendor.clone(),
                mode: tb_mode(switch.link_speed, switch.current_speed, true).to_string(),
            });
        }
    }
    buses.into_iter().map(|(_, bus)| bus).collect()
}

/// 中间表示：C raw → 可测试的输入。
#[derive(Debug, Clone, PartialEq)]
pub struct TbSwitchInfo {
    pub uid: i64,
    pub parent_uid: i64,
    pub depth: i32,
    pub link_speed: i32,
    pub current_speed: i32,
    pub vendor: String,
    pub device: String,
}

#[cfg(target_os = "macos")]
pub fn read_tb_switches() -> Vec<TbSwitchInfo> {
    let mut buffer = vec![RawTbSwitch::default(); 32];
    let count = unsafe { bmtop_read_tb_switches(buffer.as_mut_ptr(), buffer.len()) };
    buffer
        .into_iter()
        .take(count)
        .map(|raw| TbSwitchInfo {
            uid: raw.uid,
            parent_uid: raw.parent_uid,
            depth: raw.depth,
            link_speed: raw.link_speed,
            current_speed: raw.current_speed,
            vendor: c_str_field(&raw.vendor),
            device: c_str_field(&raw.device),
        })
        .collect()
}

/// 雷雳网桥成员（Mac 对 Mac 直连不会出现在 switch 树里，只在 bridge0 成员里）。
pub fn read_bridge_peers() -> Vec<String> {
    let mut members: Vec<String> = Vec::new();
    if let Ok(output) = Command::new("/sbin/ifconfig").arg("bridge0").output() {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("member:") {
                    if let Some(name) = rest.split_whitespace().next() {
                        members.push(name.to_string());
                    }
                }
            }
        }
    }
    members.retain(|name| name.starts_with("en"));
    members
}

/// 树的文本渲染（硬件页分区正文）。纯函数。
pub fn render_tb_tree(buses: &[TbBus], bridge_peers: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for bus in buses {
        let icon = if bus.is_active { "ϟ" } else { "○" };
        lines.push(format!("{icon} {} @ {}", bus.name, bus.speed_label));
        let last = bus.devices.len().saturating_sub(1);
        for (index, device) in bus.devices.iter().enumerate() {
            let prefix = if index == last {
                "  └─"
            } else {
                "  ├─"
            };
            let info = if device.vendor.is_empty() {
                device.mode.clone()
            } else {
                format!("{}, {}", device.vendor, device.mode)
            };
            lines.push(format!("{prefix} {} ({info})", device.name));
        }
    }
    if !bridge_peers.is_empty() {
        lines.push("Thunderbolt Bridge:".to_string());
        let last = bridge_peers.len() - 1;
        for (index, peer) in bridge_peers.iter().enumerate() {
            let prefix = if index == last {
                "  └─"
            } else {
                "  ├─"
            };
            lines.push(format!("{prefix} {peer} (peer host)"));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(uid: i64, parent: i64, depth: i32, link: i32, current: i32) -> TbSwitchInfo {
        TbSwitchInfo {
            uid,
            parent_uid: parent,
            depth,
            link_speed: link,
            current_speed: current,
            vendor: if depth > 0 {
                "Apple Inc.".into()
            } else {
                String::new()
            },
            device: if depth > 0 {
                "Studio Display".into()
            } else {
                String::new()
            },
        }
    }

    #[test]
    fn buses_group_devices_by_parent_uid() {
        let switches = vec![
            switch(0x25, 0, 0, 12, 0),
            switch(0x31, 0, 0, 12, 0),
            switch(0x99, 0x25, 1, 12, 8),
        ];
        let buses = build_tb_buses(&switches);
        assert_eq!(buses.len(), 2);
        assert_eq!(buses[0].name, "TB4 Bus 5");
        assert!(buses[0].is_active);
        assert_eq!(buses[0].devices.len(), 1);
        // 设备档用协商速度：8 → TB3
        assert_eq!(buses[0].devices[0].mode, "TB3");
        assert!(!buses[1].is_active);
        assert!(buses[1].devices.is_empty());
    }

    #[test]
    fn speed_thresholds_map_generations() {
        assert_eq!(tb_mode(14, 0, false), "TB5");
        assert_eq!(tb_mode(12, 0, false), "TB4");
        assert_eq!(tb_mode(8, 0, false), "TB3");
        assert_eq!(tb_mode(0, 0, false), "TB4");
    }

    #[test]
    fn tree_renders_icons_and_branches() {
        let buses = vec![TbBus {
            name: "TB4 Bus 5".into(),
            is_active: true,
            speed_label: "Up to 40 Gb/s".into(),
            devices: vec![TbDevice {
                name: "Studio Display".into(),
                vendor: "Apple Inc.".into(),
                mode: "TB3".into(),
            }],
        }];
        let text = render_tb_tree(&buses, &["en5".to_string()]);
        assert!(text.contains("ϟ TB4 Bus 5 @ Up to 40 Gb/s"));
        assert!(text.contains("  └─ Studio Display (Apple Inc., TB3)"));
        assert!(text.contains("  └─ en5 (peer host)"));
    }
}
