//! Pure scheduling rules, exercised without a terminal or a Mac.
use std::time::{Duration, Instant};

use crate::worker::{DetailRevision, DetailTicket};
use bmtop_core::AppMode;

/// One shared detail lane; network jobs repeat only after completion + cooldown.
pub(crate) struct DetailSchedule {
    revision: DetailRevision,
    active: bool,
    loaded: Option<DetailTicket>,
    in_flight: bool,
    next: Instant,
}

impl DetailSchedule {
    pub fn new(mode: AppMode, now: Instant) -> Self {
        Self {
            revision: DetailRevision::new(mode),
            active: false,
            loaded: None,
            in_flight: false,
            next: now,
        }
    }

    pub fn update(
        &mut self,
        mode: AppMode,
        paused: bool,
        flows: bool,
        force: bool,
        now: Instant,
    ) -> bool {
        let active = match mode {
            AppMode::Network => flows && !paused,
            AppMode::Overview | AppMode::Disk | AppMode::Hardware => true,
            _ => false,
        };
        let changed = self.revision.update(mode, force || active != self.active);
        self.active = active;
        if changed {
            self.loaded = None;
            self.next = now;
        }
        changed
    }

    pub fn due(&self, now: Instant) -> bool {
        self.active
            && !self.in_flight
            && self.loaded != Some(self.revision.ticket())
            && now >= self.next
    }

    pub fn start(&mut self) -> DetailTicket {
        self.in_flight = true;
        self.loaded = Some(self.revision.ticket());
        self.revision.ticket()
    }

    pub fn finish(&mut self, ticket: DetailTicket, now: Instant) -> bool {
        self.in_flight = false;
        if !self.active || !self.revision.accepts(ticket) {
            return false;
        }
        if ticket.mode == AppMode::Network {
            self.loaded = None;
            self.next = now + Duration::from_secs(1);
        }
        true
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn deadline(&self, sample: Instant) -> Instant {
        if self.active && !self.in_flight && self.loaded != Some(self.revision.ticket()) {
            sample.min(self.next)
        } else {
            sample
        }
    }
}

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
    fn network_details_are_opt_in_periodic_and_single_flight() {
        let now = Instant::now();
        let mut schedule = DetailSchedule::new(bmtop_core::AppMode::Network, now);
        schedule.update(bmtop_core::AppMode::Network, false, false, false, now);
        assert!(!schedule.due(now));
        schedule.update(bmtop_core::AppMode::Network, false, true, false, now);
        assert!(schedule.due(now));
        let ticket = schedule.start();
        assert!(!schedule.due(now + Duration::from_secs(20)));
        assert!(schedule.finish(ticket, now));
        assert!(!schedule.due(now));
        assert!(schedule.due(now + Duration::from_secs(1)));
    }

    #[test]
    fn pause_and_toggle_invalidate_old_network_results_without_busy_spin() {
        use bmtop_core::AppMode;
        let now = Instant::now();
        let mut schedule = DetailSchedule::new(AppMode::Network, now);
        schedule.update(AppMode::Network, false, true, false, now);
        let old = schedule.start();
        schedule.update(AppMode::Network, true, true, false, now);
        assert!(!schedule.due(now));
        schedule.update(AppMode::Network, false, true, false, now);
        assert!(!schedule.due(now));
        assert!(!schedule.finish(old, now));
        assert!(schedule.due(now));
        let old = schedule.start();
        schedule.update(AppMode::Network, false, false, false, now);
        schedule.update(AppMode::Network, false, true, false, now);
        assert!(!schedule.finish(old, now));
        schedule.update(AppMode::Cpu, false, true, false, now);
        assert!(!schedule.due(now + Duration::from_secs(20)));
    }

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
