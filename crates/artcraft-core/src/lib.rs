//! Core of the ArtCraft Hub: catalogue, release discovery, verification and
//! installation of the Craft Apps.

pub mod catalog;
pub mod config;
pub mod download;
pub mod error;
pub mod github;
pub mod install;
pub mod net;
pub mod ops;
pub mod paths;
pub mod progress;
pub mod state;
pub mod time;
pub mod version;
pub mod vt;

pub use error::{Error, Result};
