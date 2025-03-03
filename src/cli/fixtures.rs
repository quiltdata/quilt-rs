pub mod packages {
    pub mod default {
        pub const URI_LATEST: &str = "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs";
        pub const URI: &str =
            "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs@a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7";
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
        pub const URI: &str= "quilt+s3://data-yaml-spec-tests#package=reference/message-only@ce2ca6a39eb02725b24e3ccf158022dc80c2ab96b066e5660d87abafdbaee768";
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
        pub const LATEST_TOP_HASH: &str =
            "ae239090f2a01de382e8af719fe4a451ef1d1fa4a3ef7b21c6b36513d42c6630";
    }

    pub mod invalid {
        pub const URI: &str = "quilt+s3://some-nonsense";
        pub const SOURCE_PK: &str = "s3://any/thing";
    }
}
