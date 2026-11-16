//! A short top hash, resolved to the whole one.
//!
//! This is quilt3's `Package.resolve_hash` /
//! `PackageRegistry.resolve_top_hash`
//! (`api/python/quilt3/backends/base.py` in quiltdata/quilt), kept to its
//! rules:
//!
//! - A prefix is [`MIN_HASH_PREFIX_LEN`] to [`TOP_HASH_LEN`] characters. A
//!   shorter or longer one is `Invalid hash`, before anything is asked of the
//!   registry.
//! - A whole top hash comes back as given. Nothing checks that it names a
//!   revision, as in quilt3: the read that follows says so.
//! - A shorter prefix is matched against the manifests the registry holds
//!   under `.quilt/packages/`, whatever package they belong to. No match and
//!   several matches are errors, never a guess.
//! - Matching is case-sensitive: top hashes are lowercase hex, so `ABCDEF` is
//!   [`HashPrefixError::NotHex`], never a match.
//!
//! The registry is a bucket, not a package: like quilt3's, the answer does not
//! depend on a package name, so a prefix two packages' manifests share is
//! ambiguous.
//!
//! Deliberate differences from quilt3, none visible for a registry quilt
//! wrote:
//!
//! - A prefix that is not lowercase hex, or a 64-character string that is not
//!   a top hash, is [`HashPrefixError::NotHex`], without a listing. quilt3
//!   would list first and find zero matches, or pass the 64 characters on and
//!   fail to find the manifest.
//! - An ambiguous prefix lists at most [`MAX_LISTED_MATCHES`] of the matches
//!   and counts the rest. quilt3 lists none.
//! - Only objects named by a whole top hash count as revisions. quilt3 counts
//!   any object under `.quilt/packages/`.
//! - The listing asks S3 for the keys that start with the prefix, where quilt3
//!   lists the whole directory and filters.

use quilt_uri::Host;
pub use quilt_uri::MIN_HASH_PREFIX_LEN;
pub use quilt_uri::TOP_HASH_LEN;
use quilt_uri::is_top_hash;

use crate::Res;
use crate::error::HashPrefixError;
use crate::io::remote::Remote;

/// How many matches [`HashPrefixError::Ambiguous`] lists.
pub const MAX_LISTED_MATCHES: usize = 20;

/// The one top hash in `bucket`'s registry that starts with `prefix`.
///
/// Asks `host`'s credentials, or this computer's AWS ones when `host` is
/// `None`. See the module docs for the rules.
///
/// # Errors
///
/// [`HashPrefixError::InvalidLength`] for a prefix outside
/// [`MIN_HASH_PREFIX_LEN`]..=[`TOP_HASH_LEN`] characters;
/// [`HashPrefixError::NotHex`] for one that is not lowercase hex;
/// [`HashPrefixError::NoMatch`] and [`HashPrefixError::Ambiguous`] when zero or
/// several revisions start with it; and whatever the listing fails with.
pub async fn resolve_hash(
    remote: &impl Remote,
    host: Option<&Host>,
    bucket: &str,
    prefix: &str,
) -> Res<String> {
    let length = prefix.chars().count();
    if !(MIN_HASH_PREFIX_LEN..=TOP_HASH_LEN).contains(&length) {
        return Err(HashPrefixError::InvalidLength(prefix.to_string()).into());
    }
    if length == TOP_HASH_LEN {
        return if is_top_hash(prefix) {
            Ok(prefix.to_string())
        } else {
            Err(HashPrefixError::NotHex(prefix.to_string()).into())
        };
    }
    if !is_lowercase_hex(prefix) {
        return Err(HashPrefixError::NotHex(prefix.to_string()).into());
    }

    let found = remote
        .manifest_hashes_with_prefix(host, bucket, prefix)
        .await?;
    Ok(pick_one(prefix, found)?)
}

