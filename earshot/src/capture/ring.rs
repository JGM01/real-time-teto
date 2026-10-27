//! Copy native audio frames into the capture ring.
//!
//! The callback converts samples to `f32` and uses a preallocated ring.
//! It uses no locks, allocations, or logging.

use ringbuf::{HeapCons, HeapProd, traits::Producer as _};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// One native frame converted to `f32`. Mono frames repeat the sample.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapturedFrame {
    pub value: [f32; 2],
}

/// The capture-ring producer for complete native frames.
pub(crate) type Producer = HeapProd<CapturedFrame>;
/// The capture-ring consumer read by the normalization worker.
pub(crate) type Consumer = HeapCons<CapturedFrame>;

/// A native sample that converts to `f32`.
pub(crate) trait NativeSample: Copy {
    fn to_f32(self) -> f32;
}

impl NativeSample for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}

impl NativeSample for i16 {
    fn to_f32(self) -> f32 {
        f32::from(self) / 32768.0
    }
}

/// A shared producer-completion flag set when this guard is dropped.
pub(crate) struct Done(pub std::sync::Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Return the native-frame capacity for the configured capture duration.
#[must_use]
pub(crate) fn ring_capacity(rate_hz: u32) -> usize {
    (rate_hz as usize * crate::constants::CAPTURE_BUFFER_MS).div_ceil(1_000)
}

/// Copy complete native frames into the ring.
///
/// The counter reserves space for all complete frames. A full ring drops the
/// newest frames and counts them in `dropped`. An incomplete trailing channel
/// group is ignored.
pub(crate) fn push_frames<T: NativeSample, const CHANNELS: usize>(
    producer: &mut Producer,
    produced: &AtomicU64,
    dropped: &AtomicU64,
    samples: &[T],
) {
    let frames = samples.as_chunks::<CHANNELS>().0;
    let mut values = frames.iter().map(|frame| CapturedFrame {
        value: frame_value::<T, CHANNELS>(frame),
    });
    let pushed = producer.push_iter(&mut values);
    produced.fetch_add(pushed as u64, Ordering::Relaxed);
    if pushed < frames.len() {
        dropped.fetch_add((frames.len() - pushed) as u64, Ordering::Relaxed);
    }
}

fn frame_value<T: NativeSample, const CHANNELS: usize>(frame: &[T; CHANNELS]) -> [f32; 2] {
    match CHANNELS {
        1 => {
            let value = frame[0].to_f32();
            [value, value]
        }
        2 => [frame[0].to_f32(), frame[1].to_f32()],
        _ => unreachable!("native input has one or two channels"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::{
        HeapRb,
        traits::{Consumer as _, Observer, Split},
    };

    fn split(capacity: usize) -> (Producer, Consumer) {
        HeapRb::new(capacity).split()
    }

    #[test]
    fn frames_keep_order_and_overflow_is_counted() {
        let (mut producer, mut consumer) = split(4);
        let produced = AtomicU64::new(0);
        let dropped = AtomicU64::new(0);
        let stereo = [0.25_f32, -0.25, 0.5, -0.5, 1.0, -1.0, 0.0, 0.0, 9.0, 9.0];
        push_frames::<f32, 2>(&mut producer, &produced, &dropped, &stereo);
        assert_eq!(produced.load(Ordering::Relaxed), 4);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        let first = consumer.try_pop().unwrap();
        assert_eq!(first.value, [0.25, -0.25]);
        assert_eq!(consumer.occupied_len(), 3);
    }

    #[test]
    fn mono_native_frames_repeat_the_sample() {
        let (mut producer, mut consumer) = split(4);
        let produced = AtomicU64::new(0);
        let dropped = AtomicU64::new(0);
        push_frames::<i16, 1>(&mut producer, &produced, &dropped, &[16384, -16384]);
        assert_eq!(consumer.try_pop().unwrap().value, [0.5, 0.5]);
        assert_eq!(consumer.try_pop().unwrap().value, [-0.5, -0.5]);
        assert_eq!(produced.load(Ordering::Relaxed), 2);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }
}
