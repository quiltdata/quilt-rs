<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Head unreleased changes with the Cargo.toml version: `-dev`, no date
     (e.g. [v0.4.2-dev]), not [Unreleased]. If the cycle needs a bigger bump,
     rename both. The first PR to change the crate after a release opens the
     next patch `-dev` version and heading; the release PR drops `-dev` and
     adds the date.
     Describe the change since the last release, not each PR: when a PR makes
     an unreleased entry stale, rewrite it in place and append the PR link.
     Work no user can reach yet gets one "Under the hood" line, not a feature
     entry; it earns a real entry once reachable, even behind a switch.
     Never edit released sections.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.1.0-dev]

### Added

- Under the hood, the crate holds quiltx's catalog-name normalization (`dns`) and credential-location rules (`paths`), moved here from `quiltx-core` so quilt-rs and quiltx share them (<https://github.com/quiltdata/quilt-rs/pull/1382>)
