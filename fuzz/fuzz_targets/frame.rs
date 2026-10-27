#![no_main]
use libfuzzer_sys::fuzz_target;
use teto_llm_protocol::Request;

fuzz_target!(|data: &[u8]| {
    let _ = teto_llm_protocol::read_ipc::<Request>(&mut &data[..]);
});
