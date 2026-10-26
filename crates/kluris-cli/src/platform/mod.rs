//! The operating system's side of the self commands and the sync services.

pub mod service;
#[cfg(windows)]
pub mod user_path;
