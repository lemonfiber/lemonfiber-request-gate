//! A curator's adapter, asked the `library.curate` request operations under the request
//! key the gate holds on it.
//!
//! Every answer is held to the contract before it is read. An operation the adapter
//! answers `unsupported` is one the curator does not take requests by, and every other
//! failure is one the gate could not get past.

use std::sync::Arc;

use async_trait::async_trait;
use lemonfiber_contract::capabilities::library::curate;
use lemonfiber_contract::Contracted;
use lemonfiber_ports::http::{Fetched, Http, Method, Request, Response, Unreachable};
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{
    Catalogue, CatalogueEntry, CuratedTitle, Downloading, Failure, Fulfilling, QualityProfile,
    RegisteredFolder, TitleRequest,
};
use lemonfiber_ports::Client;

use crate::formats::gate::Upstream;
use crate::upstream::Stop;

/// The most of an answer the gate reads where the operation bounds it no tighter.
const LARGEST: usize = lemonfiber_contract::client::LARGEST;

/// The curator filing one kind of media, asked through its adapter.
pub(crate) struct Curator {
    adapter: curate::Adapter,
    media: Kind,
}

impl Curator {
    /// The curator filing `media` that `upstream` reaches, asked with `client`.
    pub(crate) fn of(client: &reqwest::Client, upstream: &Upstream, media: Kind) -> Self {
        let asked = Contracted::new(
            Arc::new(Sent(client.clone())),
            upstream.address.clone(),
            format!("the curator filing {}", media.media_type()),
            upstream.credential.reveal(),
        );
        Self {
            adapter: curate::Adapter(asked),
            media,
        }
    }

    /// The kind of media it files.
    pub(crate) const fn media(&self) -> Kind {
        self.media
    }

    /// The title its catalogue holds by `reference`, or none.
    pub(crate) async fn catalogued(&self, reference: i64) -> Result<Option<CuratedTitle>, Stop> {
        self.read(self.adapter.catalogued(self.media, reference).await)
    }

    /// Every title it holds.
    pub(crate) async fn titles(&self) -> Result<Vec<CuratedTitle>, Stop> {
        self.read(self.adapter.titles(self.media).await)
    }

    /// The title it holds as `id`, or none.
    pub(crate) async fn held_title(&self, id: i64) -> Result<Option<CuratedTitle>, Stop> {
        self.read(self.adapter.held_title(self.media, id).await)
    }

    /// The title `request` asks for, once it has taken the request.
    pub(crate) async fn request(&self, request: &TitleRequest) -> Result<CuratedTitle, Stop> {
        self.read(self.adapter.request(self.media, request).await)
    }

    /// Search for what it wants of the title it holds as `id`.
    pub(crate) async fn search(&self, id: i64) -> Result<(), Stop> {
        self.read(self.adapter.search(self.media, id).await)
    }

    /// Remove the title it holds as `id`, with its files.
    pub(crate) async fn remove(&self, id: i64) -> Result<(), Stop> {
        self.read(self.adapter.remove(self.media, id).await)
    }

    /// What it is downloading.
    pub(crate) async fn downloading(&self) -> Result<Vec<Downloading>, Stop> {
        self.read(self.adapter.downloading(self.media).await)
    }

    /// Its quality profiles.
    pub(crate) async fn quality_profiles(&self) -> Result<Vec<QualityProfile>, Stop> {
        self.read(self.adapter.quality_profiles().await)
    }

    /// The folders it files into.
    pub(crate) async fn root_folders(&self) -> Result<Vec<RegisteredFolder>, Stop> {
        self.read(self.adapter.root_folders().await)
    }

    /// What its catalogue holds for `term`.
    pub(crate) async fn lookup(&self, term: &str) -> Result<Vec<CatalogueEntry>, Stop> {
        self.read(self.adapter.lookup(self.media, term).await)
    }

    /// What the adapter answered, or where the gate stops.
    fn read<T>(&self, answer: Result<T, Failure>) -> Result<T, Stop> {
        answer.map_err(|failure| match failure {
            Failure::Unsupported { .. } => Stop::Untaken(self.media),
            _ => Stop::Unreachable,
        })
    }
}

/// The gate's own client, as the contract's client sends with it.
struct Sent(reqwest::Client);

#[async_trait]
impl Http for Sent {
    async fn send(&self, request: &Request) -> Result<Response, Unreachable> {
        let fetched = self.fetch(request, LARGEST).await?;
        let body = fetched
            .bytes
            .ok_or_else(|| Unreachable::once(&request.url, "the answer is larger than it reads"))?;
        Ok(Response {
            status: fetched.status,
            headers: fetched.headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    async fn fetch(&self, request: &Request, most: usize) -> Result<Fetched, Unreachable> {
        let unreached = |_| Unreachable::once(&request.url, "the adapter did not answer");
        let mut built = self.0.request(method(request.method), &request.url);
        for (name, value) in &request.headers {
            built = built.header(name, value);
        }
        if let Some(body) = &request.body {
            built = built.body(body.clone());
        }
        let mut answer = built.send().await.map_err(unreached)?;
        let status = answer.status().as_u16();
        let headers = answer
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let mut bytes = Vec::new();
        while let Some(chunk) = answer.chunk().await.map_err(unreached)? {
            if bytes.len() + chunk.len() > most {
                return Ok(Fetched {
                    status,
                    headers,
                    bytes: None,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Fetched {
            status,
            headers,
            bytes: Some(bytes),
        })
    }
}

/// The verb `reqwest` sends for `method`.
fn method(method: Method) -> reqwest::Method {
    match method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Patch => reqwest::Method::PATCH,
        Method::Delete => reqwest::Method::DELETE,
    }
}

#[cfg(test)]
mod tests;
