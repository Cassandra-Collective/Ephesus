//! Resolve a program using the agent's spawn environment (SDD §1.1).

use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// Resolve a bare command against the plan's PATH and, on Windows, PATHEXT.
///
/// Search directories in order, resolving relative entries against the
/// supplied working directory. A named path or a bare name with no match
/// is returned as supplied. On Windows a discovered npm batch shim can
/// resolve to its existing executable, bypassing the shell's argument limit.
pub fn resolve_executable(
    command: &OsStr,
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> PathBuf {
    if explicit(command) || command.is_empty() {
        return PathBuf::from(command);
    }

    #[cfg(windows)]
    let extensions: Vec<&str> = if Path::new(command).extension().is_none() {
        declared(environment, "PATHEXT")
            .and_then(OsStr::to_str)
            .unwrap_or(".COM;.EXE;.BAT;.CMD")
            .split(';')
            .map(str::trim)
            .filter(|extension| !extension.is_empty())
            .collect()
    } else {
        vec![""]
    };
    #[cfg(not(windows))]
    let extensions = [""];

    let path = declared(environment, "PATH").unwrap_or_default();
    for dir in env::split_paths(path) {
        let dir = dir
            .to_str()
            .map(|value| Path::new(value.trim().trim_matches('"')))
            .unwrap_or(&dir);
        if dir.as_os_str().is_empty() {
            continue;
        }
        for extension in &extensions {
            let mut name = command.to_os_string();
            name.push(extension);
            let candidate = cwd.join(dir).join(name);
            if candidate.is_file() {
                #[cfg(windows)]
                if let Some(target) = unwrap_windows_shim(&candidate) {
                    return target;
                }
                return candidate;
            }
        }
    }
    PathBuf::from(command)
}

pub(crate) fn for_spawn(
    command: &OsStr,
    absolute_cwd: &Path,
    environment: &[(OsString, OsString)],
) -> std::io::Result<PathBuf> {
    let resolved = resolve_executable(command, absolute_cwd, environment);
    if resolved.is_absolute() || explicit(command) {
        return Ok(absolute_cwd.join(resolved));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "PTY executable was not found on the spawn plan's PATH",
    ))
}

fn explicit(command: &OsStr) -> bool {
    command
        .as_encoded_bytes()
        .iter()
        .any(|&byte| matches!(byte, b'/' | b'\\'))
}

fn declared<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    environment
        .iter()
        .rev()
        .find(|(key, _)| {
            let key = key.as_encoded_bytes();
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name.as_bytes())
            } else {
                key == name.as_bytes()
            }
        })
        .map(|(_, value)| value.as_os_str())
}

