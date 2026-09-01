use quilt_rs::lineage::CommitState;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

#[derive(Debug)]
pub struct Output {
    pub commit: CommitState,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Undid the last commit; now at \"{}\"", self.commit.hash)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.undo_commit(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let package = local_domain
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::NamespaceNotFound(namespace))?;
    let commit = package.undo_commit().await?;
    Ok(Output { commit })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use tempfile::TempDir;

    use crate::cli::commit;
    use crate::cli::create;
    use crate::cli::model::Model;
    use crate::cli::model::create_model_in_temp_dir;
    use quilt_rs::flow::UserMeta;
    use quilt_rs::io::storage::ByteStream;
    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    /// A local-only package with `revisions` commits after its initial one,
    /// each writing `value.txt`. Returns the model, the namespace, and the
    /// package's working directory.
    async fn package_with(
        revisions: &[&str],
    ) -> Result<(Model, Namespace, PathBuf, TempDir), Error> {
        let (cli_model, temp_dir) = create_model_in_temp_dir().await?;
        let namespace: Namespace = ("demo", "local").into();
        let created = cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("initial".to_string()),
            })
            .await?;
        let package_home = created.installed_package.package_home().await?;
        let storage = LocalStorage::new();

        for message in revisions {
            storage
                .write_byte_stream(
                    package_home.join("value.txt"),
                    ByteStream::from(message.as_bytes().to_vec()),
                )
                .await?;
            cli_model
                .commit(commit::Input {
                    message: (*message).to_string(),
                    namespace: namespace.clone(),
                    user_meta: UserMeta::Keep,
                    workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                    host_config: None,
                })
                .await?;
        }
        Ok((cli_model, namespace, package_home, temp_dir))
    }

    /// The changeset the engine reports for the package — what `quilt status`
    /// would print.
    async fn changes(cli_model: &Model, namespace: &Namespace) -> Result<Vec<String>, Error> {
        let package = cli_model
            .get_local_domain()
            .get_installed_package(namespace)
            .await?
            .expect("installed");
        let status = package.status(None).await?;
        Ok(status
            .changes
            .keys()
            .map(|path| path.display().to_string())
            .collect())
    }

    /// The package's **tracked-path map** as persisted in the domain lineage.
    ///
    /// Deliberately not `status`: the changeset is recomputed from the
    /// manifest and the working tree, so it is blind to this map being wrong.
    /// The map is what `pull`'s touch set, `install_paths`, `uninstall_paths`
    /// and undo's own guard read, so it has to be asserted on its own terms.
    async fn tracked(cli_model: &Model, namespace: &Namespace) -> Result<Vec<String>, Error> {
        let lineage = cli_model.get_local_domain().get_lineage().await?;
        let mut keys: Vec<String> = lineage
            .packages
            .get(namespace)
            .expect("installed")
            .paths
            .keys()
            .map(|path| path.display().to_string())
            .collect();
        keys.sort();
        Ok(keys)
    }

    async fn undo(cli_model: &Model, namespace: &Namespace) -> Result<Output, Error> {
        super::model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await
    }

    #[tokio::test]
    async fn undoes_the_newest_commit() -> Result<(), Error> {
        let (cli_model, _temp_dir) = create_model_in_temp_dir().await?;
        let namespace: Namespace = ("demo", "local").into();
        let created = cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("initial".to_string()),
            })
            .await?;
        let package_home = created.installed_package.package_home().await?;
        let storage = LocalStorage::new();

        storage
            .write_byte_stream(
                package_home.join("value.txt"),
                ByteStream::from_static(b"first"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "first".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        storage
            .write_byte_stream(
                package_home.join("value.txt"),
                ByteStream::from_static(b"second"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "second".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        let output = super::model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await?;
        assert_eq!(output.commit.prev_hashes.len(), 1);
        assert_eq!(
            tokio::fs::read(package_home.join("value.txt")).await?,
            b"first"
        );
        Ok(())
    }

    /// The floor: a package's first revision has no parent, so there is
    /// nothing to undo. This is the state `create` leaves behind, not an edge
    /// case a user has to reach for.
    #[tokio::test]
    async fn refuses_at_the_initial_commit() -> Result<(), Error> {
        let (cli_model, namespace, _home, _temp) = package_with(&[]).await?;

        let err = undo(&cli_model, &namespace).await.unwrap_err();

        assert!(
            err.to_string().contains("first revision"),
            "should name the floor, got: {err}"
        );
        Ok(())
    }

    /// Undoing a *commit* must not be how an uncommitted edit is lost: the
    /// object store holds committed content only, so there would be nothing to
    /// restore it from.
    #[tokio::test]
    async fn refuses_a_dirty_tree_and_writes_nothing() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first", "second"]).await?;
        LocalStorage::new()
            .write_byte_stream(
                home.join("value.txt"),
                ByteStream::from_static(b"uncommitted"),
            )
            .await?;

        let err = undo(&cli_model, &namespace).await.unwrap_err();

        assert!(
            err.to_string().contains("value.txt"),
            "should name the dirty path, got: {err}"
        );
        // The refusal is only worth anything if it left the edit alone.
        assert_eq!(
            tokio::fs::read(home.join("value.txt")).await?,
            b"uncommitted"
        );
        Ok(())
    }

    /// The message must not send the user to a verb that does not exist:
    /// nothing discards uncommitted work on a package with no remote
    /// (<https://github.com/quiltdata/quilt-rs/issues/880>).
    #[tokio::test]
    async fn dirty_refusal_names_no_discard_verb() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first", "second"]).await?;
        LocalStorage::new()
            .write_byte_stream(home.join("value.txt"), ByteStream::from_static(b"edited"))
            .await?;

        let msg = undo(&cli_model, &namespace).await.unwrap_err().to_string();

        assert!(msg.contains("by hand"), "got: {msg}");
        for absent in ["discard", "reset", "revert", "quilt clean"] {
            assert!(!msg.contains(absent), "must not offer `{absent}`: {msg}");
        }
        Ok(())
    }

    /// A file the undone commit *added* is removed, and one it *deleted* comes
    /// back. Both are the delta arms the old rebuild-everything loop got right
    /// only by accident.
    #[tokio::test]
    async fn restores_added_and_removed_paths() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        let storage = LocalStorage::new();
        // Second commit: drop value.txt, add other.txt.
        tokio::fs::remove_file(home.join("value.txt")).await?;
        storage
            .write_byte_stream(home.join("other.txt"), ByteStream::from_static(b"new"))
            .await?;
        cli_model
            .commit(commit::Input {
                message: "swap".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        undo(&cli_model, &namespace).await?;

        assert_eq!(tokio::fs::read(home.join("value.txt")).await?, b"first");
        assert!(
            !home.join("other.txt").exists(),
            "the added file should be gone again"
        );
        Ok(())
    }

    /// Undo moves the pointer and prunes nothing, so the discarded revision is
    /// still listed. Pinning it because it is surprising, not because it is
    /// settled — see the open question on whether the log should hide it.
    #[tokio::test]
    async fn undone_revision_still_appears_in_the_log() -> Result<(), Error> {
        let (cli_model, namespace, _home, _temp) = package_with(&["first", "second"]).await?;
        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        let before = package.revisions().await?.len();

        undo(&cli_model, &namespace).await?;

        assert_eq!(package.revisions().await?.len(), before);
        Ok(())
    }

    /// The case the review raised: a path that is a file in one revision and a
    /// directory in the other. Writing before deleting is exactly wrong here —
    /// the file at `a` blocks creating `a/b`, and vice versa — so the removal
    /// that makes room has to happen first for that path.
    #[tokio::test]
    async fn handles_a_file_becoming_a_directory() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        let storage = LocalStorage::new();
        // Second commit turns the file `a` into the directory `a/`.
        storage
            .write_byte_stream(home.join("a"), ByteStream::from_static(b"a-as-file"))
            .await?;
        cli_model
            .commit(commit::Input {
                message: "a is a file".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        tokio::fs::remove_file(home.join("a")).await?;
        storage
            .write_byte_stream(home.join("a").join("b"), ByteStream::from_static(b"nested"))
            .await?;
        cli_model
            .commit(commit::Input {
                message: "a is a directory".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        undo(&cli_model, &namespace).await?;

        assert_eq!(tokio::fs::read(home.join("a")).await?, b"a-as-file");
        assert!(
            !home.join("a").join("b").exists(),
            "the nested path should be gone"
        );
        Ok(())
    }
    /// A failure part-way is **not** resumable, and that is the accepted
    /// limitation rather than an oversight — pinned so nobody re-widens the
    /// guard to admit it.
    ///
    /// Undo cannot tell its own half-finished work from a user's edits;
    /// nothing on disk records how far the last attempt got. Every rule that
    /// admits a state undo can produce also admits the user's version of the
    /// same state, and one of those is silent data loss. So the mixed tree is
    /// reported instead. Nothing is lost either way: all of it is committed
    /// content the object store still holds.
    #[tokio::test]
    async fn a_failed_undo_reports_the_mixed_tree_rather_than_resuming() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&[]).await?;
        let storage = LocalStorage::new();
        for (name, body) in [("one.txt", "one-v1"), ("two.txt", "two-v1")] {
            storage
                .write_byte_stream(home.join(name), ByteStream::from(body.as_bytes().to_vec()))
                .await?;
        }
        cli_model
            .commit(commit::Input {
                message: "v1".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        for (name, body) in [("one.txt", "one-v2"), ("two.txt", "two-v2")] {
            storage
                .write_byte_stream(home.join(name), ByteStream::from(body.as_bytes().to_vec()))
                .await?;
        }
        cli_model
            .commit(commit::Input {
                message: "v2".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        // Break the second write, so the first has already landed when it fails.
        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        let mut entries = tokio::fs::read_dir(package.paths.objects_dir()).await?;
        let mut victim = None;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file()
                && tokio::fs::read(entry.path()).await? == b"two-v1"
            {
                victim = Some(entry.path());
            }
        }
        let victim = victim.expect("the previous content of two.txt is in the object store");
        let bytes = tokio::fs::read(&victim).await?;
        tokio::fs::remove_file(&victim).await?;

        assert!(undo(&cli_model, &namespace).await.is_err());
        assert_eq!(
            tokio::fs::read(home.join("one.txt")).await?,
            b"one-v1",
            "the first write landed before the failure"
        );

        // Even with the cause repaired, the mixed tree is reported rather than
        // silently completed — and the path it names is the one undo itself
        // moved, which is exactly the ambiguity it refuses to guess about.
        tokio::fs::write(&victim, &bytes).await?;
        let err = undo(&cli_model, &namespace).await.unwrap_err();
        assert!(err.to_string().contains("one.txt"), "got: {err}");
        assert_eq!(
            tokio::fs::read(home.join("two.txt")).await?,
            b"two-v2",
            "nothing further was written"
        );
        Ok(())
    }

    /// The other side of admitting "already removed": a path the undone commit
    /// added, which the user has since *edited*, is real uncommitted work even
    /// though the undo was going to delete it. It must still refuse.
    #[tokio::test]
    async fn refuses_an_edit_to_a_path_it_would_remove() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        let storage = LocalStorage::new();
        storage
            .write_byte_stream(
                home.join("added.txt"),
                ByteStream::from_static(b"committed"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "adds a file".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        storage
            .write_byte_stream(
                home.join("added.txt"),
                ByteStream::from_static(b"then edited"),
            )
            .await?;

        let err = undo(&cli_model, &namespace).await.unwrap_err();

        assert!(err.to_string().contains("added.txt"), "got: {err}");
        assert_eq!(
            tokio::fs::read(home.join("added.txt")).await?,
            b"then edited"
        );
        Ok(())
    }

    /// A path the undo would remove, which cannot be *read* to tell whether it
    /// holds uncommitted work. Unlinking needs only the parent directory's
    /// permission, so classifying an unreadable file as clean would delete
    /// contents nobody verified. A symlink loop is the portable way to make a
    /// read fail as neither not-found nor a directory — a `chmod`-based test
    /// would pass vacuously as root.
    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_when_a_path_it_would_remove_cannot_be_read() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        LocalStorage::new()
            .write_byte_stream(
                home.join("added.txt"),
                ByteStream::from_static(b"committed"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "adds a file".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        tokio::fs::remove_file(home.join("added.txt")).await?;
        std::os::unix::fs::symlink("added.txt", home.join("added.txt"))?;

        let err = undo(&cli_model, &namespace).await.unwrap_err();

        // Surfaced as the I/O failure it is, not reported as a clean removal
        // and not dressed up as an uncommitted change.
        assert!(
            !err.to_string().contains("uncommitted changes"),
            "an unreadable path is not a known edit: {err}"
        );
        assert!(
            home.join("added.txt").symlink_metadata().is_ok(),
            "nothing should have been removed"
        );
        Ok(())
    }

    /// The restore writes paths the *previous* revision carries, which is not
    /// the set the dirty check walks (`lineage.paths` is the current
    /// revision's). A destination the current revision does not track is
    /// therefore never checked — so an untracked file sitting there is
    /// overwritten without a word.
    ///
    /// Minimal shape: the undone commit *deleted* a file, so undo restores it,
    /// and the user has since created their own file at that path.
    #[tokio::test]
    async fn refuses_to_overwrite_an_untracked_file_at_a_restore_destination() -> Result<(), Error>
    {
        let (cli_model, namespace, home, _temp) = package_with(&[]).await?;
        let storage = LocalStorage::new();
        storage
            .write_byte_stream(home.join("x"), ByteStream::from_static(b"committed-x"))
            .await?;
        cli_model
            .commit(commit::Input {
                message: "adds x".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        tokio::fs::remove_file(home.join("x")).await?;
        cli_model
            .commit(commit::Input {
                message: "deletes x".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        // The user makes their own file where the restore wants to write.
        storage
            .write_byte_stream(
                home.join("x"),
                ByteStream::from_static(b"mine, never committed"),
            )
            .await?;

        let err = undo(&cli_model, &namespace).await.unwrap_err();

        assert!(err.to_string().contains('x'), "should name the path: {err}");
        assert_eq!(
            tokio::fs::read(home.join("x")).await?,
            b"mine, never committed",
            "the user file must survive"
        );
        Ok(())
    }

    /// The rebuilt **tracked-path map** must describe the tree undo just
    /// wrote — gaining what it restored and losing what it removed.
    ///
    /// Every other test here reads file contents, and `status` cannot stand in
    /// for this: it recomputes the changeset from the manifest and the tree, so
    /// it stays green with the map entirely wrong. That map is what `pull`'s
    /// touch set, the `install_paths` / `uninstall_paths` pair and undo's own
    /// guard read, so a stale or missing entry is a real defect that nothing
    /// else here would notice.
    #[tokio::test]
    async fn the_tracked_path_map_matches_the_restored_revision() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        let storage = LocalStorage::new();
        // A revision that touches all three delta arms, so the rebuilt path
        // states cover a modification, an addition and a removal.
        storage
            .write_byte_stream(home.join("kept.txt"), ByteStream::from_static(b"kept-v1"))
            .await?;
        cli_model
            .commit(commit::Input {
                message: "v1".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        storage
            .write_byte_stream(home.join("kept.txt"), ByteStream::from_static(b"kept-v2"))
            .await?;
        storage
            .write_byte_stream(home.join("gained.txt"), ByteStream::from_static(b"gained"))
            .await?;
        tokio::fs::remove_file(home.join("value.txt")).await?;
        cli_model
            .commit(commit::Input {
                message: "v2".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;
        assert_eq!(
            tracked(&cli_model, &namespace).await?,
            ["gained.txt", "kept.txt"],
            "v2 tracks the addition and the modification, not the removal"
        );

        undo(&cli_model, &namespace).await?;

        // v1's rows exactly: `value.txt` comes back, `gained.txt` goes, and
        // `kept.txt` stays but at its earlier content.
        assert_eq!(
            tracked(&cli_model, &namespace).await?,
            ["kept.txt", "value.txt"]
        );
        assert!(changes(&cli_model, &namespace).await?.is_empty());
        Ok(())
    }

    /// Undoing *to* the initial commit empties the working tree — the initial
    /// revision has no rows, so every tracked file is a trailing removal.
    /// `refuses_at_the_initial_commit` covers the refusal one step later; this
    /// covers getting there.
    #[tokio::test]
    async fn undoing_to_the_initial_commit_empties_the_tree() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first"]).await?;
        assert!(home.join("value.txt").exists());

        undo(&cli_model, &namespace).await?;

        let mut entries = tokio::fs::read_dir(&home).await?;
        let mut left = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            left.push(entry.file_name().to_string_lossy().into_owned());
        }
        left.sort();
        assert!(left.is_empty(), "files left behind: {left:?}");
        assert!(
            tracked(&cli_model, &namespace).await?.is_empty(),
            "the tracked-path map should be empty too"
        );
        Ok(())
    }

    /// The guard is a gate, not a wall: clearing what it named lets the undo
    /// through. Every other refusal test stops at the refusal, which would
    /// also pass if undo had become permanently stuck.
    #[tokio::test]
    async fn clearing_the_obstruction_lets_the_undo_through() -> Result<(), Error> {
        let (cli_model, namespace, home, _temp) = package_with(&["first", "second"]).await?;
        LocalStorage::new()
            .write_byte_stream(home.join("value.txt"), ByteStream::from_static(b"edited"))
            .await?;

        assert!(undo(&cli_model, &namespace).await.is_err());

        // Put back what the revision being undone committed, and retry.
        LocalStorage::new()
            .write_byte_stream(home.join("value.txt"), ByteStream::from_static(b"second"))
            .await?;
        undo(&cli_model, &namespace).await?;

        assert_eq!(tokio::fs::read(home.join("value.txt")).await?, b"first");
        assert!(changes(&cli_model, &namespace).await?.is_empty());
        Ok(())
    }
}
