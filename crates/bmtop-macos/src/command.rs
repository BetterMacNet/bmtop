//! Fixed-program subprocesses with wall-clock and output limits.
//! No shell expansion. A dedicated process group lets a timeout reap descendants
//! as well as the child, so a pipe cannot keep a reader thread alive indefinitely.
use crate::CollectorError;
use std::{
    ffi::OsStr,
    io::{self, Read},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const STDOUT_LIMIT: usize = 16 * 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;

fn failure(program: &OsStr, message: impl Into<String>) -> CollectorError {
    CollectorError::Command {
        program: program.to_string_lossy().into_owned(),
        message: message.into(),
    }
}

fn bounded_reader<R: Read + Send + 'static>(
    reader: R,
    limit: usize,
) -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .and_then(|_| {
                if bytes.len() > limit {
                    Err(io::Error::other("command output exceeded size limit"))
                } else {
                    Ok(bytes)
                }
            });
        let _ = tx.send(result);
    });
    rx
}

fn terminate_group(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        // process_group(0) creates a group whose ID is the child PID. The
        // caller's terminal / process group is never targeted.
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn output(program: &OsStr, args: &[&str], timeout: Duration) -> Result<Output, CollectorError> {
    if timeout.is_zero() {
        return Err(failure(program, "timeout must be positive"));
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| failure(program, e.to_string()))?;
    let out = bounded_reader(child.stdout.take().expect("piped stdout"), STDOUT_LIMIT);
    let err = bounded_reader(child.stderr.take().expect("piped stderr"), STDERR_LIMIT);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate_group(&mut child);
                return Err(failure(program, error.to_string()));
            }
        }
        if Instant::now() >= deadline {
            terminate_group(&mut child);
            return Err(failure(
                program,
                format!("timed out after {}ms", timeout.as_millis()),
            ));
        }
        thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    };
    let receive = |rx: mpsc::Receiver<io::Result<Vec<u8>>>| -> Result<Vec<u8>, CollectorError> {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| failure(program, "timed out draining command output"))?
            .map_err(|e| failure(program, e.to_string()))
    };
    let bytes = receive(out).and_then(|stdout| receive(err).map(|stderr| (stdout, stderr)));
    match bytes {
        Ok((stdout, stderr)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Err(error) => {
            terminate_group(&mut child);
            Err(error)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn fixed_program_returns_stdout() {
        let value = output(
            OsStr::new("/bin/echo"),
            &["fixture"],
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(value.status.success());
        assert_eq!(value.stdout, b"fixture\n");
    }
    #[test]
    fn slow_program_has_a_deadline() {
        let start = Instant::now();
        let error =
            output(OsStr::new("/bin/sleep"), &["10"], Duration::from_millis(50)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn output_reader_rejects_oversized_payloads() {
        let rx = bounded_reader(io::Cursor::new(vec![b'x'; 100]), 16);
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_err());
    }
}
