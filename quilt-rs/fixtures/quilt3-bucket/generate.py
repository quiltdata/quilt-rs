# /// script
# requires-python = ">=3.10"
# dependencies = ["quilt3==7.3.0", "moto[s3]==5.*"]
# ///
"""Writes what real quilt3 leaves in a versioned bucket when it pushes a
package twice: `v1/` holds every object after the first push, `v2/` every
object the second push added or replaced.

Each file sits at its S3 key. Data objects are versioned; their version IDs
are in the manifests' physical keys. Not run in CI. Regenerate with:

    uv run quilt-rs/fixtures/quilt3-bucket/generate.py
"""

import os
import shutil
import time
from pathlib import Path

os.environ.update(
    AWS_ACCESS_KEY_ID="testing",
    AWS_SECRET_ACCESS_KEY="testing",
    AWS_DEFAULT_REGION="us-east-1",
    QUILT_DISABLE_CACHE="true",
)

import boto3  # noqa: E402
from moto import mock_aws  # noqa: E402

BUCKET = "quilt3-bucket"
NAME = "quilt3/pkg"
HERE = Path(__file__).resolve().parent


def snapshot(s3):
    out = {}
    for page in s3.get_paginator("list_object_versions").paginate(Bucket=BUCKET):
        for v in page.get("Versions", []):
            if v["IsLatest"]:
                out[(v["Key"], v["VersionId"])] = s3.get_object(
                    Bucket=BUCKET, Key=v["Key"], VersionId=v["VersionId"]
                )["Body"].read()
    return out


def write(step, objects):
    root = HERE / step
    shutil.rmtree(root, ignore_errors=True)
    for (key, _), body in objects.items():
        path = root / key
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)


@mock_aws
def main():
    import quilt3

    s3 = boto3.client("s3")
    s3.create_bucket(Bucket=BUCKET)
    s3.put_bucket_versioning(
        Bucket=BUCKET, VersioningConfiguration={"Status": "Enabled"}
    )
    work = HERE / "_work"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    (work / "data.txt").write_text("one\n")
    (work / "same.txt").write_text("same\n")

    pkg = quilt3.Package()
    pkg.set("data.txt", str(work / "data.txt"))
    pkg.set("same.txt", str(work / "same.txt"))
    pkg.push(NAME, registry=f"s3://{BUCKET}", message="v1", force=True)
    v1 = snapshot(s3)

    # Pointers are named in whole seconds; keep the two pushes apart.
    time.sleep(1.5)
    (work / "data.txt").write_text("two\n")
    pkg = quilt3.Package.browse(NAME, registry=f"s3://{BUCKET}")
    pkg.set("data.txt", str(work / "data.txt"))
    pkg.push(NAME, registry=f"s3://{BUCKET}", message="v2")
    v2 = {k: b for k, b in snapshot(s3).items() if v1.get(k) != b}

    shutil.rmtree(work)
    write("v1", v1)
    write("v2", v2)
    print("quilt3", quilt3.__version__)


main()
