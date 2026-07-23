//! It is public only for documentation.
//! This module namespace contains higher-order operations with packages.
//! But you don't need to use it outise of this crate.
//! All these functions are wrapped with `LocalDomain` methods with some context.

mod apply_update;
mod browse;
mod certify_latest;
mod commit;
mod create_package;
mod install_package;
mod install_paths;
mod publish;
mod pull;
mod pull_outcome;
pub(crate) mod push;
mod recommit;
mod reset_to_latest;
mod status;
mod uninstall_package;
mod uninstall_paths;

pub(crate) use apply_update::apply_latest_update;
pub use browse::browse_remote_manifest as browse;
pub use browse::cache_remote_manifest;
pub use certify_latest::certify_latest;
pub use commit::UserMeta;
pub use commit::commit_package as commit;
pub use create_package::create_package as create;
pub use install_package::install_package;
pub use install_paths::install_paths;
pub use publish::CommitOptions;
pub use publish::PublishOutcome;
pub use publish::publish_package as publish;
pub use pull::PullSnapshot;
pub use pull::pull_package as pull;
pub use pull::snapshot_for_pull;
pub use pull_outcome::PullOutcome;
pub use pull_outcome::classify_pull;
pub(crate) use pull_outcome::remote_delta;
pub use push::PushResult;
pub use push::push_package as push;
pub use recommit::recommit_for_remote as recommit;
pub use reset_to_latest::reset_to_latest;
pub use status::create_status as status;
pub use status::refresh_latest_hash;
pub use uninstall_package::uninstall_package;
pub use uninstall_paths::uninstall_paths;
