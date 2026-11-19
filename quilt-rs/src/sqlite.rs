//! A SQLite connection whose work runs on tokio's blocking threads, shared by
//! `state.db` and `queue.db`.

use std::path::Path;
use std::sync::Arc;
use std::sync::PoisonError;

/// What a [`Connection`] itself fails with, before its caller's own errors.
#[derive(Debug)]
pub(crate) enum ConnectionError {
    Sqlite(rusqlite::Error),
    /// The connection's worker thread is gone, or it was closed.
    Closed,
}

impl From<rusqlite::Error> for ConnectionError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

#[derive(Clone)]
pub(crate) struct Connection(Arc<std::sync::Mutex<Option<rusqlite::Connection>>>);

impl Connection {
    pub(crate) async fn open(path: &Path) -> Result<Self, ConnectionError> {
        let path = path.to_owned();
        Self::open_with(move || rusqlite::Connection::open(path)).await
    }

    pub(crate) async fn open_in_memory() -> Result<Self, ConnectionError> {
        Self::open_with(rusqlite::Connection::open_in_memory).await
    }

    async fn open_with<F>(open: F) -> Result<Self, ConnectionError>
    where
        F: FnOnce() -> rusqlite::Result<rusqlite::Connection> + Send + 'static,
    {
        let db = tokio::task::spawn_blocking(open)
            .await
            .map_err(|_| ConnectionError::Closed)??;
        Ok(Self(Arc::new(std::sync::Mutex::new(Some(db)))))
    }

    pub(crate) async fn call<T, E, F>(&self, f: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<ConnectionError> + Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> Result<T, E> + Send + 'static,
    {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            // An op that panicked left SQLite to roll its transaction back,
            // so the connection is sound for the next one.
            let mut guard = inner.lock().unwrap_or_else(PoisonError::into_inner);
            f(guard.as_mut().ok_or(ConnectionError::Closed)?)
        })
        .await
        .map_err(|_| ConnectionError::Closed)?
    }

    /// Closes on a blocking thread: dropping the last handle would close on
    /// the async one.
    pub(crate) async fn close(&self) -> Result<(), ConnectionError> {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let db = inner.lock().unwrap_or_else(PoisonError::into_inner).take();
            match db {
                Some(db) => db.close().map_err(|(_, err)| err.into()),
                None => Ok(()),
            }
        })
        .await
        .map_err(|_| ConnectionError::Closed)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    /// The queue has no copy to fall back on, so a panic in one op must not
    /// close the connection for every op after it.
    #[test(tokio::test)]
    async fn an_op_that_panics_leaves_the_connection_for_the_next() -> Result<(), ConnectionError> {
        let db = Connection::open_in_memory().await?;
        db.call(|db| Ok::<_, ConnectionError>(db.execute_batch("CREATE TABLE t (x)")?))
            .await?;

        let panicked = db
            .call(|db| -> Result<(), ConnectionError> {
                let tx = db.transaction()?;
                tx.execute("INSERT INTO t VALUES (1)", [])?;
                panic!("an op that panics mid-transaction");
            })
            .await;
        let rows = db
            .call(|db| {
                Ok::<i64, ConnectionError>(
                    db.query_row("SELECT count(*) FROM t", [], |r| r.get(0))?,
                )
            })
            .await;

        assert!(
            matches!(panicked, Err(ConnectionError::Closed)),
            "{panicked:?}"
        );
        assert_eq!(rows.ok(), Some(0));
        Ok(())
    }
}
