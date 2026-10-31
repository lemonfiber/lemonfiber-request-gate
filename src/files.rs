//! The gate's three files: the routes and the tokens, read on each use, and the record,
//! written whole.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::formats::gate::{File, Kept, Outcome, Record, Tokens, Upstreams};
use tokio::sync::Mutex;

/// The configuration directory, and the one lock that keeps two entries from writing
/// the record at once.
pub(crate) struct Files {
    config: PathBuf,
    recording: Mutex<()>,
}

impl Files {
    /// The files in `config`.
    pub(crate) fn new(config: PathBuf) -> Self {
        Self {
            config,
            recording: Mutex::new(()),
        }
    }

    /// The routes the core wrote, none where it has written none yet, or that the file is
    /// there and cannot be read.
    pub(crate) async fn upstreams(&self) -> Routes {
        match tokio::fs::read_to_string(self.config.join(File::Upstreams.name())).await {
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Routes::Unwritten,
            Err(_) => Routes::Unreadable,
            Ok(text) => Upstreams::read(&text).map_or(Routes::Unreadable, Routes::Written),
        }
    }

    /// Whether `token` is one `route` accepts. A file that cannot be read accepts
    /// nothing, and neither does a call that carries no token.
    pub(crate) async fn accepts(&self, route: &str, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        read(&self.config, File::Tokens)
            .await
            .and_then(|text| Tokens::read(&text).ok())
            .is_some_and(|tokens| tokens.accepts(route, token))
    }

    /// Add one entry to the record, and say whether it was written.
    ///
    /// A record that cannot be read is started again rather than left unwritten, and
    /// the copy is renamed over it, so the core never reads half of one.
    pub(crate) async fn record(
        &self,
        route: &str,
        method: &str,
        path: &str,
        outcome: Outcome,
    ) -> bool {
        let _recording = self.recording.lock().await;
        let record = read(&self.config, File::Record)
            .await
            .and_then(|text| Record::read(&text).ok())
            .unwrap_or_default()
            .with(now(), route, method, path, outcome, Kept::standard());
        let written = self.config.join(format!("{}.writing", File::Record.name()));
        tokio::fs::write(&written, record.written()).await.is_ok()
            && tokio::fs::rename(&written, self.config.join(File::Record.name()))
                .await
                .is_ok()
    }
}

/// The routes the core wrote, as the gate finds them.
pub(crate) enum Routes {
    /// The core has written none yet, so the gate admits nothing.
    Unwritten,
    /// The file is there and cannot be read.
    Unreadable,
    /// The routes, read.
    Written(Upstreams),
}

/// The text of `file` in `config`, where it can be read.
async fn read(config: &Path, file: File) -> Option<String> {
    tokio::fs::read_to_string(config.join(file.name()))
        .await
        .ok()
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests;
