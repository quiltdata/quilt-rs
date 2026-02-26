# Migrated Issues from quiltdata/QuiltSync

Issues migrated from the original [quiltdata/QuiltSync](https://github.com/quiltdata/QuiltSync) repository.

## Elegant handling for S3 NoSuchKey errors

> Original: [QuiltSync#387](https://github.com/quiltdata/QuiltSync/issues/387)
> Author: @QuiltSimon | Created: 2025-12-04

### Summary

When an S3 `NoSuchKey` error occurs (e.g., fetching a package revision
that doesn't exist), the application displays a raw error dump on the main page:

```text
Quilt error: S3 error for None: Failed to get object stream: S3 error: service error:
NoSuchKey: The specified key does not exist...
path: .quilt/named_packages/team/package-002/latest
```

### Desired behavior

1. Catch the error gracefully.
2. Display a user-friendly message (e.g., "Resource not found" or
   "Package manifest missing").
3. Offer remediation steps (e.g., "Check your connection", "Refresh",
   or "Contact support").

---

## Per-package windows

> Original: [QuiltSync#108](https://github.com/quiltdata/QuiltSync/issues/108)
> Author: @drernie | Created: 2024-03-06

### User story

When editing multiple packages, each package should open in a separate
document window so users don't have to navigate back and forth to the index.

### Notes

This originally required Tauri v2, which is now the version in use.
See upstream: <https://github.com/tauri-apps/tauri/issues/1643>.

---

## Create new packages

> Original: [QuiltSync#107](https://github.com/quiltdata/QuiltSync/issues/107)
> Author: @drernie | Created: 2024-03-06

### User story

When a user has a folder containing (possibly large) experimental results
on their local machine, they should be able to use QuiltSync to create and
upload it as a new package — without unnecessary duplication — so that they
and others can analyze it in the cloud.

### Required functionality

1. Allow a user-specified folder as the working directory
2. Create a local package
3. Assign it a name
4. Specify a remote bucket (from a list? validate access?)
5. Ensure Lineage can handle this workflow
