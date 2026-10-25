/// The two agents VexOS supports. Each runs as an ACP agent on stdio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    OpenCode,
}

/// How an agent is launched. Public so callers and tests can see exactly what
/// runs; nothing here can make an agent approve on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub env: &'static [(&'static str, &'static str)],
}

/// The ACP config option both agents report their mode through (OpenCode: its
/// agent choice).
pub(crate) const MODE_OPTION: &str = "mode";

impl Agent {
    /// The mode every session starts in, and is reset to.
    pub(crate) fn start_mode(self) -> &'static str {
        self.allowed_modes()[0]
    }

    /// The modes a session may be in. Claude: `default` asks for every action
    /// that needs approval; `plan` cannot edit at all. OpenCode: only `build`,
    /// the built-in agent the managed policy covers; custom agents run their own
    /// rules and get around the managed denies.
    pub(crate) fn allowed_modes(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &["default", "plan"],
            Agent::OpenCode => &["build"],
        }
    }

    pub fn command(self) -> AgentCommand {
        match self {
            // The adapter's draft v2 protocol may change; v1 is stable.
            Agent::Claude => AgentCommand {
                program: "claude-agent-acp",
                args: &[],
                env: &[("CLAUDE_AGENT_ACP_PROTOCOL", "v1")],
            },
            Agent::OpenCode => AgentCommand {
                program: "opencode",
                args: &["acp"],
                env: &[],
            },
        }
    }
}
