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

/// Claude's only mode: every action that needs approval asks.
pub(crate) const CLAUDE_MODE: &str = "default";
/// OpenCode's only agent: the built-in one the managed policy covers. Custom
/// agents run their own rules and get around the managed denies.
pub(crate) const OPENCODE_MODE: &str = "build";
/// The ACP config option OpenCode exposes its agent choice through.
pub(crate) const OPENCODE_MODE_OPTION: &str = "mode";

impl Agent {
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
