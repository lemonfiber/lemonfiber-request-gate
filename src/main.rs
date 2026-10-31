//! The request gate: the request service's one path to the *arrs and the media server.
//!
//! It runs in the stack beside the request service, publishes no port, and is
//! reachable only from that service. It holds the upstreams' credentials, answers a
//! fixed list of calls it builds itself, and refuses and records every other. It speaks
//! `request.gate` to the core on a port of its own.

mod arr;
mod asked;
#[cfg(test)]
mod fake;
mod files;
mod formats;
mod holding;
mod jellyfin;
mod serving;
mod settings;
mod shape;
mod speaking;
mod upstream;
mod version;

use std::net::SocketAddr;
use std::process::ExitCode;

/// Where the gate listens inside its container. Nothing publishes it: the request
/// service reaches it over the one network the two share.
const LISTEN: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    crate::formats::gate::PORT,
);

/// Where the gate speaks `request.gate` inside its container, which the stack publishes
/// on loopback alone.
const CONTRACT: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 5058);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {
            let settings = settings::Settings::from(|name| std::env::var(name).ok());
            let config = settings.config.clone();
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
            tokio::select! {
                code = serving::serve(LISTEN, service, stopped) => code,
                stopped = speaking::serve(CONTRACT, &config) => {
                    eprintln!("request-gate: request.gate is not answered on {CONTRACT}: {stopped:?}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("health") => serving::healthy(LISTEN.port()).await,
        Some("version") => {
            println!("request-gate {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("request-gate: no command called {other}; run it bare to serve, `health` to ask whether it is serving, or `version` to print its version");
            ExitCode::from(2)
        }
    }
}
