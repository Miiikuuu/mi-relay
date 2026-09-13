use std::{fs::File, io};

/// Nonblocking directory ownership, released explicitly even while an unrelated
/// forked child still holds a duplicate open file description before exec.
/// Keep this field BEFORE a state lock so state waiters wake after root release.
pub(super) struct DirectoryLock(File);

impl DirectoryLock {
    pub(super) fn acquire(file: File) -> io::Result<Self> {
        fs2::FileExt::try_lock_exclusive(&file)?;
        Ok(Self(file))
    }
}

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        // Closing this fd alone is not sufficient: flock ownership survives
        // until every dup/fork copy closes, even when all fds are O_CLOEXEC.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_release_does_not_depend_on_duplicate_fd_lifetime() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lock");
        let file = File::create(&path).unwrap();
        let duplicate = file.try_clone().unwrap();
        let owner = DirectoryLock::acquire(file).unwrap();
        assert!(DirectoryLock::acquire(File::open(&path).unwrap()).is_err());
        drop(owner);
        let successor = DirectoryLock::acquire(File::open(&path).unwrap()).unwrap();
        drop(duplicate);
        // Closing the stale duplicate must not unlock the new owner's fd.
        assert!(DirectoryLock::acquire(File::open(&path).unwrap()).is_err());
        drop(successor);
        assert!(DirectoryLock::acquire(File::open(&path).unwrap()).is_ok());
    }
}
