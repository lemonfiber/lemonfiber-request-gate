use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use crate::formats::gate::File;
use crate::formats::TokenHash;
use lemonfiber_adapter_kit::web::Web;
use lemonfiber_contract::capabilities::request::gate;
use lemonfiber_contract::Contracted;
use lemonfiber_ports::service::{Gated, Gating, Target};

use super::{about, kit, serve, UPSTREAM};

struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("gate-speaking-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    fn keyed(&self) {
        let _ = std::fs::write(
            self.0.join(lemonfiber_contract::adapter::KEY_FILE),
            format!("{}\n", plugin_key()),
        );
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plugin_key() -> String {
    ('a'..='p').chain('A'..='P').collect()
}

fn media_server() -> Gated {
    Gated {
        target: Target::MediaServer {
            pins: vec!["ef".repeat(32)],
        },
        path: "jellyfin".to_owned(),
        address: "https://jellyfin:8920".to_owned(),
        majors: vec![10],
        credential: ('k'..='z').collect(),
        tokens: vec![TokenHash::of("token").as_str().to_owned()],
    }
}

async fn served(config: &Config) -> Option<SocketAddr> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .ok()?;
    let at = listener.local_addr().ok()?;
    tokio::spawn(lemonfiber_adapter_kit::serving(
        kit(plugin_key(), &config.0),
        listener,
        std::future::pending(),
    ));
    Some(at)
}

fn asked(at: SocketAddr, key: &str) -> Option<gate::Adapter> {
    let web = Web::new().ok()?;
    Some(gate::Adapter(Contracted::new(
        Arc::new(web),
        format!("http://{at}"),
        "request-gate",
        key,
    )))
}

fn free() -> Option<SocketAddr> {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|probe| probe.local_addr())
        .ok()
}

#[test]
fn it_says_it_speaks_request_gate_for_itself() {
    let about = about();

    assert_eq!(about.speaks, vec!["request.gate@1".to_owned()]);
    assert_eq!(about.upstream, UPSTREAM);
}

#[tokio::test]
async fn the_core_admits_routes_and_reads_them_back_over_the_contract() {
    let config = Config::new("contract");
    let core = served(&config)
        .await
        .and_then(|at| asked(at, &plugin_key()));
    assert!(core.is_some());
    let Some(core) = core else { return };

    assert!(core.admit(&[media_server()]).await.is_ok());
    let admitted = core.admitted().await.unwrap_or_default();

    assert_eq!(admitted.len(), 1);
    assert_eq!(
        admitted.first().map(|one| one.credential.clone()),
        Some(
            TokenHash::of(&media_server().credential)
                .as_str()
                .to_owned()
        )
    );
}

#[tokio::test]
async fn a_caller_without_the_plugins_key_is_refused_and_changes_nothing() {
    let config = Config::new("unkeyed");
    let stranger = served(&config)
        .await
        .and_then(|at| asked(at, "not-the-key"));
    assert!(stranger.is_some());
    let Some(stranger) = stranger else { return };

    assert!(stranger.admit(&[media_server()]).await.is_err());
    assert!(stranger.admitted().await.is_err());
    assert_eq!(config.read(File::Upstreams), "");
}

#[tokio::test]
async fn with_no_key_written_nothing_answers_on_the_contract_port() {
    let config = Config::new("keyless");
    let at = free();
    assert!(at.is_some());
    let Some(at) = at else { return };

    let waited =
        tokio::time::timeout(std::time::Duration::from_millis(200), serve(at, &config.0)).await;

    assert!(waited.is_err());
    assert!(tokio::net::TcpStream::connect(at).await.is_err());
}

#[tokio::test]
async fn with_its_key_written_it_answers_the_core_on_the_contract_port() {
    let config = Config::new("serving");
    config.keyed();
    let at = free();
    assert!(at.is_some());
    let Some(at) = at else { return };

    let path = config.0.clone();
    let running = tokio::spawn(async move { serve(at, &path).await });
    let core = asked(at, &plugin_key());
    let mut admitted = None;
    for _ in 0..250 {
        if let Some(core) = &core {
            if let Ok(answer) = core.admitted().await {
                admitted = Some(answer);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    running.abort();

    assert_eq!(admitted, Some(Vec::new()));
}

#[tokio::test]
async fn a_contract_port_already_taken_is_a_failure_to_serve() {
    let config = Config::new("taken");
    config.keyed();
    let taken = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await;
    assert!(taken.is_ok());
    let Ok(taken) = taken else { return };
    let Ok(at) = taken.local_addr() else { return };

    assert!(serve(at, &config.0).await.is_err());
}
