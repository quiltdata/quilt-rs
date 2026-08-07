# Security Policy

## Reporting a vulnerability

Please do not open a public issue for security vulnerabilities.

Use GitHub's private reporting instead:
[Report a vulnerability](https://github.com/quiltdata/quilt-rs/security/advisories/new).
That opens a draft advisory visible only to you and the maintainers.

Useful to include: affected version (`quilt --version`), platform, what an
attacker can do, and a reproduction if you have one.

We will acknowledge your report and keep you updated on the fix. Please give us
a chance to release a patch before disclosing publicly.

## Supported versions

Fixes land on the latest published release of each crate. There are no
long-term support branches.

## Scope notes

- Manifests record file metadata (paths, hashes, sizes) and never credentials.
  Authentication is handled by AWS credentials and OAuth outside the manifest.
- `objects/` and the manifest cache are never pruned by design, so data you
  committed locally may remain on disk after `uninstall`. This is documented
  behavior, not a vulnerability — see
  [docs/architecture.md](docs/architecture.md).
