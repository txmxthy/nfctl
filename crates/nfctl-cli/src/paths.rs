//! Where files this tool writes for itself belong. Two of them so far: the
//! completion catalogue and a timings log.

use std::path::PathBuf;

/// `$XDG_CACHE_HOME/nfctl`, or `~/.cache/nfctl`. `None` when neither is set,
/// which is the signal to do without rather than guess at a path.
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("nfctl"))
}
