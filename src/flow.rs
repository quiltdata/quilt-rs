//! It is public only for documentation.
//! This module namespace contains higher-order operations with packages.
//! But you don't need to use it outise of this crate.
//! All these functions are wrapped with `LocalDomain` methods with some context.

mod browse;
mod certify_latest;
mod commit;
mod install_package;
mod install_paths;
mod package;
mod pull;
mod push;
mod reset_to_latest;
mod status;
mod uninstall_package;
mod uninstall_paths;

pub use browse::browse_remote_manifest as browse;
pub use browse::cache_remote_manifest;
pub use certify_latest::certify_latest;
pub use commit::commit_package as commit;
pub use install_package::install_package;
pub use install_paths::install_paths;
pub use package::package_s3_prefix;
pub use pull::pull_package as pull;
pub use push::push_package as push;
pub use reset_to_latest::reset_to_latest;
pub use status::create_status as status;
pub use status::refresh_latest_hash;
pub use uninstall_package::uninstall_package;
pub use uninstall_paths::uninstall_paths;
