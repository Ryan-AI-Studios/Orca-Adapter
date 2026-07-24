//! Error types for the 3MF core library.

use camino::Utf8PathBuf;
use thiserror::Error;

/// Library result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by analyze/convert operations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error for path {path}: {source}")]
    Io {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("JSON error in {context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("XML error in {context}: {source}")]
    Xml {
        context: String,
        #[source]
        source: quick_xml::Error,
    },

    #[error("missing required ZIP member: {0}")]
    MissingMember(String),

    #[error("output path must differ from input path (refused overwrite): {0}")]
    OutputEqualsInput(Utf8PathBuf),

    #[error("{0}")]
    Message(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<Utf8PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn json(context: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Json {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn xml(context: impl Into<String>, source: quick_xml::Error) -> Self {
        Self::Xml {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
