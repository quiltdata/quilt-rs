use std::path::{Path, PathBuf};

use tokio::{
    fs,
    io::{AsyncRead, AsyncWriteExt},
};

use crate::Error;

pub type File = Box<dyn AsyncRead + Unpin + Send>;

pub async fn open(path: impl AsRef<Path>) -> Result<File, Error> {
    // real impl
    Ok(fs::File::open(path)
        .await
        // .map(|file| Box::new(file) as Box<dyn io::AsyncRead + Unpin>)
        .map(|file| Box::new(file) as File)?)

    // TODO: fake impl
}

pub async fn exists(path: impl AsRef<Path>) -> bool {
    // real impl
    fs::metadata(path).await.is_ok()
    // TODO: fake impl
}

pub async fn write(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), Error> {
    let Some(parent) = path.as_ref().parent() else {
        return Err(Error::MissingParentPath(path.as_ref().to_owned()));
    };
    fs::create_dir_all(&parent).await?;

    // TODO: Write to a temporary location, then move.
    let mut file = fs::File::create(&path).await?;

    file.write_all(bytes).await?;

    Ok(())
}

pub async fn read_to_string(path: impl AsRef<Path>) -> Result<String, std::io::Error> {
    fs::read_to_string(path).await
}

// XXX: scope?
// fn child(&self, path: &str) -> Self;

// #[derive(Clone)]
// pub struct MemoryFile {
//     contents: String,
// }
//
// impl AsyncRead for MemoryFile {
//     fn poll_read(
//         self: std::pin::Pin<&mut Self>,
//         _cx: &mut std::task::Context<'_>,
//         _buf: &mut ReadBuf<'_>,
//     ) -> std::task::Poll<std::io::Result<()>> {
//         // TODO: put the data into the buffer
//         std::task::Poll::Ready(Ok(()))
//     }
// }
//
// // XXX: use object_store? it has InMemory impl
// pub type MemoryFS = HashMap<String, String>;
//
// pub trait MemoryFSUtil {
//     fn from_strs<'a>(strs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self;
// }
//
// impl MemoryFSUtil for MemoryFS {
//     fn from_strs<'a>(strs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
//         strs.into_iter()
//             .map(|(k, v)| (k.to_string(), v.to_string()))
//             .collect()
//     }
// }

// fn get(&self, path: &str) -> Option<String> {
//     let key = self.path.join(path).into_os_string().into_string().unwrap();
//     self.root_fs.get(&key).cloned()
// }

// async fn open(&self, path: &str) -> Result<LocalFile, Error> {
//     match self.get(path) {
//         Some(contents) => Ok(Box::new(MemoryFile { contents: contents.clone() })),
//         None => Err(format!("file not found: {}", path)),
//     }
// }
//
// async fn exists(&self, path: &str) -> bool {
//     self.get(path).is_some()
// }

pub async fn get_file_modified_ts(
    path: impl AsRef<Path>,
) -> Result<chrono::DateTime<chrono::Utc>, Error> {
    let modified = tokio::fs::metadata(path).await.map(|m| m.modified())??;
    Ok(chrono::DateTime::<chrono::Utc>::from(modified))
}

pub trait RemoveFile {
    fn remove_file(
        &self,
        path: PathBuf,
    ) -> impl std::future::Future<Output = Result<(), std::io::Error>> + Send {
        async { fs::remove_file(path).await }
    }
}

pub trait FsExists {
    fn exists(&self, path: impl AsRef<Path>) -> impl std::future::Future<Output = bool> {
        async { exists(path).await }
    }
}

pub trait FsCopy {
    fn copy(
        &self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> impl std::future::Future<Output = Result<u64, std::io::Error>> {
        async { fs::copy(from, to).await }
    }
}

pub trait FsCreateDir {
    fn create_dir_all(
        &self,
        path: impl AsRef<Path>,
    ) -> impl std::future::Future<Output = Result<(), std::io::Error>> {
        async { fs::create_dir_all(path).await }
    }
}

pub trait FsModifiedDate {
    fn modified_date(
        &self,
        path: impl AsRef<Path>,
    ) -> impl std::future::Future<Output = Result<chrono::DateTime<chrono::Utc>, Error>> {
        async move { get_file_modified_ts(path).await }
    }
}

#[derive(Clone)]
pub struct RelativeFileOps {
    working_dir: PathBuf,
}

impl RemoveFile for RelativeFileOps {
    async fn remove_file(&self, relative_path: PathBuf) -> Result<(), std::io::Error> {
        let path = &self.working_dir.join(relative_path);
        fs::remove_file(path).await
    }
}

impl FsExists for RelativeFileOps {}

impl FsCopy for RelativeFileOps {}

impl FsCreateDir for RelativeFileOps {}

impl FsModifiedDate for RelativeFileOps {}

impl RelativeFileOps {
    pub fn new(working_dir: PathBuf) -> Self {
        RelativeFileOps { working_dir }
    }
}
