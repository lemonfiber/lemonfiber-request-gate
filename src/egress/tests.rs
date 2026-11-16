use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{asked, serve, serve_at, Accepting, Asked, OPENED};

#[test]
fn only_a_tunnel_to_plex_tvs_port_443_named_exactly_is_carried() {
    for head in [
        "CONNECT plex.tv:443 HTTP/1.1\r\nHost: plex.tv:443\r\n\r\n",
        "CONNECT PLEX.TV:443 HTTP/1.0\r\n\r\n",
    ] {
        assert_eq!(asked(head), Asked::Tunnel, "{head}");
    }
    for head in [
        "CONNECT discover.provider.plex.tv:443 HTTP/1.1\r\n\r\n",
        "CONNECT plex.tv:80 HTTP/1.1\r\n\r\n",
        "CONNECT plex.tv HTTP/1.1\r\n\r\n",
        "CONNECT plex.tv:443.example.org:443 HTTP/1.1\r\n\r\n",
        "CONNECT example.org:443 HTTP/1.1\r\n\r\n",
        "CONNECT plex.tv:443 HTTP/1.1 extra\r\n\r\n",
        "CONNECT plex.tv:443 SPDY/3\r\n\r\n",
        "GET https://plex.tv/users/account.json HTTP/1.1\r\n\r\n",
        "POST /health HTTP/1.1\r\n\r\n",
        "",
    ] {
        assert_eq!(asked(head), Asked::Other, "{head:?}");
    }
    assert_eq!(asked("GET /health HTTP/1.1\r\n\r\n"), Asked::Health);
}

/// An upstream standing in for plex.tv that echoes whatever it is sent, and the egress in
/// front of it.
async fn egress() -> Option<(SocketAddr, TcpListener)> {
    let upstream = TcpListener::bind("127.0.0.1:0").await.ok()?;
    let egress = TcpListener::bind("127.0.0.1:0").await.ok()?;
    let reach = upstream.local_addr().ok()?.to_string();
    let at = egress.local_addr().ok()?;
    tokio::spawn(async move { serve(&egress, reach, Box::pin(std::future::pending())).await });
    Some((at, upstream))
}

/// Everything `at` answers to `asked`, once it closes or `read` bytes have come.
async fn answer(at: SocketAddr, asked: &[u8], read: usize) -> Vec<u8> {
    let Ok(mut stream) = TcpStream::connect(at).await else {
        return Vec::new();
    };
    let _ = stream.write_all(asked).await;
    let mut answer = vec![0_u8; read];
    let mut got = 0;
    while got < read {
        match stream.read(answer.get_mut(got..).unwrap_or_default()).await {
            Ok(0) | Err(_) => break,
            Ok(count) => got += count,
        }
    }
    answer.truncate(got);
    answer
}

#[tokio::test]
async fn a_tunnel_to_plex_tv_carries_what_is_sent_both_ways_and_reads_none_of_it() {
    let served = egress().await;
    assert!(served.is_some());
    let Some((at, upstream)) = served else {
        return;
    };
    let echoing = tokio::spawn(async move {
        if let Ok((mut stream, _)) = upstream.accept().await {
            let mut sent = [0_u8; 5];
            if stream.read_exact(&mut sent).await.is_ok() {
                let _ = stream.write_all(&sent).await;
            }
            let mut after = Vec::new();
            let _ = stream.read_to_end(&mut after).await;
        }
    });

    let answered = answer(
        at,
        b"CONNECT plex.tv:443 HTTP/1.1\r\nHost: plex.tv:443\r\n\r\nhello",
        OPENED.len() + 5,
    )
    .await;

    assert_eq!(answered, [OPENED, b"hello"].concat());
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), echoing)
            .await
            .is_ok(),
        "the tunnel was never closed upstream once the gate closed its side"
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test]
async fn a_tunnel_anywhere_else_or_any_other_request_is_refused_and_carried_nowhere() {
    let served = egress().await;
    assert!(served.is_some());
    let Some((at, upstream)) = served else {
        return;
    };

    for asked in [
        b"CONNECT example.org:443 HTTP/1.1\r\n\r\n".as_slice(),
        b"CONNECT discover.provider.plex.tv:443 HTTP/1.1\r\n\r\n",
        b"GET http://plex.tv/ HTTP/1.1\r\nHost: plex.tv\r\n\r\n",
        b"no request at all",
    ] {
        let answered = answer(at, asked, 64).await;
        assert!(
            answered.starts_with(b"HTTP/1.1 403"),
            "{}",
            String::from_utf8_lossy(asked)
        );
    }
    let accepted =
        tokio::time::timeout(std::time::Duration::from_millis(200), upstream.accept()).await;
    assert!(accepted.is_err());
}

