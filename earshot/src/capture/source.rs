//! Capture the default microphone or replay owned audio samples.
//!
//! Sources start a producer worker and supply normalized chunks to the VAD worker.

#[cfg(any(test, feature = "tooling"))]
use crate::capture::ReplayPacing;
use crate::{
    Error, Result,
    capture::{
        InputSource, SourceContext, Watermark,
        normalize::{Chunk, Normalizer},
        ring::{self, Done, NativeSample, Producer, push_frames},
        validate_format,
    },
    constants::*,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Split},
};
#[cfg(any(test, feature = "tooling"))]
use std::num::NonZeroUsize;
#[cfg(any(test, feature = "tooling"))]
use std::path::Path;
#[cfg(any(test, feature = "tooling"))]
use std::time::Instant;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Native capture state for one activation.
pub(crate) struct Input {
    stereo: bool,
    producer: Option<Producer>,
    consumer: ring::Consumer,
    produced: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
    expected: u64,
    eof: bool,
    normalizer: Normalizer,
}

impl Input {
    fn new(rate_hz: u32, channels: u16) -> Self {
        let (producer, consumer) = HeapRb::new(ring::ring_capacity(rate_hz)).split();
        Self {
            stereo: channels == 2,
            producer: Some(producer),
            consumer,
            produced: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            done: Arc::new(AtomicBool::new(false)),
            expected: 0,
            eof: false,
            normalizer: Normalizer::new(rate_hz),
        }
    }

    fn watermark(&self) -> Watermark {
        Watermark {
            frames: self.produced.clone(),
            rate_hz: self.normalizer.rate_hz(),
        }
    }

    fn done(&self) -> Arc<AtomicBool> {
        self.done.clone()
    }

    /// Read native frames until a normalized chunk, EOF, or cancellation is available.
    ///
    /// # Errors
    ///
    /// Returns `Error::CaptureOverrun` for a frame gap or a final producer-count
    /// mismatch.
    fn next(&mut self, cancel: &AtomicBool) -> Result<Option<Chunk<'_>>> {
        assert!(
            self.producer.is_none(),
            "the producer transfers before reading"
        );
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            if self.normalizer.has_output() {
                break;
            }
            let target = self.target();
            if self.eof && self.normalizer.position() == target {
                return Ok(None);
            }
            self.fill(cancel)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            let target = self.target();
            if self.eof && self.normalizer.position() == target {
                return Ok(None);
            }
            self.normalizer.process(self.eof.then_some(target));
        }
        Ok(self.normalizer.take())
    }

    fn target(&self) -> u64 {
        self.expected * u64::from(SAMPLE_RATE_HZ) / u64::from(self.normalizer.rate_hz())
    }

    fn fill(&mut self, cancel: &AtomicBool) -> Result<()> {
        while !self.eof && !self.normalizer.is_full() {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(frame) = self.consumer.try_pop() {
                self.expected += 1;
                let mono = if self.stereo {
                    0.5 * frame.value[0] + 0.5 * frame.value[1]
                } else {
                    frame.value[0]
                };
                self.normalizer.write(mono);
            } else if let Some(dropped) = self.dropped_count() {
                return Err(Error::CaptureOverrun {
                    expected: self.expected,
                    actual: self.expected + dropped,
                });
            } else if self.done.load(Ordering::Acquire) {
                if !self.consumer.is_empty() {
                    continue;
                }
                let produced = self.produced.load(Ordering::Acquire);
                if produced != self.expected {
                    return Err(Error::CaptureOverrun {
                        expected: self.expected,
                        actual: produced,
                    });
                }
                self.eof = true;
            } else {
                thread::sleep(Duration::from_millis(RING_POLL_MS));
            }
        }
        Ok(())
    }

    fn dropped_count(&self) -> Option<u64> {
        let dropped = self.dropped.load(Ordering::Acquire);
        (dropped > 0).then_some(dropped)
    }

    /// Transfer the producer and owned samples to a named replay worker.
    ///
    /// # Errors
    ///
    /// Returns `Error::ThreadSpawn` if the replay worker cannot start.
    #[cfg(any(test, feature = "tooling"))]
    fn replay(
        &mut self,
        samples: Arc<[f32]>,
        chunk_size: NonZeroUsize,
        context: SourceContext,
    ) -> Result<JoinHandle<Result<()>>> {
        let producer = self.producer.take().expect("the producer transfers once");
        let replay = Replay {
            samples,
            chunk_size,
            rate_hz: self.normalizer.rate_hz(),
            context,
            produced: self.produced.clone(),
            dropped: self.dropped.clone(),
            done: self.done.clone(),
        };
        if self.stereo {
            spawn_replay::<2>(producer, replay)
        } else {
            spawn_replay::<1>(producer, replay)
        }
    }

    /// Build a device input stream and transfer the producer to its callback.
    ///
    /// # Errors
    ///
    /// Returns `Error::UnsupportedInputFormat` for a channel mismatch or
    /// `Error::DeviceError` if the device stream cannot be built.
    fn stream<T: NativeSample + cpal::SizedSample, const CHANNELS: usize>(
        &mut self,
        device: &cpal::Device,
        config: cpal::StreamConfig,
        failed: Arc<AtomicU8>,
    ) -> Result<cpal::Stream> {
        assert_eq!(
            config.channels as usize, CHANNELS,
            "stream channels match the native input"
        );
        let mut producer = self.producer.take().expect("the producer transfers once");
        let produced = self.produced.clone();
        let dropped = self.dropped.clone();
        device
            .build_input_stream(
                config,
                move |samples: &[T], _| {
                    push_frames::<T, CHANNELS>(&mut producer, &produced, &dropped, samples);
                },
                move |error| {
                    failed.store(error.kind() as u8 + 1, Ordering::Release);
                },
                None,
            )
            .map_err(Error::DeviceError)
    }
}

