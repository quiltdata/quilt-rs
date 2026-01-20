#!/usr/bin/env python3
import argparse
import json
import os
import sys
import tarfile
import tempfile
import zipfile
from datetime import datetime, timezone
from urllib.parse import urlparse

import requests


GITHUB_API_BASE = "https://api.github.com"
HUBSPOT_FILE_UPLOAD_URL = "https://api.hubapi.com/filemanager/api/v3/files/upload"
HUBSPOT_HUBDB_ROW_URL = "https://api.hubapi.com/cms/v3/hubdb/tables/{table_id}/rows/{row_id}"
HUBSPOT_HUBDB_ROWS_URL = "https://api.hubapi.com/cms/v3/hubdb/tables/{table_id}/rows"


def _log(message):
    print(message, flush=True)


def _err(message):
    print(message, file=sys.stderr, flush=True)


def _github_headers(token):
    if not token:
        return {"Accept": "application/vnd.github+json"}
    return {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": "2022-11-28",
    }


def fetch_release(repo, tag, token):
    if tag:
        url = f"{GITHUB_API_BASE}/repos/{repo}/releases/tags/{tag}"
    else:
        url = f"{GITHUB_API_BASE}/repos/{repo}/releases/latest"
    resp = requests.get(url, headers=_github_headers(token), timeout=60)
    if resp.status_code != 200:
        raise RuntimeError(f"Failed to fetch release: {resp.status_code} {resp.text}")
    return resp.json()


def download_asset(asset, dest_dir, token):
    name = asset["name"]
    url = asset["browser_download_url"]
    dest_path = os.path.join(dest_dir, name)
    headers = _github_headers(token)
    _log(f"Downloading {name}")
    with requests.get(url, headers=headers, stream=True, timeout=120) as resp:
        if resp.status_code != 200:
            raise RuntimeError(f"Failed to download {name}: {resp.status_code} {resp.text}")
        with open(dest_path, "wb") as handle:
            for chunk in resp.iter_content(chunk_size=1024 * 1024):
                if chunk:
                    handle.write(chunk)
    return dest_path


def is_archive(path):
    lower = path.lower()
    return lower.endswith(".zip") or lower.endswith(".tar.gz") or lower.endswith(".tgz")


def extract_archive(path, dest_dir):
    extracted_root = os.path.join(dest_dir, os.path.basename(path) + "_extracted")
    os.makedirs(extracted_root, exist_ok=True)
    if path.lower().endswith(".zip"):
        with zipfile.ZipFile(path, "r") as zf:
            zf.extractall(extracted_root)
    else:
        with tarfile.open(path, "r:*") as tf:
            tf.extractall(extracted_root)
    return extracted_root


def collect_files(root_dir):
    files = []
    for dirpath, _, filenames in os.walk(root_dir):
        for filename in filenames:
            files.append(os.path.join(dirpath, filename))
    return files


def find_latest_json(items):
    for item in items:
        if os.path.basename(item["source"]) == "latest.json":
            return item
    return None


