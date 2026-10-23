// Process helpers shared by every engine call.
//
// The Windows release build is a `windows_subsystem` GUI process with no console of
// its own, so every spawned console tool (ffmpeg, ffprobe, ...) would open a visible
// console window — CREATE_NO_WINDOW keeps helper runs invisible.
use std::process::Command;

pub fn tool(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))] // mutation happens in the Windows block only
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}
