//! 网络链路（Ethernet/Wi-Fi）与屏幕 FPS 的 Rust 包装（C/ObjC 实现见 bmtop_link.m）。

use bmtop_core::{DisplayFps, EthernetLink, LinkInfo, WifiLink};

#[repr(C)]
#[derive(Clone, Copy)]
struct RawEthLink {
    name: [u8; 32],
    speed_mbps: u64,
    link_up: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawWifiLink {
    name: [u8; 32],
    phy_mode: [u8; 32],
    generation: [u8; 16],
    tx_rate_mbps: i32,
    connected: i32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn bmtop_read_ethernet_links(out: *mut RawEthLink, capacity: usize) -> usize;
    fn bmtop_read_wifi_link(out: *mut RawWifiLink) -> i32;
    fn bmtop_fps_preflight() -> i32;
    fn bmtop_fps_start() -> i32;
    fn bmtop_fps_stop();
    fn bmtop_fps_read(fps: *mut i32, frame_interval_ms: *mut f64) -> i32;
}

fn c_str_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// 一次完整链路探测（getifaddrs + SIOCGIFMEDIA + CoreWLAN）。
/// 调用方负责节流（约 5s 一次即可，链路状态是人级变化）。
#[cfg(target_os = "macos")]
pub fn read_link_info() -> LinkInfo {
    const MAX_LINKS: usize = 16;
    let mut buffer = [RawEthLink {
        name: [0; 32],
        speed_mbps: 0,
        link_up: 0,
    }; MAX_LINKS];
    let count = unsafe { bmtop_read_ethernet_links(buffer.as_mut_ptr(), MAX_LINKS) };
    let ethernet = buffer[..count.min(MAX_LINKS)]
        .iter()
        .map(|link| EthernetLink {
            name: c_str_field(&link.name),
            speed_mbps: link.speed_mbps,
            is_up: link.link_up != 0,
        })
        .collect();
    let mut raw_wifi = RawWifiLink {
        name: [0; 32],
        phy_mode: [0; 32],
        generation: [0; 16],
        tx_rate_mbps: 0,
        connected: 0,
    };
    let wifi = (unsafe { bmtop_read_wifi_link(&mut raw_wifi) } == 0).then(|| WifiLink {
        name: c_str_field(&raw_wifi.name),
        generation: c_str_field(&raw_wifi.generation),
        phy_mode: c_str_field(&raw_wifi.phy_mode),
        tx_rate_mbps: raw_wifi.tx_rate_mbps.max(0) as u32,
        is_connected: raw_wifi.connected != 0,
    });
    LinkInfo { ethernet, wifi }
}

/// FPS 计数器状态（C 层持单例流，这里只是开关与读数的薄包装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsStart {
    Started,
    PermissionDenied,
    Unavailable,
}

#[cfg(target_os = "macos")]
pub fn fps_permission_granted() -> bool {
    unsafe { bmtop_fps_preflight() != 0 }
}

#[cfg(target_os = "macos")]
pub fn fps_start() -> FpsStart {
    match unsafe { bmtop_fps_start() } {
        0 => FpsStart::Started,
        -1 => FpsStart::PermissionDenied,
        _ => FpsStart::Unavailable,
    }
}

#[cfg(target_os = "macos")]
pub fn fps_stop() {
    unsafe { bmtop_fps_stop() };
}

/// 自上次读取以来的合成帧率；流未启动时 `None`，窗口太短时 fps=0。
#[cfg(target_os = "macos")]
pub fn fps_read() -> Option<DisplayFps> {
    let mut fps = 0i32;
    let mut interval_ms = 0f64;
    (unsafe { bmtop_fps_read(&mut fps, &mut interval_ms) } == 0).then(|| DisplayFps {
        fps: fps.max(0) as u32,
        frame_interval_ms: interval_ms,
    })
}
