from os import PathLike
from pathlib import Path
from typing import Any, TypeAlias

_Path: TypeAlias = str | PathLike[str]

__version__: str
# Whether this build stops at quilt-rs's failpoints (a crash-test wheel).
FAILPOINTS: bool

class QuiltError(Exception): ...

def default_dataspace(*, home: _Path | None = None) -> Path | None: ...
def list_registries(*, domain: _Path | None = None) -> list[str]: ...
def list_stored(*, domain: _Path | None = None) -> list[dict[str, Any]]: ...
def install(
    uri: str,
    *,
    dataspace: _Path | None = None,
    access: str = "sync",
    paths: list[str] = ...,
    folder: _Path | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> dict[str, Any]: ...
def create(
    uri: str,
    *,
    dataspace: _Path | None = None,
    access: str = "sync",
    folder: _Path | None = None,
    message: str | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> dict[str, Any]: ...
def commit(
    package: str,
    message: str,
    *,
    dataspace: _Path | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> dict[str, Any]: ...
def browse(
    package: str,
    *,
    dataspace: _Path | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> dict[str, Any]: ...
def resolve_hash(uri: str, prefix: str, *, domain: _Path | None = None) -> str: ...
def delete(
    name: str,
    *,
    top_hash: str | None = None,
    domain: _Path | None = None,
) -> list[dict[str, Any]]: ...
def entries(
    *,
    dataspace: _Path | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> list[dict[str, Any]]: ...
def set_package_metadata(folder: _Path, metadata: Any) -> None: ...
