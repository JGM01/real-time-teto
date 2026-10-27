//! Zero-storage trace types for builds without tooling.

/// The exact capture samples and completeness for one activation.
pub(crate) type CaptureTrace = ();

/// A reusable bounded buffer for capture samples.
pub(crate) struct TraceArena;

impl TraceArena {
    pub fn new(_capacity: usize) -> Self {
        Self
    }

    pub fn append(&mut self, _samples: &[f32]) {}

    pub fn fail(&mut self) {}

    pub fn finish(&mut self) -> CaptureTrace {}
}

/// One activation's model evaluation log.
pub(crate) struct Log;

impl Log {
    pub fn record(
        &mut self,
        _request: u64,
        _revision: u64,
        _input_end_sample: u64,
        _probability: f32,
        _processing_ms: f64,
        _positive: bool,
    ) -> Option<usize> {
        None
    }
}

/// Mark current positive evidence as accepted.
pub(crate) fn mark_accepted(_log: &mut Log, _index: Option<usize>) {}

/// Mark evidence invalidated by resumed speech.
pub(crate) fn mark_stale(_log: &mut Log, _index: Option<usize>) {}

/// Mark evidence abandoned by a non-complete finish.
pub(crate) fn mark_discarded(_log: &mut Log, _index: Option<usize>) {}