#[cfg(any(windows, test))]
fn unwrap_windows_shim(shim: &Path) -> Option<PathBuf> {
    let extension = shim.extension()?.to_str()?;
    if !extension.eq_ignore_ascii_case("cmd") && !extension.eq_ignore_ascii_case("bat") {
        return None;
    }
    let script = std::fs::read_to_string(shim).ok()?;
    // npm cmd-shim's complete direct-binary form. Any other batch body may
    // do meaningful work before or after an invocation, so run it as a shim.
    let mut lines = script.lines();
    for expected in [
        "@ECHO off",
        "GOTO start",
        ":find_dp0",
        "SET dp0=%~dp0",
        "EXIT /b",
        ":start",
        "SETLOCAL",
        "CALL :find_dp0",
    ] {
        if !lines.next()?.trim().eq_ignore_ascii_case(expected) {
            return None;
        }
    }
    let (quoted, remainder) = lines.next()?.trim().strip_prefix('"')?.split_once('"')?;
    if remainder.trim() != "%*" || lines.next().is_some() {
        return None;
    }
    let (prefix, relative) = quoted.split_at_checked("%dp0%".len())?;
    if !prefix.eq_ignore_ascii_case("%dp0%") || !relative.starts_with('\\') {
        return None;
    }
    // Extra expansions or an absolute/drive-relative name are batch syntax,
    // outside the direct pathname form.
    let relative = relative.trim_start_matches('\\');
    if relative.contains(['%', '!', '^', ':', '/']) {
        return None;
    }
    let target = relative.replace('\\', std::path::MAIN_SEPARATOR_STR);
    let candidate = shim.parent()?.join(target);
    (candidate.extension()?.eq_ignore_ascii_case("exe") && candidate.is_file()).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eph_fs::atomic::write_atomic;

    const NPM_HEADER: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n";

    #[test]
    fn explicit_paths_and_unresolved_names_survive_verbatim() {
        let cwd = tempfile::tempdir().unwrap();
        for name in ["./local-tool", "no-such-tool", "C:\\path\\tool.exe", ""] {
            assert_eq!(
                resolve_executable(OsStr::new(name), cwd.path(), &[]),
                Path::new(name)
            );
        }
    }

    #[test]
    fn uses_the_plans_cwd_and_the_first_declared_path_match() {
        let cwd = tempfile::tempdir().unwrap();
        let first = cwd.path().join("first dir");
        let last = cwd.path().join("last");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&last).unwrap();
        write_atomic(&first.join("tool.exe"), b"").unwrap();
        write_atomic(&last.join("tool.exe"), b"").unwrap();
        let path = env::join_paths(["", "first dir", "last", ""]).unwrap();
        assert_eq!(
            resolve_executable(OsStr::new("tool.exe"), cwd.path(), &[("PATH".into(), path)]),
            first.join("tool.exe")
        );
    }

    #[test]
    fn a_bare_path_miss_cannot_run_a_file_from_the_working_directory() {
        let cwd = tempfile::tempdir().unwrap();
        write_atomic(&cwd.path().join("tool.exe"), b"").unwrap();
        let absent = for_spawn(OsStr::new("tool.exe"), cwd.path(), &[]).unwrap_err();
        assert_eq!(absent.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            for_spawn(OsStr::new("./tool.exe"), cwd.path(), &[]).unwrap(),
            cwd.path().join("./tool.exe")
        );
    }

    #[test]
    fn npm_shim_uses_its_own_directory_and_requires_an_existing_target() {
        let cwd = tempfile::tempdir().unwrap();
        let bin = cwd.path().join("node_modules");
        std::fs::create_dir(&bin).unwrap();
        let target = bin.join("tool.exe");
        let shim = cwd.path().join("tool.cmd");
        let body = format!("{NPM_HEADER}\"%dP0%\\node_modules\\tool.exe\"   %*\r\n");
        write_atomic(&shim, body.as_bytes()).unwrap();
        assert_eq!(unwrap_windows_shim(&shim), None);
        write_atomic(&target, b"").unwrap();
        assert_eq!(unwrap_windows_shim(&shim), Some(target));
    }

    #[test]
    fn executable_mentions_and_scripts_with_extra_commands_keep_the_shim() {
        let cwd = tempfile::tempdir().unwrap();
        write_atomic(&cwd.path().join("tool.exe"), b"").unwrap();
        let shim = cwd.path().join("tool.cmd");
        let decoy = r#""%dp0%\tool.exe""#;
        for body in [
            format!("@echo off\r\nREM {decoy}\r\necho {decoy}\r\n"),
            format!("{NPM_HEADER}echo {decoy} %*\r\n"),
            format!("{NPM_HEADER}{decoy} %*\r\necho trailing command\r\n"),
        ] {
            write_atomic(&shim, body.as_bytes()).unwrap();
            assert_eq!(unwrap_windows_shim(&shim), None, "{body}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn pathext_order_and_case_insensitive_env_names_match_windows() {
        let cwd = tempfile::tempdir().unwrap();
        write_atomic(&cwd.path().join("tool.cmd"), b"@ECHO off\r\n").unwrap();
        write_atomic(&cwd.path().join("tool.exe"), b"").unwrap();
        for (ext, expected) in [(".CMD;.EXE", "tool.CMD"), (".EXE;.CMD", "tool.EXE")] {
            let plan = [
                ("Path".into(), cwd.path().as_os_str().to_owned()),
                ("pathext".into(), ext.into()),
            ];
            assert_eq!(
                resolve_executable(OsStr::new("tool"), cwd.path(), &plan),
                cwd.path().join(expected)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_never_adds_extensions_or_accepts_windows_env_aliases() {
        let cwd = tempfile::tempdir().unwrap();
        write_atomic(&cwd.path().join("tool.cmd"), b"").unwrap();
        let path = cwd.path().as_os_str().to_owned();
        assert_eq!(
            resolve_executable(
                OsStr::new("tool"),
                cwd.path(),
                &[("PATH".into(), path.clone())]
            ),
            Path::new("tool")
        );
        assert_eq!(
            resolve_executable(OsStr::new("tool.cmd"), cwd.path(), &[("Path".into(), path)]),
            Path::new("tool.cmd")
        );
    }
}
