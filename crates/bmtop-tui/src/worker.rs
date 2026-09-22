//! Separate, bounded executors: a slow detail job never owns the sampler.
//! Native SoC state belongs exclusively to the sample closure / sample thread.
use crate::ModeDetail;
use bmtop_core::{AppMode, SystemSnapshot};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailTicket {
    pub mode: AppMode,
    pub generation: u64,
}

pub(crate) struct SampleRequest {
    pub detail_pid: Option<i32>,
    pub fps_enabled: bool,
    pub submitted_at: Instant,
}

pub(crate) enum Response {
    Sample {
        result: Result<Box<SystemSnapshot>, String>,
        queue_time: Duration,
        panicked: bool,
        service_time: Duration,
    },
    Detail {
        ticket: DetailTicket,
        result: Result<ModeDetail, String>,
        panicked: bool,
        service_time: Duration,
    },
}

pub(crate) struct Workers {
    pub samples: mpsc::SyncSender<SampleRequest>,
    pub details: mpsc::SyncSender<DetailTicket>,
    pub responses: mpsc::Receiver<Response>,
    stopped: Arc<AtomicBool>,
}

impl Workers {
    pub fn spawn<F, D>(mut sample: F, mut detail: D) -> std::io::Result<Self>
    where
        F: FnMut(Option<i32>, bool) -> Result<SystemSnapshot, String> + Send + 'static,
        D: FnMut(AppMode) -> Result<ModeDetail, String> + Send + 'static,
    {
        let (samples, sample_requests) = mpsc::sync_channel::<SampleRequest>(1);
        let (details, detail_requests) = mpsc::sync_channel::<DetailTicket>(1);
        let (response_tx, responses) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let sample_stop = Arc::clone(&stopped);
        let sample_tx = response_tx.clone();
        thread::Builder::new()
            .name("bmtop-sampler".into())
            .spawn(move || {
                for request in sample_requests {
                    if sample_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let start = Instant::now();
                    // Contain Rust panics so the TUI can restore its terminal and
                    // report an error instead of remaining in-flight forever.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        sample(request.detail_pid, request.fps_enabled).map(Box::new)
                    }));
                    let panicked = outcome.is_err();
                    let result = outcome.unwrap_or_else(|_| Err("sampler panicked".into()));
                    let response = Response::Sample {
                        result,
                        panicked,
                        queue_time: start.saturating_duration_since(request.submitted_at),
                        service_time: start.elapsed(),
                    };
                    if sample_stop.load(Ordering::Acquire)
                        || sample_tx.send(response).is_err()
                        || panicked
                    {
                        break;
                    }
                }
            })?;
        let detail_stop = Arc::clone(&stopped);
        thread::Builder::new()
            .name("bmtop-detail".into())
            .spawn(move || {
                for ticket in detail_requests {
                    if detail_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let start = Instant::now();
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        detail(ticket.mode)
                    }));
                    let panicked = outcome.is_err();
                    let result = outcome.unwrap_or_else(|_| Err("detail worker panicked".into()));
                    let response = Response::Detail {
                        ticket,
                        result,
                        panicked,
                        service_time: start.elapsed(),
                    };
                    if detail_stop.load(Ordering::Acquire)
                        || response_tx.send(response).is_err()
                        || panicked
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            samples,
            details,
            responses,
            stopped,
        })
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        // Do not join a native/slow query while restoring the terminal. Queued
        // work is cancelled, results are dropped; subprocesses have deadlines.
        self.stopped.store(true, Ordering::Release);
    }
}

/// Generation distinguishes A -> B -> A and explicit refresh on the same page.
#[derive(Debug)]
pub(crate) struct DetailRevision {
    mode: AppMode,
    generation: u64,
}

impl DetailRevision {
    pub fn new(mode: AppMode) -> Self {
        Self {
            mode,
            generation: 0,
        }
    }
    pub fn update(&mut self, mode: AppMode, force: bool) -> bool {
        if self.mode == mode && !force {
            return false;
        }
        self.mode = mode;
        self.generation = self.generation.wrapping_add(1);
        true
    }
    pub fn ticket(&self) -> DetailTicket {
        DetailTicket {
            mode: self.mode,
            generation: self.generation,
        }
    }
    pub fn accepts(&self, ticket: DetailTicket) -> bool {
        self.ticket() == ticket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_result_rejected_even_after_returning_to_same_page() {
        let mut revision = DetailRevision::new(AppMode::Hardware);
        let old = revision.ticket();
        revision.update(AppMode::Disk, false);
        revision.update(AppMode::Hardware, false);
        assert!(!revision.accepts(old));
        let current = revision.ticket();
        revision.update(AppMode::Hardware, true);
        assert!(!revision.accepts(current));
        assert!(revision.accepts(revision.ticket()));
    }

    #[test]
    fn blocked_detail_does_not_block_sampling() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let workers = Workers::spawn(
            |_, _| Err("sample completed while detail blocked".into()),
            move |_| {
                started_tx.send(()).unwrap();
                let _ = release_rx.recv();
                Ok(ModeDetail::None)
            },
        )
        .unwrap();
        workers
            .details
            .send(DetailTicket {
                mode: AppMode::Hardware,
                generation: 0,
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        workers
            .samples
            .send(SampleRequest {
                detail_pid: None,
                fps_enabled: false,
                submitted_at: Instant::now(),
            })
            .unwrap();
        let response = workers.responses.recv_timeout(Duration::from_secs(2));
        let _ = release_tx.send(());
        assert!(matches!(
            response,
            Ok(Response::Sample { result: Err(_), .. })
        ));
    }

    #[test]
    fn failed_detail_is_a_result_not_a_stuck_inflight_job() {
        let workers =
            Workers::spawn(|_, _| Err("unused".into()), |_| panic!("fixture panic")).unwrap();
        workers
            .details
            .send(DetailTicket {
                mode: AppMode::Hardware,
                generation: 7,
            })
            .unwrap();
        let response = workers
            .responses
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(matches!(response, Response::Detail { result: Err(_), .. }));
    }
}
