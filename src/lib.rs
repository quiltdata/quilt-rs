//! For all operations instantiate `LocalDomain` and then call some of its methods.
//!
//! For example, for installing package you can create path, where everything will be stored.
//! There will be `.quilt` directory and working directory for each package.
//! ```rs
//! let path = PathBuf::from("/foo/bar");
//! ```
//! Instantiate `LocalDomain` for that path .
//! ```rs
//! let local_domain = quilt_rs::LocalDomain::new(path);
//! ```
//! Create `ManifestUri`.
//! You can do this by creating "quilt+s3" URI and convert it.
//! ```rs
//! let package_uri = S3PackageUri::try_from("quilt+s3://lorem#package=ipsum@hash-is-required")?;
//! let manifest_uri = ManifestUri::try_from(package_uri)?;
//! ```
//! Then call `install_package` method. You will get `InstalledPackage` as output.
//! ```rs
//! let installed_package = local_domain.install_package(&manifest_uri).await?;
//! ```

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

use std::path::PathBuf;

pub mod flow;

pub mod checksum;
mod error;
mod installed_package;
pub mod io;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod paths;
mod perf;
pub mod uri;

#[cfg(test)]
pub mod mocks;

pub use error::Error;
pub use installed_package::InstalledPackage;
pub use local_domain::LocalDomain;

#[no_mangle]
pub extern "system" fn Java_Quilt_install<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    domain: JString<'local>,
    uri: JString<'local>,
) -> jstring {
    let runtime = tokio::runtime::Runtime::new();
    let manifest_str: Result<String, Error> = runtime.unwrap().block_on(async {
        let domain: String = env.get_string(&domain)?.into();
        let uri: String = env.get_string(&uri)?.into();

        let local_domain = LocalDomain::new(PathBuf::from(domain));
        let remote = io::remote::RemoteS3::new();
        let uri: uri::S3PackageUri = uri.parse().unwrap();
        let manifest_uri = io::manifest::resolve_manifest_uri(&remote, &uri).await?;
        let manifest = local_domain.install_package(&manifest_uri).await;
        let manifest_str = format!("{:?}", manifest);
        Ok(manifest_str)
    });

    env.new_string(manifest_str.expect("Failed to install"))
        .expect("Couldn't create java string!")
        .into_raw()
}

pub type Res<T = ()> = std::result::Result<T, Error>;
