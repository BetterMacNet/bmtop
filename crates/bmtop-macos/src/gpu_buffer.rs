//! Bounded growth for a volatile two-phase native enumeration contract.
use bmtop_core::GpuProcessCoverage;

pub(crate) fn collect<T: Default + Clone>(
    mut read: impl FnMut(&mut [T]) -> usize,
) -> (Vec<T>, GpuProcessCoverage) {
    const INITIAL: usize = 256;
    const MAXIMUM: usize = 16_384;
    const ATTEMPTS: usize = 4;
    let mut buffer = vec![T::default(); INITIAL];
    for attempt in 0..ATTEMPTS {
        let observed = read(&mut buffer);
        if observed == usize::MAX {
            return (
                Vec::new(),
                GpuProcessCoverage {
                    capacity: buffer.len(),
                    ..GpuProcessCoverage::default()
                },
            );
        }
        if observed > buffer.len() && buffer.len() < MAXIMUM && attempt + 1 < ATTEMPTS {
            let next = observed
                .saturating_add(observed / 4)
                .max(buffer.len() * 2)
                .min(MAXIMUM);
            buffer.resize(next, T::default());
            continue;
        }
        let capacity = buffer.len();
        let retained = observed.min(capacity);
        buffer.truncate(retained);
        return (
            buffer,
            GpuProcessCoverage {
                available: true,
                observed_clients: observed,
                retained_clients: retained,
                capacity,
                truncated: observed > capacity,
            },
        );
    }
    unreachable!("last attempt always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grows_past_256_and_preserves_all_rows() {
        let (rows, coverage) = collect::<u64>(|out| {
            for (i, value) in out.iter_mut().take(600).enumerate() {
                *value = i as u64;
            }
            600
        });
        assert_eq!(rows.len(), 600);
        assert_eq!(rows[599], 599);
        assert!(!coverage.truncated);
        assert!(coverage.available);
    }
    #[test]
    fn capacity_and_retries_are_bounded_and_truncation_is_visible() {
        let mut calls = 0;
        let (_, coverage) = collect::<u64>(|_| {
            calls += 1;
            100_000
        });
        assert_eq!(coverage.capacity, 16_384);
        assert!(coverage.truncated);
        assert_eq!(coverage.observed_clients, 100_000);
        assert!(calls <= 4);
    }
    #[test]
    fn empty_success_is_different_from_unavailable() {
        let (_, empty) = collect::<u64>(|_| 0);
        let (_, unavailable) = collect::<u64>(|_| usize::MAX);
        assert!(empty.available);
        assert!(!unavailable.available);
        assert!(!empty.truncated);
    }
    #[test]
    fn constantly_growing_source_stops_after_four_attempts() {
        let mut calls = 0;
        let (_, coverage) = collect::<u64>(|out| {
            calls += 1;
            out.len() + 1
        });
        assert_eq!(calls, 4);
        assert!(coverage.truncated);
    }
}
