# Contributing to QuiltSync

## Testing

### "Golden path" Testing

#### Prerequisites

* Open QuiltSync
* Click on Profile icon
* Change Stack name to "nightly.quilttest.com"
* Click "Save settings"

#### Whenever you make some changes, verify "Golden path" works

* Go to <https://nightly.quilttest.com/b/quilt-desktop/packages/test/assets>
* Click "Get package" -> "Open in QuiltSync" -> Open that URI (by copying or clicking)
* [ ] Ensure app is opened with package contents
* [ ] Package is installed
* Go 🏠 and open package again
* [ ] Click on "Open in Catalog" button opens Catalog with the correct URL
* Back to QuiltSync, go to package, select "README.md" and click "Download selected
  paths"
* [ ] File manager is opened
* Update README.md: `date > README.md`
* Back to the QuiltSync, click "Refresh"
* [ ] You see README.md as "Modified"
* Click "Create new revision"
* Fill "Message" and "User metadata" with "Test"
* [ ] You see an error message
* Change metadata to the { "datetime": content-of-README.md }
* Click "Commit"
* [ ] You see Package page with the "Your commits are ahead of the remote" message
* Click "Push"
* [ ] Click on "Open in Catalog" button opens Catalog with the new Package revision
* Back to QuiltSync, click "Remove" to uninstall the package
* [ ] You see the list of packages without "test/assets" package or empty page

#### Troubleshooting

* If page shows error message (EOF lineage file). Click "Refresh" button

## Release Process

The full procedure is in [docs/releases.md](../docs/releases.md). What is
specific to QuiltSync:

* **Version and changelog**: the version is the one in
  `src-tauri/Cargo.toml` (not a workspace version), and the changelog is
  [CHANGELOG.md](CHANGELOG.md). Before releasing, drop `-dev` from both and
  add today's date to the changelog heading. `quilt-sync` depends on
  `quilt-rs` and `quilt-uri` by path only, so their versions never need
  updating here.
* **Workflow**:
  [`release-quilt-sync.yaml`](https://github.com/quiltdata/quilt-rs/actions/workflows/release-quilt-sync.yaml),
  run manually with an `environment` input. It builds macOS (arm64, x86_64),
  Linux and Windows bundles into a draft release tagged
  `QuiltSync/v<version>`. Install one of the bundles to check it, then approve
  the `promote` job (`release-approval` environment): it publishes the draft,
  marks it "latest", and dispatches `upload-to-hubspot.yaml`. Don't publish
  the draft by hand.
* **Rollout**: the auto-updater reads `https://www.quilt.bio/hubfs/latest.json`,
  which only `upload-to-hubspot.yaml` writes, so users get the update once the
  HubSpot mirror has run, not when the GitHub release goes public. Re-run
  `upload-to-hubspot.yaml` manually if it fails.

### Auto Updater Setup

Updates are signed; the release workflow needs these GitHub repository
secrets:

* `TAURI_SIGNING_PRIVATE_KEY`: Private key for signing updates
* `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: Password for the private key (optional)

The matching public key is embedded in `src-tauri/tauri.conf.json`.
