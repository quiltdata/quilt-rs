//! Cheap per-file stat values ("stamps") that tell a stat cache whether a file
//! must be hashed again.
//!
//! Listing a folder costs one call per folder on macOS (`getattrlistbulk`) and
//! Windows (`FileIdExtdDirectoryInfo`), and one `lstat` per entry elsewhere.
//! Both functions block and never follow symlinks.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Stat values of one file system entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// Length in bytes for regular files, 0 for everything else.
    pub size: u64,
    pub modified: SystemTime,
    /// ctime on Unix, `ChangeTime` on Windows. `None` where the platform has none: the cache then hashes.
    pub changed: Option<SystemTime>,
    /// (device, inode) on Unix, (volume serial, 128-bit file ID) on Windows.
    pub id: Option<FileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    pub volume: u64,
    pub file: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// Symlink, reparse point, device, socket and so on. Never followed.
    Other,
}

/// One folder's entries (not recursive, no "." / ".."), with a stamp each.
///
/// An entry that disappears while the folder is read is left out.
///
/// Where the folder call fails but `std::fs` can still list the folder (a file
/// system without file IDs, such as FAT), the entries come with no change time
/// and no file ID, so the cache hashes them.
pub fn read_dir_stamps(dir: &Path) -> io::Result<Vec<(OsString, EntryKind, FileStamp)>> {
    #[cfg(target_os = "macos")]
    return macos::read_dir_stamps(dir)
        .or_else(|err| untrusted::read_dir_stamps(dir).map_err(|_| err));
    #[cfg(windows)]
    return windows::read_dir_stamps(dir)
        .or_else(|err| untrusted::read_dir_stamps(dir).map_err(|_| err));
    #[cfg(not(any(target_os = "macos", windows)))]
    return portable::read_dir_stamps(dir);
}

/// One path, not following a final symlink.
pub fn stamp(path: &Path) -> io::Result<FileStamp> {
    #[cfg(windows)]
    return windows::stamp(path);
    #[cfg(not(windows))]
    return portable::stamp(path);
}

/// Whether `path` is on a local volume, whose file events can be trusted: not a
/// network share or a FUSE mount. `false` when it can't be told.
#[must_use]
pub fn on_local_volume(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    return macos::on_local_volume(path);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return linux::on_local_volume(path);
    #[cfg(windows)]
    return windows::on_local_volume(path);
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        windows
    )))]
    {
        let _ = path;
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// `statfs` magic numbers of network and FUSE file systems.
    const REMOTE: [u32; 9] = [
        0x6969,      // NFS
        0x517B,      // SMB
        0xFF53_4D42, // CIFS
        0xFE53_4D42, // SMB2
        0x7375_7245, // Coda
        0x5346_414F, // AFS
        0x0102_1997, // 9P
        0x6573_5546, // FUSE
        0x00C3_6400, // Ceph
    ];

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "`f_type` is a signed word whose low 32 bits are the magic number"
    )]
    pub(super) fn on_local_volume(path: &Path) -> bool {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `statfs` is plain data, valid zeroed.
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated and `fs` is a valid out pointer.
        if unsafe { libc::statfs(path.as_ptr(), &raw mut fs) } != 0 {
            return false;
        }
        !REMOTE.contains(&(fs.f_type as u32))
    }
}

/// `std::fs` listing with neither a change time nor a file ID: what the folder
/// calls fall back to.
#[cfg(any(target_os = "macos", windows))]
mod untrusted {
    use super::{EntryKind, FileStamp};
    use std::ffi::OsString;
    use std::io;
    use std::path::Path;

    pub(super) fn read_dir_stamps(dir: &Path) -> io::Result<Vec<(OsString, EntryKind, FileStamp)>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            // `DirEntry::metadata` does not follow symlinks.
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            let file_type = meta.file_type();
            let kind = if file_type.is_symlink() {
                EntryKind::Other
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else if file_type.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            out.push((
                entry.file_name(),
                kind,
                FileStamp {
                    size: if meta.is_file() { meta.len() } else { 0 },
                    modified: meta.modified()?,
                    changed: None,
                    id: None,
                },
            ));
        }
        Ok(out)
    }
}

