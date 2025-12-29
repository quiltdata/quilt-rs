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

### Creating new releases

1. **Update the changelog**: Add new section to [CHANGELOG.md](CHANGELOG.md) following
   <https://keepachangelog.com> format with PR links
2. **Bump version**: Update version in `src-tauri/Cargo.toml`
3. **Create release via GitHub Actions**:
    * Go to the Actions tab: <https://github.com/quiltdata/quilt-rs/actions/workflows/release-quilt-sync.yaml>
    * Click "Run workflow" button
    * The workflow will build all platforms and create a draft GitHub release
      with built assets
4. **Publish release**: Edit the draft release created by the workflow and publish
   it

The release workflow builds for all platforms and creates a draft release
using the version from `src-tauri/Cargo.toml`.
