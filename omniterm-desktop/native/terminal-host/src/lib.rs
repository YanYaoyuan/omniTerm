//! PTY sessions, the terminal screen model and the Agent operation broker.
//!
//! Electron runs this as a child process (see `main.rs`). The native app links it directly and
//! talks to the same [`host::Host`] through the same requests, without a process in between.

pub mod broker;
pub mod automation;
pub mod host;
pub mod interaction;
pub mod protocol;
pub mod redact;
pub mod screen;
pub mod session;

/// Version of the request and frame protocol.
pub const PROTOCOL_VERSION: u32 = 1;

/// The readiness script shared by the native UI and the external CLI.
pub fn shell_integration(nonce: &str) -> Option<String> {
    let valid = nonce.len() == 32
        && nonce.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    valid.then(|| include_str!("../assets/shell-integration.txt").replace("f00df00df00df00df00df00df00df00d", nonce))
}