/// Time `secs` seconds and `nanos` nanoseconds after the Unix epoch; `secs` may be negative.
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
fn epoch_time(secs: i64, nanos: i64) -> io::Result<SystemTime> {
    let nanos = u32::try_from(nanos)
        .ok()
        .filter(|n| *n < 1_000_000_000)
        .ok_or_else(|| invalid_data("timestamp nanoseconds out of range"))?;
    let base = if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs.unsigned_abs()))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
    };
    base.and_then(|t| t.checked_add(Duration::from_nanos(u64::from(nanos))))
        .ok_or_else(|| invalid_data("timestamp out of range"))
}

#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
fn invalid_data(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Reads a `T` at byte offset `at` of `buf`, at any alignment.
///
/// `T` must be plain data for which every bit pattern is valid (integers and
/// `repr(C)` structs of integers).
#[cfg(any(target_os = "macos", windows))]
fn read_at<T: Copy>(buf: &[u8], at: usize) -> io::Result<T> {
    let end = at
        .checked_add(size_of::<T>())
        .filter(|end| *end <= buf.len())
        .ok_or_else(|| invalid_data("record runs past the end of the buffer"))?;
    let bytes = &buf[at..end];
    // SAFETY: `bytes` holds exactly `size_of::<T>()` bytes, `read_unaligned`
    // has no alignment requirement, and callers only use types valid for any
    // bit pattern.
    Ok(unsafe { bytes.as_ptr().cast::<T>().read_unaligned() })
}

/// `std::fs` based listing: Linux and other Unix use it for folders, macOS for
/// single paths, and targets without a native call for both.
#[cfg(not(windows))]
mod portable {
    use super::{EntryKind, FileStamp};
    use std::ffi::OsString;
    use std::fs::{FileType, Metadata};
    use std::io;
    use std::path::Path;

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(super) fn read_dir_stamps(dir: &Path) -> io::Result<Vec<(OsString, EntryKind, FileStamp)>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            // `DirEntry::metadata` does not follow symlinks.
            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            out.push((
                entry.file_name(),
                kind_of(meta.file_type()),
                from_metadata(&meta)?,
            ));
        }
        Ok(out)
    }

    pub(super) fn stamp(path: &Path) -> io::Result<FileStamp> {
        from_metadata(&std::fs::symlink_metadata(path)?)
    }

    fn kind_of(file_type: FileType) -> EntryKind {
        if file_type.is_symlink() {
            EntryKind::Other
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        }
    }

    #[cfg(unix)]
    fn from_metadata(meta: &Metadata) -> io::Result<FileStamp> {
        use super::{FileId, epoch_time};
        use std::os::unix::fs::MetadataExt;
        Ok(FileStamp {
            size: if meta.is_file() { meta.len() } else { 0 },
            modified: epoch_time(meta.mtime(), meta.mtime_nsec())?,
            changed: Some(epoch_time(meta.ctime(), meta.ctime_nsec())?),
            id: Some(FileId {
                volume: meta.dev(),
                file: u128::from(meta.ino()),
            }),
        })
    }

    #[cfg(not(unix))]
    fn from_metadata(meta: &Metadata) -> io::Result<FileStamp> {
        Ok(FileStamp {
            size: if meta.is_file() { meta.len() } else { 0 },
            modified: meta.modified()?,
            changed: None,
            id: None,
        })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{EntryKind, FileId, FileStamp, epoch_time, invalid_data, read_at};
    use std::ffi::{CString, OsString};
    use std::io;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    const BUF_LEN: usize = 64 * 1024;
    // `enum vtype` in <sys/vnode.h>; libc does not export it.
    const VREG: u32 = 1;
    const VDIR: u32 = 2;

    const COMMON_ATTRS: libc::attrgroup_t = libc::ATTR_CMN_RETURNED_ATTRS
        | libc::ATTR_CMN_NAME
        | libc::ATTR_CMN_DEVID
        | libc::ATTR_CMN_OBJTYPE
        | libc::ATTR_CMN_MODTIME
        | libc::ATTR_CMN_CHGTIME
        | libc::ATTR_CMN_FILEID;

    struct Fd(libc::c_int);

    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: the descriptor came from `open`, is owned here and closed once.
            unsafe {
                libc::close(self.0);
            }
        }
    }

    /// A local volume has `MNT_LOCAL`; macFUSE can set it too, so FUSE is
    /// told by its type name.
    pub(super) fn on_local_volume(path: &Path) -> bool {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `statfs` is plain data, valid zeroed.
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `path` is NUL-terminated and `fs` is a valid out pointer.
        if unsafe { libc::statfs(path.as_ptr(), &raw mut fs) } != 0 {
            return false;
        }
        // SAFETY: the kernel NUL-terminates `f_fstypename`.
        let name = unsafe { std::ffi::CStr::from_ptr(fs.f_fstypename.as_ptr()) }.to_bytes();
        fs.f_flags & libc::MNT_LOCAL.cast_unsigned() != 0 && !name.windows(4).any(|w| w == b"fuse")
    }

    pub(super) fn read_dir_stamps(dir: &Path) -> io::Result<Vec<(OsString, EntryKind, FileStamp)>> {
        let c_path = CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
        // SAFETY: `c_path` is a valid NUL-terminated string.
        let raw = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = Fd(raw);

        let mut attrs = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: COMMON_ATTRS,
            volattr: 0,
            dirattr: 0,
            fileattr: libc::ATTR_FILE_DATALENGTH,
            forkattr: 0,
        };
        let mut buf = vec![0u8; BUF_LEN];
        let mut out = Vec::new();
        loop {
            // SAFETY: `fd` is an open folder, `attrs` is a valid attrlist and
            // `buf` is writable for `buf.len()` bytes.
            let count = unsafe {
                libc::getattrlistbulk(
                    fd.0,
                    (&raw mut attrs).cast(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    u64::from(libc::FSOPT_PACK_INVAL_ATTRS),
                )
            };
            if count < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if count == 0 {
                return Ok(out);
            }
            let mut pos = 0usize;
            for _ in 0..count {
                let len = usize::try_from(read_at::<u32>(&buf, pos)?)
                    .map_err(|_| invalid_data("record length too large"))?;
                let end = pos
                    .checked_add(len)
                    .filter(|end| len >= size_of::<u32>() && *end <= buf.len())
                    .ok_or_else(|| invalid_data("bad getattrlistbulk record length"))?;
                out.push(parse_record(&buf[pos..end])?);
                pos = end;
            }
        }
    }

    /// Parses one record. With `FSOPT_PACK_INVAL_ATTRS` every requested common
    /// attribute is packed, in bit order; the returned set says which are valid.
    fn parse_record(rec: &[u8]) -> io::Result<(OsString, EntryKind, FileStamp)> {
        let mut cur = size_of::<u32>();
        let returned: libc::attribute_set_t = take(rec, &mut cur)?;
        let name_ref_at = cur;
        let name_ref: libc::attrreference_t = take(rec, &mut cur)?;
        let dev: libc::dev_t = take(rec, &mut cur)?;
        let obj_type: u32 = take(rec, &mut cur)?;
        let mtime: libc::timespec = take(rec, &mut cur)?;
        let ctime: libc::timespec = take(rec, &mut cur)?;
        let file_id: u64 = take(rec, &mut cur)?;

        let has = |bit| returned.commonattr & bit != 0;
        if !has(libc::ATTR_CMN_NAME) || !has(libc::ATTR_CMN_MODTIME) {
            return Err(invalid_data("getattrlistbulk record lacks name or mtime"));
        }

        // The name offset counts from the attrreference itself.
        let name_start = isize::try_from(name_ref_at)
            .ok()
            .and_then(|at| at.checked_add(isize::try_from(name_ref.attr_dataoffset).ok()?))
            .and_then(|at| usize::try_from(at).ok());
        let name_len = usize::try_from(name_ref.attr_length).ok();
        let name = name_start
            .zip(name_len)
            .and_then(|(start, len)| rec.get(start..start.checked_add(len)?))
            .ok_or_else(|| invalid_data("getattrlistbulk name out of bounds"))?;
        let name = name.split(|b| *b == 0).next().unwrap_or_default();
        if name.is_empty() {
            return Err(invalid_data("getattrlistbulk record has an empty name"));
        }

        let kind = if !has(libc::ATTR_CMN_OBJTYPE) {
            EntryKind::Other
        } else if obj_type == VREG {
            EntryKind::File
        } else if obj_type == VDIR {
            EntryKind::Dir
        } else {
            EntryKind::Other
        };

        // File attributes follow the common ones; only regular files need them.
        let size = if kind == EntryKind::File {
            if returned.fileattr & libc::ATTR_FILE_DATALENGTH == 0 {
                return Err(invalid_data("getattrlistbulk record lacks the file length"));
            }
            let len: libc::off_t = take(rec, &mut cur)?;
            u64::try_from(len).map_err(|_| invalid_data("negative file length"))?
        } else {
            0
        };

        let changed = if has(libc::ATTR_CMN_CHGTIME) {
            Some(epoch_time(ctime.tv_sec, ctime.tv_nsec)?)
        } else {
            None
        };
        let id = (has(libc::ATTR_CMN_DEVID) && has(libc::ATTR_CMN_FILEID)).then(|| FileId {
            // Sign-extended, as `MetadataExt::dev` does.
            volume: i64::from(dev).cast_unsigned(),
            file: u128::from(file_id),
        });
        let stamp = FileStamp {
            size,
            modified: epoch_time(mtime.tv_sec, mtime.tv_nsec)?,
            changed,
            id,
        };
        Ok((OsString::from_vec(name.to_vec()), kind, stamp))
    }

    fn take<T: Copy>(rec: &[u8], cur: &mut usize) -> io::Result<T> {
        let value = read_at(rec, *cur)?;
        *cur += size_of::<T>();
        Ok(value)
    }
}

