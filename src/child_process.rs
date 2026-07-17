use command_group::{CommandGroup, GroupChild};
use std::io;
use std::process::{Command, ExitStatus};

#[cfg(windows)]
pub(crate) fn spawn_contained(command: &mut Command) -> io::Result<GroupChild> {
    let mut group = command.group();
    group.kill_on_drop(true).spawn()
}

#[cfg(unix)]
pub(crate) fn spawn_contained(command: &mut Command) -> io::Result<GroupChild> {
    command.group_spawn()
}

pub(crate) fn terminate_and_wait(child: &mut GroupChild) -> io::Result<ExitStatus> {
    allow_already_gone(child.kill())?;
    child.wait()
}

pub(crate) fn terminate_after_exit(
    child: &mut GroupChild,
    status: ExitStatus,
) -> io::Result<ExitStatus> {
    allow_already_gone(child.kill())?;
    let _ = child.wait()?;
    Ok(status)
}

fn allow_already_gone(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if (cfg!(unix) && error.raw_os_error() == Some(3))
                || matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
                ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}
