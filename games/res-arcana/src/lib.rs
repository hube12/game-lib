//! Res Arcana card data and its loader.
//!
//! The cards live in `data/cards.ra`, a small text file written with the
//! rulebook's symbols (`E L C D G` for the essences, `*` for the any-essence
//! pentagon, `T` for turning, `>` for the triangle between cost and effect).
//! The header of that file documents the notation. [`load`] parses it into
//! [`CardSpec`]s.
#![deny(missing_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(
    clippy::use_self,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

pub mod bot;
pub mod compile;
pub mod parse;
pub mod rules;
pub mod spec;

pub use compile::{CompileError, Compiled, compile, compile_all};
pub use parse::{ParseError, parse};
pub use spec::{CardKind, CardSpec, Essence};

/// The embedded card file.
pub const CARDS_TEXT: &str = include_str!("../data/cards.ra");

/// Parses the embedded card file.
pub fn load() -> Result<Vec<CardSpec>, ParseError> {
    parse(CARDS_TEXT)
}
