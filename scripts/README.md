# QuiltSync HubSpot release publishing

This folder contains automation for publishing QuiltSync release assets to HubSpot
and optionally updating a HubDB row that powers the QuiltSync download page.

## publish_quiltsync_release.py

Downloads release assets from GitHub, extracts archives, uploads files to hubfs,
rewrites `latest.json` to point to hubfs URLs, and ensures `latest.json` is
uploaded to `/hubfs/latest.json`.

Required environment variables:

- `HUBSPOT_ACCESS_TOKEN`: HubSpot private app token with Files + HubDB scopes.

Optional environment variables for HubDB updates:

- `HUBDB_TABLE_ID`: HubDB table id.
- `HUBDB_ROW_ID`: HubDB row id to update. If omitted, a new row is created.
- `HUBDB_COLUMN_MAP`: JSON mapping of logical keys to HubDB column names.
  Example:
  `{ "version": "latest_version", "latest_json_url": "latest_json_url", "latest_json": "latest_json" }`
- `HUBDB_PUBLISH`: `true` (default) to publish the table after update.

Arguments:

- `--github-repo`: Repo that hosts releases. Default `quiltdata/quilt-rs`.
- `--release-tag`: Release tag. Omit for latest.
- `--hubfs-base-path`: Base hubfs path for assets. Default `/quiltsync`.
- `--hubfs-root-url`: Base hubfs URL. Default `https://www.quilt.bio/hubfs`.
- `--latest-json-target-path`: Target path for latest.json. Default `/latest.json`.