#[tokio::test]
async fn its_health_is_answered_on_its_own_port() {
    let served = egress().await;
    assert!(served.is_some());
    let Some((at, _upstream)) = served else {
        return;
    };

    let answered = answer(at, b"GET /health HTTP/1.1\r\nHost: egress\r\n\r\n", 64).await;

    assert!(answered.starts_with(b"HTTP/1.1 200"));
}

#[tokio::test]
async fn a_tunnel_to_a_plex_tv_that_does_not_answer_is_answered_bad_gateway() {
    let Some(nobody) = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok())
    else {
        return;
    };
    let Ok(egress) = TcpListener::bind("127.0.0.1:0").await else {
        return;
    };
    let Ok(at) = egress.local_addr() else {
        return;
    };
    tokio::spawn(async move {
        serve(
            &egress,
            nobody.to_string(),
            Box::pin(std::future::pending()),
        )
        .await
    });

    let answered = answer(at, b"CONNECT plex.tv:443 HTTP/1.1\r\n\r\n", 64).await;

    assert!(answered.starts_with(b"HTTP/1.1 502"));
}

#[tokio::test]
async fn a_head_too_long_or_cut_short_is_refused_and_carried_nowhere() {
    let served = egress().await;
    assert!(served.is_some());
    let Some((at, _upstream)) = served else {
        return;
    };
    let long = [b"GET /".as_slice(), &[b'a'; 9 * 1024]].concat();
    assert!(answer(at, &long, 64).await.starts_with(b"HTTP/1.1 403"));

    let Ok(mut stream) = TcpStream::connect(at).await else {
        return;
    };
    let _ = stream.write_all(b"GET /hea").await;
    let _ = stream.shutdown().await;
    let mut answered = Vec::new();
    let _ = stream.read_to_end(&mut answered).await;
    assert!(answered.starts_with(b"HTTP/1.1 403"));
}

/// What hands over no connection, only why.
struct Failing;

impl Accepting for Failing {
    fn accepted(&self) -> Pin<Box<dyn Future<Output = std::io::Result<TcpStream>> + Send + '_>> {
        Box::pin(std::future::ready(Err(std::io::Error::other(
            "no connection to take",
        ))))
    }
}

#[tokio::test]
async fn a_connection_that_cannot_be_taken_stops_it_saying_why() {
    let stopped = serve(&Failing, String::new(), Box::pin(std::future::pending())).await;

    assert_eq!(
        stopped.map(|why| why.to_string()),
        Some("no connection to take".to_owned())
    );
}

#[tokio::test]
async fn told_to_stop_it_stops_with_nothing_wrong() {
    let Ok(listener) = TcpListener::bind("127.0.0.1:0").await else {
        return;
    };

    assert!(
        serve(&listener, String::new(), Box::pin(std::future::ready(())))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn an_address_already_taken_is_named_as_why_it_cannot_serve() {
    let Ok(taken) = TcpListener::bind("127.0.0.1:0").await else {
        return;
    };
    let Ok(at) = taken.local_addr() else {
        return;
    };

    assert!(
        serve_at(at, String::new(), Box::pin(std::future::pending()))
            .await
            .is_some()
    );
    assert!(serve_at(
        "127.0.0.1:0".parse().unwrap_or(at),
        String::new(),
        Box::pin(std::future::ready(()))
    )
    .await
    .is_none());
}
