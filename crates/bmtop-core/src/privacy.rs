//! Privacy policies shared by collectors, TUI and machine output.
//!
//! system_profiler is an unversioned, extensible format. Both safe policies
//! fail closed: only reviewed fields survive. Unknown fields (including their
//! names) are removed, rather than relying solely on an identifier blacklist.

use crate::{DiskVolume, SystemSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{fmt, str::FromStr};

pub const REDACTED: &str = "••••••";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    Strict,
    #[default]
    Default,
    Off,
}

impl fmt::Display for PrivacyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Strict => "strict",
            Self::Default => "default",
            Self::Off => "off",
        })
    }
}

impl FromStr for PrivacyMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "strict" => Ok(Self::Strict),
            "default" => Ok(Self::Default),
            "off" => Ok(Self::Off),
            _ => Err("privacy must be strict, default or off".into()),
        }
    }
}

impl PrivacyMode {
    /// Identifiers retained as redacted markers for recognizable legacy keys.
    /// Unrecognized keys are discarded even when they contain no such word.
    fn is_identifier(key: &str) -> bool {
        matches!(
            key,
            "_name"
                | "hostname"
                | "computer_name"
                | "local_host_name"
                | "ssid"
                | "bssid"
                | "ip_address"
                | "device_name"
                | "volume_name"
                | "serial_number"
                | "serial_num"
                | "device_serialnumber"
                | "controller_address"
                | "platform_uuid"
                | "provisioning_udid"
                | "hardware_uuid"
                | "uuid"
                | "udid"
                | "machine_serial"
                | "mac_address"
                | "ethernet_address"
                | "sppower_battery_serial_number"
                | "spcamera_unique-id"
                | "spdisplays_display-serial-number"
        )
    }

    /// Reviewed system_profiler containers. Dynamic dictionaries are NOT kept:
    /// device names / SSIDs can occur as object keys, not only as values.
    fn is_container(key: &str) -> bool {
        matches!(
            key,
            "SPHardwareDataType"
                | "SPDisplaysDataType"
                | "SPMemoryDataType"
                | "SPStorageDataType"
                | "SPPowerDataType"
                | "SPNetworkDataType"
                | "SPUSBDataType"
                | "SPThunderboltDataType"
                | "SPBluetoothDataType"
                | "SPAudioDataType"
                | "SPCameraDataType"
                | "_items"
                | "volumes"
                | "physical_drive"
                | "sppci_displays"
                | "sppower_battery_health_info"
                | "sppower_battery_charge_info"
                | "sppower_ac_charger_information"
                | "sppower_system_power_settings"
                | "sppower_ac_power"
                | "sppower_battery_power"
                | "sppower_ups_installed"
        )
    }

    fn is_measurement(key: &str) -> bool {
        matches!(
            key,
            "machine_model"
                | "machine_name"
                | "model"
                | "chip_type"
                | "cpu_type"
                | "cpu_speed"
                | "current_processor_speed"
                | "number_processors"
                | "total_number_cores"
                | "packages"
                | "physical_memory"
                | "boot_rom_version"
                | "os_loader_version"
                | "l2_cache_per_core"
                | "l3_cache"
                | "sppci_model"
                | "sppci_vendor"
                | "spdisplays_vendor"
                | "spdisplays_device-id"
                | "spdisplays_revision-id"
                | "spdisplays_metal"
                | "spdisplays_vram"
                | "spdisplays_cores"
                | "spdisplays_ndrvs"
                | "spdisplays_resolution"
                | "spdisplays_pixels"
                | "spdisplays_depth"
                | "spdisplays_main"
                | "spdisplays_online"
                | "spdisplays_connection_type"
                | "spdisplays_display_type"
                | "spdisplays_ambient_brightness"
                | "spdisplays_refresh_rate"
                | "dimm_size"
                | "dimm_type"
                | "dimm_speed"
                | "dimm_status"
                | "global_ecc_state"
                | "size_in_bytes"
                | "free_space_in_bytes"
                | "file_system"
                | "writable"
                | "ignore_ownership"
                | "volume_uuid_is_unsupported"
                | "medium_type"
                | "protocol"
                | "smart_status"
                | "trim_support"
                | "sppower_battery_cycle_count"
                | "sppower_battery_condition"
                | "sppower_battery_maximum_capacity"
                | "sppower_battery_fully_charged"
                | "sppower_battery_is_charging"
                | "sppower_battery_state_of_charge"
                | "sppower_battery_charge_remaining"
                | "sppower_battery_full_charge_capacity"
                | "sppower_battery_health"
                | "sppower_ac_charger_watts"
                | "sppower_ac_charger_connected"
                | "sppower_ac_charger_charging"
                | "sppower_display_sleep_timer"
                | "sppower_disk_sleep_timer"
                | "sppower_system_sleep_timer"
                | "sppower_wake_on_lan"
                | "sppower_hibernate_mode"
                | "sppower_restart_on_power_loss"
                | "bus_power"
                | "bus_power_used"
                | "extra_current_used"
                | "device_speed"
                | "manufacturer"
                | "vendor_id"
                | "product_id"
                | "bcd_device"
                | "link_speed"
                | "link_status"
                | "receptacle"
                | "switch_uid_is_unsupported"
                | "coreaudio_device_input"
                | "coreaudio_device_output"
                | "coreaudio_device_srate"
                | "coreaudio_device_transport"
                | "spcamera_model_id"
                | "interface"
                | "type"
                | "hardware"
                | "status"
        )
    }

