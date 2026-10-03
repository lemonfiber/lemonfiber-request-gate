//! Where the gate finds its files.

use std::path::PathBuf;

/// Where the gate finds its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// The configuration directory the stack mounts: the core's two files, read-only,
    /// and the record this service writes.
    pub(crate) config: PathBuf,
}

/// The configuration directory, unless `LEMONFIBER_REQUEST_GATE_CONFIG` names another.
const CONFIG: &str = "/config";

impl Settings {
    /// The settings `variable` gives, each falling back to the stack's default.
    pub(crate) fn from(variable: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            config: variable("LEMONFIBER_REQUEST_GATE_CONFIG")
                .map_or_else(|| PathBuf::from(CONFIG), PathBuf::from),
        }
    }
}

#[cfg(test)]
mod tests;
