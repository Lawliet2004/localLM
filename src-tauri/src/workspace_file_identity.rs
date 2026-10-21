//! File identity from an already opened capability handle. Shared by native
//! transaction preflight and platform compile fixtures; never follows a path
//! to infer identity, and fails closed if identity cannot be established.
use cap_std::fs::File;

pub(crate) fn single_link_identity(file: &File) -> Result<(u64, u64), String> {
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot inspect transaction file: {e}"))?;
    if !metadata.is_file() {
        return Err("Transactions require regular files.".into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("Hard-linked files cannot participate in native transactions. Use an independent file copy.".into());
        }
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the borrowed capability File owns a live handle for this
        // call; information is a valid, initialized output structure. The API
        // neither takes ownership nor changes the file's contents.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
            .map_err(|e| format!("Cannot establish Windows transaction file identity: {e}"))?;
        if information.nNumberOfLinks != 1 {
            return Err("Hard-linked files cannot participate in native transactions. Use an independent file copy.".into());
        }
        Ok((
            information.dwVolumeSerialNumber as u64,
            ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    Err("Native transactions require a platform with supported file identity checks.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_open_files_have_distinct_stable_identities() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), "same bytes").unwrap();
        std::fs::write(root.path().join("b"), "same bytes").unwrap();
        let dir =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let a = dir.open("a").unwrap();
        let reopened = dir.open("a").unwrap();
        let b = dir.open("b").unwrap();
        assert_eq!(
            single_link_identity(&a).unwrap(),
            single_link_identity(&reopened).unwrap()
        );
        assert_ne!(
            single_link_identity(&a).unwrap(),
            single_link_identity(&b).unwrap()
        );
    }
    #[test]
    fn a_new_hard_link_is_detected_from_the_original_open_handle() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), "original").unwrap();
        let dir =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let a = dir.open("a").unwrap();
        assert!(single_link_identity(&a).is_ok());
        std::fs::hard_link(root.path().join("a"), root.path().join("alias")).unwrap();
        assert!(single_link_identity(&a)
            .unwrap_err()
            .contains("Hard-linked"));
        assert!(single_link_identity(&dir.open("alias").unwrap()).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a")).unwrap(),
            "original"
        );
    }
}
