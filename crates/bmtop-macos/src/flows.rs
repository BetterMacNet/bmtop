//! Apple's installed nettop is invoked at runtime; no packet capture or bundled code.
use crate::CollectorError;
use bmtop_core::{NetworkFlow, NetworkFlowSnapshot};
use std::{collections::BTreeMap, time::Duration};

const MAX_FLOWS: usize = 512;
const HEADER: &str = "time,,interface,state,bytes_in,bytes_out,";
const DAY_SECONDS: f64 = 86_400.0;

fn invalid() -> CollectorError {
    CollectorError::Command {
        program: "nettop".into(),
        message: "unsupported or incomplete socket accounting output".into(),
    }
}

pub fn network_flows(interval: Duration) -> Result<NetworkFlowSnapshot, CollectorError> {
    if interval.as_secs() < 1 || interval.subsec_nanos() != 0 || interval.as_secs() > 60 {
        return Err(CollectorError::Command {
            program: "nettop".into(),
            message: "flow interval must be a whole number of seconds between 1 and 60".into(),
        });
    }
    #[cfg(target_os = "macos")]
    {
        let seconds = interval.as_secs().to_string();
        let output = crate::command::output(
            std::ffi::OsStr::new("/usr/bin/nettop"),
            &[
                "-L",
                "3",
                "-s",
                &seconds,
                "-n",
                "-x",
                "-J",
                "time,interface,state,bytes_in,bytes_out",
            ],
            interval * 2 + Duration::from_secs(10),
        )?;
        if !output.status.success() {
            return Err(invalid());
        }
        // The initial sample is a bootstrap: its next update can arrive immediately.
        let text = std::str::from_utf8(&output.stdout).map_err(|_| invalid())?;
        let headers: Vec<_> = text
            .match_indices(HEADER)
            .map(|(offset, _)| offset)
            .collect();
        if headers.len() != 3 || headers[0] != 0 {
            return Err(invalid());
        }
        parse_flow_samples(&output.stdout[headers[1]..])
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(CollectorError::UnsupportedPlatform)
    }
}

fn timestamp(value: &str) -> Result<f64, CollectorError> {
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let hour = parts[0].parse::<u32>().map_err(|_| invalid())?;
    let minute = parts[1].parse::<u32>().map_err(|_| invalid())?;
    let second = parts[2].parse::<f64>().map_err(|_| invalid())?;
    if hour >= 24 || minute >= 60 || !(0.0..60.0).contains(&second) {
        return Err(invalid());
    }
    Ok(f64::from(hour * 3600 + minute * 60) + second)
}

fn elapsed(before: f64, after: f64) -> Option<f64> {
    let delta = (after - before).rem_euclid(DAY_SECONDS);
    (delta > 0.0 && delta <= 120.0).then_some(delta)
}

fn counter(value: &str) -> Result<Option<u64>, CollectorError> {
    if value.is_empty() {
        Ok(None)
    } else {
        value.parse().map(Some).map_err(|_| invalid())
    }
}

type FlowKey = (i32, String, String, String);
type FlowRows = BTreeMap<FlowKey, (f64, NetworkFlow)>;

fn parse_sample(
    text: &str,
) -> Result<(f64, FlowRows, std::collections::BTreeSet<FlowKey>), CollectorError> {
    let mut process: Option<(i32, String)> = None;
    let mut rows = BTreeMap::new();
    let mut sample_time = None;
    let mut ambiguous = std::collections::BTreeSet::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        // nettop does not quote process names. Anchor the timestamp and four trailing columns.
        let (time_text, rest) = line.split_once(',').ok_or_else(invalid)?;
        let trailing: Vec<_> = rest.rsplitn(6, ',').collect();
        if trailing.len() != 6 || !trailing[0].is_empty() {
            return Err(invalid());
        }
        let fields = [
            time_text,
            trailing[5],
            trailing[4],
            trailing[3],
            trailing[2],
            trailing[1],
        ];
        let time = timestamp(fields[0])?;
        sample_time.get_or_insert(time);
        let name = fields[1];
        let socket = ["tcp4 ", "tcp6 ", "udp4 ", "udp6 "]
            .iter()
            .find(|prefix| name.starts_with(**prefix));
        let Some(prefix) = socket else {
            let (command, pid) = name.rsplit_once('.').ok_or_else(invalid)?;
            let pid = pid
                .parse::<i32>()
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or_else(invalid)?;
            process = Some((pid, command.to_owned()));
            continue;
        };
        let (pid, command) = process.as_ref().ok_or_else(invalid)?;
        let (local, remote) = name[prefix.len()..].split_once("<->").ok_or_else(invalid)?;
        let flow = NetworkFlow {
            pid: *pid,
            command: Some(command.clone()),
            protocol: prefix.trim().to_owned(),
            local_endpoint: Some(local.to_owned()),
            remote_endpoint: Some(remote.to_owned()),
            interface: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
            state: (!fields[3].is_empty()).then(|| fields[3].to_owned()),
            received_bytes: counter(fields[4])?,
            sent_bytes: counter(fields[5])?,
            receive_bytes_per_second: None,
            send_bytes_per_second: None,
        };
        let key = (*pid, command.clone(), name.to_owned(), fields[2].to_owned());
        if rows.insert(key.clone(), (time, flow)).is_some() {
            ambiguous.insert(key);
        }
    }
    Ok((sample_time.ok_or_else(invalid)?, rows, ambiguous))
}

