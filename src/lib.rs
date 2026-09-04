#![forbid(unsafe_code)]

#[cfg(feature = "desktop")]
pub mod bridge_registry;
pub mod cli;
pub mod client;
pub mod config;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod fsutil;
pub mod http_source;
pub mod model;
pub mod protocol;
pub mod server;
pub mod source;
pub mod state;
pub mod storage;
pub mod sync;
pub mod tus_client;
pub mod wallpaper;
