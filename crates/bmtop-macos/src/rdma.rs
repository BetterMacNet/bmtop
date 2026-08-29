//! RDMA 状态（macOS 26.2+ 的 rdma_ctl / ibv_devinfo）。子进程 + 纯解析。

use bmtop_core::{RdmaDevice, RdmaStatus};
use std::process::Command;

/// 一次完整探测。调用方负责节流（约 10s；两个子进程不便宜）。
pub fn read_rdma_status() -> RdmaStatus {
    let output = Command::new("rdma_ctl").arg("status").output();
    let Ok(output) = output else {
        return RdmaStatus {
            available: false,
            status: "unavailable (rdma_ctl missing or macOS < 26.2)".to_string(),
            devices: Vec::new(),
        };
    };
    if !output.status.success() {
        return RdmaStatus {
            available: false,
            status: "unavailable (rdma_ctl failed)".to_string(),
            devices: Vec::new(),
        };
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_rdma_ctl(&stdout, || {
        Command::new("ibv_devinfo")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    })
}

/// rdma_ctl 输出判定。先判 disabled 再判 enabled（"not enabled" 不能算开启）。
pub fn parse_rdma_ctl(stdout: &str, devinfo: impl FnOnce() -> String) -> RdmaStatus {
    let normalized = stdout.trim().to_lowercase();
    if normalized.contains("disabled") || normalized.contains("not enabled") {
        return RdmaStatus {
            available: false,
            status: "Disabled".to_string(),
            devices: Vec::new(),
        };
    }
    if normalized.contains("enabled") {
        return RdmaStatus {
            available: true,
            status: "Enabled".to_string(),
            devices: parse_ibv_devinfo(&devinfo()),
        };
    }
    RdmaStatus {
        available: false,
        status: stdout.trim().to_string(),
        devices: Vec::new(),
    }
}

/// ibv_devinfo 行解析。注意多口 HCA 的口级字段是"最后者胜"（mactop 同款近似）。
pub fn parse_ibv_devinfo(stdout: &str) -> Vec<RdmaDevice> {
    let mut devices: Vec<RdmaDevice> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let value_of = |prefix: &str| -> Option<String> {
            line.strip_prefix(prefix)
                .map(|rest| rest.trim_start_matches(':').trim().to_string())
        };
        if let Some(name) = value_of("hca_id") {
            let interface = name.strip_prefix("rdma_").unwrap_or("").to_string();
            devices.push(RdmaDevice {
                name,
                transport: String::new(),
                node_guid: String::new(),
                port_state: String::new(),
                active_mtu: 0,
                link_layer: String::new(),
                interface,
            });
            continue;
        }
        let Some(device) = devices.last_mut() else {
            continue;
        };
        if let Some(value) = value_of("transport") {
            device.transport = value.split('(').next().unwrap_or("").trim().to_string();
        } else if let Some(value) = value_of("node_guid") {
            device.node_guid = value;
        } else if let Some(value) = value_of("state") {
            device.port_state = value.split('(').next().unwrap_or("").trim().to_string();
        } else if let Some(value) = value_of("active_mtu") {
            device.active_mtu = value
                .split_whitespace()
                .next()
                .and_then(|token| token.parse().ok())
                .unwrap_or(0);
        } else if let Some(value) = value_of("link_layer") {
            device.link_layer = value;
        }
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DEVINFO: &str = "\
hca_id: rdma_en2
    transport:          InfiniBand (0)
    node_guid:          0011:2233:4455:6677
    port:   1
        state:          PORT_ACTIVE (4)
        active_mtu:     4096 (5)
        link_layer:     Ethernet
";

    #[test]
    fn disabled_wins_over_enabled_substring() {
        let status = parse_rdma_ctl("rdma is not enabled", String::new);
        assert!(!status.available);
        assert_eq!(status.status, "Disabled");
        let status = parse_rdma_ctl("RDMA: Disabled", String::new);
        assert!(!status.available);
    }

    #[test]
    fn enabled_parses_devices() {
        let status = parse_rdma_ctl("rdma: enabled", || SAMPLE_DEVINFO.to_string());
        assert!(status.available);
        assert_eq!(status.devices.len(), 1);
        let device = &status.devices[0];
        assert_eq!(device.name, "rdma_en2");
        assert_eq!(device.interface, "en2");
        assert_eq!(device.transport, "InfiniBand");
        assert_eq!(device.node_guid, "0011:2233:4455:6677");
        assert_eq!(device.port_state, "PORT_ACTIVE");
        assert_eq!(device.active_mtu, 4096);
        assert_eq!(device.link_layer, "Ethernet");
    }

    #[test]
    fn unknown_output_passes_through() {
        let status = parse_rdma_ctl("weird text", String::new);
        assert!(!status.available);
        assert_eq!(status.status, "weird text");
    }
}