    pub fn redact_profiler(self, sections: &mut Map<String, Value>) {
        if self != Self::Off {
            self.redact_map(sections, 0);
        }
    }

    fn redact_map(self, object: &mut Map<String, Value>, depth: usize) {
        // Bound recursion even for hand-authored / future malformed fixtures.
        if depth >= 48 {
            object.clear();
            return;
        }
        object.retain(|key, value| {
            if Self::is_identifier(&key.to_ascii_lowercase()) {
                // Do not preserve arbitrary dynamic keys containing identifiers.
                // Known profiler identifiers use bounded technical field names.
                if !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return false;
                }
                *value = Value::String(REDACTED.into());
                return true;
            }
            if Self::is_container(key) {
                self.redact_container(value, depth + 1);
                return value.is_object() || value.is_array();
            }
            if Self::is_measurement(key) && !value.is_object() && !value.is_array() {
                if self == Self::Strict
                    && matches!(key.as_str(), "manufacturer" | "model" | "machine_name")
                {
                    *value = Value::String(REDACTED.into());
                }
                return true;
            }
            false
        });
    }

    fn redact_container(self, value: &mut Value, depth: usize) {
        if depth >= 48 {
            *value = Value::Null;
            return;
        }
        match value {
            Value::Object(object) => self.redact_map(object, depth),
            Value::Array(items) => {
                for item in items.iter_mut() {
                    self.redact_container(item, depth + 1);
                }
                items.retain(|item| item.is_array() || item.is_object());
            }
            _ => *value = Value::Null,
        }
    }

    /// Apply after filtering so a strict export can still use `ps --user`.
    pub fn redact_snapshot(self, snapshot: &mut SystemSnapshot) {
        if self == Self::Off {
            return;
        }
        for process in &mut snapshot.processes {
            process.arguments = None;
            if self == Self::Strict {
                process.path = None;
                process.name = REDACTED.into();
                process.user = REDACTED.into();
                if let Some(threads) = &mut process.threads {
                    for thread in threads {
                        thread.name = None;
                    }
                }
            }
        }
    }

    pub fn redact_disks(self, disks: &mut [DiskVolume]) {
        if self == Self::Strict {
            for disk in disks {
                if disk.mountpoint != "/" {
                    disk.mountpoint = REDACTED.into();
                }
                // SMB / NFS filesystem labels may contain user and host names.
                if !disk.filesystem.starts_with("/dev/disk") {
                    disk.filesystem = REDACTED.into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn closed_schema_drops_new_fields_and_dynamic_key_names() {
        let mut value = json!({"SPHardwareDataType": [{
            "machine_model": "Mac15,9", "serial_number": "SECRET",
            "provisioning_UDID": "SECRET", "new_apple_identifier": "SECRET",
            "CUSTOMER-SECRET": {"model": "SECRET"},
            "_name": "My private computer"
        }]});
        PrivacyMode::Default.redact_profiler(value.as_object_mut().unwrap());
        let text = value.to_string();
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("private computer"));
        assert_eq!(value["SPHardwareDataType"][0]["machine_model"], "Mac15,9");
        assert_eq!(value["SPHardwareDataType"][0]["serial_number"], REDACTED);
    }

    #[test]
    fn nested_arrays_cannot_bypass_redaction() {
        let mut value = json!({"SPUSBDataType": [[{"_items": [[{
            "serial_num": "SECRET", "unexpected": ["SECRET"], "vendor_id": "0x1234"
        }]]}]]});
        PrivacyMode::Default.redact_profiler(value.as_object_mut().unwrap());
        assert!(!value.to_string().contains("SECRET"));
        assert!(value.to_string().contains("0x1234"));
    }

    #[test]
    fn off_preserves_original_and_strict_is_idempotent() {
        let input = json!({"SPHardwareDataType": [{"machine_model": "Mac15,9", "serial_number": "secret"}]});
        let mut off = input.clone();
        PrivacyMode::Off.redact_profiler(off.as_object_mut().unwrap());
        assert_eq!(input, off);
        let mut strict = input;
        PrivacyMode::Strict.redact_profiler(strict.as_object_mut().unwrap());
        let once = strict.clone();
        PrivacyMode::Strict.redact_profiler(strict.as_object_mut().unwrap());
        assert_eq!(strict, once);
    }

    #[test]
    fn policies_parse_explicitly() {
        assert_eq!(
            "strict".parse::<PrivacyMode>().unwrap(),
            PrivacyMode::Strict
        );
        assert_eq!(PrivacyMode::default().to_string(), "default");
        assert!("disabled".parse::<PrivacyMode>().is_err());
    }
}
