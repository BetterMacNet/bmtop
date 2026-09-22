//! Deterministic CSV formatters. Header is emitted once per watch stream.
use anyhow::Result;
use bmtop_core::SystemSnapshot;
use serde_json::Value;

fn cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn optional(value: Option<f64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}
fn number(value: &Value) -> String {
    if value.is_number() {
        value.to_string()
    } else {
        String::new()
    }
}
fn header(out: &mut String, enabled: bool, text: &str) {
    if enabled {
        out.push_str(text);
        out.push('\n');
    }
}
fn row(out: &mut String, values: &[String]) {
    out.push_str(&values.join(","));
    out.push('\n');
}

pub fn snapshot_csv(kind: &str, snapshot: &SystemSnapshot, with_header: bool) -> Result<String> {
    let mut out = String::new();
    match kind {
        "processes" => {
            header(
                &mut out,
                with_header,
                "pid,cpu_percent,resident_bytes,threads,user,name",
            );
            for p in &snapshot.processes {
                row(
                    &mut out,
                    &[
                        p.pid.to_string(),
                        optional(p.cpu_percent),
                        p.resident_bytes.map(|v| v.to_string()).unwrap_or_default(),
                        p.thread_count.map(|v| v.to_string()).unwrap_or_default(),
                        cell(&p.user),
                        cell(&p.name),
                    ],
                );
            }
        }
        "network" => {
            header(
                &mut out,
                with_header,
                "name,received_bytes,sent_bytes,receive_bytes_per_second,send_bytes_per_second",
            );
            for interface in &snapshot.interfaces {
                row(
                    &mut out,
                    &[
                        cell(&interface.name),
                        interface.received_bytes.to_string(),
                        interface.sent_bytes.to_string(),
                        optional(interface.receive_bytes_per_second),
                        optional(interface.send_bytes_per_second),
                    ],
                );
            }
        }
        "cpu" => {
            header(
                &mut out,
                with_header,
                "captured_at,total_percent,user_percent,system_percent,idle_percent",
            );
            let cpu = &snapshot.cpu;
            row(
                &mut out,
                &[
                    cell(&snapshot.captured_at),
                    optional(cpu.total_percent),
                    optional(cpu.user_percent),
                    optional(cpu.system_percent),
                    optional(cpu.idle_percent),
                ],
            );
        }
        "memory" => {
            header(&mut out, with_header, "captured_at,total_bytes,used_bytes,free_bytes,compressed_bytes,swap_used_bytes,pressure_percent");
            let m = &snapshot.memory;
            row(
                &mut out,
                &[
                    cell(&snapshot.captured_at),
                    m.total_bytes.to_string(),
                    m.used_bytes.to_string(),
                    m.free_bytes.to_string(),
                    m.compressed_bytes.to_string(),
                    m.swap_used_bytes.to_string(),
                    optional(m.pressure_percent),
                ],
            );
        }
        "gpu" => {
            header(&mut out, with_header, "captured_at,utilization_percent,idle_percent,name,process_times_available,process_times_truncated");
            let gpu = snapshot.gpu.as_ref();
            row(
                &mut out,
                &[
                    cell(&snapshot.captured_at),
                    optional(gpu.map(|g| g.utilization_percent)),
                    optional(gpu.map(|g| g.idle_percent)),
                    cell(gpu.and_then(|g| g.name.as_deref()).unwrap_or_default()),
                    snapshot
                        .gpu_process_coverage
                        .as_ref()
                        .map(|c| c.available.to_string())
                        .unwrap_or_default(),
                    snapshot
                        .gpu_process_coverage
                        .as_ref()
                        .filter(|c| c.available)
                        .map(|c| c.truncated.to_string())
                        .unwrap_or_default(),
                ],
            );
        }
        _ => return Err(super::usage(format!("CSV is not supported for {kind}"))),
    }
    Ok(out)
}

pub fn report_csv(kind: &str, data: &Value, with_header: bool) -> Result<String> {
    let mut out = String::new();
    match kind {
        "disk" => {
            header(
                &mut out,
                with_header,
                "filesystem,mountpoint,total_bytes,used_bytes,available_bytes,used_percent",
            );
            for disk in data.as_array().into_iter().flatten() {
                row(
                    &mut out,
                    &[
                        cell(disk["filesystem"].as_str().unwrap_or_default()),
                        cell(disk["mountpoint"].as_str().unwrap_or_default()),
                        number(&disk["total_bytes"]),
                        number(&disk["used_bytes"]),
                        number(&disk["available_bytes"]),
                        number(&disk["used_percent"]),
                    ],
                );
            }
        }
        "network_connections" => {
            header(&mut out, with_header, "pid,command,protocol,endpoint,state");
            for connection in data["connections"].as_array().into_iter().flatten() {
                row(
                    &mut out,
                    &[
                        number(&connection["pid"]),
                        cell(connection["command"].as_str().unwrap_or_default()),
                        cell(connection["protocol"].as_str().unwrap_or_default()),
                        cell(connection["endpoint"].as_str().unwrap_or_default()),
                        cell(connection["state"].as_str().unwrap_or_default()),
                    ],
                );
            }
        }
        _ => return Err(super::usage(format!("CSV is not supported for {kind}"))),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn escaping_and_single_watch_header() {
        let data = json!({"connections": [{"pid": 42, "command": "a,\"b\"", "protocol": "TCP", "endpoint": "[::1]:9", "state": "LISTEN"}]});
        let first = report_csv("network_connections", &data, true).unwrap();
        let next = report_csv("network_connections", &data, false).unwrap();
        assert!(first.starts_with("pid,command,protocol,endpoint,state\n"));
        assert!(!next.contains("pid,command"));
        assert!(next.contains("\"a,\"\"b\"\"\""));
    }
    #[test]
    fn unknown_metrics_remain_empty_instead_of_zero() {
        let data = json!([{"filesystem":"/dev/disk1", "mountpoint":"/", "total_bytes":null, "used_bytes":0}]);
        let result = report_csv("disk", &data, false).unwrap();
        assert!(result.contains("\"/\",,0,,"));
    }
    #[test]
    fn unsupported_formats_fail_explicitly() {
        assert!(report_csv("hardware", &json!({}), true).is_err());
    }
}
