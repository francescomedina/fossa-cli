use std::{
    borrow::Cow,
    collections::HashSet,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use clap::Parser;
use getset::Getters;
use millhone::{
    api::{self, ApiSnippet},
    extract::Snippet,
};
use secrecy::Secret;
use serde::{Deserialize, Serialize};
use stable_eyre::{eyre::Context, Report};
use tap::Pipe;
use tracing::warn;
use typed_builder::TypedBuilder;
use walkdir::DirEntry;

pub mod analyze;
pub mod commit;
pub mod ingest;
pub mod ping;

/// Arguments for API authentication.
#[derive(Debug, Parser, Getters)]
#[getset(get = "pub")]
pub struct ApiAuthentication {
    /// Provide the API Key ID for authentication.
    #[clap(long)]
    api_key_id: String,

    /// Provide the API Secret for authentication.
    #[clap(long)]
    api_secret: Secret<String>,
}

impl ApiAuthentication {
    /// Create credentials from these options.
    fn as_credentials(&self) -> api::Credentials {
        api::Credentials::new(self.api_key_id.clone(), self.api_secret.clone())
    }
}

/// A snippet match found in a local file during analysis.
#[derive(Debug, Clone, Serialize, Deserialize, TypedBuilder)]
pub struct MatchingSnippet {
    /// The local path in which the match was found, rendered as a string.
    /// Any invalid UTF8 content is replaced by `U+FFFD`.
    #[builder(setter(into))]
    found_in: String,

    /// A copy of the file content indicated by `found_at`, rendered as a string.
    /// Any invalid UTF8 content is replaced by `U+FFFD`.
    #[builder(setter(into))]
    local_text: String,

    /// The snippet that was identified in the local project.
    local_snippet: Snippet,

    /// Snippets in the knowledgebase that match this snippet.
    #[builder(setter(into))]
    matching_snippets: HashSet<ApiSnippet>,
}

/// Unwrap a directory entry, warning on error.
#[tracing::instrument]
fn unwrap_dir_entry(entry: Result<DirEntry, walkdir::Error>) -> Option<DirEntry> {
    match entry {
        Ok(entry) => Some(entry),
        Err(err) => {
            let depth = err.depth();
            let path = err
                .path()
                .map(|p| p.to_string_lossy())
                .unwrap_or_else(|| Cow::from("<none>"));
            if let Some(err) = err.io_error() {
                warn!(%path, %depth, %err, "walk: io error");
            } else if let Some(ancestor) = err.loop_ancestor() {
                let ancestor = ancestor.to_string_lossy();
                warn!(%path, %depth, %ancestor, "walk: symlink loop detected");
            } else {
                warn!(%path, %depth, "walk: generic error");
            }
            None
        }
    }
}

/// Resolves the path for an entry, with special handling for symlinks.
#[tracing::instrument]
fn resolve_path(entry: &DirEntry) -> Result<PathBuf, Report> {
    if entry.path_is_symlink() {
        std::fs::read_link(entry.path())
            .wrap_err_with(|| format!("resolve symlink of '{}'", entry.path().display()))
    } else {
        entry.path().to_path_buf().pipe(Ok)
    }
}

/// A simple atomic counter.
#[derive(Debug, Default)]
struct AtomicCounter {
    inner: AtomicUsize,
}

impl AtomicCounter {
    /// Increment the counter by 1.
    fn increment(&self) {
        self.increment_by(1)
    }

    /// Increment the counter by `n`.
    fn increment_by(&self, n: usize) {
        self.inner.fetch_add(n, Ordering::Relaxed);
    }

    /// Consumes the counter and returns the contained value.
    /// This is safe because passing self by value guarantees that no other threads are concurrently accessing the atomic data.
    fn into_inner(self) -> usize {
        self.inner.into_inner()
    }
}
