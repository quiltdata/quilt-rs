#!/usr/bin/env python3
import argparse
import os
import sys

import requests


HUBSPOT_HUBDB_TABLES_URL = "https://api.hubapi.com/cms/v3/hubdb/tables"
HUBSPOT_HUBDB_COLUMNS_URL = "https://api.hubapi.com/cms/v3/hubdb/tables/{table_id}/columns"

REQUIRED_COLUMNS = [
    {"name": "version", "label": "Version", "type": "TEXT"},
    {"name": "release_tag", "label": "Release Tag", "type": "TEXT"},
    {"name": "latest_json_url", "label": "Latest JSON URL", "type": "URL"},
    {"name": "latest_json", "label": "Latest JSON", "type": "RICH_TEXT"},
    {"name": "downloads_json", "label": "Downloads JSON", "type": "RICH_TEXT"},
    {"name": "updated_at", "label": "Updated At", "type": "DATETIME"},
]


def _err(message):
    print(message, file=sys.stderr, flush=True)


def resolve_table_id(token, table_name):
    headers = {"Authorization": f"Bearer {token}"}
    resp = requests.get(HUBSPOT_HUBDB_TABLES_URL, headers=headers, params={"name": table_name}, timeout=30)
    if resp.status_code == 200:
        for table in resp.json().get("results", []):
            if table.get("name") == table_name or table.get("label") == table_name:
                return table.get("id")

    resp = requests.get(HUBSPOT_HUBDB_TABLES_URL, headers=headers, timeout=30)
    if resp.status_code != 200:
        raise RuntimeError(f"Failed to list tables: {resp.status_code} {resp.text}")
    for table in resp.json().get("results", []):
        if table.get("name") == table_name or table.get("label") == table_name:
            return table.get("id")
    raise RuntimeError(f"HubDB table not found: {table_name}")


def list_columns(token, table_id):
    headers = {"Authorization": f"Bearer {token}"}
    url = HUBSPOT_HUBDB_COLUMNS_URL.format(table_id=table_id)
    resp = requests.get(url, headers=headers, timeout=30)
    if resp.status_code != 200:
        raise RuntimeError(f"Failed to list columns: {resp.status_code} {resp.text}")
    return resp.json().get("results", [])


def create_column(token, table_id, column):
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    url = HUBSPOT_HUBDB_COLUMNS_URL.format(table_id=table_id)
    resp = requests.post(url, headers=headers, json=column, timeout=30)
    if resp.status_code not in (200, 201):
        raise RuntimeError(f"Failed to create column {column['name']}: {resp.status_code} {resp.text}")


def main():
    parser = argparse.ArgumentParser(description="Ensure HubDB columns exist for QuiltSync assets.")
    parser.add_argument("--table", required=True, help="HubDB table name (e.g. quiltsync_assets)")
    args = parser.parse_args()

    token = os.environ.get("HUBSPOT_ACCESS_TOKEN")
    if not token:
        _err("HUBSPOT_ACCESS_TOKEN is required")
        return 1

    table_id = resolve_table_id(token, args.table)
    existing = {col.get("name") for col in list_columns(token, table_id)}

    for column in REQUIRED_COLUMNS:
        if column["name"] in existing:
            continue
        create_column(token, table_id, column)

    print(f"HubDB columns ensured for table {args.table} (id={table_id})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