#[cfg(any(test, feature = "tooling"))]
struct Replay {
    samples: Arc<[f32]>,
    chunk_size: NonZeroUsize,
    rate_hz: u32,
    context: SourceContext,
    produced: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
}

#[cfg(any(test, feature = "tooling"))]
fn spawn_replay<const CHANNELS: usize>(
    mut producer: Producer,
    replay: Replay,
) -> Result<JoinHandle<Result<()>>> {
    let Replay {
        samples,
        chunk_size,
        rate_hz,
        context,
        produced,
        dropped,
        done,
    } = replay;
    let worker_context = context.clone();
    context.spawn_audio(move || {
        let _done = Done(done);
        if !worker_context.wait_until_listening() {
            return Ok(());
        }
        let origin = Instant::now();
        let mut sent = 0_u64;
        let chunk_samples = chunk_size.get().saturating_mul(CHANNELS);
        for chunk in samples.chunks(chunk_samples) {
            let mut rest = chunk;
            while !rest.is_empty() {
                if worker_context.cancel_flag().load(Ordering::Acquire) {
                    return Ok(());
                }
                let available = producer.vacant_len().min(rest.len() / CHANNELS);
                if available == 0 {
                    thread::sleep(Duration::from_millis(RING_POLL_MS));
                    continue;
                }
                if worker_context.pacing == ReplayPacing::Realtime {
                    let due = origin
                        + Duration::from_secs_f64(
                            (sent + available as u64) as f64 / f64::from(rate_hz),
                        );
                    while Instant::now() < due {
                        if worker_context.cancel_flag().load(Ordering::Acquire) {
                            return Ok(());
                        }
                        thread::sleep(
                            due.saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(RING_POLL_MS)),
                        );
                    }
                }
                push_frames::<f32, CHANNELS>(
                    &mut producer,
                    &produced,
                    &dropped,
                    &rest[..available * CHANNELS],
                );
                sent += available as u64;
                rest = &rest[available * CHANNELS..];
            }
        }
        Ok(())
    })
}

/// Owned WAV or in-memory audio replayed through a native-frame ring.
///
/// Chunk sizes count native frames. Startup transfers the samples to the producer.
#[cfg(any(test, feature = "tooling"))]
pub(crate) struct FileAudioSource {
    samples: Option<Arc<[f32]>>,
    chunk_size: NonZeroUsize,
    input: Input,
}

#[cfg(any(test, feature = "tooling"))]
impl FileAudioSource {
    /// Create a replay source from owned 16 kHz mono samples.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidConfig` if the native-frame chunk size is zero.
    pub fn from_samples(audio: Arc<[f32]>, chunk_size: usize) -> Result<Self> {
        let chunk_size =
            NonZeroUsize::new(chunk_size).ok_or(Error::InvalidConfig("chunk size is zero"))?;
        Ok(Self {
            samples: Some(audio),
            chunk_size,
            input: Input::new(SAMPLE_RATE_HZ, 1),
        })
    }

