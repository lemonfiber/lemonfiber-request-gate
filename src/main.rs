//! The request gate: the request service's one path to the *arrs and the media server.
//!
//! It runs in the stack beside the request service, publishes no port, and is
//! reachable only from that service. It holds the upstreams' credentials, answers a
//! fixed list of calls it builds itself, and refuses and records every other.

mod arr;
mod asked;
#[cfg(test)]
mod fake;
mod files;
mod jellyfin;
mod serving;
mod settings;
mod shape;
mod upstream;
mod version;

use std::net::SocketAddr;
use std::process::ExitCode;

/// Where the gate listens inside its container. Nothing publishes it: the request
/// service reaches it over the one network the two share.
const LISTEN: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    lemonfiber_sidecar::gate::PORT,
);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {
            let settings = settings::Settings::from(|name| std::env::var(name).ok());
            let service = match serving::client() {
                Ok(client) => serving::Service::new(settings, client),
                Err(error) => {
                    eprintln!(
                        "request-gate: could not make the client every call is sent with: {error}"
                    );
                    return ExitCode::FAILURE;
                }
            };
            let stopped = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            serving::serve(LISTEN, service, stopped).await
        }
        Some("health") => serving::healthy(LISTEN.port()).await,
        Some(other) => {
            eprintln!("request-gate: no command called {other}; run it bare to serve, or `health` to ask whether it is serving");
            ExitCode::from(2)
        }
    }
}
