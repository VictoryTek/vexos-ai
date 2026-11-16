/// The agents VexOS supports. Each runs as an ACP agent on stdio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
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

/// The ACP config option OpenCode reports its mode (its agent choice) through,
/// and the others can too.
pub(crate) const MODE_OPTION: &str = "mode";

impl Agent {
    /// The mode every session starts in, and is reset to.
    pub(crate) fn start_mode(self) -> &'static str {
        self.allowed_modes()[0]
    }

    /// The modes a session may be in. Claude: `default` asks for every action
    /// that needs approval; `plan` cannot edit at all. Codex: only
    /// `read-only`, the one mode id every codex-acp release has; the others
    /// (`agent`, `auto`, `full-access`) write inside a sandbox or run with no
    /// prompt, which the managed policy also refuses. OpenCode: only `build`,
    /// the built-in agent the managed policy covers; custom agents run their own
    /// rules and get around the managed denies.
    pub(crate) fn allowed_modes(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &["default", "plan"],
            Agent::Codex => &["read-only"],
            Agent::OpenCode => &["build"],
        }
    }

    /// Whether the agent takes its mode through `session/set_mode` (Claude and
    /// Codex) or as the `mode` config option (OpenCode, whose modes are its agents).
    pub(crate) fn mode_is_config_option(self) -> bool {
        matches!(self, Agent::OpenCode)
    }

    pub fn command(self) -> AgentCommand {
        match self {
            // The adapter's draft v2 protocol may change; v1 is stable.
            Agent::Claude => AgentCommand {
                program: "claude-agent-acp",
                args: &[],
                env: &[("CLAUDE_AGENT_ACP_PROTOCOL", "v1")],
            },
            Agent::Codex => AgentCommand {
                program: "codex-acp",
                args: &[],
                env: &[],
            },
            Agent::OpenCode => AgentCommand {
                program: "opencode",
                args: &["acp"],
                env: &[],
            },
        }
    }
}