def load_latest_json(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def find_release_version(latest_json, fallback_tag):
    if isinstance(latest_json, dict):
        version = latest_json.get("version")
        if isinstance(version, str) and version.strip():
            return version.strip()
    return fallback_tag or "unknown"


def build_upload_map(items, base_path, hubfs_root_url):
    mapping = {}
    base_path = "/" + base_path.strip("/")
    hubfs_root_url = hubfs_root_url.rstrip("/")
    for item in items:
        rel_path = item["rel_path"]
        target_path = f"{base_path}/{rel_path}".replace("//", "/")
        url = f"{hubfs_root_url}{target_path}"
        mapping[rel_path] = {
            "path": target_path,
            "url": url,
            "basename": os.path.basename(rel_path),
            "source": item["source"],
        }
    return mapping


def render_base_path(template, version, tag):
    try:
        return template.format(version=version, tag=tag)
    except (KeyError, ValueError):
        return template


def update_latest_json_urls(latest_json, upload_map, hubfs_root_url, base_path):
    base_path = "/" + base_path.strip("/")
    hubfs_root_url = hubfs_root_url.rstrip("/")
    basename_to_url = {info["basename"]: info["url"] for info in upload_map.values()}

    def replace_url(value):
        if not isinstance(value, str):
            return value
        parsed = urlparse(value)
        basename = os.path.basename(parsed.path) if parsed.path else os.path.basename(value)
        if basename in basename_to_url:
            return basename_to_url[basename]
        if not parsed.scheme:
            rel = value.lstrip("/")
            return f"{hubfs_root_url}{base_path}/{rel}".replace("//", "/")
        return f"{hubfs_root_url}{base_path}/{basename}".replace("//", "/")

    def walk(obj):
        if isinstance(obj, dict):
            for key, value in obj.items():
                if key == "url":
                    obj[key] = replace_url(value)
                else:
                    walk(value)
        elif isinstance(obj, list):
            for item in obj:
                walk(item)

    walk(latest_json)
    return latest_json


def upload_file_to_hubspot(token, file_path, target_path):
    headers = {"Authorization": f"Bearer {token}"}
    folder_path = os.path.dirname(target_path).replace(os.sep, "/")
    if not folder_path:
        folder_path = "/"
    params = {"overwrite": "true"}
    data = {"folderPath": folder_path, "options": json.dumps({"access": "PUBLIC_NOT_INDEXABLE"})}
    with open(file_path, "rb") as handle:
        files = {"file": (os.path.basename(target_path), handle)}
        resp = requests.post(
            HUBSPOT_FILE_UPLOAD_URL,
            headers=headers,
            params=params,
            data=data,
            files=files,
            timeout=120,
        )
    if resp.status_code not in (200, 201):
        raise RuntimeError(f"HubSpot upload failed for {target_path}: {resp.status_code} {resp.text}")
    return resp.json()


def update_hubdb_row(token, table_id, row_id, values, publish):
    url = HUBSPOT_HUBDB_ROW_URL.format(table_id=table_id, row_id=row_id)
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    payload = {"values": values, "publish": publish}
    resp = requests.patch(url, headers=headers, data=json.dumps(payload), timeout=60)
    if resp.status_code not in (200, 201):
        raise RuntimeError(f"HubDB update failed: {resp.status_code} {resp.text}")
    return resp.json()


def create_hubdb_row(token, table_id, values, publish):
    url = HUBSPOT_HUBDB_ROWS_URL.format(table_id=table_id)
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    payload = {"values": values, "publish": publish}
    resp = requests.post(url, headers=headers, data=json.dumps(payload), timeout=60)
    if resp.status_code not in (200, 201):
        raise RuntimeError(f"HubDB create failed: {resp.status_code} {resp.text}")
    return resp.json()


def build_hubdb_values(latest_json, release_tag, latest_json_url, column_map):
    downloads = []

    def collect_urls(obj):
        if isinstance(obj, dict):
            for key, value in obj.items():
                if key == "url" and isinstance(value, str):
                    downloads.append(value)
                else:
                    collect_urls(value)
        elif isinstance(obj, list):
            for item in obj:
                collect_urls(item)

    collect_urls(latest_json)
    version = find_release_version(latest_json, release_tag)
    resolved = {
        "version": version,
        "release_tag": release_tag,
        "latest_json_url": latest_json_url,
        "latest_json": json.dumps(latest_json, separators=(",", ":")),
        "downloads_json": json.dumps(downloads, separators=(",", ":")),
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }
    values = {}
    for key, column in column_map.items():
        if key in resolved:
            values[column] = resolved[key]
    return values


def parse_column_map(raw):
    if not raw:
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"Invalid HUBDB_COLUMN_MAP JSON: {exc}") from exc
    if not isinstance(parsed, dict):
        raise RuntimeError("HUBDB_COLUMN_MAP must be a JSON object")
    return parsed


