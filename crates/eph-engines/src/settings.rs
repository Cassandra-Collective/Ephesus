//! Only the current private launch declarations enter the settings (ADR-0044).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::{
    AdapterError, ConfigDir, ConfigError, SpawnConfig,
    config::{CLAUDE_SETTINGS, project_key, read_object, refused, write_json},
};

const EVENTS: [(&str, &str); 9] = [
    ("SessionStart", "session-start"),
    ("UserPromptSubmit", "prompt-submitted"),
    ("PreToolUse", "pre-tool"),
    ("PostToolUse", "post-tool"),
    ("Notification", "notification"),
    ("Stop", "stop"),
    ("PreCompact", "compact-start"),
    ("PostCompact", "compact-end"),
    ("SessionEnd", "session-end"),
];

/// One regenerated private settings document.
pub struct SettingsPlan {
    config: ConfigDir,
    content: Value,
}

impl SettingsPlan {
    /// The settings the engine will read.
    pub fn path(&self) -> PathBuf {
        self.config.path().join(CLAUDE_SETTINGS)
    }

    /// Inspect the rendered settings without using the engine.
    pub fn content(&self) -> &Value {
        &self.content
    }

    /// Verify the current file at install time and replace it atomically.
    pub fn install(&self) -> Result<(), ConfigError> {
        let path = self.config.ensure()?.join(CLAUDE_SETTINGS);
        let _current = read_object(&path)?;
        write_json(&path, &self.content)
    }

    /// Finish the installation. The reference adapter retains its last private
    /// document for inspection and regenerates it at the next launch (ADR-0044).
    pub fn uninstall(&self) {}
}

pub(crate) fn render(
    config: &SpawnConfig<'_>,
    hook: &Path,
    usage: Option<&(PathBuf, PathBuf)>,
) -> Result<SettingsPlan, AdapterError> {
    let hook = shell_path(hook)?;
    let mut hooks = Map::new();
    for (name, event) in EVENTS {
        let fields = if matches!(event, "pre-tool" | "post-tool") {
            concat!(
                " --field tool=tool_name --classify tool",
                " --class \"file=Read,Write,Edit,MultiEdit,NotebookEdit,Glob,Grep\"",
                " --class \"shell=Bash,BashOutput,KillShell,KillBash\"",
                " --class \"web=WebFetch,WebSearch\" --class \"ledger=TodoWrite\"",
                " --class-prefix \"mcp=mcp__\""
            )
        } else {
            ""
        };
        let command = format!("{hook} --event {event}{fields} --session-field session_id");
        hooks.insert(
            name.into(),
            json!([{ "hooks": [{ "type": "command", "command": command }] }]),
        );
    }
    if config.unattended.len() > 12 {
        return Err(AdapterError::Unattended);
    }
    let agent_dir = config.identity_path.parent().ok_or_else(|| {
        refused(
            config.identity_path,
            "the agent's identity must have an owned directory",
        )
    })?;
    let agent_dir = project_key(agent_dir)?;
    let mut allow = vec![
        format!("Read({agent_dir}/**)"),
        format!("Edit({agent_dir}/**)"),
    ];
    let mut directories = vec![agent_dir];
    if let Some(playbooks) = config.playbooks_dir {
        let playbooks = project_key(playbooks)?;
        allow.push(format!("Read({playbooks}/**)"));
        directories.push(playbooks);
    }
    if let Some(refresh) = config.credential_refresh.filter(|cmd| !cmd.is_empty()) {
        allow.push("Bash($EPH_GH_TOKEN)".into());
        allow.push(format!(
            "Bash({})",
            refresh.replace(std::path::MAIN_SEPARATOR, "/")
        ));
    }
    allow.extend(config.unattended.iter().map(|grant| grant.bash_rule()));
    let mut content = json!({
        "attribution": { "commit": "", "pr": "", "sessionUrl": false },
        "includeCoAuthoredBy": false,
        "hooks": hooks,
        "permissions": { "allow": allow, "additionalDirectories": directories }
    });
    if let Some((program, reports)) = usage {
        let (program, reports) = (shell_path(program)?, shell_path(reports)?);
        content["statusLine"] =
            json!({ "type": "command", "command": format!("{program} --dir {reports}") });
    }
    Ok(SettingsPlan {
        config: config.engine_config.clone(),
        content,
    })
}

fn shell_path(path: &Path) -> Result<String, ConfigError> {
    if !path.is_absolute() {
        return Err(refused(
            path,
            "the hook or status line requires an absolute path",
        ));
    }
    let value = path
        .to_str()
        .ok_or_else(|| refused(path, "the hook or status line requires a Unicode path"))?;
    Ok(format!("\"{}\"", value.replace('"', "\\\"")))
}
