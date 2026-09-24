pub mod packages {
    pub mod default {
        pub const BUCKET: &str = "data-yaml-spec-tests";
        pub const URI_LATEST: &str = "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs";
        pub const URI: &str = "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs@a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7";
        pub const README_LK: &str = "one/two two/three three three/READ ME.md";
        pub const README_LK_ESCAPED: &str = "one/two%20two/three%20three%20three/READ%20ME.md";
        pub const README_PK: &str = "s3://data-yaml-spec-tests/reference/quilt-rs/one/two%20two/three%20three%20three/READ%20ME.md?versionId=aIOyttmoQaE2cMcwEEoRod5G_3TZEHAW";
        pub const TIMESTAMP_LK: &str = "timestamp.txt";
        pub const TIMESTAMP_PK: &str = "s3://data-yaml-spec-tests/reference/quilt-rs/timestamp.txt?versionId=by4o4I2atAvVQDq1wyOJuP7y2pAh8Gqx";
        pub const NAMESPACE_STR: &str = "reference/quilt-rs";
        pub const NAMESPACE: (&str, &str) = ("reference", "quilt-rs");
        pub const TOP_HASH: &str =
            "a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7";
    }

    pub mod workflow_null {
        pub const URI: &str = "quilt+s3://udp-spec#package=reference/message-only@095017e53f4c8e0a07c82e562d088aa0e0f7a9ecaf2dce74a7607fac9085e98f";
        pub const TOP_HASH: &str =
            "095017e53f4c8e0a07c82e562d088aa0e0f7a9ecaf2dce74a7607fac9085e98f";
        pub const NAMESPACE: (&str, &str) = ("reference", "message-only");
        pub const NAMESPACE_STR: &str = "reference/message-only";
        pub const MESSAGE: &str = "#Test message 1234!?#";
    }

    pub mod my_workflow {
        pub const URI: &str = "quilt+s3://udp-spec#package=reference/with-workflow@4a9a3d39f655a03659333aad787b182e477e335e0fa78dd4d029521a9ca18dad";
        pub const TOP_HASH: &str =
            "4a9a3d39f655a03659333aad787b182e477e335e0fa78dd4d029521a9ca18dad";
        pub const MESSAGE: &str = "Test message";
        pub const NAMESPACE: (&str, &str) = ("reference", "with-workflow");
    }

    pub mod no_workflows_message_only {
        pub const URI: &str = "quilt+s3://data-yaml-spec-tests#package=reference/message-only@ce2ca6a39eb02725b24e3ccf158022dc80c2ab96b066e5660d87abafdbaee768";
        pub const TOP_HASH: &str =
            "ce2ca6a39eb02725b24e3ccf158022dc80c2ab96b066e5660d87abafdbaee768";
        pub const NAMESPACE: (&str, &str) = ("reference", "message-only");
        pub const MESSAGE: &str = "#Test message 1234!?#";
    }

    pub mod no_workflows_with_meta {
        pub const URI: &str = "quilt+s3://data-yaml-spec-tests#package=reference/meta@a0e161c9a281f38382007f4775e7d6ecbb50f929a197ba3e84443ec911ab6388";
        pub const TOP_HASH: &str =
            "a0e161c9a281f38382007f4775e7d6ecbb50f929a197ba3e84443ec911ab6388";
        pub const NAMESPACE: (&str, &str) = ("reference", "meta");
    }

    pub mod outdated {
        pub const URI: &str = "quilt+s3://data-yaml-spec-tests#package=scale/10u@f8216f57739c9824f22f1f7a1f8ded59fd50791c92bf9c317d06376811ecbfef";
        pub const NAMESPACE: (&str, &str) = ("scale", "10u");
        pub const NAMESPACE_STR: &str = "scale/10u";
        pub const LATEST_TOP_HASH: &str =
            "ae239090f2a01de382e8af719fe4a451ef1d1fa4a3ef7b21c6b36513d42c6630";
    }

    /// Three revisions in `udp-spec`, built to exercise the revision report's
    /// groups and pinned by hash so they never move.
    ///
    /// | revision | top-hash | shape |
    /// |---|---|---|
    /// | r1 | `fcd8f7cb…5182ec` | `keep.txt`, `modify.txt`, `remove.txt` |
    /// | r2 | `63ddbb5c…4f7d93` | +6 added, `modify.txt` modified, `remove.txt` gone |
    /// | r3 | `b27709e1…234106` | +`add/six.txt`, `keep.txt` modified — currently `latest` |
    ///
    /// Deliberate properties, each one a row of the manual test:
    ///
    /// - **`keep.txt` is untouched by r2**, so it is the file to edit locally for
    ///   kept local work, or to create with differing content for a both-added
    ///   conflict.
    /// - **Six adds in r2** exceeds the three-name cap the toast and the row use,
    ///   so the overflow wording has a fixture.
    /// - **One deeply-nested path**, so a surface with limited room has something
    ///   to wrap or truncate.
    /// - **r3 exists**, so installing r1 makes a pull span two revisions — and
    ///   only r3's message is in hand, there being no parent pointer to walk.
    ///
    /// Reachable with ambient `~/.aws` credentials, like the other live
    /// fixtures: the URI carries no `&catalog=`, so no stack login is needed.
    pub mod revision_report {
        pub const NAMESPACE_STR: &str = "reference/revision-report";

        /// r1 — install this one to be behind.
        pub const R1_URI: &str = "quilt+s3://udp-spec#package=reference/revision-report@fcd8f7cb81bc700ff4c8294ea173e3c3aa0899bcc61a41f22fe2bf18485182ec";

        /// r2 — install this one to reach r3 in a single revision, and to get a
        /// path r3 leaves alone (`add/one.txt`) for a local edit to survive on.
        /// Every path r1 holds is touched by r3, so kept local work needs r2.
        pub const R2_URI: &str = "quilt+s3://udp-spec#package=reference/revision-report@63ddbb5c7ec58d229147db6482dbb5c95e7e4a672551ce55e6bce0d5ba4f7d93";

        /// r3, which a pull from r1 lands on.
        pub const R3_TOP_HASH: &str =
            "b27709e128d7cca3f6755dbec847628457ac86db5df5876c3bc88610eb234106";
        pub const R3_MESSAGE: &str = "r3: adds add/six.txt, modifies keep.txt";
    }

    /// A package whose objects are large enough that a pull can be observed —
    /// and interrupted — while it is still running. Everything else in here is
    /// sized for classification and reporting, where a few small text files are
    /// ideal and a timing window does not exist.
    ///
    /// Two objects of 24 MiB of random (so incompressible) bytes, both replaced
    /// in r2, making a pull move ~48 MiB. Reachable with ambient `~/.aws`
    /// credentials like the other live fixtures; the URIs carry no `&catalog=`.
    pub mod large {
        pub const NAMESPACE_STR: &str = "reference/large";

        /// r1 — install this one to be behind.
        pub const R1_URI: &str = "quilt+s3://udp-spec#package=reference/large@eedcbe821a3d1bb691f01866b70a55034c1fc073debd1aa906f7e381bba22827";

        /// r2 — both objects replaced, so a pull from r1 refetches everything.
        pub const R2_TOP_HASH: &str =
            "38a8d7c2de5e00346afebc981e73a45b8e5b353f71d1ee9e010c6bfa34152049";

        pub const PATHS: [&str; 2] = ["bulk-a.bin", "bulk-b.bin"];
    }

    /// A package whose object gets **smaller** in r2, which is what makes the
    /// memory-mapping test deterministic rather than a race.
    ///
    /// Replacing a file by copying onto it truncates the destination first, so a
    /// mapping of it is invalidated; if the replacement were the same size, the
    /// window in which a read faults would last only as long as the copy. A file
    /// that ends up shorter leaves the tail permanently past end-of-file, so the
    /// fault is certain. Replacing by rename never invalidates the mapping at
    /// all, whatever the sizes.
    ///
    /// 4 MiB down to 64 KiB, which is plenty of tail and costs the live suite
    /// almost nothing.
    pub mod shrinking {
        pub const NAMESPACE_STR: &str = "reference/shrinking";

        /// r1 — 4 MiB. Install this one and map it.
        pub const R1_URI: &str = "quilt+s3://udp-spec#package=reference/shrinking@a445691b1a7740971e334235c3fbfe4975c3cce94bc3c6c712b37f16a0412cca";

        /// r2 — the same path at 64 KiB.
        pub const R2_TOP_HASH: &str =
            "ba1e5279aea0cc3ae92b72eb5d32251542ab423b17e6956cbc71b5708bb1cf60";

        pub const MAPPED: &str = "mapped.bin";
        pub const R1_LEN: usize = 4 * 1024 * 1024;
    }

    /// Two revisions on `fiskus-sandbox-dev`, a bucket **without versioning**,
    /// so every row's `physical_key` is a bare key with no `?versionId`.
    ///
    /// | revision | top-hash | shape |
    /// |---|---|---|
    /// | r1 | `cfc6d21a…503590` | `a.txt` = `a v1`, `b.txt` = `b v1` |
    /// | r2 | `88d45bb1…bba211` | `b.txt` = `b v2` — currently `latest` |
    ///
    /// Both revisions point `b.txt` at the same key, so r2's push replaced r1's
    /// bytes: the remote no longer holds r1's `b.txt`. `a.txt` is unchanged.
    ///
    /// Never push to this package again. A push that writes `b v1` back would
    /// make r1's `b.txt` fetchable, and the live test that relies on it being
    /// gone would fail.
    ///
    /// Reachable with ambient `~/.aws` credentials: the URI carries no
    /// `&catalog=`, so no stack login is needed.
    pub mod unversioned {
        pub const NAMESPACE_STR: &str = "fiskus/unversioned-skip";

        /// r1 — its `b.txt` was overwritten by r2.
        pub const R1_URI: &str = "quilt+s3://fiskus-sandbox-dev#package=fiskus/unversioned-skip@cfc6d21adc2e8153f319965cdd80477e4a7443b9f64a21bae1b9048b05503590";

        /// Unchanged by r2, so r1's bytes are still on the remote.
        pub const KEPT: &str = "a.txt";
        pub const KEPT_R1_BODY: &[u8] = b"a v1\n";

        /// Replaced by r2 at the same bare key.
        pub const REPLACED: &str = "b.txt";
    }

    pub mod invalid {
        pub const URI: &str = "quilt+s3://some-nonsense";
    }
}

pub fn get_browse_output() -> Result<String, std::io::Error> {
    let path = std::env::current_dir()?.join("fixtures/reference-quilt-rs-browse-output.txt");
    std::fs::read_to_string(path)
}
