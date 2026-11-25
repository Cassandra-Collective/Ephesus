//! Construct exact child environments from the base allowlist and named grants.

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use crate::{ConfigDir, SpawnConfig};

const BASE_KEYS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SHELL",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "TEMP",
    "TMP",
    "TMPDIR",
    "APPDATA",
    "LOCALAPPDATA",
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "USER",
    "USERNAME",
    "USERDOMAIN",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TZ",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "PROCESSOR_ARCHITECTURE",
];

/// Select the shared allowlist used before role grants and per-engine fields.
pub fn base_environment(parent: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    parent
        .iter()
        .filter(|(name, _)| {
            BASE_KEYS
                .iter()
                .any(|key| name.as_encoded_bytes().eq_ignore_ascii_case(key.as_bytes()))
        })
        .cloned()
        .collect()
}

fn same_name(left: &OsStr, right: &OsStr) -> bool {
    if cfg!(windows) {
        left.as_encoded_bytes()
            .eq_ignore_ascii_case(right.as_encoded_bytes())
    } else {
        left == right
    }
}

fn get<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    env.iter()
        .rev()
        .find(|(key, _)| same_name(key, OsStr::new(name)))
        .map(|(_, value)| value.as_os_str())
}

pub(crate) fn put(
    env: &mut Vec<(OsString, OsString)>,
    name: impl Into<OsString>,
    value: impl Into<OsString>,
) {
    let name = name.into();
    env.retain(|(key, _)| !same_name(key, &name));
    env.push((name, value.into()));
}

pub(crate) fn spawn(config: &SpawnConfig<'_>) -> Vec<(OsString, OsString)> {
    let mut environment = base_environment(config.parent_environment);
    for (name, value) in config.env_grants {
        put(&mut environment, name.as_str(), value);
    }
    if let Some(identity) = config.commit_identity {
        for (name, value) in [
            ("GIT_AUTHOR_NAME", identity.name),
            ("GIT_AUTHOR_EMAIL", identity.email),
            ("GIT_COMMITTER_NAME", identity.name),
            ("GIT_COMMITTER_EMAIL", identity.email),
        ] {
            put(&mut environment, name, value);
        }
        put(
            &mut environment,
            "EPH_COAUTHOR",
            format!("Co-authored-by: {} <{}>", config.agent_id, identity.email),
        );
    }
    for (name, value) in [
        ("EPH_AGENT_ID", config.agent_id.as_str()),
        ("EPH_HOOK_TOKEN", config.hook_token),
        ("EPH_HOOK_ENDPOINT", config.hook_endpoint),
    ] {
        put(&mut environment, name, value);
    }
    for (name, value) in [
        ("EPH_RECALL", config.recall_command),
        ("EPH_GH_TOKEN", config.credential_refresh),
    ] {
        environment.retain(|(key, _)| !same_name(key, OsStr::new(name)));
        if let Some(value) = value.filter(|text| !text.is_empty()) {
            environment.push((name.into(), value.into()));
        }
    }
    environment
}

pub(crate) fn claude(
    config: &ConfigDir,
    parent: &[(OsString, OsString)],
    environment: &mut Vec<(OsString, OsString)>,
) {
    let credentials = match get(parent, "CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        Some(directory) => directory.to_owned(),
        None => get(parent, "HOME")
            .or_else(|| get(parent, "USERPROFILE"))
            .filter(|dir| !dir.is_empty())
            .map(|directory| PathBuf::from(directory).join(".claude").into_os_string())
            .unwrap_or_default(),
    };
    put(environment, "CLAUDE_CONFIG_DIR", config.path());
    put(environment, "CLAUDE_SECURESTORAGE_CONFIG_DIR", credentials);
    put(environment, "DISABLE_AUTOUPDATER", "1");
}
