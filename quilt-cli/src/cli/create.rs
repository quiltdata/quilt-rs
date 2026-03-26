use std::path::PathBuf;

use quilt_rs::uri::RevisionPointer;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub uri: String,
    pub source: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Output {
    installed_package: quilt_rs::InstalledPackage,
}

#[cfg(test)]
impl Output {
    pub fn get_installed_package(self) -> quilt_rs::InstalledPackage {
        self.installed_package
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"Created package "{}""##, self.installed_package.namespace)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.create(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { uri, source }: Input,
) -> Result<Output, Error> {
    let uri: quilt_rs::uri::S3PackageUri = uri.parse()?;
    if uri.path.is_some() {
        return Err(Error::Quilt(quilt_rs::Error::PackageURI(
            "create URI must not include a path".to_string(),
        )));
    }
    match &uri.revision {
        RevisionPointer::Tag(tag) if tag == quilt_rs::uri::LATEST_TAG => {}
        _ => {
            return Err(Error::Quilt(quilt_rs::Error::PackageURI(
                "create URI must not include a specific revision".to_string(),
            )))
        }
    }

    let installed_package = local_domain.create_package(&uri, source.as_ref()).await?;
    Ok(Output { installed_package })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;
    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;

    #[test(tokio::test)]
    async fn test_create_no_source() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;
        let output = model(
            m.get_local_domain(),
            Input {
                uri: "quilt+s3://bucket#package=foo/bar".to_string(),
                source: None,
            },
        )
        .await?;

        assert_eq!(format!("{output}"), r#"Created package "foo/bar""#);
        let installed_package = output.get_installed_package();
        let lineage = installed_package.lineage().await?;
        assert_eq!(installed_package.namespace.to_string(), "foo/bar");
        assert_eq!(lineage.remote.bucket, "bucket");
        assert!(lineage.remote_hash.is_none());
        assert!(lineage.base_hash.is_none());
        assert!(lineage.latest_hash.is_none());

        let storage = LocalStorage::new();
        assert!(storage.exists(temp_dir.path().join("foo/bar")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_source() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;
        let source_dir = temp_dir.path().join("seed");
        std::fs::create_dir_all(source_dir.join("nested"))?;
        std::fs::write(source_dir.join("nested/keep.txt"), b"hello")?;
        std::fs::write(source_dir.join("skip.log"), b"ignore me")?;
        std::fs::write(source_dir.join(".quiltignore"), b"*.log\n")?;

        model(
            m.get_local_domain(),
            Input {
                uri: "quilt+s3://bucket#package=foo/bar".to_string(),
                source: Some(source_dir.clone()),
            },
        )
        .await?;

        let storage = LocalStorage::new();
        assert!(storage
            .exists(temp_dir.path().join("foo/bar/nested/keep.txt"))
            .await);
        assert!(!storage.exists(temp_dir.path().join("foo/bar/skip.log")).await);
        Ok(())
    }
}
