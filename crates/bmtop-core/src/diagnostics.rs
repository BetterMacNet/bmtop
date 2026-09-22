//! Bounded runtime statistics with opt-in export. Timings/counts, never snapshots.
use serde::Serialize;
use std::{collections::VecDeque, time::Duration};

const WINDOW: usize = 2048;

#[derive(Debug, Default)]
pub struct TimingWindow {
    samples_ms: VecDeque<f64>,
    total: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct TimingSummary {
    pub total_observations: u64,
    pub retained_observations: usize,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

impl TimingWindow {
    pub fn record(&mut self, duration: Duration) {
        self.total = self.total.saturating_add(1);
        if self.samples_ms.len() == WINDOW {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(duration.as_secs_f64() * 1000.0);
    }

    pub fn summary(&self) -> TimingSummary {
        let mut values: Vec<f64> = self.samples_ms.iter().copied().collect();
        values.sort_by(f64::total_cmp);
        let percentile = |percent: usize| {
            if values.is_empty() {
                None
            } else {
                let rank = (values.len() * percent).div_ceil(100).saturating_sub(1);
                values.get(rank).copied()
            }
        };
        TimingSummary {
            total_observations: self.total,
            retained_observations: values.len(),
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            max_ms: values.last().copied(),
        }
    }
}

#[derive(Debug, Default)]
pub struct RuntimeStats {
    pub samples: u64,
    pub sample_errors: u64,
    pub detail_errors: u64,
    pub discarded_details: u64,
    pub renders: u64,
    pub input_events: u64,
    pub sample_overruns: u64,
    pub sample_queue: TimingWindow,
    pub sample_service: TimingWindow,
    pub detail_service: TimingWindow,
    pub render_service: TimingWindow,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReport {
    pub diagnostics_version: u8,
    pub elapsed_seconds: f64,
    pub samples: u64,
    pub sample_errors: u64,
    pub detail_errors: u64,
    pub discarded_details: u64,
    pub renders: u64,
    pub input_events: u64,
    pub sample_overruns: u64,
    pub sample_queue: TimingSummary,
    pub sample_service: TimingSummary,
    pub detail_service: TimingSummary,
    pub render_service: TimingSummary,
}

impl RuntimeStats {
    pub fn report(&self, elapsed: Duration) -> RuntimeReport {
        RuntimeReport {
            diagnostics_version: 1,
            elapsed_seconds: elapsed.as_secs_f64(),
            samples: self.samples,
            sample_errors: self.sample_errors,
            detail_errors: self.detail_errors,
            discarded_details: self.discarded_details,
            renders: self.renders,
            input_events: self.input_events,
            sample_overruns: self.sample_overruns,
            sample_queue: self.sample_queue.summary(),
            sample_service: self.sample_service.summary(),
            detail_service: self.detail_service.summary(),
            render_service: self.render_service.summary(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_windows_are_unknown_not_zero() {
        let summary = TimingWindow::default().summary();
        assert_eq!(summary.p50_ms, None);
        assert_eq!(summary.total_observations, 0);
    }
    #[test]
    fn nearest_rank_percentiles_and_bounded_memory() {
        let mut window = TimingWindow::default();
        for i in 1..=100 {
            window.record(Duration::from_millis(i));
        }
        assert_eq!(window.summary().p50_ms, Some(50.0));
        assert_eq!(window.summary().p95_ms, Some(95.0));
        for _ in 0..3000 {
            window.record(Duration::from_millis(10));
        }
        assert_eq!(window.summary().retained_observations, WINDOW);
        assert_eq!(window.summary().total_observations, 3100);
    }
}
