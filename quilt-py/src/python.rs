//! The `quilt._native` extension module: [`engine`]'s calls, for Python.
//!
//! The module owns one multi-threaded Tokio runtime, made on first use. A
//! call blocks on it until the engine is done, with the GIL released, so
//! other Python threads run meanwhile. Results come back as dicts and lists;
//! a failure raises `quilt.QuiltError` with the engine's message.

use std::path::PathBuf;
use std::sync::OnceLock;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use serde_json::Value;
use tokio::runtime::Runtime;

use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::engine;

create_exception!(
    quilt,
    QuiltError,
    PyException,
    "A quilt engine call failed."
);

static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();

/// Runs `call` to completion on the module's runtime, the GIL released.
fn block_on<T, F>(py: Python<'_>, call: F) -> PyResult<T>
where
    T: Send,
    F: Future<Output = engine::Result<T>> + Send,
{
    let runtime = RUNTIME
        .get_or_init(Runtime::new)
        .as_ref()
        .map_err(|err| QuiltError::new_err(format!("cannot start the runtime: {err}")))?;
    py.detach(|| runtime.block_on(call))
        .map_err(|err| QuiltError::new_err(err.to_string()))
}

/// `value` as Python's `json` module would load it.
fn to_python(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let text = serde_json::to_string(value).map_err(|err| QuiltError::new_err(err.to_string()))?;
    Ok(py.import("json")?.call_method1("loads", (text,))?.unbind())
}

fn remote(home: PathBuf) -> RemoteS3 {
    RemoteS3::new(DomainPaths::new(home), LocalStorage::new())
}

/// The dataspace a call that names none uses, or None.
#[pyfunction]
#[pyo3(signature = (*, home=None))]
fn default_dataspace(py: Python<'_>, home: Option<PathBuf>) -> PyResult<Option<PathBuf>> {
    block_on(py, async move {
        let home = engine::home(home.as_deref())?;
        engine::default_dataspace(&home).await
    })
}

/// The registries the Domain holds locally, as `s3://<bucket>` and
/// `quilt+local://<name>`.
#[pyfunction]
#[pyo3(signature = (*, domain=None))]
fn list_registries(py: Python<'_>, domain: Option<PathBuf>) -> PyResult<Vec<String>> {
    block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        engine::list_registries(&domain).await
    })
}

/// Every package the Domain's stores hold, across every registry.
#[pyfunction]
#[pyo3(signature = (*, domain=None))]
fn list_stored(py: Python<'_>, domain: Option<PathBuf>) -> PyResult<Py<PyAny>> {
    let packages = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        engine::list_stored(&domain).await
    })?;
    to_python(py, &Value::Array(packages))
}

/// Installs a package into a dataspace: adds or merges its entry under
/// `access`, then reads it.
#[pyfunction]
#[pyo3(signature = (uri, *, dataspace=None, access="sync".to_string(), paths=Vec::new(), folder=None, domain=None, home=None))]
#[allow(clippy::too_many_arguments, reason = "Python keyword arguments")]
fn install(
    py: Python<'_>,
    uri: String,
    dataspace: Option<PathBuf>,
    access: String,
    paths: Vec<String>,
    folder: Option<PathBuf>,
    domain: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    let installed = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        let home = engine::home(home.as_deref())?;
        let dataspace = engine::dataspace(dataspace, &home).await?;
        let request = engine::InstallRequest {
            uri,
            access,
            paths,
            folder,
        };
        engine::install(&remote(home), &domain, &dataspace, request).await
    })?;
    to_python(py, &installed)
}

/// Creates a package in a dataspace, as a build: declares it and, under
/// `sync`, commits its working folder as its first commit.
#[pyfunction]
#[pyo3(signature = (uri, *, dataspace=None, access="sync".to_string(), folder=None, message=None, domain=None, home=None))]
#[allow(clippy::too_many_arguments, reason = "Python keyword arguments")]
fn create(
    py: Python<'_>,
    uri: String,
    dataspace: Option<PathBuf>,
    access: String,
    folder: Option<PathBuf>,
    message: Option<String>,
    domain: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    let created = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        let home = engine::home(home.as_deref())?;
        let dataspace = engine::dataspace(dataspace, &home).await?;
        let request = engine::CreateRequest {
            uri,
            access,
            folder,
            message,
        };
        engine::create(&remote(home.clone()), &domain, &home, &dataspace, request).await
    })?;
    to_python(py, &created)
}

/// Commits a package's working folder in a dataspace as a local commit.
#[pyfunction]
#[pyo3(signature = (package, message, *, dataspace=None, domain=None, home=None))]
fn commit(
    py: Python<'_>,
    package: String,
    message: String,
    dataspace: Option<PathBuf>,
    domain: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    let committed = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        let home = engine::home(home.as_deref())?;
        let dataspace = engine::dataspace(dataspace, &home).await?;
        let remote = remote(home.clone());
        engine::commit(&remote, &domain, &home, &dataspace, &package, &message).await
    })?;
    to_python(py, &committed)
}

/// Opens a package of a dataspace at the revision its lock holds: the last
/// local commit, else the base. The manifest comes back as JSON Lines text.
#[pyfunction]
#[pyo3(signature = (package, *, dataspace=None, domain=None, home=None))]
fn browse(
    py: Python<'_>,
    package: String,
    dataspace: Option<PathBuf>,
    domain: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    let browsed = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        let home = engine::home(home.as_deref())?;
        let dataspace = engine::dataspace(dataspace, &home).await?;
        engine::browse(&remote(home), &domain, &dataspace, &package).await
    })?;
    to_python(py, &browsed)
}

/// The whole top hash of the one revision of a package, in its local store,
/// that starts with `prefix`.
#[pyfunction]
#[pyo3(signature = (uri, prefix, *, domain=None))]
fn resolve_hash(
    py: Python<'_>,
    uri: String,
    prefix: String,
    domain: Option<PathBuf>,
) -> PyResult<String> {
    block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        engine::resolve_hash(&domain, &uri, &prefix).await
    })
}

/// Deletes a package, or one revision of it, from every local Package Store.
#[pyfunction]
#[pyo3(signature = (name, *, top_hash=None, domain=None))]
fn delete(
    py: Python<'_>,
    name: String,
    top_hash: Option<String>,
    domain: Option<PathBuf>,
) -> PyResult<Py<PyAny>> {
    let deleted = block_on(py, async move {
        let domain = engine::domain(domain.as_deref())?;
        engine::delete(&domain, &name, top_hash.as_deref()).await
    })?;
    to_python(py, &Value::Array(deleted))
}

#[pymodule]
#[pyo3(name = "_native")]
fn native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("QuiltError", m.py().get_type::<QuiltError>())?;
    m.add_function(wrap_pyfunction!(default_dataspace, m)?)?;
    m.add_function(wrap_pyfunction!(list_registries, m)?)?;
    m.add_function(wrap_pyfunction!(list_stored, m)?)?;
    m.add_function(wrap_pyfunction!(install, m)?)?;
    m.add_function(wrap_pyfunction!(create, m)?)?;
    m.add_function(wrap_pyfunction!(commit, m)?)?;
    m.add_function(wrap_pyfunction!(browse, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_hash, m)?)?;
    m.add_function(wrap_pyfunction!(delete, m)?)?;
    Ok(())
}
