use serde::Serialize;

/// Runtime information used by `doctor`; optional fields stay `null` when the
/// operating system does not expose the corresponding probe.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimePlatform {
    pub os_product_version: Option<String>,
    pub os_build: Option<String>,
    pub kernel_release: Option<String>,
    pub hardware_model: Option<String>,
    pub machine_arch: Option<String>,
    pub chip_brand: Option<String>,
    pub running_arch: String,
    pub translated: Option<bool>,
}

pub fn runtime_platform() -> RuntimePlatform {
    RuntimePlatform {
        os_product_version: sysctl_string("kern.osproductversion"),
        os_build: sysctl_string("kern.osversion"),
        kernel_release: sysctl_string("kern.osrelease"),
        hardware_model: sysctl_string("hw.model"),
        machine_arch: sysctl_string("hw.machine"),
        chip_brand: sysctl_string("machdep.cpu.brand_string"),
        running_arch: std::env::consts::ARCH.to_string(),
        translated: sysctl_i32("sysctl.proc_translated").map(|value| value != 0),
    }
}

fn parse_sysctl_value(raw: &[u8]) -> Option<String> {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    let value = String::from_utf8_lossy(&raw[..end]).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut size = 0usize;
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 || size == 0 {
        return None;
    }
    let mut raw = vec![0u8; size];
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            raw.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    parse_sysctl_value(&raw[..size.min(raw.len())])
}

#[cfg(not(target_os = "macos"))]
fn sysctl_string(_name: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn sysctl_i32(name: &str) -> Option<i32> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = 0i32;
    let mut size = std::mem::size_of_val(&value);
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut i32 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && size == std::mem::size_of_val(&value)).then_some(value)
}

#[cfg(not(target_os = "macos"))]
fn sysctl_i32(_name: &str) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_terminated_sysctl_values() {
        assert_eq!(parse_sysctl_value(b"27.2\0"), Some("27.2".to_string()));
        assert_eq!(
            parse_sysctl_value(b" 26B123 \0"),
            Some("26B123".to_string())
        );
        assert_eq!(parse_sysctl_value(b"\0"), None);
        assert_eq!(parse_sysctl_value(b""), None);
    }

    #[test]
    fn runtime_manifest_always_reports_running_arch() {
        assert!(!runtime_platform().running_arch.is_empty());
    }
}
