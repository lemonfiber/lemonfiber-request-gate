//! The request gate: the request service's one path to the *arrs and the media server.
//!
//! It runs in the stack beside the request service, publishes no port, and is
//! reachable only from that service (ADR-0032). This build answers its health and
//! refuses every other request.

mod serving;

use std::net::SocketAddr;
use std::process::ExitCode;

/// Where the gate listens inside its container. Nothing publishes it: the request
/// service reaches it over the one network the two share.
const LISTEN: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8080);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => serving::serve(LISTEN).await,
        Some("health") => serving::healthy(LISTEN.port()).await,
        Some(other) => {
            eprintln!("request-gate: no command called {other}; run it bare to serve, or `health` to ask whether it is serving");
            ExitCode::from(2)
        }
    }
}
