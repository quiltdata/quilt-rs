# quilt (Python)

The Python binding of [`quilt-rs`](../quilt-rs/), built with
[maturin](https://www.maturin.rs/) into platform wheels and published to PyPI
as `quilt` 3.0. quilt3 depends on it to work with dataspaces through the
engine, as [quilt-specs#297](https://github.com/quiltdata/quilt-specs/pull/297)
rules. It replaces the legacy Quilt 2 library on PyPI (2.9.15), so
`pip install -U quilt` on a Quilt 2 install gets this one.

```python
import quilt

quilt.install("quilt+s3://bucket#package=team/raw", dataspace="~/data")
quilt.create("quilt+s3://bucket#package=team/new", dataspace="~/data")
quilt.commit("team/raw", "first", dataspace="~/data")
quilt.list_registries()
```

Every call blocks until the engine is done, without holding the GIL, on a
Tokio runtime the module owns. A call that names no `dataspace` uses the
default dataspace chosen for the application folder, `home`, as `quilt home`
chooses it; `home` defaults to the folder the `quilt` CLI and QuiltSync keep,
which also holds the credentials. A call that names no `domain` uses
`QUILT_DOMAIN`, else `bio.quilt.dataspace` in the OS's local data folder. A
failure raises `quilt.QuiltError`.

## Develop

```sh
cd quilt-py
uv venv && uv pip install maturin pytest
uv run maturin develop
uv run pytest tests
```

`cargo test -p quilt-py` tests the engine layer without Python.
