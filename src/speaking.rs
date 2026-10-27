//! The contract the gate speaks to the core, on a port of its own.
//!
//! It answers `request.gate` under the plugin's key, on a port published on loopback
//! alone, apart from the port the request service reaches it on.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use lemonfiber_adapter_kit::Kit;
use lemonfiber_contract::adapter::About;
use lemonfiber_contract::capabilities::request::gate;

use crate::holding::Holding;

/// What the gate calls itself as the upstream it answers for.
pub(crate) const UPSTREAM: &str = "lemonfiber request gate";

/// What the gate says it is: the one contract it speaks, for itself.
pub(crate) fn about() -> About {
    About {
        speaks: vec![lemonfiber_contract::spoken(gate::CAPABILITY, gate::MAJOR)],
        upstream: UPSTREAM.to_owned(),
        releases: Vec::new(),
    }
}

/// `request.gate`, served under `key` from what is held in `config`.
pub(crate) fn kit(key: String, config: &Path) -> Kit {
    Kit::new(key, about()).serving(gate::served(Arc::new(Holding::in_directory(config))))
}

/// Answer the core on `at` under the plugin's key in `config`, for as long as the gate
/// runs: it ends only where `at` cannot be served, with why.
///
/// Where the core has written no key, nothing answers there: a contract asked with no key
/// to check it against would answer anybody.
pub(crate) async fn serve(at: SocketAddr, config: &Path) -> std::io::Result<()> {
    let Some(key) = lemonfiber_adapter_kit::key_in(config)
        .ok()
        .filter(|key| !key.is_empty())
    else {
        eprintln!("request-gate: the core has written no key, so request.gate is not answered");
        return std::future::pending().await;
    };
    let listener = tokio::net::TcpListener::bind(at).await?;
    lemonfiber_adapter_kit::serving(kit(key, config), listener, std::future::pending()).await
}

#[cfg(test)]
mod tests;
