//! "Run `just rebuild`": a user action, never the agent's. The command is
//! fixed here and takes no input, it runs only after a click and a
//! confirmation, and a conversation holds no reference to it.

/// The one command this can run, and where. Spawned directly, without a
/// shell, so nothing is left to type into once it exits.
const ARGV: &[&str] = &["just", "rebuild"];
const DIR: &str = "/etc/nixos";

/// Permission to run the fixed command once. Only [`Rebuild::confirm`] makes
/// one.
#[derive(Debug, PartialEq, Eq)]
pub struct RebuildCommand(());

impl RebuildCommand {
    pub fn argv(&self) -> &'static [&'static str] {
        ARGV
    }

    pub fn dir(&self) -> &'static str {
        DIR
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Rebuild {
    #[default]
    Idle,
    /// The confirmation dialog is open.
    Confirming,
    Running,
    /// How the last run ended.
    Finished(String),
}

impl Rebuild {
    /// The button was clicked: ask for confirmation, unless a run is going on.
    pub fn request(&mut self) -> bool {
        match self {
            Rebuild::Idle | Rebuild::Finished(_) => {
                *self = Rebuild::Confirming;
                true
            }
            Rebuild::Confirming | Rebuild::Running => false,
        }
    }

    /// The confirmation was declined or dismissed.
    pub fn dismiss(&mut self) {
        if *self == Rebuild::Confirming {
            *self = Rebuild::Idle;
        }
    }

    /// The user confirmed: the command to run, once.
    pub fn confirm(&mut self) -> Option<RebuildCommand> {
        if *self != Rebuild::Confirming {
            return None;
        }
        *self = Rebuild::Running;
        Some(RebuildCommand(()))
    }

    pub fn failed_to_start(&mut self, why: &str) {
        if *self == Rebuild::Running {
            *self = Rebuild::Finished(format!("could not start: {why}"));
        }
    }

    /// The child exited with this wait status.
    pub fn exited(&mut self, status: i32) {
        if *self == Rebuild::Running {
            *self = Rebuild::Finished(describe_wait_status(status));
        }
    }

    /// The terminal may close only when nothing runs in it.
    pub fn can_close(&self) -> bool {
        *self != Rebuild::Running
    }
}

/// A `waitpid` status in words.
pub fn describe_wait_status(status: i32) -> String {
    let signal = status & 0x7f;
    if signal == 0 {
        match (status >> 8) & 0xff {
            0 => "finished".to_owned(),
            code => format!("failed with exit status {code}"),
        }
    } else {
        format!("stopped by signal {signal}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_runs_only_after_a_click_and_a_confirmation() {
        let mut r = Rebuild::default();
        assert_eq!(r.confirm(), None);
        assert!(r.request());
        r.dismiss();
        assert_eq!(r.confirm(), None);
        assert!(r.request());
        let command = r.confirm().expect("confirmed");
        assert_eq!(r, Rebuild::Running);
        assert_eq!(r.confirm(), None);
        assert_eq!(command.argv(), ["just", "rebuild"]);
        assert_eq!(command.dir(), "/etc/nixos");
    }

    #[test]
    fn one_run_at_a_time_and_no_closing_while_it_runs() {
        let mut r = Rebuild::default();
        r.request();
        r.confirm().unwrap();
        assert!(!r.request());
        assert!(!r.can_close());
        r.exited(1 << 8);
        assert_eq!(r, Rebuild::Finished("failed with exit status 1".into()));
        assert!(r.can_close());
        assert!(r.request());
    }

    #[test]
    fn a_failed_start_ends_the_run() {
        let mut r = Rebuild::default();
        r.request();
        r.confirm().unwrap();
        r.failed_to_start("just: not found");
        assert_eq!(
            r,
            Rebuild::Finished("could not start: just: not found".into())
        );
    }

    #[test]
    fn wait_statuses_in_words() {
        assert_eq!(describe_wait_status(0), "finished");
        assert_eq!(describe_wait_status(2 << 8), "failed with exit status 2");
        assert_eq!(describe_wait_status(9), "stopped by signal 9");
    }
}
