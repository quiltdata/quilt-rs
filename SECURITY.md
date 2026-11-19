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

- The manifest schema itself has no credential fields: manifests record file
  metadata such as paths, hashes, and sizes, and authentication uses AWS
  credentials and OAuth handled outside the manifest. Package-level and
  per-entry user metadata accept arbitrary JSON, though, so a manifest can
  still carry any secret a caller chose to put there. When handling secrets or
  reviewing an incident that involves a shared manifest, check user metadata,
  not just the file listing.
- By default, file contents stay on disk in `objects/` after a commit or an
  `uninstall`, and remote manifests stay in the `packages/` cache. Objects
  are shared between packages without reference counting, so only the
  commands below delete them. This is documented behavior, not a
  vulnerability — see [docs/architecture.md](docs/architecture.md).
  - `quilt uninstall --prune` deletes the uninstalled package's objects that
    no other installed package uses.
  - `quilt remove-revisions` deletes a package's old revisions and the
    objects only those revisions use.
  - `quilt gc` deletes every object no installed revision of any installed
    package uses, the whole `packages/` cache, and files an interrupted
    command left in `staging/`.
  - These commands can keep some files, for example while another quilt
    process runs or after an interrupted command. To be sure a secret is
    gone from this machine, stop every quilt process, then delete the
    folder of every package that holds it and the whole `.quilt/` folder.
