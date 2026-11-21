"""The quilt-rs engine for Python: dataspaces, as quilt3 uses them.

Every call blocks until the engine is done, without holding the GIL. A call
that names no ``dataspace`` uses the default dataspace chosen for the
application folder (``home``); one that names no
``domain`` uses ``QUILT_DOMAIN``, else ``bio.quilt.dataspace`` in the OS's
local data folder. A failure raises :class:`QuiltError`.
"""

from ._native import (
    QuiltError,
    __version__,
    browse,
    commit,
    create,
    default_dataspace,
    delete,
    install,
    list_registries,
    list_stored,
    resolve_hash,
)

__all__ = [
    "QuiltError",
    "__version__",
    "browse",
    "commit",
    "create",
    "default_dataspace",
    "delete",
    "install",
    "list_registries",
    "list_stored",
    "resolve_hash",
]