    /// Read a WAV file into owned native samples and create its replay input.
    ///
    /// Accept float32 or PCM16 samples at a supported rate with one or two channels.
    /// The chunk size counts native frames.
    ///
    /// # Errors
    ///
    /// Returns an error for zero chunk size, source I/O failure, malformed WAV data,
    /// an incomplete interleaved frame, or an unsupported input format.
    pub fn from_wav(path: impl AsRef<Path>, chunk_size: usize) -> Result<Self> {
        let chunk_size =
            NonZeroUsize::new(chunk_size).ok_or(Error::InvalidConfig("chunk size is zero"))?;
        let file = std::fs::File::open(path).map_err(Error::SourceIo)?;
        let mut reader =
            hound::WavReader::new(std::io::BufReader::new(file)).map_err(Error::InvalidWav)?;
        let spec = reader.spec();
        validate_format(spec.sample_rate, spec.channels)?;
        if u64::from(reader.duration()) * u64::from(SAMPLE_RATE_HZ)
            > TRACE_SAMPLES as u64 * u64::from(spec.sample_rate)
        {
            return Err(Error::InvalidConfig("replay exceeds 308 seconds"));
        }
        let samples = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, 32) => reader
                .samples::<f32>()
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::InvalidWav)?,
            (hound::SampleFormat::Int, 16) => reader
                .samples::<i16>()
                .map(|sample| sample.map(|value| f32::from(value) / 32768.0))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::InvalidWav)?,
            _ => return Err(Error::UnsupportedInputFormat),
        };
        if !samples.len().is_multiple_of(spec.channels as usize) {
            return Err(Error::InvalidWav(hound::Error::FormatError(
                "incomplete interleaved frame",
            )));
        }
        Ok(Self {
            samples: Some(Arc::from(samples)),
            chunk_size,
            input: Input::new(spec.sample_rate, spec.channels),
        })
    }

    /// Transfer owned replay samples to the native-input producer worker.
    ///
    /// # Errors
    ///
    /// Returns a producer-worker startup error.
    pub(crate) fn start(&mut self, context: SourceContext) -> Result<JoinHandle<Result<()>>> {
        let samples = self.samples.take().expect("the source starts once");
        self.input.replay(samples, self.chunk_size, context)
    }

    /// Read the next replay chunk through the native-input consumer.
    ///
    /// # Errors
    ///
    /// Returns `Error::CaptureOverrun` if the native input has a frame gap.
    pub(crate) fn next_chunk(&mut self, cancel: &AtomicBool) -> Result<Option<Chunk<'_>>> {
        self.input.next(cancel)
    }
}

/// A source that captures the default device configuration.
///
/// Each instance permits one start attempt and retains its initialized consumer.
pub(crate) enum MicAudioSource {
    Unopened,
    Started(Box<Input>),
    Spent,
}

impl MicAudioSource {
    /// Create a microphone source without opening a device or starting a worker.
    pub fn new() -> Self {
        Self::Unopened
    }

