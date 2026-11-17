//! Dispatch's core: everything that can be tested without a window. The Tauri app in
//! `app/` and the `dispatch-cli` binary only call into this crate.

pub mod accents;
pub mod add;
pub mod alerts;
pub mod backup;
pub mod bibfile;
pub mod catalogue;
pub mod cite;
pub mod companion;
pub mod deadlines;
pub mod documents;
pub mod fetch;
pub mod follow;
pub mod health;
pub mod hints;
pub mod item;
pub mod links;
pub mod netlog;
pub mod news;
pub mod paths;
pub mod pdf;
pub mod rank;
pub mod refresh;
pub mod s2;
pub mod schedule;
pub mod search;
pub mod settings;
pub mod sources;
pub mod stats;
pub mod status;
pub mod store;
pub mod sync;
pub mod theme;
pub mod topics;
pub mod vault;
pub mod zotero;

pub use item::{Item, Section};
pub use store::Store;
pub use topics::Topics;

/// File names inside the data folder.
pub const DB_FILE: &str = "dispatch.sqlite";
pub const TOPICS_FILE: &str = "topics.json";
