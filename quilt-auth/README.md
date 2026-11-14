# quilt-auth

Transport-free authentication helpers shared by quilt-rs and
[quiltx](https://github.com/quiltdata/quiltx).

No network, no keychain, no Python: pure functions over strings and
environment values.

- `dns::normalize_dns` — how a catalog name typed by a person becomes the
  bare lowercase DNS name that keys its credentials.
- `paths` — where quiltx keeps its credential files, and the key of an entry
  in the OS keyring.

quiltx depends on this crate for both, so the two projects name a catalog the
same way. The HTTP, keyring and file-store halves stay in quiltx for now.
