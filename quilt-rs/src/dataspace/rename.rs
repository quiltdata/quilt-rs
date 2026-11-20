//! `rename`: this machine's local registry given a new name, an explicit
//! user act, after which the old name still names it, as an alias (25.10).
//! What `queue.db` keeps under the old name moves to the new one (25l item
//! 14). Contract and lock keys, and `conflict.json` rows, move on their
//! next use (25l items 4 and 12), so no dataspace is written here.

use thiserror::Error;
use tracing::info;

use crate::domain::DomainRoot;
use crate::domain::LocalNames;
use crate::domain::check_name;
use crate::io::storage::Storage;
use crate::queue::Queue;
use crate::queue::QueueError;

/// What [`rename`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Renamed {
    /// The registry's names now: the new one, and every one it had.
    pub names: LocalNames,
    /// The queue's jobs, failures and kept revisions moved to the new name.
    pub moved: usize,
}

/// Why the registry was not renamed, or its queue not moved.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenameError {
    /// The name is not one a local registry may keep. Nothing changed.
    #[error(transparent)]
    Name(Box<crate::Error>),

    /// The registry was renamed, but what the queue keeps under an old name
    /// was not moved. Renaming to the same name again moves it.
    #[error("the local registry is renamed, but its queued work was not moved: {0}")]
    Queue(Box<QueueError>),

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<crate::Error> for RenameError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// Renames `domain`'s local registry to `name`, its current name kept as an
/// alias, then moves what `queue` keeps under any name it had, jobs,
/// failures and kept revisions, to `name`. Renaming to the name it keeps
/// changes no name, and still moves what the queue keeps under an alias, so
/// a rename whose queue move failed is finished by running it again.
///
/// A worker that read the old name before the rename can still queue work
/// under it, which the next rename moves; the queue holds nothing else that
/// a rename leaves behind.
pub async fn rename(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    queue: &Queue,
    name: &str,
) -> Result<Renamed, RenameError> {
    check_name(name).map_err(|err| RenameError::Name(Box::new(err)))?;
    let names = domain.local().rename(storage, name).await?;
    let moved = queue
        .rename_local(&names.aliases, &names.name)
        .await
        .map_err(|err| RenameError::Queue(Box::new(err)))?;
    info!(
        "✔️ Renamed the local registry to {}, moving {moved} queued rows",
        names.name
    );
    Ok(Renamed { names, moved })
}

#[cfg(test)]
mod tests;