    /// Attempt default-device startup once and retain the initialized native input.
    ///
    /// The worker owns the device stream and reports its runtime error kind through
    /// an atomic notification.
    ///
    /// # Errors
    ///
    /// Returns a device, format, channel, or worker-startup error.
    ///
    /// # Panics
    ///
    /// Panics if the source starts twice.
    pub(crate) fn start(&mut self, context: SourceContext) -> Result<JoinHandle<Result<()>>> {
        assert!(
            matches!(self, Self::Unopened),
            "the microphone source starts once"
        );
        *self = Self::Spent;
        let capture_context = context.clone();
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(STARTUP_CHANNEL_CAPACITY);
        let handle = context.spawn_audio(move || {
            let device = cpal::default_host()
                .default_input_device()
                .ok_or(Error::NoInputDevice)?;
            let supported = device.default_input_config().map_err(Error::DeviceError)?;
            let float = match supported.sample_format() {
                cpal::SampleFormat::F32 => true,
                cpal::SampleFormat::I16 => false,
                _ => return Err(Error::UnsupportedInputFormat),
            };
            let config = supported.config();
            validate_format(config.sample_rate, config.channels)?;
            let mut input = Input::new(config.sample_rate, config.channels);
            let _done = Done(input.done());
            let failed = Arc::new(AtomicU8::new(0));
            let stream = match (float, config.channels) {
                (true, 1) => input.stream::<f32, 1>(&device, config, failed.clone())?,
                (true, 2) => input.stream::<f32, 2>(&device, config, failed.clone())?,
                (false, 1) => input.stream::<i16, 1>(&device, config, failed.clone())?,
                (false, 2) => input.stream::<i16, 2>(&device, config, failed.clone())?,
                _ => return Err(Error::UnsupportedInputFormat),
            };
            if !capture_context.wait_until_listening() {
                return Ok(());
            }
            stream.play().map_err(Error::DeviceError)?;
            ready_tx.send(input).map_err(|_| Error::Closed)?;
            while !capture_context.cancel_flag().load(Ordering::Acquire) {
                let code = failed.load(Ordering::Acquire);
                if code != 0 {
                    return Err(Error::DeviceError(stream_error_from_code(code)));
                }
                thread::sleep(Duration::from_millis(QUEUE_WAIT_MS));
            }
            drop(stream);
            Ok(())
        })?;
        match ready_rx.recv() {
            Ok(input) => {
                *self = Self::Started(Box::new(input));
                Ok(handle)
            }
            Err(_) => match handle.join() {
                Ok(Err(error)) => Err(error),
                Err(_) => Err(Error::WorkerPanicked("earshot-capture")),
                Ok(Ok(())) if context.cancel.load(Ordering::Acquire) => Err(Error::Closed),
                Ok(Ok(())) => unreachable!("an uncancelled capture announces native input"),
            },
        }
    }

    /// Read the next microphone chunk through the initialized native-input consumer.
    ///
    /// # Errors
    ///
    /// Returns `Error::CaptureOverrun` if the native input has a frame gap.
    ///
    /// # Panics
    ///
    /// Panics if the source has not started.
    pub(crate) fn next_chunk(&mut self, cancel: &AtomicBool) -> Result<Option<Chunk<'_>>> {
        match self {
            Self::Started(input) => input.next(cancel),
            _ => panic!("the microphone source starts before reading"),
        }
    }

    fn watermark(&self) -> Watermark {
        match self {
            Self::Started(input) => input.watermark(),
            _ => panic!("the microphone source starts before reading"),
        }
    }
}

impl Default for MicAudioSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in source implementations keep their worker management inside the crate.
pub(crate) enum Source {
    #[cfg(any(test, feature = "tooling"))]
    File(Box<FileAudioSource>),
    Mic(MicAudioSource),
}

impl Source {
    pub(crate) fn prepare(input: InputSource) -> Result<Self> {
        match input {
            InputSource::Microphone => Ok(Self::Mic(MicAudioSource::new())),
            #[cfg(any(test, feature = "tooling"))]
            InputSource::File {
                path, chunk_size, ..
            } => FileAudioSource::from_wav(path, chunk_size)
                .map(|source| Self::File(Box::new(source))),
            #[cfg(any(test, feature = "tooling"))]
            InputSource::Samples {
                audio, chunk_size, ..
            } => FileAudioSource::from_samples(audio, chunk_size)
                .map(|source| Self::File(Box::new(source))),
        }
    }

    pub(crate) fn start(&mut self, context: SourceContext) -> Result<JoinHandle<Result<()>>> {
        match self {
            #[cfg(any(test, feature = "tooling"))]
            Self::File(source) => source.start(context),
            Self::Mic(source) => source.start(context),
        }
    }

    pub(crate) fn next_chunk(&mut self, cancel: &AtomicBool) -> Result<Option<Chunk<'_>>> {
        match self {
            #[cfg(any(test, feature = "tooling"))]
            Self::File(source) => source.next_chunk(cancel),
            Self::Mic(source) => source.next_chunk(cancel),
        }
    }

    pub(crate) fn watermark(&self) -> Watermark {
        match self {
            #[cfg(any(test, feature = "tooling"))]
            Self::File(source) => source.input.watermark(),
            Self::Mic(source) => source.watermark(),
        }
    }
}

