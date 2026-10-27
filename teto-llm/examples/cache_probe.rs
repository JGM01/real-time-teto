//! Run the six-case cache probe on this host. Requires the desktop model.
use std::{io::stdout, path::PathBuf, process::Command};
use teto_llm::*;

fn output(program: &str, arguments: &[&str]) -> String {
    String::from_utf8(
        Command::new(program)
            .args(arguments)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned()
}

fn main() -> Result<()> {
    let assets = PathBuf::from(std::env::var_os("TETO_LLM_ASSETS").expect("set TETO_LLM_ASSETS"));
    let host = HostIdentity::new(
        output("/usr/sbin/sysctl", &["-n", "hw.model"]),
        output("/usr/bin/sw_vers", &["-productVersion"]),
    );
    let qualified = cache_probe::run(&assets, ProfileId::Desktop, &host, &mut stdout().lock())?;
    if !qualified {
        return Err(failure(ErrorKind::Generation));
    }
    Ok(())
}