fn rate(before: Option<u64>, after: Option<u64>, seconds: f64) -> Option<f64> {
    Some(after?.checked_sub(before?)? as f64 / seconds)
}

fn parse_flow_samples(bytes: &[u8]) -> Result<NetworkFlowSnapshot, CollectorError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let samples: Vec<_> = text.split(HEADER).collect();
    if samples.len() != 3 || !samples[0].is_empty() {
        return Err(invalid());
    }
    let (before_time, before, before_ambiguous) = parse_sample(samples[1])?;
    let (after_time, after, after_ambiguous) = parse_sample(samples[2])?;
    let elapsed_seconds = elapsed(before_time, after_time).ok_or_else(invalid)?;
    let truncated =
        after.len() > MAX_FLOWS || !before_ambiguous.is_empty() || !after_ambiguous.is_empty();
    let mut flows: Vec<_> = after
        .into_iter()
        .filter(|(key, _)| !before_ambiguous.contains(key) && !after_ambiguous.contains(key))
        .map(|(key, (time, flow))| {
            let previous = before
                .get(&key)
                .filter(|(_, previous)| previous.state == flow.state);
            let rates = previous
                .and_then(|(old_time, old)| {
                    elapsed(*old_time, time).map(|seconds| {
                        (
                            rate(old.received_bytes, flow.received_bytes, seconds),
                            rate(old.sent_bytes, flow.sent_bytes, seconds),
                        )
                    })
                })
                .unwrap_or((None, None));
            NetworkFlow {
                receive_bytes_per_second: rates.0,
                send_bytes_per_second: rates.1,
                ..flow
            }
        })
        .collect();
    let total_rate = |flow: &NetworkFlow| {
        (flow.receive_bytes_per_second.is_some() || flow.send_bytes_per_second.is_some()).then(
            || {
                flow.receive_bytes_per_second.unwrap_or_default()
                    + flow.send_bytes_per_second.unwrap_or_default()
            },
        )
    };
    flows.sort_by(|left, right| {
        total_rate(right)
            .partial_cmp(&total_rate(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    flows.truncate(MAX_FLOWS);
    Ok(NetworkFlowSnapshot {
        flows,
        elapsed_seconds,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(first: &str, second: &str) -> Vec<u8> {
        format!("{HEADER}\n{first}{HEADER}\n{second}").into_bytes()
    }
    #[test]
    fn rates_use_socket_timestamps_and_never_process_summaries() {
        let bytes = fixture(
            "12:00:00.000000,Example.42,,,999,999,\n12:00:00.250000,tcp4 127.0.0.1:8<->127.0.0.1:9,lo0,Established,100,200,\n",
            "12:00:02.000000,Example.42,,,999,999,\n12:00:02.250000,tcp4 127.0.0.1:8<->127.0.0.1:9,lo0,Established,300,600,\n");
        let report = parse_flow_samples(&bytes).unwrap();
        assert_eq!(report.flows.len(), 1);
        assert_eq!(report.elapsed_seconds, 2.0);
        assert_eq!(report.flows[0].receive_bytes_per_second, Some(100.0));
        assert_eq!(report.flows[0].send_bytes_per_second, Some(200.0));
        let mut private = report.clone();
        bmtop_core::PrivacyMode::Strict.redact_network_flows(&mut private);
        assert_eq!(private.flows[0].command, None);
        assert_eq!(private.flows[0].local_endpoint, None);
        assert_eq!(private.flows[0].remote_endpoint, None);
        assert_eq!(private.flows[0].received_bytes, Some(300));
    }
    #[test]
    fn new_reset_and_missing_counters_have_unknown_rates() {
        let report = parse_flow_samples(&fixture(
            "23:59:59.000000,Example.42,,,500,500,\n23:59:59.000000,udp6 [::1].8<->[::1].9,lo0,,500,500,\n",
            "00:00:01.000000,Example.42,,,0,0,\n00:00:01.000000,udp6 [::1].8<->[::1].9,lo0,,10,,\n00:00:01.000000,tcp4 *:22<->*:*,en0,Listen,,,\n")).unwrap();
        assert_eq!(report.elapsed_seconds, 2.0);
        assert!(report
            .flows
            .iter()
            .all(|f| f.receive_bytes_per_second.is_none() && f.send_bytes_per_second.is_none()));
    }
    #[test]
    fn invalid_and_ambiguous_output_is_rejected_without_echoing_it() {
        for bytes in [
            b"secret".to_vec(),
            fixture("", ""),
            fixture("12:00:00.0,tcp4 a<->b,en0,,1,2,\n", ""),
            fixture(
                "12:00:00.0,Example.42,,,1,2,\n",
                "12:00:01.0,Example.42,,,1,2,\n12:00:01.0,udp4 a<->b,en0,,bad,2,\n",
            ),
        ] {
            assert!(parse_flow_samples(&bytes).is_err());
        }
    }
    #[test]
    fn ambiguous_socket_identities_are_omitted_and_marked_partial() {
        let sample = "12:00:00.0,Example,worker.42,,,10,20,\n12:00:00.0,udp4 a<->b,en0,,1,2,\n12:00:00.0,udp4 a<->b,en0,,3,4,\n";
        let report =
            parse_flow_samples(&fixture(sample, &sample.replace("12:00:00", "12:00:01"))).unwrap();
        assert!(report.flows.is_empty());
        assert!(report.truncated);
    }
    #[test]
    fn highest_rate_survives_the_row_limit() {
        let before = (0..MAX_FLOWS + 1)
            .map(|i| format!("12:00:00.0,udp4 a:{i:04}<->b,en0,,0,0,\n"))
            .collect::<String>();
        let after = (0..MAX_FLOWS + 1)
            .map(|i| format!("12:00:01.0,udp4 a:{i:04}<->b,en0,,{i},0,\n"))
            .collect::<String>();
        let report = parse_flow_samples(&fixture(
            &format!("12:00:00.0,Example.42,,,0,0,\n{before}"),
            &format!("12:00:01.0,Example.42,,,0,0,\n{after}"),
        ))
        .unwrap();
        assert_eq!(
            report.flows[0].receive_bytes_per_second,
            Some(MAX_FLOWS as f64)
        );
        assert_eq!(report.flows.len(), MAX_FLOWS);
    }
    #[test]
    fn known_one_sided_rate_ranks_above_idle_rows_at_cutoff() {
        let idle = (0..MAX_FLOWS)
            .map(|i| format!("12:00:00.0,udp4 a:{i}<->b,en0,,0,0,\n"))
            .collect::<String>();
        let before =
            format!("12:00:00.0,Example.42,,,0,0,\n{idle}12:00:00.0,udp4 z<->b,en0,,0,,\n");
        let after = before
            .replace("12:00:00", "12:00:01")
            .replace("udp4 z<->b,en0,,0,,", "udp4 z<->b,en0,,9999,,");
        let report = parse_flow_samples(&fixture(&before, &after)).unwrap();
        assert_eq!(report.flows[0].receive_bytes_per_second, Some(9999.0));
        assert_eq!(report.flows[0].send_bytes_per_second, None);
        assert_eq!(report.flows.len(), MAX_FLOWS);
    }
    #[test]
    fn rows_are_bounded_and_truncation_is_explicit() {
        let rows = (0..MAX_FLOWS + 1)
            .map(|i| format!("12:00:01.0,udp4 a:{i}<->b:9,en0,,1,2,\n"))
            .collect::<String>();
        let report = parse_flow_samples(&fixture(
            "12:00:00.0,Example.42,,,0,0,\n",
            &format!("12:00:01.0,Example.42,,,0,0,\n{rows}"),
        ))
        .unwrap();
        assert_eq!(report.flows.len(), MAX_FLOWS);
        assert!(report.truncated);
    }
}