fn is_lowercase_hex(text: &str) -> bool {
    text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The single hash among `found` that starts with `prefix`.
///
/// `found` is whatever the registry listed; it is checked again here, so a
/// remote that over-lists cannot make a prefix ambiguous, and the same hash
/// listed twice counts once.
fn pick_one(prefix: &str, found: Vec<String>) -> Result<String, HashPrefixError> {
    let mut matches: Vec<String> = found
        .into_iter()
        .filter(|hash| is_top_hash(hash) && hash.starts_with(prefix))
        .collect();
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => Err(HashPrefixError::NoMatch(prefix.to_string())),
        1 => Ok(matches.remove(0)),
        total => {
            matches.truncate(MAX_LISTED_MATCHES);
            Err(HashPrefixError::Ambiguous {
                prefix: prefix.to_string(),
                more: total - matches.len(),
                matches,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::Error;
    use crate::io::remote::mocks::MockRemote;
    use quilt_uri::S3Uri;

    const BUCKET: &str = "registry";
    /// Two revisions that share their first eight characters.
    const ALPHA: &str = "abcdef01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BETA: &str = "abcdef01bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const GAMMA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    async fn remote_with(hashes: &[&str]) -> Res<MockRemote> {
        let remote = MockRemote::default();
        for hash in hashes {
            let uri = S3Uri::try_from(format!("s3://{BUCKET}/.quilt/packages/{hash}").as_str())?;
            remote.put_object(None, &uri, b"{}".to_vec()).await?;
        }
        Ok(remote)
    }

    fn prefix_error(err: Error) -> HashPrefixError {
        match err {
            Error::HashPrefix(inner) => inner,
            other => panic!("expected a hash prefix error, got {other:?}"),
        }
    }

    #[test(tokio::test)]
    async fn a_unique_prefix_resolves_to_the_whole_hash() -> Res {
        let remote = remote_with(&[ALPHA, BETA, GAMMA]).await?;

        assert_eq!(resolve_hash(&remote, None, BUCKET, "012345").await?, GAMMA);
        // Long enough to tell the two that share eight characters apart.
        assert_eq!(
            resolve_hash(&remote, None, BUCKET, "abcdef01a").await?,
            ALPHA
        );
        assert_eq!(&ALPHA[..9], "abcdef01a");
        Ok(())
    }

    /// The shortest prefix quilt3 accepts is six characters.
    #[test(tokio::test)]
    async fn a_prefix_of_exactly_the_minimum_length_resolves() -> Res {
        let remote = remote_with(&[GAMMA]).await?;

        let six = &GAMMA[..MIN_HASH_PREFIX_LEN];
        assert_eq!(six.len(), 6);
        assert_eq!(resolve_hash(&remote, None, BUCKET, six).await?, GAMMA);
        // One character fewer is refused even though it is just as unique.
        let err = resolve_hash(&remote, None, BUCKET, &GAMMA[..5])
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::InvalidLength(GAMMA[..5].to_string())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_that_is_too_short_or_too_long_is_invalid() -> Res {
        let remote = remote_with(&[GAMMA]).await?;

        for bad in ["", "0", "01234", &format!("{GAMMA}0")] {
            let err = resolve_hash(&remote, None, BUCKET, bad).await.unwrap_err();
            assert_eq!(
                prefix_error(err),
                HashPrefixError::InvalidLength(bad.to_string()),
                "{bad:?}"
            );
        }
        Ok(())
    }

    /// The length check is on characters, as Python's `len` is.
    #[test(tokio::test)]
    async fn the_length_counts_characters_not_bytes() -> Res {
        let remote = remote_with(&[GAMMA]).await?;

        // Five characters, ten bytes.
        let err = resolve_hash(&remote, None, BUCKET, "ééééé")
            .await
            .unwrap_err();
        assert!(matches!(
            prefix_error(err),
            HashPrefixError::InvalidLength(_)
        ));
        // Six characters pass the length check, then are not hex.
        let err = resolve_hash(&remote, None, BUCKET, "éééééé")
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NotHex(_)));
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_nothing_starts_with_matches_zero() -> Res {
        let remote = remote_with(&[ALPHA, GAMMA]).await?;

        let err = resolve_hash(&remote, None, BUCKET, "123456")
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::NoMatch("123456".to_string())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_empty_registry_matches_zero() -> Res {
        let remote = MockRemote::default();

        let err = resolve_hash(&remote, None, BUCKET, "abcdef")
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NoMatch(_)));
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_two_revisions_share_is_ambiguous_and_names_them() -> Res {
        let remote = remote_with(&[ALPHA, BETA, GAMMA]).await?;

        let err = resolve_hash(&remote, None, BUCKET, "abcdef")
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::Ambiguous {
                prefix: "abcdef".to_string(),
                matches: vec![ALPHA.to_string(), BETA.to_string()],
                more: 0,
            }
        );
        Ok(())
    }

    /// A prefix a whole registry shares lists the first few and counts the
    /// rest, so the message stays short.
    #[test(tokio::test)]
    async fn a_prefix_many_revisions_share_lists_only_the_first_few() -> Res {
        let total = MAX_LISTED_MATCHES + 5;
        let hashes: Vec<String> = (0..total).map(|n| format!("abcdef{n:058x}")).collect();
        let remote = remote_with(&hashes.iter().map(String::as_str).collect::<Vec<_>>()).await?;

        let err = resolve_hash(&remote, None, BUCKET, "abcdef")
            .await
            .unwrap_err();
        let HashPrefixError::Ambiguous { matches, more, .. } = prefix_error(err) else {
            panic!("expected Ambiguous");
        };
        assert_eq!(matches, hashes[..MAX_LISTED_MATCHES]);
        assert_eq!(more, 5);

        let message = HashPrefixError::Ambiguous {
            prefix: "abcdef".to_string(),
            matches,
            more,
        }
        .to_string();
        assert!(
            message.contains(&format!("starts {total} revisions")),
            "{message}"
        );
        assert!(message.ends_with(", and 5 more)"), "{message}");
        Ok(())
    }

    /// quilt3 returns a 64-character hash without asking the registry.
    #[test(tokio::test)]
    async fn a_whole_hash_comes_back_as_given_even_if_no_revision_has_it() -> Res {
        let remote = MockRemote::default();

        assert_eq!(resolve_hash(&remote, None, BUCKET, GAMMA).await?, GAMMA);
        Ok(())
    }

    /// quilt3 matches with `str.startswith`: `ABCDEF` is not `abcdef`.
    #[test(tokio::test)]
    async fn matching_is_case_sensitive() -> Res {
        let remote = remote_with(&[ALPHA]).await?;

        assert_eq!(resolve_hash(&remote, None, BUCKET, "abcdef").await?, ALPHA);
        for upper in ["ABCDEF", "AbCdEf"] {
            let err = resolve_hash(&remote, None, BUCKET, upper)
                .await
                .unwrap_err();
            assert_eq!(
                prefix_error(err),
                HashPrefixError::NotHex(upper.to_string())
            );
        }
        let err = resolve_hash(&remote, None, BUCKET, &ALPHA.to_uppercase())
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NotHex(_)));
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_that_is_not_hex_is_refused_as_such() -> Res {
        let remote = remote_with(&[ALPHA]).await?;

        for bad in [
            "zzzzzz",
            "abcde/",
            "abcdef ",
            "abc.ef",
            &"z".repeat(TOP_HASH_LEN),
        ] {
            let err = resolve_hash(&remote, None, BUCKET, bad).await.unwrap_err();
            assert_eq!(
                prefix_error(err),
                HashPrefixError::NotHex(bad.to_string()),
                "{bad:?}"
            );
        }
        Ok(())
    }

    /// The registry is the bucket: manifests of other packages count, and
    /// objects that are not manifests do not.
    #[test(tokio::test)]
    async fn only_whole_hash_names_under_packages_are_revisions() -> Res {
        let remote = remote_with(&[ALPHA]).await?;
        // `{BETA}/nested` makes `BETA` itself a directory, not a manifest.
        for stray in [
            "abcdef01-notes.txt",
            "abcdef01/nested",
            "abcdef.tmp",
            &format!("{BETA}/nested"),
        ] {
            let uri = S3Uri::try_from(format!("s3://{BUCKET}/.quilt/packages/{stray}").as_str())?;
            remote.put_object(None, &uri, b"x".to_vec()).await?;
        }
        // Not under `.quilt/packages/`, so not a manifest at all.
        let elsewhere = S3Uri::try_from(format!("s3://{BUCKET}/abcdef02{}", &BETA[8..]).as_str())?;
        remote.put_object(None, &elsewhere, b"x".to_vec()).await?;

        assert_eq!(resolve_hash(&remote, None, BUCKET, "abcdef").await?, ALPHA);
        Ok(())
    }

    #[test(tokio::test)]
    async fn another_bucket_is_another_registry() -> Res {
        let remote = remote_with(&[ALPHA]).await?;

        let err = resolve_hash(&remote, None, "elsewhere", "abcdef")
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NoMatch(_)));
        Ok(())
    }

    #[test]
    fn pick_one_ignores_what_does_not_start_with_the_prefix_and_repeats() {
        let listed = vec![
            GAMMA.to_string(),
            ALPHA.to_string(),
            ALPHA.to_string(),
            "abcdef-not-a-hash".to_string(),
        ];
        assert_eq!(pick_one("abcdef", listed).as_deref(), Ok(ALPHA));
    }
}
