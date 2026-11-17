//! The move of a domain's lineage from `data.json` to refs, run in a child.

use super::World;
use crate::Res;

/// What [`super::Operation::MigrateUp`] runs on `world`'s domain.
#[expect(clippy::unused_async, reason = "the move to refs is not built yet")]
pub(super) async fn up(world: &World) -> Res {
    let _ = world;
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the move to refs is not built yet",
    )
    .into())
}
