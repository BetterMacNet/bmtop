//! Pure scheduling rules, exercised without a terminal or a Mac.
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct RedrawGate(bool);
impl Default for RedrawGate {
    fn default() -> Self {
        Self(true)
    }
}
impl RedrawGate {
    pub fn invalidate(&mut self) {
        self.0 = true;
    }
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.0)
    }
}

pub(crate) fn poll_timeout(
    now: Instant,
    next_sample: Instant,
    paused: bool,
    in_flight: bool,
) -> Duration {
    // crossterm has no mpsc wake integration. Only poll at 40ms while waiting
    // for work; otherwise block until the next sample or actual terminal input.
    if in_flight {
        Duration::from_millis(40)
    } else if paused {
        Duration::from_secs(60)
    } else {
        next_sample.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_loop_does_not_redraw() {
        let mut gate = RedrawGate::default();
        assert!(gate.take());
        for _ in 0..10_000 {
            assert!(!gate.take());
        }
        gate.invalidate();
        gate.invalidate();
        assert!(gate.take());
        assert!(!gate.take());
    }
    #[test]
    fn idle_deadline_is_not_an_80ms_busy_poll() {
        let now = Instant::now();
        assert_eq!(
            poll_timeout(now, now + Duration::from_secs(60), false, false),
            Duration::from_secs(60)
        );
        assert_eq!(poll_timeout(now, now, true, false), Duration::from_secs(60));
        assert_eq!(
            poll_timeout(now, now, false, true),
            Duration::from_millis(40)
        );
    }
}
