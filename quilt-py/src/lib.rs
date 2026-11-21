//! The Python binding of `quilt-rs`, published as the `quilt` package on `PyPI`.
//!
//! [`engine`] is the binding's Rust side: each call quilt3 makes, over
//! [`quilt_rs::dataspace`], taking and returning plain values, so it is
//! tested without Python. `python` (the `python` feature) wraps it in the
//! `quilt._native` extension module, which maturin builds.

// The engine layer's only caller is this crate's Python module, and every
// fn returns `engine::Result`, so a per-fn "# Errors" section would add
// noise, as in quilt-rs.
#![allow(clippy::missing_errors_doc)]
// `test_log`'s `#[test]` and `?` in tests, as in quilt-rs.
#![cfg_attr(test, allow(clippy::items_after_statements, clippy::unnecessary_wraps))]

pub mod engine;

#[cfg(feature = "python")]
mod python;
