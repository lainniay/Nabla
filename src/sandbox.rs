pub mod detect;
pub mod linux_bwrap;
pub mod macos_seatbelt;
pub mod process;
pub mod profile;
pub mod request;

pub use detect::{SandboxCapability, detect};
pub use request::{FilesystemProfile, NetworkProfile, SandboxExecRequest, SandboxProfile};

pub fn run_probe() -> Result<(), String> {
    let capability = detect();
    println!(
        "{}",
        serde_json::to_string(&capability).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn run_exec() -> Result<(), String> {
    let request = request::read_request()?;
    let compiled = profile::compile(&request)?;
    let code = process::run(&request, &compiled)?;
    std::process::exit(code);
}