#[cfg(windows)]
mod windows {
    use super::{EntryKind, FileId, FileStamp, epoch_time, invalid_data, read_at};
    use std::ffi::OsString;
    use std::io;
    use std::mem::offset_of;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;
    use std::time::SystemTime;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAGS_AND_ATTRIBUTES, FILE_ID_EXTD_DIR_INFO,
        FILE_ID_INFO, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO,
        FileAttributeTagInfo, FileBasicInfo, FileIdExtdDirectoryInfo,
        FileIdExtdDirectoryRestartInfo, FileIdInfo, FileStandardInfo, GetDriveTypeW,
        GetFileInformationByHandleEx, OPEN_EXISTING, SYNCHRONIZE,
    };

    const BUF_LEN: usize = 64 * 1024;
    const TICKS_PER_SEC: i64 = 10_000_000;
    // 100ns ticks from 1601-01-01 to 1970-01-01.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

    /// `GetDriveTypeW` results, from `WinBase.h`.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_RAMDISK: u32 = 6;

    /// By the drive letter's type. A `\\server\share` path is a network one.
    pub(super) fn on_local_volume(path: &Path) -> bool {
        use std::path::{Component, Prefix};
        let Ok(absolute) = std::path::absolute(path) else {
            return false;
        };
        let Some(Component::Prefix(prefix)) = absolute.components().next() else {
            return false;
        };
        let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
            return false;
        };
        let root: Vec<u16> = format!("{}:\\", char::from(letter))
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `root` is NUL-terminated.
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        matches!(kind, DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_RAMDISK)
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from `CreateFileW`, is owned here and closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// `path` as a NUL-terminated `\\?\` path, which has no length limit.
    /// std adds the same prefix for its own long paths.
    fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
        let absolute: Vec<u16> = std::path::absolute(path)?
            .as_os_str()
            .encode_wide()
            .collect();
        let wide = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let mut out = if absolute.starts_with(&wide(r"\\?\")) {
            absolute
        } else if let Some(share) = absolute.strip_prefix(wide(r"\\").as_slice()) {
            [wide(r"\\?\UNC\").as_slice(), share].concat()
        } else {
            [wide(r"\\?\").as_slice(), &absolute].concat()
        };
        out.push(0);
        Ok(out)
    }

    fn open(path: &Path, access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> io::Result<Handle> {
        let wide = verbatim(path)?;
        // SAFETY: `wide` is NUL-terminated; null security attributes and template are allowed.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                flags,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(handle))
        }
    }

    /// # Safety
    ///
    /// `class` must be the information class whose output struct is `T`.
    unsafe fn query<T: Default>(
        handle: &Handle,
        class: FILE_INFO_BY_HANDLE_CLASS,
    ) -> io::Result<T> {
        let mut info = T::default();
        let size = u32::try_from(size_of::<T>()).map_err(|_| invalid_data("struct too large"))?;
        // SAFETY: `info` is writable for `size` bytes and the caller ties `class` to `T`.
        let ok =
            unsafe { GetFileInformationByHandleEx(handle.0, class, (&raw mut info).cast(), size) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(info)
        }
    }

    fn filetime(ticks: i64) -> io::Result<SystemTime> {
        let rel = ticks
            .checked_sub(UNIX_EPOCH_TICKS)
            .ok_or_else(|| invalid_data("timestamp out of range"))?;
        epoch_time(
            rel.div_euclid(TICKS_PER_SEC),
            rel.rem_euclid(TICKS_PER_SEC) * 100,
        )
    }

    /// A link is a reparse point whose tag is a name surrogate, as std's
    /// `is_symlink` reads it. Other reparse points, such as cloud files, are
    /// ordinary files and folders.
    fn kind_of(attributes: u32, reparse_tag: u32) -> EntryKind {
        const NAME_SURROGATE: u32 = 0x2000_0000;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && reparse_tag & NAME_SURROGATE != 0 {
            EntryKind::Other
        } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            EntryKind::Dir
        } else {
            EntryKind::File
        }
    }

    struct Raw {
        attributes: u32,
        reparse_tag: u32,
        end_of_file: i64,
        last_write: i64,
        change: i64,
        /// Volume serial and file ID, where the file system has them.
        id: Option<(u64, [u8; 16])>,
    }

    fn make_stamp(raw: &Raw) -> io::Result<(EntryKind, FileStamp)> {
        let kind = kind_of(raw.attributes, raw.reparse_tag);
        let size = if kind == EntryKind::File {
            u64::try_from(raw.end_of_file).map_err(|_| invalid_data("negative file length"))?
        } else {
            0
        };
        // File systems without a change time (FAT, exFAT) report zero.
        let changed = if raw.change == 0 {
            None
        } else {
            Some(filetime(raw.change)?)
        };
        Ok((
            kind,
            FileStamp {
                size,
                modified: filetime(raw.last_write)?,
                changed,
                id: raw.id.map(|(volume, file_id)| FileId {
                    volume,
                    file: u128::from_le_bytes(file_id),
                }),
            },
        ))
    }

    pub(super) fn read_dir_stamps(dir: &Path) -> io::Result<Vec<(OsString, EntryKind, FileStamp)>> {
        let handle = open(
            dir,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_FLAG_BACKUP_SEMANTICS,
        )?;
        // SAFETY: `FileIdInfo` fills a `FILE_ID_INFO`.
        let dir_id: FILE_ID_INFO = unsafe { query(&handle, FileIdInfo)? };

        // `u64` elements keep the buffer 8-byte aligned, as the call requires.
        let mut buf = vec![0u64; BUF_LEN / size_of::<u64>()];
        let byte_len = u32::try_from(BUF_LEN).map_err(|_| invalid_data("buffer too large"))?;
        let mut class = FileIdExtdDirectoryRestartInfo;
        let mut out = Vec::new();
        loop {
            // SAFETY: `buf` is writable for `byte_len` bytes and both classes
            // fill a chain of `FILE_ID_EXTD_DIR_INFO`.
            let ok = unsafe {
                GetFileInformationByHandleEx(handle.0, class, buf.as_mut_ptr().cast(), byte_len)
            };
            if ok == 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error().and_then(|c| u32::try_from(c).ok())
                    == Some(ERROR_NO_MORE_FILES)
                {
                    return Ok(out);
                }
                return Err(err);
            }
            class = FileIdExtdDirectoryInfo;
            // SAFETY: a `u64` slice is valid as bytes; `bytes` borrows `buf` immutably.
            let bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), BUF_LEN) };
            parse_chain(bytes, dir_id.VolumeSerialNumber, &mut out)?;
        }
    }

    fn parse_chain(
        bytes: &[u8],
        volume: u64,
        out: &mut Vec<(OsString, EntryKind, FileStamp)>,
    ) -> io::Result<()> {
        type Info = FILE_ID_EXTD_DIR_INFO;
        let mut pos = 0usize;
        loop {
            let field = |offset: usize| pos.saturating_add(offset);
            let next: u32 = read_at(bytes, field(offset_of!(Info, NextEntryOffset)))?;
            let name_len: u32 = read_at(bytes, field(offset_of!(Info, FileNameLength)))?;
            let name_at = field(offset_of!(Info, FileName));
            let name_bytes = usize::try_from(name_len)
                .ok()
                .filter(|len| len % 2 == 0)
                .and_then(|len| bytes.get(name_at..name_at.checked_add(len)?))
                .ok_or_else(|| invalid_data("directory entry name out of bounds"))?;
            let wide: Vec<u16> = name_bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            let dot = u16::from(b'.');
            if wide != [dot] && wide != [dot, dot] {
                let raw = Raw {
                    attributes: read_at(bytes, field(offset_of!(Info, FileAttributes)))?,
                    reparse_tag: read_at(bytes, field(offset_of!(Info, ReparsePointTag)))?,
                    end_of_file: read_at(bytes, field(offset_of!(Info, EndOfFile)))?,
                    last_write: read_at(bytes, field(offset_of!(Info, LastWriteTime)))?,
                    change: read_at(bytes, field(offset_of!(Info, ChangeTime)))?,
                    id: Some((volume, read_at(bytes, field(offset_of!(Info, FileId)))?)),
                };
                let (kind, stamp) = make_stamp(&raw)?;
                out.push((OsString::from_wide(&wide), kind, stamp));
            }
            if next == 0 {
                return Ok(());
            }
            let next = usize::try_from(next).map_err(|_| invalid_data("bad entry offset"))?;
            pos = pos
                .checked_add(next)
                .filter(|p| *p < bytes.len())
                .ok_or_else(|| invalid_data("directory entry offset out of bounds"))?;
        }
    }

    pub(super) fn stamp(path: &Path) -> io::Result<FileStamp> {
        let handle = open(
            path,
            FILE_READ_ATTRIBUTES,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        // SAFETY: each class fills the struct it is paired with.
        let (basic, standard, tag, id) = unsafe {
            (
                query::<FILE_BASIC_INFO>(&handle, FileBasicInfo)?,
                query::<FILE_STANDARD_INFO>(&handle, FileStandardInfo)?,
                query::<FILE_ATTRIBUTE_TAG_INFO>(&handle, FileAttributeTagInfo).ok(),
                // FAT has no file IDs: such a stamp is never trusted.
                query::<FILE_ID_INFO>(&handle, FileIdInfo).ok(),
            )
        };
        let raw = Raw {
            attributes: basic.FileAttributes,
            reparse_tag: tag.map_or(0, |tag| tag.ReparseTag),
            end_of_file: standard.EndOfFile,
            last_write: basic.LastWriteTime,
            change: basic.ChangeTime,
            id: id.map(|id| (id.VolumeSerialNumber, id.FileId.Identifier)),
        };
        Ok(make_stamp(&raw)?.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    type Listing = HashMap<OsString, (EntryKind, FileStamp)>;

    fn listing(dir: &Path) -> Listing {
        read_dir_stamps(dir)
            .unwrap()
            .into_iter()
            .map(|(name, kind, stamp)| (name, (kind, stamp)))
            .collect()
    }

    #[test]
    fn a_temp_folder_is_on_a_local_volume() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(on_local_volume(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_folder_is_not_known_to_be_local() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!on_local_volume(&tmp.path().join("missing")));
    }

    #[test]
    fn lists_kinds_sizes_and_names() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", tmp.path().join("link")).unwrap();

        let entries = listing(tmp.path());
        let expected = if cfg!(unix) { 3 } else { 2 };
        assert_eq!(entries.len(), expected);

        let (kind, file) = entries[&OsString::from("a.txt")];
        assert_eq!(kind, EntryKind::File);
        assert_eq!(file.size, 5);
        let (kind, dir) = entries[&OsString::from("sub")];
        assert_eq!(kind, EntryKind::Dir);
        assert_eq!(dir.size, 0);

        let from_stamp = stamp(&tmp.path().join("a.txt")).unwrap();
        assert_eq!(from_stamp.size, file.size);
        assert_eq!(from_stamp.modified, file.modified);
        if cfg!(any(unix, windows)) {
            assert_eq!(from_stamp, file);
            assert!(file.changed.is_some());
            assert!(file.id.is_some());
            assert_eq!(stamp(&tmp.path().join("sub")).unwrap(), dir);
        }

        #[cfg(unix)]
        {
            let (kind, link) = entries[&OsString::from("link")];
            assert_eq!(kind, EntryKind::Other);
            assert_eq!(link.size, 0);
            assert_eq!(stamp(&tmp.path().join("link")).unwrap(), link);
            assert_ne!(link.id, file.id);
        }
    }

    #[test]
    fn empty_folder_has_no_entries() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_dir_stamps(tmp.path()).unwrap().len(), 0);
    }

    #[test]
    fn missing_paths_are_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing");
        assert!(read_dir_stamps(&missing).is_err());
        assert!(stamp(&missing).is_err());
    }

    #[test]
    fn listing_a_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        fs::write(&file, b"x").unwrap();
        assert!(read_dir_stamps(&file).is_err());
    }

    /// Many entries with names of varied byte lengths, so the folder takes
    /// several native calls and records land at odd offsets.
    #[cfg(unix)]
    #[test]
    fn many_entries_match_lstat() {
        use std::os::unix::fs::MetadataExt;

        let tmp = tempfile::tempdir().unwrap();
        let count = 600;
        for i in 0..count {
            let name = format!("{i}-{}{}", "é".repeat(i % 61), "x".repeat(i % 7));
            fs::write(tmp.path().join(name), vec![b'a'; i]).unwrap();
        }
        for i in 0..5 {
            fs::create_dir(tmp.path().join(format!("dir-{i}"))).unwrap();
        }

        let entries = read_dir_stamps(tmp.path()).unwrap();
        assert_eq!(entries.len(), count + 5);
        for (name, kind, entry) in entries {
            let meta = fs::symlink_metadata(tmp.path().join(&name)).unwrap();
            let expected_kind = if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            assert_eq!(kind, expected_kind, "{name:?}");
            let expected_size = if meta.is_file() { meta.len() } else { 0 };
            assert_eq!(entry.size, expected_size, "{name:?}");
            assert_eq!(entry.modified, meta.modified().unwrap(), "{name:?}");
            let changed = entry.changed.unwrap().duration_since(UNIX_EPOCH).unwrap();
            assert_eq!(
                (changed.as_secs(), changed.subsec_nanos()),
                (
                    u64::try_from(meta.ctime()).unwrap(),
                    u32::try_from(meta.ctime_nsec()).unwrap()
                ),
                "{name:?}"
            );
            assert_eq!(
                entry.id,
                Some(FileId {
                    volume: meta.dev(),
                    file: u128::from(meta.ino()),
                }),
                "{name:?}"
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn rewrite_and_restored_mtime_move_changed() {
        use std::thread::sleep;
        // Some file systems keep coarse change times; wait well past a tick.
        let tick = Duration::from_millis(50);

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        fs::write(&path, b"one").unwrap();
        let first = stamp(&path).unwrap();

        sleep(tick);
        fs::write(&path, b"two").unwrap();
        let rewritten = stamp(&path).unwrap();
        assert_eq!(rewritten.size, first.size);
        assert_ne!(rewritten.changed, first.changed);
        assert_eq!(rewritten.id, first.id);

        sleep(tick);
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(first.modified).unwrap();
        drop(file);
        let restored = stamp(&path).unwrap();
        assert_eq!(restored.modified, first.modified);
        assert_ne!(restored.changed, rewritten.changed);

        let listed = listing(tmp.path())[&OsString::from("f")].1;
        assert_eq!(listed, restored);
    }
}
