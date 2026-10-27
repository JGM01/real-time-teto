//! Thin, isolated iOS C ABI. All engine logic stays in teto-llm.
use std::{
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use teto_llm::*;

static STOP: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Cancel at the next clause boundary; the engine's immediate cancellation probe is separate.
#[unsafe(no_mangle)]
pub extern "C" fn teto_phone_cancel() {
    STOP.store(true, Ordering::Release);
}

/// Run on a background thread. Return zero on success, one on a bounded failure.
///
/// # Safety
/// Both pointer/length pairs must reference valid UTF-8 for the duration of the call.
/// Neither length may exceed 4096. `report` must name a new file in the app sandbox.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn teto_phone_spike(
    assets: *const u8,
    assets_len: usize,
    report: *const u8,
    report_len: usize,
) -> i32 {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return 1;
    }
    let _guard = RunGuard;
    STOP.store(false, Ordering::Release);
    std::panic::catch_unwind(|| {
        if assets.is_null() || report.is_null() || assets_len > 4096 || report_len > 4096 {
            return 1;
        }
        // SAFETY: the caller owns the live Swift UTF-8 buffers until this synchronous call returns.
        let assets = unsafe { std::slice::from_raw_parts(assets, assets_len) };
        let report = unsafe { std::slice::from_raw_parts(report, report_len) };
        let (Ok(assets), Ok(report)) = (std::str::from_utf8(assets), std::str::from_utf8(report))
        else {
            return 1;
        };
        measure(PathBuf::from(assets), PathBuf::from(report))
            .map_or(1, |()| if STOP.load(Ordering::Acquire) { 2 } else { 0 })
    })
    .unwrap_or(1)
}
fn measure(assets: PathBuf, report: PathBuf) -> Result<()> {
    let config = OpenConfig {
        assets,
        profile: ProfileId::phone_spike(),
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 7,
        journal: JournalAccess::Disabled,
    };
    let mut report = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report)
        .map_err(|_| failure(ErrorKind::Worker))?;
    let result = qualification::measure(
        config,
        &mut report,
        &STOP,
        policy::PHONE_SPIKE_TURNS,
        Duration::from_secs(policy::PHONE_SPIKE_SECONDS),
    );
    if let Err(error) = &result {
        let _ = writeln!(
            report,
            "{{\"spike_error\":\"{:?}\",\"qualified\":false}}",
            error.kind()
        );
        let _ = report.flush();
    }
    result
}
