//! Terminal backend of omniTerm Desktop, run as a child process of the Electron main process.

use omniterm_terminal_host::PROTOCOL_VERSION;
use omniterm_terminal_host::host::Host;
use omniterm_terminal_host::protocol::Outbox;
use omniterm_terminal_host::protocol::Request;
use omniterm_terminal_host::protocol::write_frames;
use std::io::BufRead;
use std::thread;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--protocol-version") {
        println!("{PROTOCOL_VERSION}");
        return;
    }
    let (outbox, frames) = Outbox::channel();
    thread::spawn(move || write_frames(frames, std::io::stdout().lock()));
    let host = Host::new(outbox.clone());
    outbox.event("host.ready", serde_json::json!({ "protocolVersion": PROTOCOL_VERSION }));
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        match serde_json::from_str::<Request>(&line) {
            Ok(request) => outbox.respond(request.id, host.handle(&request.method, request.params)),
            Err(error) => outbox.event("host.error", serde_json::json!({ "message": error.to_string() })),
        }
    }
    // Stdin closes when Electron exits, even if it crashed: no PTY may outlive the app.
    host.shutdown();
}
