//! gamelib lib
#![deny(missing_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::use_self)]
// The engine API is deliberately explicit and verbose; these pedantic lints add noise without value
// here.
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
#![cfg_attr(
    feature = "document-features",
    cfg_attr(doc, doc =::document_features::document_features!())
)]

pub mod bot;
pub mod card;
pub mod cost;
pub mod counter;
pub mod effect;
pub mod engine;
pub mod event;
pub mod flow;
pub mod id;
pub mod market;
pub mod modifier;
pub mod pay;
pub mod resource;
pub mod rng;
pub mod state;
pub mod view;
pub mod zone;
