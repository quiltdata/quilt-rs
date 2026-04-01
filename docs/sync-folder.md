# Job Stories: Syncing Packages from Folders

## Context

Quilt packages follow a Git-like workflow, but the current CLI exposes it as
multiple steps (create/install, commit, push). The new `sync` command unifies
this into a single verb: point at a folder, associate it with a package URI,
and push — validating auth, namespace, and bucket in one shot. QuiltSync (the
Tauri desktop app) currently supports install/commit/push/pull/merge but does
**not** yet expose a sync/create flow.

### Design constraints

- **Folder = working directory**: The specified folder (or cwd) becomes the
  package's working directory in-place. No files are copied into a separate
  location.
- **Catalog-first**: Every package must be associated with an existing Quilt
  catalog (for auth/governance). The URI always includes the bucket
  (`quilt+s3://bucket#package=owner/name`) and may optionally include the
  catalog. If omitted: auto-select if exactly one catalog is configured,
  prompt if zero or multiple.
- **Full round-trip by default**: `sync` registers + commits + pushes by
  default, validating everything works. Flags allow doing less (e.g.,
  `--no-push`, `--register-only`).

---

## CLI (`quilt-cli`)

### Sync (happy path)

#### 1. Sync cwd as a new package

- **When** I have a folder of data files ready to share,
- **I want to** run `quilt sync quilt+s3://bucket#package=owner/name` from
  that folder,
- **So I can** register, commit, and push the package in one step —
  confirming auth and permissions immediately.

#### 2. Sync a different folder

- **When** the folder I want to sync isn't my cwd,
- **I want to** run `quilt sync <uri> --folder ./path/to/data`,
- **So I can** target a specific directory without cd-ing into it.

#### 3. Re-sync after changes

- **When** I've already synced a package and I've modified files in the folder,
- **I want to** run `quilt sync` again (same URI or inferred from prior
  registration),
- **So I can** commit and push the latest folder state incrementally.

### Sync (less-than-full)

#### 4. Sync without pushing

- **When** I want to register and commit locally but not push yet,
- **I want to** run `quilt sync <uri> --no-push`,
- **So I can** review the commit before uploading.

#### 5. Register only

- **When** I only want to register the folder as a package without committing,
- **I want to** run `quilt sync <uri> --register-only`,
- **So I can** set up the association and add files before my first commit.

### Catalog resolution

#### 6. Explicit catalog in URI

- **When** my URI includes a catalog origin,
- **I want to** the package to use that catalog directly,
- **So I can** be explicit about which registry governs this package.

#### 7. Auto-select single catalog

- **When** my URI omits the catalog and I have exactly one configured,
- **I want to** `sync` to auto-select it,
- **So I can** skip boilerplate when there's no ambiguity.

#### 8. Prompt for catalog

- **When** my URI omits the catalog and I have zero or multiple configured,
- **I want to** be prompted to select one (or told to configure one),
- **So I can** always associate my package with a valid catalog.

### Change detection (manual)

#### 9. Check folder status

- **When** I want to check if my synced folder has uncommitted changes,
- **I want to** run `quilt status` (or `quilt status --namespace owner/name`),
- **So I can** see which files are modified, added, or removed before deciding
  to sync.

#### 10. Status shows changes

- **When** `quilt status` shows changes,
- **I want to** see a concise summary (counts by type) and optionally a
  detailed file list,
- **So I can** quickly decide whether to sync now or keep working.

#### 11. Status shows no changes

- **When** my synced folder has no changes,
- **I want to** `quilt status` to confirm "up to date" (or "ahead" if
  committed but not pushed),
- **So I can** know there's nothing to do.

### Ignore & filtering

#### 12. Exclude files via .quiltignore

- **When** my folder has files that shouldn't be packaged (logs, temp files,
  large intermediates),
- **I want to** add patterns to `.quiltignore` before syncing,
- **So I can** keep my working folder messy without polluting the package
  manifest.

### Error cases

#### 13. Namespace conflict

- **When** I run `sync` and the namespace already exists locally pointing at
  a different folder,
- **I want to** get a clear error explaining the conflict,
- **So I can** decide whether to update the existing registration or choose a
  different namespace.

#### 14. Push failure

- **When** `sync` fails during push (bad permissions, nonexistent bucket,
  auth expired),
- **I want to** see which step failed and why,
- **So I can** fix the issue and re-run `sync` to retry from where it left
  off.

---

## QuiltSync (Desktop App)

### Sync (create new)

#### 15. Sync a new folder from the UI

- **When** I'm on the installed packages list and I want to sync a new folder
  as a package,
- **I want to** click "Sync Folder" and pick the folder, bucket, and
  namespace,
- **So I can** create and push a package without using the terminal.

#### 16. Pre-select single catalog

- **When** I have exactly one catalog configured,
- **I want to** QuiltSync to pre-select it during sync setup,
- **So I can** skip unnecessary choices.

#### 17. Pick from multiple catalogs

- **When** I have multiple catalogs,
- **I want to** pick which one the new package belongs to,
- **So I can** target the right registry.

#### 18. Preview files before sync

- **When** I pick a folder to sync,
- **I want to** see a preview of the files that will be included (respecting
  `.quiltignore`),
- **So I can** confirm the right data is being packaged before proceeding.

#### 19. Land on detail page after sync

- **When** QuiltSync syncs the folder successfully,
- **I want to** land on the package detail page showing the files and
  "pushed" status,
- **So I can** confirm everything worked.

### Automatic change detection

#### 20. Auto-update status in package list

- **When** QuiltSync is running and files change in a synced folder,
- **I want to** the package list to automatically update its status indicator
  (e.g., badge or icon showing "modified"),
- **So I can** see at a glance which packages have pending changes without
  manually checking.

#### 21. Visual cue on package card

- **When** QuiltSync detects changes in a synced folder,
- **I want to** see a notification or visual cue on the package card,
- **So I can** decide whether to open the package and re-sync.

#### 22. Detail view of detected changes

- **When** I open a package that QuiltSync has flagged as changed,
- **I want to** see the specific modified/added/removed files on the detail
  page,
- **So I can** review what changed before committing.

#### 23. Ignored files excluded from detection

- **When** QuiltSync is watching a folder and I add files that match
  `.quiltignore` patterns,
- **I want to** those changes to be silently excluded from the status,
- **So I can** avoid false "modified" indicators for irrelevant files.

### Sync (update existing)

#### 24. One-click re-sync

- **When** I've reviewed the detected changes and I'm ready to push,
- **I want to** click "Sync" with a commit message,
- **So I can** update the remote package in one action.

#### 25. Bulk replace via folder swap

- **When** I want to replace all files in a package with a new export from my
  instrument/pipeline,
- **I want to** swap the folder contents and re-sync,
- **So I can** do a bulk update where removed files are reflected in the new
  revision.

### Error & edge cases

#### 26. Login redirect

- **When** I try to sync but I'm not logged in to the target catalog,
- **I want to** be redirected to the login page with a "back" link to the
  sync flow,
- **So I can** authenticate and resume without losing my inputs.

#### 27. No catalogs configured

- **When** I have no catalogs configured and try to sync,
- **I want to** be told to configure a catalog first (with a link to
  settings),
- **So I can** unblock myself without guessing what's wrong.

#### 28. Retry after failure

- **When** sync fails mid-push due to permissions or network issues,
- **I want to** see the error and be able to retry,
- **So I can** recover without re-entering all my inputs.
