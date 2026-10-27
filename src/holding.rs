//! What the core hands the gate over `request.gate`, held in its configuration directory:
//! the routes it admits with the credential it presents on each, and the hashes of the
//! tokens it admits there.
//!
//! Each file is written whole, from a copy made afresh and renamed over it, so a call never
//! reads half of one, and readable by the gate's own user alone.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use lemonfiber_ports::media;
use lemonfiber_ports::service::{Admitted, Failure, Gated, Gating, Target};
use lemonfiber_sidecar::gate::{Accepted, Credential, File, Kind, Tokens, Upstream, Upstreams};
use lemonfiber_sidecar::TokenHash;

/// The name the gate gives itself in a failure.
const SERVICE: &str = "request-gate";

/// How many lower-case hexadecimal digits a token's SHA-256 is written in.
const HASH_DIGITS: usize = 64;

/// Who may read what it holds: the gate's own user alone.
const OWNER_ONLY: u32 = 0o600;

/// The routes and the tokens, held in the configuration directory `config`, written one at
/// a time.
pub(crate) struct Holding {
    config: PathBuf,
    writing: tokio::sync::Mutex<()>,
}

impl Holding {
    /// What is held in `config`.
    pub(crate) fn in_directory(config: &Path) -> Self {
        Self {
            config: config.to_path_buf(),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    /// The text of `file`, where it can be read.
    async fn read(&self, file: File) -> Option<String> {
        tokio::fs::read_to_string(self.config.join(file.name()))
            .await
            .ok()
    }

    /// Write each of `files` whole, from a copy made afresh, so a copy left behind lends it
    /// neither its permissions nor a link.
    async fn written(&self, files: Vec<(File, String)>) -> Result<(), Failure> {
        let _writing = self.writing.lock().await;
        let config = self.config.clone();
        let done = tokio::task::spawn_blocking(move || {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            for (file, text) in files {
                let record = config.join(file.name());
                let writing = config.join(format!("{}.writing", file.name()));
                std::fs::remove_file(&writing).or_else(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })?;
                let mut out = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(OWNER_ONLY)
                    .open(&writing)?;
                out.write_all(text.as_bytes())?;
                out.sync_all()?;
                std::fs::rename(&writing, &record)?;
            }
            Ok::<(), std::io::Error>(())
        })
        .await;
        done.ok()
            .and_then(Result::ok)
            .ok_or_else(|| refused("the routes could not be written"))
    }
}

#[async_trait]
impl Gating for Holding {
    async fn admit(&self, routes: &[Gated]) -> Result<(), Failure> {
        let upstreams = Upstreams::of(routes.iter().map(upstream).collect::<Result<_, _>>()?);
        let tokens = Tokens::of(routes.iter().map(accepted).collect::<Result<_, _>>()?);
        self.written(vec![
            (File::Tokens, tokens.written()),
            (File::Upstreams, upstreams.written()),
        ])
        .await
    }

    async fn admitted(&self) -> Result<Vec<Admitted>, Failure> {
        let upstreams = self
            .read(File::Upstreams)
            .await
            .and_then(|text| Upstreams::read(&text).ok())
            .map(|read| read.upstreams)
            .unwrap_or_default();
        let tokens = self
            .read(File::Tokens)
            .await
            .and_then(|text| Tokens::read(&text).ok())
            .map(|read| read.routes)
            .unwrap_or_default();
        Ok(upstreams
            .iter()
            .map(|upstream| {
                let admits = tokens
                    .iter()
                    .find(|one| one.route == upstream.route)
                    .map(|one| {
                        one.tokens
                            .iter()
                            .map(|hash| hash.as_str().to_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                Admitted::of(&Gated {
                    target: target(upstream.kind),
                    path: upstream.route.clone(),
                    address: upstream.address.clone(),
                    majors: upstream.majors.clone(),
                    credential: upstream.credential.reveal().to_owned(),
                    tokens: admits,
                })
            })
            .collect())
    }
}

/// `route` as the routes file holds it.
fn upstream(route: &Gated) -> Result<Upstream, Failure> {
    Ok(Upstream {
        route: route.path.clone(),
        kind: kind(&route.target)?,
        address: route.address.clone(),
        credential: Credential::new(route.credential.clone()),
        majors: route.majors.clone(),
    })
}

/// The tokens `route` admits, as the tokens file holds them, or why one is refused.
fn accepted(route: &Gated) -> Result<Accepted, Failure> {
    if !route.tokens.iter().all(|token| hashed(token)) {
        return Err(refused(
            "a token is not a SHA-256 in lower-case hexadecimal",
        ));
    }
    Ok(Accepted {
        route: route.path.clone(),
        tokens: route
            .tokens
            .iter()
            .map(|hash| TokenHash::written(hash))
            .collect(),
    })
}

/// What the gate calls `target`: a curator of video by the kind it fetches, and the media
/// server.
fn kind(target: &Target) -> Result<Kind, Failure> {
    match target {
        Target::Curator { media_type } => match media::Kind::for_media_type(media_type) {
            Some(media::Kind::Tv) => Ok(Kind::Sonarr),
            Some(media::Kind::Movies) => Ok(Kind::Radarr),
            None => Err(refused(&format!(
                "it reaches no curator filing {media_type}"
            ))),
        },
        Target::MediaServer => Ok(Kind::Jellyfin),
    }
}

/// What a route of `kind` reaches.
fn target(kind: Kind) -> Target {
    match kind {
        Kind::Sonarr => Target::Curator {
            media_type: media::Kind::Tv.media_type().to_owned(),
        },
        Kind::Radarr => Target::Curator {
            media_type: media::Kind::Movies.media_type().to_owned(),
        },
        Kind::Jellyfin => Target::MediaServer,
    }
}

/// Whether `token` is a SHA-256 in lower-case hexadecimal.
fn hashed(token: &str) -> bool {
    token.len() == HASH_DIGITS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A refusal saying `detail`.
fn refused(detail: &str) -> Failure {
    Failure::Refused {
        service: SERVICE.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests;