/// Decode a nonzero CPAL error-kind notification without backend message text.
///
/// # Panics
///
/// Panics if the code does not match a pinned CPAL error kind plus one.
fn stream_error_from_code(code: u8) -> cpal::Error {
    use cpal::ErrorKind::*;
    let kinds = [
        DeviceBusy,
        DeviceChanged,
        DeviceNotAvailable,
        HostUnavailable,
        InvalidInput,
        PermissionDenied,
        RealtimeDenied,
        ResourceExhausted,
        StreamInvalidated,
        UnsupportedConfig,
        UnsupportedOperation,
        Xrun,
        BackendError,
        Other,
    ];
    let kind = kinds
        .into_iter()
        .find(|kind| *kind as u8 + 1 == code)
        .unwrap_or_else(|| {
            unreachable!("stream error code comes from the pinned CPAL ErrorKind enum")
        });
    cpal::Error::new(kind)
}

#[cfg(test)]
mod tests {
    //! Check source replay, format validation, and stream-error mapping.

    use super::*;
    use std::thread;

    fn wav(path: &Path, rate: u32, channels: u16, float: bool, frames: usize) {
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: if float { 32 } else { 16 },
                sample_format: if float {
                    hound::SampleFormat::Float
                } else {
                    hound::SampleFormat::Int
                },
            },
        )
        .unwrap();
        for frame in 0..frames {
            for channel in 0..channels {
                let value = if frame % 17 == 0 {
                    0.25
                } else {
                    channel as f32 * 0.125
                };
                if float {
                    writer.write_sample(value).unwrap();
                } else {
                    writer.write_sample((value * 32768.0) as i16).unwrap();
                }
            }
        }
        writer.finalize().unwrap();
    }

    fn collect(path: &Path, chunk: usize) -> Vec<f32> {
        let mut source = FileAudioSource::from_wav(path, chunk).unwrap();
        let context = SourceContext::test_context();
        let worker = source.start(context.clone()).unwrap();
        let mut output = Vec::new();
        while let Some(chunk) = source.next_chunk(context.cancel_flag()).unwrap() {
            assert_eq!(chunk.start_sample, output.len() as u64);
            assert!(chunk.samples.len() <= VAD_FRAME_SAMPLES);
            output.extend_from_slice(chunk.samples);
        }
        worker.join().unwrap().unwrap();
        output
    }

    #[test]
    fn all_native_formats_and_partitions_have_identical_normalized_output() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.wav");
        for rate in SUPPORTED_INPUT_RATES_HZ {
            for channels in [1, 2] {
                for float in [false, true] {
                    let frames = rate as usize / 12 + 7;
                    wav(&path, rate, channels, float, frames);
                    let reference = collect(&path, 4096);
                    assert_eq!(
                        reference.len(),
                        frames * SAMPLE_RATE_HZ as usize / rate as usize
                    );
                    assert!(reference.iter().all(|sample| sample.is_finite()));
                    for chunk in [1, 511, 512, 513] {
                        let output = collect(&path, chunk);
                        assert_eq!(
                            output.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                            reference.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                            "{rate}Hz {channels}ch float={float} chunk={chunk}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn short_eof_and_delay_alignment_preserve_only_the_source_timeline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("short.wav");
        for rate in SUPPORTED_INPUT_RATES_HZ {
            for frames in [
                0,
                1,
                2,
                17,
                rate as usize / 100 - 1,
                rate as usize / 100,
                rate as usize / 100 + 1,
            ] {
                wav(&path, rate, 1, true, frames);
                assert_eq!(
                    collect(&path, 1).len(),
                    frames * SAMPLE_RATE_HZ as usize / rate as usize,
                    "{rate}Hz, {frames} input frames"
                );
            }
            let mut writer = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for index in 0..rate / 2 {
                writer
                    .write_sample(if index == rate / 5 { 1.0_f32 } else { 0.0 })
                    .unwrap();
            }
            writer.finalize().unwrap();
            let output = collect(&path, 513);
            let peak = output
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0;
            assert_eq!(
                peak,
                SAMPLE_RATE_HZ as usize / 5,
                "{rate}Hz impulse alignment"
            );
        }
    }

    #[test]
    fn scaling_mixing_and_native_float_bypass_are_exact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("arithmetic.wav");
        for channels in [1, 2] {
            let mut writer = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels,
                    sample_rate: SAMPLE_RATE_HZ,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            let samples = [-32768_i16, 32767, 0, 8192];
            for sample in samples {
                writer.write_sample(sample).unwrap();
                if channels == 2 {
                    writer.write_sample(sample).unwrap();
                }
            }
            writer.finalize().unwrap();
            assert_eq!(
                collect(&path, 1),
                samples.map(|sample| f32::from(sample) / 32768.0)
            );
        }
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: SAMPLE_RATE_HZ,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for pair in [[0.75_f32, -0.75], [1.0, 0.0], [-0.25, -0.25]] {
            for sample in pair {
                writer.write_sample(sample).unwrap();
            }
        }
        writer.finalize().unwrap();
        assert_eq!(collect(&path, 1), [0.0, 0.5, -0.25]);
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE_HZ,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let samples = [0.0_f32, -0.0, 0.125, f32::MIN_POSITIVE];
        for sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(
            collect(&path, 1)
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            samples.map(f32::to_bits)
        );
    }

    #[test]
    fn validation_errors_are_typed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.wav");
        for (rate, channels, bits) in [(22_050, 1, 16), (48_000, 3, 16), (48_000, 1, 24)] {
            let writer = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: bits,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            writer.finalize().unwrap();
            assert!(matches!(
                FileAudioSource::from_wav(&path, 1),
                Err(Error::UnsupportedInputFormat)
            ));
        }
        std::fs::write(&path, b"not a WAV").unwrap();
        assert!(matches!(
            FileAudioSource::from_wav(&path, 1),
            Err(Error::InvalidWav(_))
        ));
        assert!(matches!(
            FileAudioSource::from_wav(directory.path().join("missing"), 1),
            Err(Error::SourceIo(_))
        ));
        assert!(matches!(
            FileAudioSource::from_wav(&path, 0),
            Err(Error::InvalidConfig(_))
        ));
        assert!(matches!(
            FileAudioSource::from_samples(Arc::from(Vec::new()), 0),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn ring_gaps_are_external_failures() {
        let mut input = Input::new(SAMPLE_RATE_HZ, 1);
        let mut producer = input.producer.take().unwrap();
        let produced = AtomicU64::new(0);
        let dropped = AtomicU64::new(0);
        push_frames::<f32, 1>(&mut producer, &produced, &dropped, &[0.25, 0.5]);
        input.dropped.store(1, Ordering::Release);
        assert!(matches!(
            input.next(&AtomicBool::new(false)),
            Err(Error::CaptureOverrun {
                expected: 2,
                actual: 3
            })
        ));
    }

    #[test]
    fn cancellation_during_empty_input_returns_none() {
        let mut input = Input::new(SAMPLE_RATE_HZ, 2);
        let _producer = input.producer.take().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let reader_cancel = cancel.clone();
        let worker = thread::Builder::new()
            .name("vad".into())
            .spawn(move || input.next(&reader_cancel).map(|chunk| chunk.is_some()))
            .unwrap();
        cancel.store(true, Ordering::Release);
        assert!(!worker.join().unwrap().unwrap());
    }

    #[test]
    fn backend_failure_kind_survives_atomic_notification() {
        for kind in [
            cpal::ErrorKind::Xrun,
            cpal::ErrorKind::PermissionDenied,
            cpal::ErrorKind::DeviceNotAvailable,
        ] {
            assert_eq!(stream_error_from_code(kind as u8 + 1).kind(), kind);
        }
    }

    #[test]
    fn inference_watermark_includes_partial_native_and_vad_frames() {
        for rate in SUPPORTED_INPUT_RATES_HZ {
            let watermark = Watermark {
                frames: Arc::new(AtomicU64::new(0)),
                rate_hz: rate,
            };
            assert_eq!(watermark.complete_sample(), 0);
            watermark.frames.store(u64::from(rate), Ordering::Release);
            assert_eq!(watermark.complete_sample(), 16_384);
            let boundary = u64::from(rate) * 32 / 1_000;
            if boundary * 1_000 == u64::from(rate) * 32 {
                watermark.frames.store(boundary, Ordering::Release);
                assert_eq!(watermark.complete_sample(), 512);
                watermark.frames.store(boundary + 1, Ordering::Release);
                assert_eq!(watermark.complete_sample(), 1_024);
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

        #[test]
        fn arbitrary_wav_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4_096)) {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("arbitrary.wav");
            std::fs::write(&path, bytes).unwrap();
            let _outcome = FileAudioSource::from_wav(&path, 512);
        }
    }
}
