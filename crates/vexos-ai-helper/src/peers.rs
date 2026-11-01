//! What the bus and `/proc` say about a caller's process, turned into the
//! `Identity` the gate checks. Everything here fails closed: a missing field, an
//! unreadable file, a dead process, a deleted or oddly named program is a
//! `LookupFailure`, and the gate refuses on every one.
//!
//! The bus (not the caller) fills in `GetConnectionCredentials`. It gives a
//! user id, a process id, and, from dbus 1.15 on, a process file descriptor
//! (`ProcessFD`, a pidfd). A process id can be reused by another process once
//! the first one exits; a descriptor cannot be mistaken for a different one.
//! So the descriptor decides which process this is: its `/proc/self/fdinfo`
//! entry names the process (`Pid:`, or `-1` once it has exited), the program is
//! read from `/proc/<that pid>/exe`, and the descriptor is read again afterwards
//! to show the process was alive, and the same, for the whole read.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

use crate::gate::{Identity, LookupFailure};

/// What `GetConnectionCredentials` returned, as far as the helper uses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Credentials {
    pub uid: Option<u32>,
    pub pid: Option<u32>,
    /// The process descriptor, as a descriptor number valid in this process
    /// for as long as the reply's descriptor list lives.
    pub pidfd: Option<RawFd>,
}

/// The user id on the `Uid:` line of `/proc/self/status` (the effective one),
/// or `None`. (`/proc/self` itself can be owned by another id inside a build
/// sandbox, so its owner is not used.)
pub fn own_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    let ids: Vec<u32> = line["Uid:".len()..]
        .split_whitespace()
        .map(|id| id.parse().ok())
        .collect::<Option<_>>()?;
    match ids.as_slice() {
        [_real, effective, _saved, _fs] => Some(*effective),
        _ => None,
    }
}

/// The process id on the `Pid:` line of a pidfd's `fdinfo`; `-1` after the
/// process exited.
pub fn fdinfo_pid(text: &str) -> Option<i64> {
    let line = text.lines().find(|l| l.starts_with("Pid:"))?;
    line["Pid:".len()..].trim().parse().ok()
}

/// The file name of the program a `/proc/<pid>/exe` link points at. Refused: a
/// link that is not an absolute path, one that ends in ` (deleted)` (the kernel
/// appends it when the file is gone), and one with no file name.
pub fn program_name(link: &Path) -> Result<String, LookupFailure> {
    let text = link.to_str().ok_or(LookupFailure::Program)?;
    if !link.is_absolute()
        || text.ends_with(" (deleted)")
        || text.ends_with('/')
        || text.ends_with("/.")
    {
        return Err(LookupFailure::Program);
    }
    link.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or(LookupFailure::Program)
}

