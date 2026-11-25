use std::{
    ffi::{OsStr, OsString},
    fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use eph_agora::agora::Agora;
use eph_engines::{
    AdapterRegistry, Authentication, BinarySpec, CommandSpec, ConfigDir, EngineAdapter,
    Environment, ResolvedTools, SpawnConfig, SpawnPlan, ToolGrant, ToolRoot, ToolRoots,
    VerifiedExecutable, VersionCheck, base_environment, resolve_tool_grants,
};
use eph_fs::atomic::write_atomic;
use eph_git::worktrees::{WorktreeOutcome, WorktreePlan, WorktreeRemoval, Worktrees};
use eph_schema::{
    lifecycle::CheckoutResult,
    names::{AgentId, EnvName},
};

use crate::{Error, Hire, HookRegistration};

pub(crate) struct Runtime {
    pub(crate) home: PathBuf,
    pub(crate) agora: Arc<Agora>,
    pub(crate) agora_root: PathBuf,
    pub(crate) adapters: AdapterRegistry,
    pub(crate) worktrees: Worktrees,
    pub(crate) parent: Environment,
}

pub(crate) struct Workspace {
    pub(crate) cwd: PathBuf,
    identity: PathBuf,
    private: ConfigDir,
    playbooks: Option<PathBuf>,
    tools: ResolvedTools,
}

pub(crate) struct Checked {
    pub(crate) workspace: Workspace,
    pub(crate) executable: VerifiedExecutable,
    pub(crate) login: Option<CommandSpec>,
    pub(crate) auth_notice: Option<String>,
}

impl Checked {
    pub(crate) fn missing_tools(&self) -> &[PathBuf] {
        self.workspace.tools.missing()
    }
}

pub(crate) struct Setup {
    pub(crate) checkout: Option<PathBuf>,
    pub(crate) result: Result<Checked, Error>,
}

pub(crate) struct Prepared {
    pub(crate) plan: SpawnPlan,
    pub(crate) executable: VerifiedExecutable,
    pub(crate) resume_args: Vec<OsString>,
    pub(crate) secrets: Vec<String>,
    pub(crate) memory: bool,
}

pub(crate) struct LaunchInputs {
    pub(crate) registration: HookRegistration,
    pub(crate) grants: Vec<(EnvName, OsString)>,
    pub(crate) memory: String,
    pub(crate) resume: Option<String>,
}

impl Runtime {
    pub(crate) fn adapter(&self, hire: &Hire) -> Result<&dyn EngineAdapter, Error> {
        self.adapters.get(hire.roster.engine).ok_or_else(|| {
            Error::Refused(format!(
                "no production adapter is registered for {:?}",
                hire.roster.engine
            ))
        })
    }

    pub(crate) fn target(&self, hire: &Hire) -> Result<PathBuf, Error> {
        let target = fs::canonicalize(&hire.target)?;
        self.check_cwd(&hire.id, &target)?;
        Ok(target)
    }

    pub(crate) fn version(&self, binary: BinarySpec, cwd: &Path) -> Result<VersionCheck, Error> {
        let environment = base_environment(&self.parent);
        binary
            .check_version(cwd, &environment)
            .map_err(|error| Error::Refused(error.to_string()))
    }

    pub(crate) fn setup(
        &self,
        hire: &Hire,
        target: PathBuf,
        executable: VerifiedExecutable,
    ) -> Setup {
        let mut checkout = None;
        let result = self.check_workspace(hire, target, executable, &mut checkout);
        Setup { checkout, result }
    }

    fn check_workspace(
        &self,
        hire: &Hire,
        target: PathBuf,
        executable: VerifiedExecutable,
        checkout: &mut Option<PathBuf>,
    ) -> Result<Checked, Error> {
        let adapter = self.adapter(hire)?;
        let cwd = if hire.isolated {
            let parent = owned_dir(&self.home, Path::new("worktrees"), true)?;
            let path = parent.join(hire.id.as_str());
            check_leaf_dir(&path)?;
            let name = hire.id.as_str();
            let branch = format!("agent/{}", name.strip_prefix("agent.").unwrap_or(name));
            match self.worktrees.create(&WorktreePlan {
                repo: target.clone(),
                path,
                branch,
            }) {
                WorktreeOutcome::Ready { path, .. } => {
                    *checkout = Some(path.clone());
                    fs::canonicalize(path)?
                }
                WorktreeOutcome::Refused(reason) => return Err(Error::Refused(reason)),
            }
        } else {
            target.clone()
        };
        self.check_cwd(&hire.id, &cwd)?;
        let executable = if cwd == target {
            executable
        } else {
            verified(self.version(adapter.binary(), &cwd)?)?
        };
        let agora = &self.agora_root;
        let assigned = Path::new("agents").join(hire.id.as_str());
        let identity = owned_dir(agora, &assigned, true)?.join("identity.md");
        check_leaf_file(&identity)?;
        let private = ConfigDir::for_agent(&self.home, hire.roster.engine, &hire.id)
            .map_err(|error| Error::Refused(error.to_string()))?;
        let relative = private.path().strip_prefix(&self.home).map_err(|_| {
            Error::Refused("private configuration is outside the harness home".into())
        })?;
        owned_dir(&self.home, relative, true)?;
        let playbooks = hire.playbooks.as_ref().map(fs::canonicalize).transpose()?;
        if let Some(path) = &playbooks {
            self.check_grant(&hire.id, path)?;
        }
        let grants: Vec<ToolGrant<'_>> = hire
            .tools
            .iter()
            .map(|tool| ToolGrant {
                root: tool.root,
                path: &tool.path,
            })
            .collect();
        let tools_root = if grants.iter().any(|grant| grant.root == ToolRoot::Home) {
            owned_dir(&self.home, Path::new("tools"), false)?
        } else {
            self.home.join("tools")
        };
        let tools = resolve_tool_grants(
            &grants,
            ToolRoots {
                target: &cwd,
                home: &tools_root,
            },
        )
        .map_err(|error| Error::Refused(error.to_string()))?;
        for directory in tools.directories() {
            self.check_grant(&hire.id, directory)?;
        }
        let (login, auth_notice) =
            self.authenticate(adapter, &hire.id, &private, &cwd, &executable)?;
        Ok(Checked {
            workspace: Workspace {
                cwd,
                identity,
                private,
                playbooks,
                tools,
            },
            executable,
            login,
            auth_notice,
        })
    }

    fn authenticate(
        &self,
        adapter: &dyn EngineAdapter,
        agent: &AgentId,
        private: &ConfigDir,
        cwd: &Path,
        executable: &VerifiedExecutable,
    ) -> Result<(Option<CommandSpec>, Option<String>), Error> {
        let Some(spec) = adapter.binary().authentication() else {
            return Ok((None, None));
        };
        let environment = adapter
            .authentication_environment(agent, private, &self.parent)
            .map_err(|error| Error::Refused(error.to_string()))?
            .ok_or_else(|| {
                Error::Refused("the declared authentication probe has no environment".into())
            })?;
        match spec.run_at(executable, cwd, &environment) {
            Ok(result) => match result.answer {
                Authentication::Authenticated => Ok((None, None)),
                Authentication::NeedsLogin => Ok((Some(spec.login()), None)),
                Authentication::Inconclusive => Ok((
                    None,
                    Some("the authentication probe could not establish login".into()),
                )),
            },
            Err(error) => Ok((
                None,
                Some(format!(
                    "the authentication probe could not establish login: {error}"
                )),
            )),
        }
    }

    pub(crate) fn prepare(
        &self,
        hire: &Hire,
        checked: Checked,
        inputs: LaunchInputs,
    ) -> Result<Prepared, Error> {
        let adapter = self.adapter(hire)?;
        let Workspace {
            cwd,
            identity,
            private,
            playbooks,
            tools,
            ..
        } = checked.workspace;
        check_leaf_file(&identity)?;
        write_atomic(&identity, hire.identity.as_bytes())?;
        adapter
            .prepare(&private)
            .map_err(|error| Error::Refused(error.to_string()))?;
        let prompts = owned_dir(&self.home, Path::new("prompts"), false)?;
        let protocol = self.agora_root.join("PROTOCOL.md");
        check_leaf_file(&protocol)?;
        let plan = adapter
            .plan(&SpawnConfig {
                agent_id: &hire.id,
                hook_token: &inputs.registration.token,
                hook_endpoint: &inputs.registration.endpoint,
                cwd: &cwd,
                engine_config: &private,
                parent_environment: &self.parent,
                env_grants: &inputs.grants,
                commit_identity: hire.commit_identity.as_ref().map(|identity| {
                    eph_engines::CommitIdentity {
                        name: &identity.name,
                        email: &identity.email,
                    }
                }),
                prompts_dir: &prompts,
                identity_path: &identity,
                protocol_path: &protocol,
                memory: &inputs.memory,
                recall_command: hire.recall_command.as_deref(),
                credential_refresh: hire.credential_refresh.as_deref(),
                playbooks_dir: playbooks.as_deref(),
                tools: &tools,
                unattended: &hire.unattended,
                autonomy: hire.autonomy,
            })
            .map_err(|error| Error::Refused(error.to_string()))?;
        if let Some(settings) = plan.settings() {
            settings
                .install()
                .map_err(|error| Error::Refused(error.to_string()))?;
        }
        let resume_args = inputs
            .resume
            .as_deref()
            .and_then(|id| adapter.resume(id))
            .unwrap_or_default();
        let mut secrets = vec![inputs.registration.token];
        secrets.extend(
            inputs
                .grants
                .into_iter()
                .filter_map(|(_, value)| value.into_string().ok()),
        );
        Ok(Prepared {
            plan,
            executable: checked.executable,
            resume_args,
            secrets,
            memory: !inputs.memory.is_empty(),
        })
    }

    pub(crate) fn cleanup(&self, repo: Option<&Path>, checkout: Option<&Path>) -> CheckoutResult {
        let Some(checkout) = checkout else {
            return CheckoutResult::None;
        };
        let Some(repo) = repo else {
            return CheckoutResult::Unconfirmed {
                reason: "the checkout's verified repository is unavailable".into(),
            };
        };
        match self.worktrees.remove(repo, checkout) {
            WorktreeRemoval::Removed { residue } => CheckoutResult::Removed { residue },
            WorktreeRemoval::Kept { reason, changes } => {
                CheckoutResult::Retained { reason, changes }
            }
        }
    }

    fn check_cwd(&self, agent: &AgentId, path: &Path) -> Result<(), Error> {
        let agora = &self.agora_root;
        let own = agora.join("agents").join(agent.as_str());
        if !path.is_dir()
            || agora.starts_with(path)
            || (path.starts_with(agora) && !path.starts_with(own))
        {
            return Err(Error::Refused(format!(
                "workspace {} overlaps Agora storage outside the hire's grant",
                path.display()
            )));
        }
        self.check_grant(agent, path)
    }

    fn check_grant(&self, agent: &AgentId, path: &Path) -> Result<(), Error> {
        let metadata = self.agora_root.join(".git");
        let agents = self.agora_root.join("agents");
        let own = agents.join(agent.as_str());
        let other_hire = path.starts_with(&agents) && !path.starts_with(own);
        let worktrees = self.home.join("worktrees");
        let own_checkout = worktrees.join(agent.as_str());
        let other_checkout = path.starts_with(&worktrees) && !path.starts_with(own_checkout);
        if !path.is_dir()
            || overlaps(path, &metadata)
            || agents.starts_with(path)
            || other_hire
            || overlaps(path, &self.home.join("engines"))
            || worktrees.starts_with(path)
            || other_checkout
        {
            return Err(Error::Refused(format!(
                "grant {} would include Agora metadata or another hire",
                path.display()
            )));
        }
        Ok(())
    }
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

pub(crate) fn owned_dir(root: &Path, relative: &Path, create: bool) -> Result<PathBuf, Error> {
    let mut directory = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(Error::Refused(
                "an owned path must remain below its declared root".into(),
            ));
        };
        directory.push(name);
        if create && !check_leaf_dir(&directory)? {
            match fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        if !check_leaf_dir(&directory)? {
            return Err(Error::Refused(format!(
                "owned directory {} is missing",
                directory.display()
            )));
        }
    }
    Ok(directory)
}

fn check_leaf_dir(path: &Path) -> Result<bool, Error> {
    inspect_owned(path, fs::Metadata::is_dir)
}

fn check_leaf_file(path: &Path) -> Result<(), Error> {
    inspect_owned(path, fs::Metadata::is_file).map(|_| ())
}

fn inspect_owned(path: &Path, expected: impl FnOnce(&fs::Metadata) -> bool) -> Result<bool, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if redirected(&metadata) || !expected(&metadata) {
        return Err(Error::Refused(format!(
            "owned path {} is redirected or has the wrong type",
            path.display()
        )));
    }
    Ok(true)
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}

pub(crate) fn verified(check: VersionCheck) -> Result<VerifiedExecutable, Error> {
    match check {
        VersionCheck::Ready(executable) => Ok(executable),
        VersionCheck::Missing => Err(Error::Refused(
            "the engine has no executable on the configured PATH".into(),
        )),
        VersionCheck::Unrecognized(result) => Err(Error::Refused(format!(
            "the engine version probe did not report a recognized release ({result:?})"
        ))),
    }
}

pub(crate) fn installer_path(
    command: CommandSpec,
    cwd: &Path,
    environment: &Environment,
) -> Result<PathBuf, Error> {
    eph_pty::path::find_executable(OsStr::new(command.program), cwd, environment)?.ok_or_else(
        || {
            Error::Refused(format!(
                "the installer {} is missing from PATH",
                command.program
            ))
        },
    )
}

/// A plugin path declared relative to one approved root.
#[derive(Debug, Clone)]
pub struct DeclaredTool {
    /// Which of the adapter's roots supplies the plugin.
    pub root: ToolRoot,
    /// The plugin path relative to that root.
    pub path: PathBuf,
}