def main():
    parser = argparse.ArgumentParser(description="Publish QuiltSync release assets to HubSpot hubfs.")
    parser.add_argument("--github-repo", default="quiltdata/quilt-rs")
    parser.add_argument("--release-tag", default=None)
    parser.add_argument("--hubfs-base-path", default="/quiltsync")
    parser.add_argument("--hubfs-root-url", default="https://www.quilt.bio/hubfs")
    parser.add_argument("--latest-json-target-path", default="/latest.json")
    args = parser.parse_args()

    github_token = os.environ.get("GITHUB_TOKEN")
    hubspot_token = os.environ.get("HUBSPOT_ACCESS_TOKEN")
    if not hubspot_token:
        _err("HUBSPOT_ACCESS_TOKEN is required")
        return 1

    release = fetch_release(args.github_repo, args.release_tag, github_token)
    release_tag = release.get("tag_name") or args.release_tag or "unknown"
    assets = release.get("assets", [])
    if not assets:
        _err("No assets found on release")
        return 1

    with tempfile.TemporaryDirectory() as temp_dir:
        downloaded = [download_asset(asset, temp_dir, github_token) for asset in assets]
        items = []
        for path in downloaded:
            if is_archive(path):
                extracted_root = extract_archive(path, temp_dir)
                for extracted in collect_files(extracted_root):
                    rel_path = os.path.relpath(extracted, start=extracted_root).replace(os.sep, "/")
                    items.append({"source": extracted, "rel_path": rel_path})
            else:
                items.append({"source": path, "rel_path": os.path.basename(path)})

        latest_item = find_latest_json(items)
        if not latest_item:
            _err("latest.json not found in release assets")
            return 1

        latest_json = load_latest_json(latest_item["source"])
        release_version = find_release_version(latest_json, release_tag)
        resolved_base_path = render_base_path(args.hubfs_base_path, release_version, release_tag)

        upload_candidates = [item for item in items if item != latest_item]
        upload_map = build_upload_map(upload_candidates, resolved_base_path, args.hubfs_root_url)

        latest_json = update_latest_json_urls(
            latest_json, upload_map, args.hubfs_root_url, resolved_base_path
        )

        updated_latest_path = os.path.join(temp_dir, "latest.json")
        with open(updated_latest_path, "w", encoding="utf-8") as handle:
            json.dump(latest_json, handle, indent=2, sort_keys=True)

        for info in upload_map.values():
            _log(f"Uploading {info['path']}")
            upload_file_to_hubspot(hubspot_token, info["source"], info["path"])

        latest_target_path = args.latest_json_target_path
        _log(f"Uploading {latest_target_path}")
        upload_file_to_hubspot(hubspot_token, updated_latest_path, latest_target_path)

        hubdb_table_id = os.environ.get("HUBDB_TABLE_ID")
        hubdb_row_id = os.environ.get("HUBDB_ROW_ID")
        hubdb_column_map = parse_column_map(os.environ.get("HUBDB_COLUMN_MAP"))
        hubdb_publish = os.environ.get("HUBDB_PUBLISH", "true").lower() == "true"

        if hubdb_table_id and hubdb_column_map:
            latest_json_url = args.hubfs_root_url.rstrip("/") + args.latest_json_target_path
            values = build_hubdb_values(latest_json, release_tag, latest_json_url, hubdb_column_map)
            if values:
                if hubdb_row_id:
                    _log(f"Updating HubDB row {hubdb_row_id}")
                    update_hubdb_row(hubspot_token, hubdb_table_id, hubdb_row_id, values, hubdb_publish)
                else:
                    _log("Creating HubDB row")
                    create_hubdb_row(hubspot_token, hubdb_table_id, values, hubdb_publish)
            else:
                _log("HubDB column map provided but no values resolved")
        else:
            _log("Skipping HubDB update (set HUBDB_TABLE_ID and HUBDB_COLUMN_MAP to enable)")

    _log("Done")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
