//! Transport-free authentication helpers, shared by quilt-rs and quiltx.
//!
//! Nothing here does network or keychain I/O, and nothing needs Python. quiltx
//! builds its Python extension (`quiltx._core`) on top of this crate; quilt-rs
//! uses it to name a catalog.
//!
//! * [`dns`]: `normalize_dns`, how a catalog name typed by a person becomes the
//!   bare lowercase DNS name that keys its credentials. A port of quiltx's Python
//!   original, with its error messages.
//! * [`paths`]: where quiltx keeps its credential files and the key an entry has in
//!   the OS keyring, derived by the same rules as Python's `platformdirs`.

pub mod dns;
pub mod paths;
