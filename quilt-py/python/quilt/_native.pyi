from os import PathLike
from pathlib import Path
from typing import Any, TypeAlias

_Path: TypeAlias = str | PathLike[str]

__version__: str

class QuiltError(Exception): ...

def default_dataspace(*, domain: _Path | None = None) -> Path | None: ...
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
def commit(
    package: str,
    message: str,
    *,
    dataspace: _Path | None = None,
    domain: _Path | None = None,
    home: _Path | None = None,
) -> dict[str, Any]: ...
