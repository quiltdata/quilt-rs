//! The `QuiltSync` frontend, as a library over two thin binaries.
//!
//! # Why a `[lib]` at all
//!
//! The app and the debug gallery are two binaries that share the `src/kit/` tree.
//! An item can be used in one and unused in the other (`kit::render` is used by
//! the pages and never by the gallery), and `dead_code` runs per target. In a
//! binary, `#[allow(dead_code)]` never expires, and `#[expect(dead_code)]` fails
//! in whichever binary does use the item, because an unfulfilled `expect` is an
//! error under the workspace's `warnings = "deny"`.
//!
//! `dead_code` does not flag an unused `pub` item in a library, because a
//! library's callers are outside it. So the shared modules live in this library,
//! and the two binaries are its callers.
//!
//! # Nothing moved on disk
//!
//! `stylance`'s `import_crate_style!` paths are relative to the crate root and its
//! scan is configured `folders = ["./src/kit/", "./src/pages/"]`. Both would break
//! if these files moved, so they did not: this declares the same modules from a
//! new root.

// Two pedantic lints exist for PUBLISHED library APIs and started firing the
// moment this crate had a library at all: `missing_errors_doc` wants an
// `# Errors` section on each of 64 `Result`-returning commands, and
// `must_use_candidate` a `#[must_use]` on 51 more. This crate is
// `publish = false` and its only callers are the two binaries beside it, so both
// would be boilerplate addressed to nobody.
//
// Not the same trade as the `dead_code` allows this file exists to retire. Those
// suppressed a true signal about this code — that an item was unused — and could
// never expire, because the item was live in one binary and dead in the other.
// These decline advice aimed at a kind of crate this is not.
#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

// The tests moved here with the modules, and they are DOM tests: without this
// they run outside a browser and 159 of them fail on a missing `window`. Each
// binary root carries its own copy for its own remaining tests — the macro is
// per test target, not per crate.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

pub mod commands;
pub mod components;
pub mod error_handler;
pub mod kit;
pub mod pages;
pub mod panic_report;
pub mod routes;
pub mod tauri;
#[cfg(test)]
pub(crate) mod test_support;
pub mod theme;
pub mod util;
