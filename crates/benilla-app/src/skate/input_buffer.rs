//! Bounded input history. Equal gesture states replace their newest sample; edges stay ordered.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(super) const MAX_AGE: Duration = Duration::from_millis(80);
const CAPACITY: usize = 8;

#[derive(Clone)]
pub(super) struct Sample<T> {
    pub(super) at: Instant,
    pub(super) generation: u64,
    pub(super) value: T,
}

pub(super) struct InputBuffer<T, K> {
    pending: VecDeque<(K, Sample<T>)>,
    held: Option<Sample<T>>,
    pub(super) dropped: u64,
}

impl<T, K> Default for InputBuffer<T, K> {
    fn default() -> Self {
        Self {
            pending: VecDeque::with_capacity(CAPACITY),
            held: None,
            dropped: 0,
        }
    }
}

impl<T: Clone, K: PartialEq> InputBuffer<T, K> {
    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.held = None;
    }

    pub(super) fn push(&mut self, key: K, sample: Sample<T>) {
        self.held = Some(sample.clone());
        if let Some((previous_key, previous)) = self.pending.back_mut() {
            if *previous_key == key && previous.generation == sample.generation {
                *previous = sample;
                return;
            }
        }
        if self.pending.len() == CAPACITY {
            self.pending.pop_front();
            self.dropped += 1;
        }
        self.pending.push_back((key, sample));
    }

    pub(super) fn take(&mut self, now: Instant) -> Option<Sample<T>> {
        while self
            .pending
            .front()
            .is_some_and(|(_, s)| now.saturating_duration_since(s.at) > MAX_AGE)
        {
            self.pending.pop_front();
            self.dropped += 1;
        }
        self.pending.pop_front().map(|(_, s)| s).or_else(|| {
            self.held
                .as_ref()
                .filter(|s| now.saturating_duration_since(s.at) <= MAX_AGE)
                .cloned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(value: u16, at: Instant, generation: u64) -> Sample<u16> {
        Sample {
            value,
            at,
            generation,
        }
    }

    #[test]
    fn continuous_sampling_cannot_build_a_backlog() {
        let now = Instant::now();
        let mut buffer = InputBuffer::default();
        for i in 0..100_000u32 {
            buffer.push(0, sample(i as u16, now, 1));
        }
        assert_eq!(buffer.pending.len(), 1);
        assert_eq!(buffer.take(now).unwrap().value, 99_999u32 as u16);
    }

    #[test]
    fn tap_and_flick_keyframes_survive_between_ticks() {
        let now = Instant::now();
        let mut buffer = InputBuffer::default();
        for (key, value) in [(0, 0), (1, 0x1000), (0, 0), (2, 20), (3, 30), (0, 0)] {
            buffer.push(key, sample(value, now, 1));
        }
        let values: Vec<_> = (0..6).map(|_| buffer.take(now).unwrap().value).collect();
        assert_eq!(values, [0, 0x1000, 0, 20, 30, 0]);
    }

    #[test]
    fn stalled_consumer_recovers_to_fresh_input() {
        let now = Instant::now();
        let mut buffer = InputBuffer::default();
        for i in 0..100u16 {
            buffer.push(i, sample(i, now, 1));
        }
        assert_eq!(buffer.pending.len(), CAPACITY);
        let recovered = now + Duration::from_secs(1);
        buffer.push(100, sample(100, recovered, 1));
        assert_eq!(buffer.take(recovered).unwrap().value, 100);
        assert_eq!(buffer.pending.len(), 0);
        assert!(buffer.dropped >= 99);
    }

    #[test]
    fn expired_held_input_is_not_replayed_forever() {
        let now = Instant::now();
        let mut buffer = InputBuffer::default();
        buffer.push(1, sample(0x1000, now, 1));
        assert!(buffer
            .take(now + MAX_AGE + Duration::from_nanos(1))
            .is_none());
    }

    #[test]
    fn reconnect_reset_discards_all_old_edges() {
        let now = Instant::now();
        let mut buffer = InputBuffer::default();
        buffer.push(1, sample(0x1000, now, 1));
        buffer.clear();
        buffer.push(0, sample(0, now, 2));
        let fresh = buffer.take(now).unwrap();
        assert_eq!((fresh.value, fresh.generation), (0, 2));
        assert_eq!(buffer.pending.len(), 0);
    }
}