/// The identity of the process behind `creds`.
///
/// `read_fdinfo` reads the process descriptor's `fdinfo` text; `read_exe`
/// reads the `/proc/<pid>/exe` link of a process id.
pub fn identify(
    creds: &Credentials,
    read_fdinfo: &mut dyn FnMut() -> Option<String>,
    read_exe: &dyn Fn(u32) -> Option<PathBuf>,
) -> Result<Identity, LookupFailure> {
    let uid = creds.uid.ok_or(LookupFailure::NoUserId)?;
    // The descriptor, not the process id, says which process this is.
    creds.pidfd.ok_or(LookupFailure::NoProcessFd)?;
    let alive = |text: Option<String>| {
        text.as_deref()
            .and_then(fdinfo_pid)
            .filter(|pid| *pid > 0)
            .ok_or(LookupFailure::ProcessGone)
    };
    let pid = alive(read_fdinfo())?;
    // The id the bus also gave must agree; a disagreement is not guessed at.
    if creds.pid.is_some_and(|given| i64::from(given) != pid) {
        return Err(LookupFailure::Inconsistent);
    }
    let link = u32::try_from(pid)
        .ok()
        .and_then(read_exe)
        .ok_or(LookupFailure::Program)?;
    let exe = program_name(&link)?;
    // Still the same live process: what was read above was this process's.
    if alive(read_fdinfo())? != pid {
        return Err(LookupFailure::ProcessGone);
    }
    Ok(Identity { uid, exe })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Own user id.

    #[test]
    fn the_effective_user_id_is_read_from_the_status_text() {
        let status =
            "Name:\tcat\nUmask:\t0022\nUid:\t1000\t1001\t1000\t1001\nGid:\t100\t100\t100\t100\n";
        assert_eq!(own_uid(status), Some(1001));
        assert_eq!(own_uid("Uid:\t0\t0\t0\t0\n"), Some(0));
        assert_eq!(own_uid("Uid:\t1000\t1000\t1000\t1000"), Some(1000));
    }

    #[test]
    fn a_status_text_without_a_usable_user_id_gives_none() {
        for status in [
            "",
            "Name:\tcat\n",
            "Uid:\n",
            "Uid:\t1000\n",
            "Uid:\tx\ty\tz\tw\n",
            "Uid:\t-1\t-1\t-1\t-1\n",
            "Uid:\t99999999999\t99999999999\t1\t1\n",
            "XUid:\t1\t1\t1\t1\n",
        ] {
            assert_eq!(own_uid(status), None, "{status:?}");
        }
    }

    // The pidfd's fdinfo.

    #[test]
    fn the_process_id_is_read_from_the_fdinfo_text() {
        assert_eq!(
            fdinfo_pid("pos:\t0\nflags:\t02000000\nPid:\t1307\nNSpid:\t1307\n"),
            Some(1307)
        );
        assert_eq!(fdinfo_pid("Pid:\t-1\nNSpid:\n"), Some(-1));
        assert_eq!(fdinfo_pid("Pid:\t8"), Some(8));
    }

    #[test]
    fn an_fdinfo_text_without_a_process_id_gives_none() {
        for text in [
            "",
            "pos:\t0\n",
            "Pid:\n",
            "Pid:\tabc\n",
            "NSpid:\t5\n",
            "XPid:\t5\n",
        ] {
            assert_eq!(fdinfo_pid(text), None, "{text:?}");
        }
    }

    // The program.

    #[test]
    fn the_program_is_the_file_name_of_the_link() {
        for (link, name) in [
            (
                "/nix/store/abc-gnome-shell-50.5/bin/.gnome-shell-wrapped",
                ".gnome-shell-wrapped",
            ),
            ("/usr/bin/gnome-shell", "gnome-shell"),
            ("/gnome-shell", "gnome-shell"),
        ] {
            assert_eq!(program_name(Path::new(link)), Ok(name.to_owned()), "{link}");
        }
    }

    #[test]
    fn an_odd_program_path_is_a_failed_lookup() {
        for link in [
            "/usr/bin/gnome-shell (deleted)",
            "/nix/store/abc/bin/.gnome-shell-wrapped (deleted)",
            "gnome-shell",
            "bin/gnome-shell",
            "./gnome-shell",
            "",
            "/",
            "/usr/bin/",
            "/proc/self/exe/..",
            "/usr/bin/.",
        ] {
            assert_eq!(
                program_name(Path::new(link)),
                Err(LookupFailure::Program),
                "{link:?}"
            );
        }
        // a path that is not valid UTF-8 is refused too
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let bytes = Path::new(OsStr::from_bytes(b"/usr/bin/gnome-shell\xff"));
        assert_eq!(program_name(bytes), Err(LookupFailure::Program));
        // an odd but readable name is returned as it is; the gate compares it
        assert_eq!(
            program_name(Path::new("/usr/bin/gnome-shell\n")),
            Ok("gnome-shell\n".to_owned())
        );
    }

    // Identifying a process.

    fn creds(uid: Option<u32>, pid: Option<u32>, pidfd: Option<RawFd>) -> Credentials {
        Credentials { uid, pid, pidfd }
    }

    fn fdinfo(pid: i64) -> String {
        format!("pos:\t0\nflags:\t02000000\nPid:\t{pid}\nNSpid:\t{pid}\n")
    }

    /// Runs `identify` with the fdinfo texts returned in order, and the program
    /// `exe` for process 1307 (none for any other).
    fn run(
        c: Credentials,
        texts: &[Option<String>],
        exe: Option<&str>,
    ) -> Result<Identity, LookupFailure> {
        let next = RefCell::new(texts.iter().cloned());
        let mut read = || next.borrow_mut().next().flatten();
        let exe = exe.map(PathBuf::from);
        identify(&c, &mut read, &|pid| {
            if pid == 1307 { exe.clone() } else { None }
        })
    }

    #[test]
    fn a_live_process_is_identified_by_its_descriptor() {
        let got = run(
            creds(Some(1000), Some(1307), Some(9)),
            &[Some(fdinfo(1307)), Some(fdinfo(1307))],
            Some("/nix/store/abc/bin/.gnome-shell-wrapped"),
        );
        assert_eq!(
            got,
            Ok(Identity {
                uid: 1000,
                exe: ".gnome-shell-wrapped".to_owned()
            })
        );
        // the process id from the bus is only a cross-check: absent is fine
        let got = run(
            creds(Some(1000), None, Some(9)),
            &[Some(fdinfo(1307)), Some(fdinfo(1307))],
            Some("/usr/bin/gnome-shell"),
        );
        assert_eq!(got.map(|i| i.exe), Ok("gnome-shell".to_owned()));
    }

    #[test]
    fn every_missing_piece_is_a_failed_lookup() {
        let ok = [Some(fdinfo(1307)), Some(fdinfo(1307))];
        let exe = Some("/usr/bin/gnome-shell");
        assert_eq!(
            run(creds(None, Some(1307), Some(9)), &ok, exe),
            Err(LookupFailure::NoUserId)
        );
        assert_eq!(
            run(creds(Some(1000), Some(1307), None), &ok, exe),
            Err(LookupFailure::NoProcessFd)
        );
        // no descriptor, even with a process id: the id alone is not trusted
        assert_eq!(
            run(creds(Some(1000), Some(1307), None), &ok, exe),
            Err(LookupFailure::NoProcessFd)
        );
        assert_eq!(
            run(creds(Some(1000), Some(1307), Some(9)), &[None], exe),
            Err(LookupFailure::ProcessGone)
        );
        assert_eq!(
            run(creds(Some(1000), Some(1307), Some(9)), &ok, None),
            Err(LookupFailure::Program)
        );
        for text in ["", "garbage", "Pid:\tabc\n", "Pid:\n"] {
            assert_eq!(
                run(
                    creds(Some(1000), Some(1307), Some(9)),
                    &[Some(text.to_owned()), Some(fdinfo(1307))],
                    exe
                ),
                Err(LookupFailure::ProcessGone),
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_exited_process_is_refused_before_and_after_the_read() {
        let exe = Some("/usr/bin/gnome-shell");
        // already gone at the first read (Pid: -1)
        assert_eq!(
            run(
                creds(Some(1000), Some(1307), Some(9)),
                &[Some(fdinfo(-1)), Some(fdinfo(-1))],
                exe
            ),
            Err(LookupFailure::ProcessGone)
        );
        // gone by the second read: the program read may have been another
        // process's, if the number was reused
        assert_eq!(
            run(
                creds(Some(1000), Some(1307), Some(9)),
                &[Some(fdinfo(1307)), Some(fdinfo(-1))],
                exe
            ),
            Err(LookupFailure::ProcessGone)
        );
        // the descriptor suddenly names another process
        assert_eq!(
            run(
                creds(Some(1000), Some(1307), Some(9)),
                &[Some(fdinfo(1307)), Some(fdinfo(1308))],
                exe
            ),
            Err(LookupFailure::ProcessGone)
        );
        // the second read failing is the same
        assert_eq!(
            run(
                creds(Some(1000), Some(1307), Some(9)),
                &[Some(fdinfo(1307)), None],
                exe
            ),
            Err(LookupFailure::ProcessGone)
        );
    }

    #[test]
    fn a_process_id_that_disagrees_with_the_descriptor_is_refused() {
        assert_eq!(
            run(
                creds(Some(1000), Some(4242), Some(9)),
                &[Some(fdinfo(1307)), Some(fdinfo(1307))],
                Some("/usr/bin/gnome-shell")
            ),
            Err(LookupFailure::Inconsistent)
        );
    }

    #[test]
    fn a_deleted_or_odd_program_path_is_refused() {
        for exe in ["/usr/bin/gnome-shell (deleted)", "gnome-shell", "/", ""] {
            assert_eq!(
                run(
                    creds(Some(1000), Some(1307), Some(9)),
                    &[Some(fdinfo(1307)), Some(fdinfo(1307))],
                    Some(exe)
                ),
                Err(LookupFailure::Program),
                "{exe:?}"
            );
        }
    }

    #[test]
    fn a_process_id_of_zero_or_below_is_not_a_process() {
        for pid in [0_i64, -1, -5] {
            assert_eq!(
                run(
                    creds(Some(1000), None, Some(9)),
                    &[Some(fdinfo(pid)), Some(fdinfo(pid))],
                    Some("/usr/bin/gnome-shell")
                ),
                Err(LookupFailure::ProcessGone),
                "{pid}"
            );
        }
    }
}
