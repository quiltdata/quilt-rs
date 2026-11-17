//! The move of a domain's lineage from refs back to `data.json`, run in a
//! child.

use super::World;
use crate::Res;

/// What [`super::Operation::MigrateDown`] runs on `world`'s domain.
#[expect(clippy::unused_async, reason = "the rollback is not built yet")]
pub(super) async fn down(world: &World) -> Res {
    let _ = world;
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the rollback is not built yet",
    )
    .into())
}
