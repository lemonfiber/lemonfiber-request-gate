//! plex.tv's front over TLS, on the certificate the core makes for it.
//!
//! In `fronted` mode the core writes a certificate naming only plex.tv's hosts, signed by
//! an authority it makes for the gate alone, and its key, beneath the configuration
//! directory. Each connection is accepted on them as they are then, so a certificate the
//! core replaces is the one the next connection meets, and where there is none the
//! connection is closed unanswered.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

/// Where beneath the configuration directory the certificate and its key are.
const DIRECTORY: &str = "plex-tv";

/// The certificate, its chain after it.
const CERTIFICATE: &str = "certificate.pem";

/// Its private key.
const KEY: &str = "key.pem";

/// The certificate and key beneath `config`, as a server's TLS configuration, where both
/// are there and read as one.
pub(crate) fn configured(config: &Path) -> Option<Arc<ServerConfig>> {
    let directory = config.join(DIRECTORY);
    let certificates = CertificateDer::pem_file_iter(directory.join(CERTIFICATE))
        .ok()?
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .filter(|certificates| !certificates.is_empty())?;
    let key = PrivateKeyDer::from_pem_file(directory.join(KEY)).ok()?;
    ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .ok()?
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .ok()
        .map(Arc::new)
}

/// Answer `router` over TLS on `stream`, on the certificate beneath `config` as it is
/// now; where there is none, or the other end speaks no TLS, it is closed unanswered.
pub(crate) async fn connection(stream: TcpStream, config: &Path, router: Router) {
    let Some(tls) = configured(config) else {
        return;
    };
    let Ok(stream) = TlsAcceptor::from(tls).accept(stream).await else {
        return;
    };
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(router))
        .await;
}

#[cfg(test)]
mod tests;
