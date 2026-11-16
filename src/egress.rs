//! The gate's egress: a second service from the same image, holding nothing, that carries
//! the gate's one call to plex.tv out of the machine.
//!
//! The gate keeps no route off the host. It reaches plex.tv only through this service,
//! over a network the two share alone, as an HTTPS proxy: the gate asks for a tunnel to
//! plex.tv's port 443 and runs its own TLS to plex.tv through it, so nothing here reads what
//! it carries. A tunnel to anywhere else, and every other request, is refused and carried
//! nowhere. It answers its own health on the same port.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Where the egress listens inside its container, which only the gate reaches.
pub(crate) const PORT: u16 = 3128;

/// The one place it carries anything to, as a tunnel names it.
pub(crate) const PLEX_TV: &str = "plex.tv:443";

/// The most of a request's head it reads before it refuses it.
const HEAD: usize = 8 * 1024;

/// How long a request's head may take to arrive.
const PATIENCE: Duration = Duration::from_secs(10);

/// What a tunnel opened is answered with.
const OPENED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

/// What its health is answered with.
const HEALTHY: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// What every other request is answered with.
const REFUSED: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// What a tunnel to plex.tv that could not be opened is answered with.
const UNREACHED: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// What one request asks of the egress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asked {
    /// A tunnel to plex.tv's port 443.
    Tunnel,
    /// Its health.
    Health,
    /// Anything else.
    Other,
}

/// What the request whose head is `head` asks: a tunnel to plex.tv's port 443 named exactly,
/// its health, or anything else.
pub(crate) fn asked(head: &str) -> Asked {
    let mut words = head.lines().next().unwrap_or_default().split(' ');
    match (words.next(), words.next(), words.next(), words.next()) {
        (Some("CONNECT"), Some(authority), Some(version), None)
            if authority.eq_ignore_ascii_case(PLEX_TV) && version.starts_with("HTTP/1.") =>
        {
            Asked::Tunnel
        }
        (Some("GET"), Some(crate::serving::HEALTH), Some(_), None) => Asked::Health,
        _ => Asked::Other,
    }
}

/// What tells the egress to stop.
pub(crate) type Stopping = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What hands the egress each connection to carry.
pub(crate) trait Accepting: Sync {
    /// The next connection, or why none can be taken.
    fn accepted(&self) -> Pin<Box<dyn Future<Output = std::io::Result<TcpStream>> + Send + '_>>;
}

impl Accepting for TcpListener {
    fn accepted(&self) -> Pin<Box<dyn Future<Output = std::io::Result<TcpStream>> + Send + '_>> {
        Box::pin(async move { self.accept().await.map(|(stream, _)| stream) })
    }
}

/// Carry tunnels to plex.tv, reached at `plex_tv`, for each connection on `at`, until one
/// cannot be taken or `stopped` resolves; why it stopped, or nothing where it was told to.
pub(crate) async fn serve_at(
    at: SocketAddr,
    plex_tv: String,
    stopped: Stopping,
) -> Option<std::io::Error> {
    match TcpListener::bind(at).await {
        Ok(listener) => serve(&listener, plex_tv, stopped).await,
        Err(error) => Some(error),
    }
}

/// Carry tunnels to plex.tv, reached at `plex_tv`, for each connection `accepting` hands
/// over, until one cannot be taken or `stopped` resolves; why it stopped, or nothing where
/// it was told to.
pub(crate) async fn serve(
    accepting: &dyn Accepting,
    plex_tv: String,
    mut stopped: Stopping,
) -> Option<std::io::Error> {
    loop {
        tokio::select! {
            () = &mut stopped => return None,
            accepted = accepting.accepted() => match accepted {
                Ok(stream) => spawned(stream, &plex_tv),
                Err(error) => return Some(error),
            },
        }
    }
}

/// Carry the connection `stream` on a task of its own, to plex.tv at `plex_tv`.
fn spawned(stream: TcpStream, plex_tv: &str) {
    let plex_tv = plex_tv.to_owned();
    tokio::spawn(async move { carried(stream, &plex_tv).await });
}

/// Answer the one request `stream` makes, carrying it to plex.tv at `plex_tv` where it asks
/// for a tunnel there.
async fn carried(mut stream: TcpStream, plex_tv: &str) {
    let Ok(Some((head, rest))) = tokio::time::timeout(PATIENCE, head_of(&mut stream))
        .await
        .unwrap_or(Ok(None))
    else {
        let _ = stream.write_all(REFUSED).await;
        return;
    };
    match asked(&head) {
        Asked::Health => {
            let _ = stream.write_all(HEALTHY).await;
        }
        Asked::Other => {
            let _ = stream.write_all(REFUSED).await;
        }
        Asked::Tunnel => {
            let Ok(mut upstream) = TcpStream::connect(plex_tv).await else {
                let _ = stream.write_all(UNREACHED).await;
                return;
            };
            let _ = tunnelled(&mut stream, &mut upstream, &rest).await;
        }
    }
}

/// Open the tunnel on `stream`, hand `upstream` what came after the request's head, then
/// carry each side to the other until either closes.
async fn tunnelled(
    stream: &mut TcpStream,
    upstream: &mut TcpStream,
    rest: &[u8],
) -> std::io::Result<(u64, u64)> {
    stream.write_all(OPENED).await?;
    upstream.write_all(rest).await?;
    tokio::io::copy_bidirectional(stream, upstream).await
}

/// The head of the request `stream` makes, and whatever came after it, or none where it
/// does not end within [`HEAD`] bytes.
async fn head_of(stream: &mut TcpStream) -> std::io::Result<Option<(String, Vec<u8>)>> {
    let mut read = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(end) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = read.split_off(end + 4);
            return Ok(String::from_utf8(read).ok().map(|head| (head, rest)));
        }
        if read.len() > HEAD {
            return Ok(None);
        }
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Ok(None);
        }
        read.extend_from_slice(chunk.get(..count).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests;
