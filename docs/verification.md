# File Integrity Verification

For debugging and verification purposes across all projects.

## SHA256-Chunked Verification

### 0Mb Files

```bash
sha256sum ./FILE | xxd -r -p | base64
```

### <= 8Mb Files

```bash
sha256sum ./FILE | xxd -r -p | sha256sum | xxd -r -p | base64
```

### > 8Mb Files

```bash
split -b 8388608 ./FILE --filter='sha256sum' | xxd -r -p | \
  sha256sum | xxd -r -p | base64
```

## Verify Packages

```bash
split -l 1 ~/MANIFEST.jsonl --filter="jq -cSM 'del(.physical_keys)'" | \
  tr -d '\n' | sha256sum
```

**Note**: If your JSONL manifest contains `"meta": null` entries, you need to
convert them to `"meta": {}` first to match the quilt3 implementation's hashing
behavior:

```bash
split -l 1 ~/MANIFEST.jsonl \
  --filter="jq -cSM 'if .meta == null then .meta = {} else . end | \
    del(.physical_keys)'" | \
  tr -d '\n' | sha256sum
```

## CRC64/NVMe Verification

CRC64-NVMe is a whole-file checksum (no chunking). The digest is 8 bytes,
base64-encoded for storage.

### Remote objects (AWS CLI v2.22+)

S3 stores CRC64-NVMe checksums automatically for new objects. Retrieve it
with:

```bash
aws s3api head-object \
  --bucket BUCKET --key KEY \
  --checksum-mode ENABLED
```

**Note**: Requires AWS CLI v2.22+ (or v1.36+). Older versions do not
support CRC64-NVMe headers.

### Local files

[crc-fast](https://github.com/awesomized/crc-fast-rust) provides a
SIMD-accelerated CLI tool. Its output is hex, so convert to base64 to
match the S3 format:

```bash
cargo install crc-fast --features cli
checksum -a CRC-64/NVME -f ./FILE | xxd -r -p | base64
```
