use std::ffi::OsString;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::WallpaperConfig;
use crate::model::DeliveryRecord;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    NotConfigured,
    Applied,
}

#[derive(Debug)]
pub struct ApplyError {
    pub message: String,
    pub uncertain: bool,
}

pub fn apply(
    config: &WallpaperConfig,
    record: &DeliveryRecord,
) -> Result<ApplyOutcome, ApplyError> {
    if config.command.is_empty() {
        return Ok(ApplyOutcome::NotConfigured);
    }

    let program = &config.command[0];
    let mut arguments = Vec::<OsString>::new();
    for argument in config.command.iter().skip(1) {
        if argument == "{path}" {
            arguments.push(record.stored_path.as_os_str().to_owned());
        } else {
            arguments.push(
                argument
                    .replace("{id}", &record.id)
                    .replace("{sha256}", &record.sha256)
                    .into(),
            );
        }
    }

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(config.timeout_seconds))
        .ok_or_else(|| ApplyError {
            message: "wallpaper timeout is too large for the system clock".into(),
            uncertain: false,
        })?;

    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn().map_err(|error| ApplyError {
        message: format!("failed to start wallpaper command {program:?}: {error}"),
        uncertain: false,
    })?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(ApplyOutcome::Applied),
            Ok(Some(status)) => {
                return Err(ApplyError {
                    message: format!("wallpaper command exited with status {status}"),
                    uncertain: false,
                });
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let kill_result = kill_command(&mut child);
                let _ = child.wait();
                return Err(ApplyError {
                    message: match kill_result {
                        Ok(()) => format!(
                            "wallpaper command exceeded the {} second timeout and was killed",
                            config.timeout_seconds
                        ),
                        Err(error) => format!(
                            "wallpaper command exceeded the {} second timeout and could not be killed: {}",
                            config.timeout_seconds, error
                        ),
                    },
                    uncertain: true,
                });
            }
            Err(error) => {
                return Err(ApplyError {
                    message: format!("failed to wait for wallpaper command: {error}"),
                    uncertain: true,
                });
            }
        }
    }
}

fn kill_command(child: &mut Child) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let pid = i32::try_from(child.id())
            .map_err(|_| std::io::Error::other("child process id does not fit in i32"))?;
        match killpg(Pid::from_raw(pid), Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(std::io::Error::other(error)),
        }
    }
    #[cfg(not(unix))]
    {
        child.kill()
    }
}
