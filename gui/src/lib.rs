//! A game-agnostic GUI layer for gamelib.
//!
//! It knows nothing about any particular game. A game implements [`GameUi`]
//! (names, colors, card text, a bot) and gets a [`Session`] with snapshots, a
//! JSON [`api`] that any transport can carry (a Tauri command, the dev server,
//! a test), and shared web components in [`web`].
#![deny(missing_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(
    clippy::use_self,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

pub mod board;
pub mod describe;
pub mod description;
pub mod record;
pub mod session;

pub use description::{CardInfo, GameDescription, NamedId, Names, ResourceInfo, ScoreLine};
pub use session::{GameUi, NewGame, Session, Snapshot};
