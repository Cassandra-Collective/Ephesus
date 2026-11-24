//! Declared commands and matchers backed by the recorded probe fixtures.

use eph_schema::registry::EngineId;

/// An executable name and its literal arguments (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// The executable name resolved from the configured PATH.
    pub program: &'static str,
    /// Arguments after the executable.
    pub args: &'static [&'static str],
}

/// The result a completed auth probe establishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authentication {
    /// The probe confirmed that the CLI can use a session.
    Authenticated,
    /// The probe expressly reports that the CLI needs a login.
    NeedsLogin,
    /// The output did not establish whether the CLI has a session.
    Inconclusive,
}

/// The reference adapter's authentication command and its remedy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticationSpec {
    pub(crate) probe: CommandSpec,
    login: CommandSpec,
}

impl AuthenticationSpec {
    /// Noninteractive command that tests the agent's authentication.
    pub fn probe(self) -> CommandSpec {
        self.probe
    }

    /// Interactive command the Architect can run to repair missing authentication.
    pub fn login(self) -> CommandSpec {
        self.login
    }

    /// Classify a completed probe's stdout. An exit code on its own
    /// establishes no authentication state.
    pub fn classify(self, stdout: &[u8]) -> Authentication {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout)
            && let Some(answer) = value.get("loggedIn").and_then(serde_json::Value::as_bool)
        {
            return if answer {
                Authentication::Authenticated
            } else {
                Authentication::NeedsLogin
            };
        }
        let Ok(text) = std::str::from_utf8(stdout) else {
            return Authentication::Inconclusive;
        };
        if text
            .lines()
            .any(|line| line == "Not logged in. Run claude auth login to authenticate.")
        {
            return Authentication::NeedsLogin;
        }
        if text
            .lines()
            .any(|line| line == "Login method: Claude Max account")
        {
            return Authentication::Authenticated;
        }
        Authentication::Inconclusive
    }
}

/// The binary, install offer and noninteractive probes an adapter declares.
#[derive(Debug, Clone, Copy)]
pub struct BinarySpec {
    pub(crate) engine: EngineId,
    install: CommandSpec,
    pub(crate) version: CommandSpec,
    pub(crate) authentication: Option<AuthenticationSpec>,
    parser: for<'a> fn(&'a str) -> Option<&'a str>,
}

impl BinarySpec {
    /// The engine that declared this binary.
    pub fn engine(self) -> EngineId {
        self.engine
    }

    /// The installation command to offer the Architect (FR-1.6).
    pub fn install(self) -> CommandSpec {
        self.install
    }

    /// The engine's noninteractive version command.
    pub fn version(self) -> CommandSpec {
        self.version
    }

    /// Authentication command and remedy, where declared.
    pub fn authentication(self) -> Option<AuthenticationSpec> {
        self.authentication
    }

    /// Read the version from the engine's recorded banner form.
    pub fn parse_version(self, stdout: &[u8]) -> Option<&str> {
        let text = std::str::from_utf8(stdout).ok()?.trim();
        (self.parser)(text)
    }
}

fn release(version: &str) -> Option<&str> {
    let (numbers, suffix) = version
        .find(['-', '+'])
        .map_or((version, ""), |index| version.split_at(index));
    let mut parts = numbers.split('.');
    let triplet = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && parts.next().is_none();
    let suffix_is_valid = suffix.is_empty()
        || (suffix.len() > 1
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte)));
    (triplet && suffix_is_valid).then_some(version)
}

fn claude_release(text: &str) -> Option<&str> {
    release(text.strip_suffix(" (Claude Code)")?)
}

fn codex_release(text: &str) -> Option<&str> {
    release(text.strip_prefix("codex-cli ")?)
}

pub(crate) const CLAUDE: BinarySpec = BinarySpec {
    engine: EngineId::Claude,
    install: CommandSpec {
        program: "npm",
        args: &["install", "-g", "@anthropic-ai/claude-code"],
    },
    version: CommandSpec {
        program: "claude",
        args: &["--version"],
    },
    authentication: Some(AuthenticationSpec {
        probe: CommandSpec {
            program: "claude",
            args: &["auth", "status"],
        },
        login: CommandSpec {
            program: "claude",
            args: &["auth", "login"],
        },
    }),
    parser: claude_release,
};

pub(crate) const CODEX: BinarySpec = BinarySpec {
    engine: EngineId::Codex,
    install: CommandSpec {
        program: "npm",
        args: &["install", "-g", "@openai/codex"],
    },
    version: CommandSpec {
        program: "codex",
        args: &["--version"],
    },
    authentication: None,
    parser: codex_release,
};

pub(crate) const GEMINI: BinarySpec = BinarySpec {
    engine: EngineId::Gemini,
    install: CommandSpec {
        program: "npm",
        args: &["install", "-g", "@google/gemini-cli"],
    },
    version: CommandSpec {
        program: "gemini",
        args: &["--version"],
    },
    authentication: None,
    parser: release,
};

#[cfg(test)]
pub(crate) const FAKE: BinarySpec = BinarySpec {
    engine: EngineId::Custom,
    install: CommandSpec {
        program: "eph-fake-engine",
        args: &[],
    },
    version: CommandSpec {
        program: "eph-fake-engine",
        args: &[],
    },
    authentication: None,
    parser: release,
};
