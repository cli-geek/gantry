use std::path::PathBuf;

use crate::{PlatformError, os};

/// A Chromium-family browser that can be driven over CDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserInfo {
    pub name: String,
    pub path: PathBuf,
}

pub trait BrowserLocator: std::fmt::Debug {
    /// `Ok(None)` means none is installed; Chrome for Testing is the
    /// fallback the browser milestone (M6) adds.
    fn locate(&self) -> Result<Option<BrowserInfo>, PlatformError>;
}

#[derive(Debug, Default)]
pub struct OsBrowserLocator;

impl BrowserLocator for OsBrowserLocator {
    fn locate(&self) -> Result<Option<BrowserInfo>, PlatformError> {
        os::locate_browser()
    }
}
