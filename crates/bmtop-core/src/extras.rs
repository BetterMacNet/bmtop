//! SoC 之外的补充指标模型：电池、磁盘 I/O、网络链路、屏幕 FPS、
//! 雷雳拓扑与 RDMA。均为增量字段（serde default），缺失即 None/空。

use serde::{Deserialize, Serialize};

/// 内置电池状态。外层 `Option`（快照字段）= 无电池机型。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BatteryInfo {
    /// 电量百分比；`None` = 有电池但电量未知（容量键缺失）。
    pub percent: Option<u8>,
    pub charging: bool,
    pub on_ac: bool,
}

/// 系统级磁盘 I/O 速率（全卷聚合）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskIoRates {
    pub read_bytes_per_second: f64,
    pub write_bytes_per_second: f64,
    pub read_ops_per_second: f64,
    pub write_ops_per_second: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EthernetLink {
    pub name: String,
    pub speed_mbps: u64,
    pub is_up: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WifiLink {
    pub name: String,
    /// "Wi-Fi 4".."Wi-Fi 7"，PHY 未知时为空。
    pub generation: String,
    pub phy_mode: String,
    pub tx_rate_mbps: u32,
    pub is_connected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    pub ethernet: Vec<EthernetLink>,
    pub wifi: Option<WifiLink>,
}

impl LinkInfo {
    /// 首选链路标签：Ethernet 优先、平局 Ethernet 赢（mactop 同款）。
    pub fn best_label(&self) -> Option<String> {
        let best_ethernet = self
            .ethernet
            .iter()
            .filter(|link| link.is_up)
            .map(|link| link.speed_mbps)
            .max()
            .unwrap_or(0);
        let wifi = self.wifi.as_ref().filter(|wifi| wifi.is_connected);
        let wifi_rate = wifi.map_or(0, |wifi| u64::from(wifi.tx_rate_mbps));
        if best_ethernet > 0 && best_ethernet >= wifi_rate {
            return Some(format_link_speed(best_ethernet));
        }
        wifi.map(|wifi| {
            if wifi.generation.is_empty() {
                format!("{}Mbps", wifi.tx_rate_mbps)
            } else if wifi.tx_rate_mbps > 0 {
                format!("{} @ {}Mbps", wifi.generation, wifi.tx_rate_mbps)
            } else {
                wifi.generation.clone()
            }
        })
    }
}

/// `1GbE` / `2.5GbE` / `100Mbps`。
pub fn format_link_speed(mbps: u64) -> String {
    if mbps >= 10_000 || (mbps >= 1000 && mbps.is_multiple_of(1000)) {
        format!("{}GbE", mbps / 1000)
    } else if mbps >= 1000 {
        format!("{:.1}GbE", mbps as f64 / 1000.0)
    } else if mbps > 0 {
        format!("{mbps}Mbps")
    } else {
        "--".to_string()
    }
}

/// 屏幕合成帧率（需屏幕录制授权，默认关闭）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayFps {
    pub fps: u32,
    pub frame_interval_ms: f64,
}

/// 雷雳总线（含挂接设备），由 IOKit switch 列表组树而来。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TbBus {
    pub name: String,
    pub is_active: bool,
    pub speed_label: String,
    pub devices: Vec<TbDevice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TbDevice {
    pub name: String,
    pub vendor: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RdmaDevice {
    pub name: String,
    pub transport: String,
    pub node_guid: String,
    pub port_state: String,
    pub active_mtu: u32,
    pub link_layer: String,
    /// `rdma_en2` → `en2`；无法映射时为空。
    pub interface: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RdmaStatus {
    pub available: bool,
    pub status: String,
    pub devices: Vec<RdmaDevice>,
}

/// GPU 理论算力（FP32 TFLOPS）：核数 × MHz × 0.000256（128 ALU × FMA=2）。
/// FP16 恒为 FP32 的 2 倍。频率或核数未知时不给数（不学 mactop 的机型兜底表）。
pub fn gpu_tflops_fp32(gpu_cores: u32, max_freq_mhz: u32) -> Option<f64> {
    (gpu_cores > 0 && max_freq_mhz > 0)
        .then(|| f64::from(gpu_cores) * f64::from(max_freq_mhz) * 0.000_256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_speed_formats_match_mactop() {
        assert_eq!(format_link_speed(10_000), "10GbE");
        assert_eq!(format_link_speed(1000), "1GbE");
        assert_eq!(format_link_speed(2500), "2.5GbE");
        assert_eq!(format_link_speed(100), "100Mbps");
        assert_eq!(format_link_speed(0), "--");
    }

    #[test]
    fn best_label_prefers_ethernet_on_tie() {
        let link = LinkInfo {
            ethernet: vec![EthernetLink {
                name: "en0".into(),
                speed_mbps: 1000,
                is_up: true,
            }],
            wifi: Some(WifiLink {
                name: "en1".into(),
                generation: "Wi-Fi 6".into(),
                phy_mode: "802.11ax".into(),
                tx_rate_mbps: 1000,
                is_connected: true,
            }),
        };
        assert_eq!(link.best_label(), Some("1GbE".to_string()));
    }

    #[test]
    fn best_label_falls_back_to_wifi() {
        let link = LinkInfo {
            ethernet: vec![EthernetLink {
                name: "en0".into(),
                speed_mbps: 0,
                is_up: false,
            }],
            wifi: Some(WifiLink {
                name: "en1".into(),
                generation: "Wi-Fi 6".into(),
                phy_mode: "802.11ax".into(),
                tx_rate_mbps: 866,
                is_connected: true,
            }),
        };
        assert_eq!(link.best_label(), Some("Wi-Fi 6 @ 866Mbps".to_string()));
        assert_eq!(LinkInfo::default().best_label(), None);
    }

    #[test]
    fn tflops_requires_both_inputs() {
        let value = gpu_tflops_fp32(40, 1600).unwrap();
        assert!((value - 16.384).abs() < 1e-9);
        assert_eq!(gpu_tflops_fp32(0, 1600), None);
        assert_eq!(gpu_tflops_fp32(40, 0), None);
    }
}
