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

