#![cfg(all(target_os = "linux", target_env = "gnu"))]

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::Command,
    sync::{Arc, Barrier},
    thread::{self, JoinHandle},
};

use mirelay::directory::{receiver::Receiver, sender::Sender};
use nix::{fcntl::OFlag, unistd::pipe2};

// Hold a real child between fork and exec. O_CLOEXEC has not taken effect yet,
// so it still owns copies of the parent's open file descriptions. No user
// application, relay or directories are touched. Only async-signal-safe syscalls
// run in pre_exec; both the parent and child have bounded readiness waits.
struct ForkWindow {
    release: File,
    child: Option<JoinHandle<io::Result<()>>>,
}

impl ForkWindow {
    fn open() -> Self {
        let (ready_read, ready_write) = pipe2(OFlag::O_CLOEXEC).unwrap();
        let (release_read, release_write) = pipe2(OFlag::O_CLOEXEC).unwrap();
        let mut ready_read = File::from(ready_read);
        let ready_write = File::from(ready_write);
        let release_read = File::from(release_read);
        let child = thread::spawn(move || {
            let mut command = Command::new("/bin/true");
            // SAFETY: this closure only uses write/poll/read and raw errno
            // construction, without allocating, taking Rust locks or unwinding.
            unsafe {
                command.pre_exec(move || {
                    let byte = [1u8];
                    if libc::write(ready_write.as_raw_fd(), byte.as_ptr().cast(), 1) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::EIO));
                    }
                    let mut poll = libc::pollfd {
                        fd: release_read.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    if libc::poll(&mut poll, 1, 10_000) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
                    }
                    let mut release = [0u8];
                    if libc::read(release_read.as_raw_fd(), release.as_mut_ptr().cast(), 1) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::EIO));
                    }
                    Ok(())
                });
            }
            let status = command.spawn()?.wait()?;
            if !status.success() {
                return Err(io::Error::other("fork-window child failed"));
            }
            Ok(())
        });
        let window = Self {
            release: release_write.into(),
            child: Some(child),
        };
        let mut poll = libc::pollfd {
            fd: ready_read.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll refers to one live descriptor and initialized pollfd.
        assert_eq!(unsafe { libc::poll(&mut poll, 1, 10_000) }, 1);
        let mut ready = [0u8];
        ready_read.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
        window
    }

    fn finish(mut self) {
        self.release.write_all(&[1]).unwrap();
        self.child.take().unwrap().join().unwrap().unwrap();
    }
}

impl Drop for ForkWindow {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = self.release.write_all(&[1]);
            let _ = child.join();
        }
    }
}

#[test]
fn receiver_reopens_while_unrelated_child_is_before_exec() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("files");
    let state = temp.path().join("state");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep.txt"), b"original").unwrap();
    let receiver = Receiver::open(&directory, &state, "scope").unwrap();
    let window = ForkWindow::open();
    // Live ownership must still reject a different ledger for the same root.
    assert!(Receiver::open(&directory, &temp.path().join("competitor"), "other").is_err());
    drop(receiver);
    let reopened = Receiver::open(&directory, &state, "scope");
    window.finish();
    let _reopened =
        reopened.expect("released receiver lock must not wait for unrelated child exec");
    assert_eq!(fs::read(directory.join("keep.txt")).unwrap(), b"original");
}

#[test]
fn sender_reopens_while_unrelated_child_is_before_exec() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("files");
    let state = temp.path().join("state");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep.txt"), b"original").unwrap();
    let sender = Sender::open(&directory, &state, "scope").unwrap();
    let window = ForkWindow::open();
    assert!(Sender::open(&directory, &temp.path().join("competitor"), "other").is_err());
    drop(sender);
    let reopened = Sender::open(&directory, &state, "scope");
    window.finish();
    let _reopened = reopened.expect("released sender lock must not wait for unrelated child exec");
    assert_eq!(fs::read(directory.join("keep.txt")).unwrap(), b"original");
}

fn same_state_handoff<T: Send>(open: fn(&Path, &Path, &str) -> anyhow::Result<T>) {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("files");
    let state = temp.path().join("state");
    fs::create_dir(&directory).unwrap();
    for _ in 0..64 {
        let owner = open(&directory, &state, "scope").unwrap();
        thread::scope(|scope| {
            let barrier = Arc::new(Barrier::new(2));
            let waiter_barrier = barrier.clone();
            let directory = &directory;
            let state = &state;
            let waiter = scope.spawn(move || {
                waiter_barrier.wait();
                open(directory, state, "scope")
            });
            barrier.wait();
            drop(owner);
            let _successor = waiter
                .join()
                .unwrap()
                .expect("state waiter must not overtake root unlock");
        });
    }
}

#[test]
fn receiver_same_state_waiter_does_not_observe_root_busy() {
    same_state_handoff(Receiver::open);
}

#[test]
fn sender_same_state_waiter_does_not_observe_root_busy() {
    same_state_handoff(Sender::open);
}
