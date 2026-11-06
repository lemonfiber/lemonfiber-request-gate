use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::CertificateDer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

use super::{configured, connection};
use crate::fake::Config;

/// A certificate naming plex.tv, written beneath `config` as the core writes it, and its
/// DER, which a client trusts.
fn certified(config: &Path) -> Option<CertificateDer<'static>> {
    let made = rcgen::generate_simple_self_signed(vec!["plex.tv".to_owned()]).ok()?;
    let directory = config.join("plex-tv");
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::write(directory.join("certificate.pem"), made.cert.pem()).ok()?;
    std::fs::write(directory.join("key.pem"), made.signing_key.serialize_pem()).ok()?;
    Some(made.cert.der().clone())
}

/// What `at` answers over TLS to `GET /` asked of plex.tv, trusting `trusted` alone.
async fn asked(at: SocketAddr, trusted: CertificateDer<'static>) -> Option<String> {
    let mut roots = RootCertStore::empty();
    roots.add(trusted).ok()?;
    let client =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .ok()?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let stream = TcpStream::connect(at).await.ok()?;
    let mut stream = TlsConnector::from(Arc::new(client))
        .connect(ServerName::try_from("plex.tv").ok()?, stream)
        .await
        .ok()?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: plex.tv\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.ok()?;
    Some(String::from_utf8_lossy(&answer).into_owned())
}

fn answering() -> Router {
    Router::new().fallback(|| async { "answered" })
}

async fn served(config: &Config) -> Option<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .ok()?;
    let at = listener.local_addr().ok()?;
    let path = config.path();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let path = path.clone();
            tokio::spawn(async move { connection(stream, &path, answering()).await });
        }
    });
    Some(at)
}

#[tokio::test]
async fn the_front_answers_over_tls_on_the_certificate_the_core_wrote() {
    let config = Config::new("front-tls");
    let trusted = certified(&config.path());
    let at = served(&config).await;

    let answer = match (at, trusted) {
        (Some(at), Some(trusted)) => asked(at, trusted).await,
        _ => None,
    };

    let answer = answer.unwrap_or_default();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("answered"), "{answer}");
}

#[tokio::test]
async fn with_no_certificate_written_a_connection_is_closed_unanswered() {
    let config = Config::new("front-no-tls");
    let elsewhere = Config::new("front-trusted-elsewhere");
    let trusted = certified(&elsewhere.path());
    let at = served(&config).await;

    let answer = match (at, trusted) {
        (Some(at), Some(trusted)) => asked(at, trusted).await,
        _ => Some(String::new()),
    };

    assert_eq!(answer, None);
}

#[test]
fn a_certificate_and_key_are_configured_only_where_both_are_there_and_read() {
    let config = Config::new("front-configured");
    assert!(configured(&config.path()).is_none());

    let directory = config.path().join("plex-tv");
    let _ = std::fs::create_dir_all(&directory);
    let _ = std::fs::write(directory.join("certificate.pem"), "not a certificate");
    let _ = std::fs::write(directory.join("key.pem"), "not a key");
    assert!(configured(&config.path()).is_none());

    let _ = certified(&config.path());
    assert!(configured(&config.path()).is_some());
    let _ = std::fs::write(directory.join("key.pem"), "not a key");
    assert!(configured(&config.path()).is_none());
}

#[tokio::test]
async fn a_connection_that_is_no_tls_is_closed_unanswered_and_the_front_goes_on() {
    let config = Config::new("front-plain");
    let trusted = certified(&config.path());
    let Some(at) = served(&config).await else {
        return;
    };

    let mut plain = TcpStream::connect(at).await.ok();
    if let Some(plain) = plain.as_mut() {
        let _ = plain
            .write_all(b"GET / HTTP/1.1\r\nHost: plex.tv\r\n\r\n")
            .await;
        let mut answer = Vec::new();
        let _ = plain.read_to_end(&mut answer).await;
        assert!(!String::from_utf8_lossy(&answer).contains("answered"));
    }

    let answer = match trusted {
        Some(trusted) => asked(at, trusted).await,
        None => None,
    };
    assert!(answer.unwrap_or_default().ends_with("answered"));
}
