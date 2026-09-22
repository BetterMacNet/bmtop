use crate::NetworkConnection;

/// Parse lsof -F0pcfnPT. NUL terminates fields; LF separates process/file
/// records even in NUL mode. A file descriptor, not a PID, identifies a row.
pub fn parse_lsof_connections(bytes: &[u8]) -> Vec<NetworkConnection> {
    let mut rows = Vec::new();
    let mut pid: Option<i32> = None;
    let mut command: Option<String> = None;
    let mut current: Option<NetworkConnection> = None;
    for field in bytes.split(|byte| *byte == 0) {
        let field = field.strip_prefix(b"\n").unwrap_or(field);
        let Some((&tag, raw)) = field.split_first() else {
            continue;
        };
        let value = String::from_utf8_lossy(raw)
            .trim_end_matches('\n')
            .to_string();
        match tag {
            b'p' => {
                if let Some(row) = current.take() {
                    rows.push(row);
                }
                pid = value.parse::<i32>().ok().filter(|pid| *pid > 0);
                command = None;
            }
            b'c' => command = Some(value),
            b'f' => {
                if let Some(row) = current.take() {
                    rows.push(row);
                }
                if let Some(pid) = pid {
                    current = Some(NetworkConnection {
                        pid,
                        command: command.clone(),
                        protocol: None,
                        endpoint: None,
                        state: None,
                    });
                }
            }
            b'P' => {
                if let Some(row) = &mut current {
                    row.protocol = Some(value);
                }
            }
            b'n' => {
                if let Some(row) = &mut current {
                    row.endpoint = Some(value);
                }
            }
            b'T' => {
                if let Some(state) = value.strip_prefix("ST=") {
                    if let Some(row) = &mut current {
                        row.state = Some(state.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(row) = current {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_multiple_files_per_process_and_record_newlines() {
        let rows = parse_lsof_connections(b"p12\0cBrowser\0\nf3\0PTCP\0n127.0.0.1:1->127.0.0.1:2\0TST=ESTABLISHED\0\nf4\0PUDP\0n*:5353\0\np99\0cServer\0\nf8\0PTCP\0n[::1]:8080\0TST=LISTEN\0\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].pid, 12);
        assert_eq!(rows[1].pid, 12);
        assert_eq!(rows[1].command.as_deref(), Some("Browser"));
        assert_eq!(rows[1].protocol.as_deref(), Some("UDP"));
        assert_eq!(rows[1].state, None);
        assert_eq!(rows[2].pid, 99);
        assert_eq!(rows[2].endpoint.as_deref(), Some("[::1]:8080"));
    }
    #[test]
    fn malformed_or_missing_pid_does_not_inherit_previous_process() {
        let rows = parse_lsof_connections(b"p1\0cGood\0\npbad\0cBad\0\nf3\0PTCP\0nsecret\0");
        assert!(rows.is_empty());
        assert!(parse_lsof_connections(b"").is_empty());
    }
    #[test]
    fn unknown_and_non_utf8_fields_do_not_panic() {
        let rows = parse_lsof_connections(b"p42\0c\xff\0\nf1\0Zfuture\0n*:42\0");
        assert_eq!(rows.len(), 1);
    }
}
