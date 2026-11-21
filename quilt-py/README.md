# quilt (Python)

The Python binding of [`quilt-rs`](../quilt-rs/), built with
[maturin](https://www.maturin.rs/) into platform wheels and published to PyPI
as `quilt` 3.0. quilt3 depends on it to work with dataspaces through the
engine, as [quilt-specs#297](https://github.com/quiltdata/quilt-specs/pull/297)
rules. It replaces the legacy Quilt 2 library on PyPI (2.9.15), so
`pip install -U quilt` on a Quilt 2 install gets this one.

```python
from pathlib import Path

import quilt

data = Path("~/data").expanduser()
quilt.install("quilt+s3://bucket#package=team/raw", dataspace=data)
quilt.create("quilt+s3://bucket#package=team/new", dataspace=data)
quilt.commit("team/raw", "first", dataspace=data)
quilt.list_registries()
quilt.browse("team/raw", dataspace=data)
quilt.resolve_hash("quilt+s3://bucket#package=team/raw", "1a2b3c4")
quilt.delete("team/raw")
```

Every call blocks until the engine is done, without holding the GIL, on a
Tokio runtime the module owns. A call that names no `dataspace` uses the
default dataspace chosen for the application folder, `home`, as `quilt home`
chooses it; `home` defaults to the folder the `quilt` CLI and QuiltSync keep,
which also holds the credentials. A call that names no `domain` uses
`QUILT_DOMAIN`, else `bio.quilt.dataspace` in the OS's local data folder. A
failure raises `quilt.QuiltError`.

`create` and `commit` check the commit against its bucket's current
workflows config and record it either way: the dict they return lists what
the check found under `warnings`, each with `kind` (`inconsistent`, so a push
there will be refused until it is fixed, or `unchecked`, where the config
could not be read or its validator is misconfigured), `bucket`, `reason` and
the `message` the CLI prints. A package in the local registry is not checked.

`browse` opens a package of a dataspace at the revision its lock holds, its
last local commit, else its base, and returns its `manifest` as JSON Lines
text with its `hash`. `resolve_hash` matches a short top hash among the
package's own installed manifests in its store. `delete` removes a package,
or with `top_hash` one revision of it, from every local Package Store and the
local registry, and returns what it removed from each; the dataspaces, the
buckets and the data objects are left.

`entries` lists the packages a dataspace's contract declares, each with its
`access`, `folder` and `paths`, so quilt3's build can tell a build onto an
existing entry from a new package. `set_package_metadata` replaces the
package metadata in a working folder's `metadata.toml`, which its next
commit publishes.

## Develop

```sh
cd quilt-py
uv venv && uv pip install maturin pytest
uv run maturin develop
uv run pytest tests
```

`cargo test -p quilt-py` tests the engine layer without Python.
