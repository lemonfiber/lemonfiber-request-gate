//! Where the gate finds its files.

use std::path::PathBuf;

/// Where the gate finds its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// The configuration directory the stack mounts: the core's two files, read-only,
    /// and the record this service writes.
    pub(crate) config: PathBuf,
    /// The egress plex.tv is reached through, as an HTTPS proxy's address, where the stack
    /// names one; plex.tv is reached directly where it names none.
    pub(crate) plex_tv_through: Option<String>,
}

/// The configuration directory, unless `LEMONFIBER_REQUEST_GATE_CONFIG` names another.
const CONFIG: &str = "/config";

/// The variable naming the egress plex.tv is reached through.
const PLEX_TV_THROUGH: &str = "LEMONFIBER_REQUEST_GATE_PLEX_TV_THROUGH";

impl Settings {
    /// The settings `variable` gives, each falling back to the stack's default.
    pub(crate) fn from(variable: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            config: variable("LEMONFIBER_REQUEST_GATE_CONFIG")
                .map_or_else(|| PathBuf::from(CONFIG), PathBuf::from),
            plex_tv_through: variable(PLEX_TV_THROUGH).filter(|through| !through.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests;
