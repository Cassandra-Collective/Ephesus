//! `Worktrees`: an agent's own checkout of a target repository (FR-1.5; SRS
//! UC-01 alternate 2a).
//!
//! Contract. Two rules are enforced here rather than trusted to the caller:
//! - **Never the Agora.** A worktree of the company's own repository would put a
//!   second working copy behind the single committer's back (ADR-0004). A
//!   repository at or under the Agora's root is refused on every create.
//! - **Never destroy work** (FR-1.5). Nothing at a worktree path is deleted to
//!   make room, and `--force` is never passed. A worktree with uncommitted
//!   changes is not removed: it is reported, with the changes, so the Architect
//!   decides. The one removal is of a directory git left behind already empty.
//!
//! A respawn lands where its agent left off. A checkout that is a worktree of
//! the planned repository is the agent's own and is reused where it stands, on
//! whatever branch it is on (M8c.9: the repository link is the proof of
//! ownership, not the branch name). A path that holds nothing is used as it
//! stands, since `git worktree add` accepts an empty directory. Anything else
//! is refused, and the refusal names the way out.
//!
//! Paths are compared after resolving every part of them that exists
//! (`fs::canonicalize`), so an Agora reached through a link, or written in
//! another case on Windows, is still the Agora. Every method answers; none
//! panics. The runner is the caller's: [`Git::target`] in production, so the
//! user's configuration (`core.longpaths`) applies.

use std::fs;
use std::path::{self, Path, PathBuf};

use crate::runner::{Git, GitResult};

/// Where a spawn's own checkout goes, and on which branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreePlan {
    /// The target repository. Never the Agora.
    pub repo: PathBuf,
    /// The new working copy, outside the repository.
    pub path: PathBuf,
    /// The branch to check out there, made from `HEAD` when it does not exist.
    pub branch: String,
}

/// What [`Worktrees::create`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeOutcome {
    /// The checkout is there: made now, or the agent's own reused.
    Ready {
        /// Its absolute path.
        path: PathBuf,
        /// The branch it is on, or `detached at <sha>`.
        branch: String,
        /// Whether this call made the branch.
        branch_created: bool,
    },
    /// Refused or failed, saying why and, for a refusal, what to do.
    Refused(String),
}

/// What `git status` said about a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeState {
    /// Its porcelain lines verbatim, or one `status failed: …` line when it
    /// could not be read: the evidence behind a refusal.
    pub changes: Vec<String>,
}

impl WorktreeState {
    /// Whether it may be removed. A worktree that could not be read is not.
    pub fn is_clean(&self) -> bool {
        self.changes.is_empty()
    }
}

/// What [`Worktrees::remove`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRemoval {
    /// Unregistered and gone.
    Removed {
        /// A sentence for the Architect when git unregistered the worktree but
        /// its directory survived and could not be tidied.
        residue: Option<String>,
    },
    /// Left in place.
    Kept {
        /// Why.
        reason: String,
        /// The uncommitted changes that kept it, if that is why.
        changes: Vec<String>,
    },
}

/// Whether a path may take a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vacancy {
    /// An empty real directory: it holds nobody's work.
    Vacant,
    /// Anything else, with the clause that says so.
    Occupied(String),
}

/// The worktree operations, for one Agora.
#[derive(Debug, Clone)]
pub struct Worktrees {
    git: Git,
    forbidden_root: PathBuf,
}

impl Worktrees {
    /// Worktrees made with `git`, refusing any repository at or under
    /// `forbidden_root`, the Agora's root.
    pub fn new(git: Git, forbidden_root: &Path) -> Worktrees {
        Worktrees {
            git,
            forbidden_root: forbidden_root.to_path_buf(),
        }
    }

    /// Makes `plan.path` a worktree of `plan.repo`, reuses the agent's own, or
    /// says why not. See the module contract.
    pub fn create(&self, plan: &WorktreePlan) -> WorktreeOutcome {
        let repo = resolved(&plan.repo);
        if repo.starts_with(resolved(&self.forbidden_root)) {
            return WorktreeOutcome::Refused(format!(
                "worktree refused: \"{}\" is inside the Agora, which has exactly one working copy (ADR-0004)",
                plan.repo.display()
            ));
        }
        let path = path::absolute(&plan.path).unwrap_or_else(|_| plan.path.clone());
        if resolved(&path).starts_with(&repo) {
            return WorktreeOutcome::Refused(format!(
                "worktree refused: \"{}\" is inside the target repository, which would dirty it",
                plan.path.display()
            ));
        }
        if !self.git.run(&plan.repo, &["rev-parse", "--git-dir"]).ok() {
            return WorktreeOutcome::Refused(format!(
                "worktree refused: \"{}\" is not a git repository",
                plan.repo.display()
            ));
        }
        if fs::symlink_metadata(&path).is_ok() {
            let head = self.git.run(&path, &["rev-parse", "--abbrev-ref", "HEAD"]);
            let common = self.git.run(&path, &["rev-parse", "--git-common-dir"]);
            let linked =
                common.ok() && resolved(&path.join(common.stdout.trim())).starts_with(&repo);
            if head.ok() && linked {
                // The agent's own: reused where it stands, on whatever branch
                // holds its work.
                return WorktreeOutcome::Ready {
                    branch: self.head_of(&path, &head),
                    path,
                    branch_created: false,
                };
            }
            if let Vacancy::Occupied(because) = path_is_vacant(&path) {
                return WorktreeOutcome::Refused(format!(
                    "worktree refused: \"{}\" {because}{}",
                    plan.path.display(),
                    recovery_for(common.ok(), &plan.repo, &plan.path)
                ));
            }
            // Git may still hold an entry for the emptied path, and would then
            // refuse it as registered.
            let _pruned = self.git.run(&plan.repo, &["worktree", "prune"]);
        }
        let branch_ref = format!("refs/heads/{}", plan.branch);
        let branch_created = !self
            .git
            .run(
                &plan.repo,
                &["rev-parse", "--verify", "--quiet", &branch_ref],
            )
            .ok();
        if let Some(parent) = path.parent() {
            // A parent that cannot be made fails the add below, in git's words.
            let _made = fs::create_dir_all(parent);
        }
        let at = path.to_string_lossy();
        let args: Vec<&str> = if branch_created {
            vec!["worktree", "add", "-b", &plan.branch, &at]
        } else {
            vec!["worktree", "add", &at, &plan.branch]
        };
        let added = self.git.run(&plan.repo, &args);
        if !added.ok() {
            return WorktreeOutcome::Refused(format!("worktree add failed: {}", said(&added)));
        }
        WorktreeOutcome::Ready {
            path,
            branch: plan.branch.clone(),
            branch_created,
        }
    }

    /// Whether the worktree at `path` holds uncommitted work.
    pub fn state(&self, path: &Path) -> WorktreeState {
        let status = self.git.run(path, &["status", "--porcelain"]);
        if !status.ok() {
            return WorktreeState {
                changes: vec![format!("status failed: {}", said(&status))],
            };
        }
        WorktreeState {
            changes: status
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string)
                .collect(),
        }
    }

    /// Removes a clean worktree of `repo` and prunes git's entry for it, or
    /// keeps a dirty one and returns its changes. Never forced.
    pub fn remove(&self, repo: &Path, path: &Path) -> WorktreeRemoval {
        if fs::symlink_metadata(path).is_err() {
            // Already gone: prune the stale entry, so the next create at the
            // same path is not refused by git's own bookkeeping.
            let _pruned = self.git.run(repo, &["worktree", "prune"]);
            return WorktreeRemoval::Removed { residue: None };
        }
        let state = self.state(path);
        if !state.is_clean() {
            return WorktreeRemoval::Kept {
                reason: format!(
                    "worktree \"{}\" has {} uncommitted change(s); left in place",
                    path.display(),
                    state.changes.len()
                ),
                changes: state.changes,
            };
        }
        let removed = self
            .git
            .run(repo, &["worktree", "remove", &path.to_string_lossy()]);
        if !removed.ok() {
            return WorktreeRemoval::Kept {
                reason: format!("worktree remove failed: {}", said(&removed)),
                changes: Vec::new(),
            };
        }
        let _pruned = self.git.run(repo, &["worktree", "prune"]);
        WorktreeRemoval::Removed {
            residue: sweep_empty_residue(path),
        }
    }

    /// The branch `head` names, or, for a detached checkout, where it is:
    /// `rev-parse --abbrev-ref` answers the literal `HEAD` then, which is not a
    /// branch name to put on an agent's card.
    fn head_of(&self, path: &Path, head: &GitResult) -> String {
        let branch = head.stdout.trim();
        if branch != "HEAD" {
            return branch.to_string();
        }
        let at = self.git.run(path, &["rev-parse", "--short", "HEAD"]);
        if at.ok() {
            format!("detached at {}", at.stdout.trim())
        } else {
            "detached".to_string()
        }
    }
}

/// Whether `target` is an empty real directory. Reads only; changes nothing.
///
/// "Empty" carries the whole safety argument, so it is read literally: one
/// entry of any kind, a file, a link standing in for a directory (ADR-0021: the
/// checkout would land somewhere nobody approved), or a path that cannot be
/// read each keep the refusal.
pub fn path_is_vacant(target: &Path) -> Vacancy {
    let entries = fs::symlink_metadata(target).and_then(|meta| {
        if meta.is_dir() {
            fs::read_dir(target).map(|entries| Some(entries.count()))
        } else {
            Ok(None)
        }
    });
    match entries {
        Ok(Some(0)) => Vacancy::Vacant,
        Ok(_) => Vacancy::Occupied("already exists".to_string()),
        Err(error) => Vacancy::Occupied(format!("already exists and could not be read: {error}")),
    }
}

/// The sentence after a refusal, naming the way out. A checkout of another
/// repository is that repository's to unregister; a plain directory is the
/// Architect's to move.
fn recovery_for(is_checkout: bool, repo: &Path, path: &Path) -> String {
    if is_checkout {
        format!(
            " — it is a git checkout of a different repository. Unregister it from the repository that owns it (`git worktree remove --force \"{}\"` then `git worktree prune`, run inside that repository), or move the directory aside, and activate again.",
            path.display()
        )
    } else {
        format!(
            " — nothing here is a checkout of \"{}\". Move that directory aside (or delete it if you know what is in it) and activate again; the harness never removes it for you, because it cannot tell your work from residue.",
            repo.display()
        )
    }
}

/// Removes the directory git left behind, only when it is empty. `None` when
/// nothing was left or it was swept; a sentence when it could not be.
fn sweep_empty_residue(path: &Path) -> Option<String> {
    if fs::symlink_metadata(path).is_err() {
        return None;
    }
    fs::read_dir(path)
        .and_then(|entries| match entries.count() {
            0 => fs::remove_dir(path).map(|()| None),
            left => Ok(Some(format!(
                "worktree \"{}\" was removed but {left} file(s) remain in its directory; left in place",
                path.display()
            ))),
        })
        .unwrap_or_else(|error| {
            Some(format!(
                "worktree \"{}\" was removed but its directory could not be tidied: {error}",
                path.display()
            ))
        })
}

/// A failed git's words, or a placeholder when it said nothing.
fn said(result: &GitResult) -> String {
    let stderr = result.stderr.trim();
    if stderr.is_empty() {
        "git said nothing".to_string()
    } else {
        stderr.to_string()
    }
}

/// `path` made absolute, with its longest existing prefix resolved by the
/// filesystem (links followed, case as stored), so two spellings of one place
/// compare equal.
fn resolved(path: &Path) -> PathBuf {
    let absolute = path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(existing) {
            return rest.iter().rev().fold(real, |whole, part| whole.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return absolute.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::runner::tests::{git, link_dir, must, repo_with_commit, scratch, write};

    /// A scratch directory with an Agora root and a target repository.
    struct Rig {
        dir: PathBuf,
        agora: PathBuf,
        target: PathBuf,
        worktrees: Worktrees,
    }

    impl Rig {
        fn new(name: &str) -> Rig {
            let dir = scratch(&format!("worktrees-{name}"));
            let agora = dir.join("agora");
            fs::create_dir_all(&agora).unwrap();
            let target = dir.join("target");
            repo_with_commit(&target);
            Rig {
                worktrees: Worktrees::new(git(), &agora),
                dir,
                agora,
                target,
            }
        }

        fn plan(&self, agent: &str) -> WorktreePlan {
            WorktreePlan {
                repo: self.target.clone(),
                path: self.dir.join("worktrees").join(agent),
                branch: format!("agent/{agent}"),
            }
        }

        fn create(&self, plan: &WorktreePlan) -> (PathBuf, String, bool) {
            match self.worktrees.create(plan) {
                WorktreeOutcome::Ready {
                    path,
                    branch,
                    branch_created,
                } => (path, branch, branch_created),
                WorktreeOutcome::Refused(reason) => panic!("refused: {reason}"),
            }
        }

        fn refused(&self, plan: &WorktreePlan) -> String {
            match self.worktrees.create(plan) {
                WorktreeOutcome::Refused(reason) => reason,
                ready => panic!("not refused: {ready:?}"),
            }
        }

        fn registered(&self) -> String {
            must(&self.target, &["worktree", "list", "--porcelain"])
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _gone = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn an_agent_gets_its_own_checkout_on_its_own_branch() {
        let rig = Rig::new("own");
        let plan = rig.plan("mason");
        let (path, branch, created) = rig.create(&plan);
        assert_eq!(path, path::absolute(&plan.path).unwrap());
        assert_eq!(branch, "agent/mason");
        assert!(created);
        assert!(path.join("README.md").exists());
        assert_eq!(
            must(&rig.target, &["status", "--porcelain"]),
            "",
            "the target is left clean"
        );
        assert_eq!(
            must(&path, &["branch", "--show-current"]).trim(),
            "agent/mason"
        );
    }

    #[test]
    fn a_respawn_reuses_its_branch_after_a_remove() {
        let rig = Rig::new("respawn");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        assert_eq!(
            rig.worktrees.remove(&rig.target, &path),
            WorktreeRemoval::Removed { residue: None }
        );
        let (_, branch, created) = rig.create(&plan);
        assert_eq!(branch, "agent/mason");
        assert!(!created, "the branch outlived the process");
    }

    #[test]
    fn the_agora_and_anything_inside_it_are_refused_but_a_sibling_is_not() {
        let rig = Rig::new("agora");
        repo_with_commit(&rig.agora);
        let inside = rig.agora.join("agents").join("mason");
        repo_with_commit(&inside);
        for repo in [&rig.agora, &inside] {
            let plan = WorktreePlan {
                repo: repo.clone(),
                ..rig.plan("mason")
            };
            let reason = rig.refused(&plan);
            assert!(reason.contains("is inside the Agora"), "{reason}");
            assert!(!plan.path.exists(), "nothing was made");
        }
        let sibling = rig.dir.join("agora2");
        repo_with_commit(&sibling);
        let plan = WorktreePlan {
            repo: sibling,
            ..rig.plan("mason")
        };
        rig.create(&plan);
    }

    #[test]
    fn an_agora_reached_through_a_link_is_still_the_agora() {
        let rig = Rig::new("agora-link");
        repo_with_commit(&rig.agora);
        let link = rig.dir.join("via-link");
        link_dir(&rig.agora, &link);
        let plan = WorktreePlan {
            repo: link,
            ..rig.plan("mason")
        };
        assert!(rig.refused(&plan).contains("is inside the Agora"));
    }

    #[cfg(windows)]
    #[test]
    fn an_agora_written_in_another_case_is_still_the_agora_on_windows() {
        let rig = Rig::new("agora-case");
        repo_with_commit(&rig.agora);
        let shouted = PathBuf::from(rig.agora.to_string_lossy().to_uppercase());
        let plan = WorktreePlan {
            repo: shouted,
            ..rig.plan("mason")
        };
        assert!(rig.refused(&plan).contains("is inside the Agora"));
    }

    #[test]
    fn a_worktree_inside_its_own_repository_is_refused() {
        let rig = Rig::new("inside-repo");
        let plan = WorktreePlan {
            path: rig.target.join("nested"),
            ..rig.plan("mason")
        };
        assert!(
            rig.refused(&plan)
                .contains("is inside the target repository")
        );
        assert!(!plan.path.exists());
    }

    #[test]
    fn a_target_that_is_not_a_repository_is_refused_by_name() {
        let rig = Rig::new("not-repo");
        let plain = rig.dir.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let plan = WorktreePlan {
            repo: plain.clone(),
            ..rig.plan("mason")
        };
        let reason = rig.refused(&plan);
        assert_eq!(
            reason,
            format!(
                "worktree refused: \"{}\" is not a git repository",
                plain.display()
            )
        );
    }

    #[test]
    fn a_populated_path_that_is_no_checkout_is_refused_and_kept() {
        let rig = Rig::new("populated");
        let plan = rig.plan("mason");
        write(&plan.path.join("notes.md"), "someone's work");
        let reason = rig.refused(&plan);
        assert!(reason.contains("already exists"), "{reason}");
        assert!(reason.contains("Move that directory aside"), "{reason}");
        assert!(
            reason.contains(&rig.target.display().to_string()),
            "{reason}"
        );
        assert!(
            !reason.contains("worktree remove"),
            "a plain directory is not git's: {reason}"
        );
        assert_eq!(
            fs::read_to_string(plan.path.join("notes.md")).unwrap(),
            "someone's work"
        );
    }

    #[test]
    fn a_file_where_the_worktree_goes_is_refused() {
        let rig = Rig::new("file");
        let plan = rig.plan("mason");
        write(&plan.path, "a file");
        assert!(rig.refused(&plan).contains("already exists"));
        assert_eq!(fs::read_to_string(&plan.path).unwrap(), "a file");
    }

    #[test]
    fn a_checkout_of_another_repository_is_refused_whatever_its_branch() {
        let rig = Rig::new("other-repo");
        let other = rig.dir.join("other");
        repo_with_commit(&other);
        let plan = rig.plan("mason");
        let other_worktrees = Worktrees::new(git(), &rig.agora);
        let theirs = WorktreePlan {
            repo: other,
            ..plan.clone()
        };
        rig_ready(other_worktrees.create(&theirs));
        let reason = rig.refused(&plan);
        assert!(
            reason.contains("a git checkout of a different repository"),
            "{reason}"
        );
        assert!(reason.contains("git worktree remove --force"), "{reason}");
        assert!(reason.contains("git worktree prune"), "{reason}");
        assert!(reason.contains("run inside that repository"), "{reason}");
    }

    fn rig_ready(outcome: WorktreeOutcome) {
        assert!(
            matches!(outcome, WorktreeOutcome::Ready { .. }),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_agents_own_kept_checkout_is_reused_where_it_stands() {
        let rig = Rig::new("kept");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        write(&path.join("wip.md"), "unpushed");
        let (again, branch, created) = rig.create(&plan);
        assert_eq!(again, path);
        assert_eq!(branch, "agent/mason");
        assert!(!created);
        assert_eq!(fs::read_to_string(path.join("wip.md")).unwrap(), "unpushed");
    }

    #[test]
    fn a_checkout_of_this_repository_is_reused_on_whatever_branch_it_sits() {
        let rig = Rig::new("any-branch");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        must(&path, &["switch", "-c", "fix/latitude-sign"]);
        let (_, branch, created) = rig.create(&plan);
        assert_eq!(branch, "fix/latitude-sign", "the branch its work is on");
        assert!(!created);
    }

    #[test]
    fn a_detached_checkout_is_reused_and_says_where_it_is() {
        let rig = Rig::new("detached");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        must(&path, &["switch", "--detach", "HEAD"]);
        let short = must(&path, &["rev-parse", "--short", "HEAD"]);
        let (_, branch, _) = rig.create(&plan);
        assert_eq!(branch, format!("detached at {}", short.trim()));
    }

    #[test]
    fn an_empty_leftover_directory_takes_the_checkout() {
        let rig = Rig::new("empty-leftover");
        let plan = rig.plan("mason");
        fs::create_dir_all(&plan.path).unwrap();
        let (path, _, created) = rig.create(&plan);
        assert!(created);
        assert!(path.join("README.md").exists());
    }

    #[test]
    fn an_emptied_path_git_still_lists_is_pruned_and_reused() {
        let rig = Rig::new("stale-entry");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        fs::remove_dir_all(&path).unwrap();
        fs::create_dir_all(&path).unwrap();
        assert!(
            rig.registered().contains("agent/mason"),
            "git still lists it"
        );
        let (again, _, created) = rig.create(&plan);
        assert_eq!(again, path);
        assert!(!created, "the branch was already there");
        assert!(path.join("README.md").exists());
    }

    #[test]
    fn a_link_where_the_worktree_goes_is_refused_and_its_target_left_empty() {
        let rig = Rig::new("link");
        let plan = rig.plan("mason");
        let elsewhere = rig.dir.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::create_dir_all(plan.path.parent().unwrap()).unwrap();
        link_dir(&elsewhere, &plan.path);
        assert_eq!(
            path_is_vacant(&plan.path),
            Vacancy::Occupied("already exists".to_string())
        );
        assert!(rig.refused(&plan).contains("already exists"));
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn vacancy_is_read_literally_and_changes_nothing() {
        let rig = Rig::new("vacancy");
        let empty = rig.dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(path_is_vacant(&empty), Vacancy::Vacant);
        assert!(
            empty.exists(),
            "an empty directory is reported, not removed"
        );
        write(&empty.join("one"), "1");
        assert_eq!(
            path_is_vacant(&empty),
            Vacancy::Occupied("already exists".to_string())
        );
        assert!(empty.join("one").exists());
        let Vacancy::Occupied(missing) = path_is_vacant(&rig.dir.join("absent")) else {
            panic!("a path that cannot be read is not vacant");
        };
        assert!(
            missing.starts_with("already exists and could not be read: "),
            "{missing}"
        );
    }

    #[test]
    fn a_clean_worktree_is_removed_and_its_entry_pruned() {
        let rig = Rig::new("remove");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        assert!(rig.worktrees.state(&path).is_clean());
        assert_eq!(
            rig.worktrees.remove(&rig.target, &path),
            WorktreeRemoval::Removed { residue: None }
        );
        assert!(!path.exists());
        assert!(!rig.registered().contains("agent/mason"));
    }

    #[test]
    fn a_dirty_worktree_is_kept_with_its_changes_named() {
        let rig = Rig::new("dirty");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        write(&path.join("wip.md"), "unpushed");
        let WorktreeRemoval::Kept { reason, changes } = rig.worktrees.remove(&rig.target, &path)
        else {
            panic!("a dirty worktree was removed");
        };
        assert!(
            reason.contains("1 uncommitted change(s); left in place"),
            "{reason}"
        );
        assert_eq!(changes, ["?? wip.md"]);
        write(&path.join("README.md"), "edited");
        let WorktreeRemoval::Kept { changes, .. } = rig.worktrees.remove(&rig.target, &path) else {
            panic!("a modified tracked file was removed");
        };
        assert!(changes.contains(&" M README.md".to_string()), "{changes:?}");
        assert_eq!(fs::read_to_string(path.join("wip.md")).unwrap(), "unpushed");
    }

    #[test]
    fn a_worktree_already_gone_is_pruned_and_its_path_reusable() {
        let rig = Rig::new("gone");
        let plan = rig.plan("mason");
        let (path, _, _) = rig.create(&plan);
        fs::remove_dir_all(&path).unwrap();
        assert_eq!(
            rig.worktrees.remove(&rig.target, &path),
            WorktreeRemoval::Removed { residue: None }
        );
        assert!(!rig.registered().contains("agent/mason"));
        rig.create(&plan);
    }

    #[test]
    fn a_worktree_that_cannot_be_read_is_not_clean() {
        let rig = Rig::new("unreadable");
        let plain = rig.dir.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let state = rig.worktrees.state(&plain);
        assert!(!state.is_clean());
        assert_eq!(state.changes.len(), 1);
        assert!(state.changes[0].starts_with("status failed: "), "{state:?}");
        assert!(
            state.changes[0].contains("not a git repository"),
            "{state:?}"
        );
    }

    #[test]
    fn a_failed_add_and_a_failed_remove_say_what_git_said() {
        let rig = Rig::new("failures");
        let bad = WorktreePlan {
            branch: "bad..name".to_string(),
            ..rig.plan("mason")
        };
        let reason = rig.refused(&bad);
        assert!(reason.starts_with("worktree add failed: "), "{reason}");
        assert!(reason.contains("bad..name"), "{reason}");
        let plan = rig.plan("locked");
        let (path, _, _) = rig.create(&plan);
        must(&rig.target, &["worktree", "lock", &path.to_string_lossy()]);
        let WorktreeRemoval::Kept { reason, changes } = rig.worktrees.remove(&rig.target, &path)
        else {
            panic!("a locked worktree was removed");
        };
        assert!(reason.starts_with("worktree remove failed: "), "{reason}");
        assert!(reason.contains("locked"), "{reason}");
        assert!(changes.is_empty());
        assert!(path.join("README.md").exists());
    }

    #[test]
    fn residue_is_swept_only_when_empty_and_reported_otherwise() {
        let rig = Rig::new("residue");
        assert_eq!(sweep_empty_residue(&rig.dir.join("absent")), None);
        let empty = rig.dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(sweep_empty_residue(&empty), None);
        assert!(!empty.exists(), "an empty leftover is bookkeeping");
        let kept = rig.dir.join("kept");
        write(&kept.join("left.txt"), "?");
        let said = sweep_empty_residue(&kept).unwrap();
        assert!(
            said.contains("1 file(s) remain in its directory; left in place"),
            "{said}"
        );
        assert!(kept.join("left.txt").exists());
        let file = rig.dir.join("a-file");
        write(&file, "x");
        let said = sweep_empty_residue(&file).unwrap();
        assert!(
            said.contains("its directory could not be tidied: "),
            "{said}"
        );
        assert!(file.exists());
    }

    #[test]
    fn two_agents_at_once_get_two_checkouts() {
        let rig = Rig::new("two");
        let (left, right) = thread::scope(|scope| {
            let left = scope.spawn(|| rig.worktrees.create(&rig.plan("left")));
            let right = scope.spawn(|| rig.worktrees.create(&rig.plan("right")));
            (left.join().unwrap(), right.join().unwrap())
        });
        rig_ready(left);
        rig_ready(right);
        assert_eq!(must(&rig.target, &["status", "--porcelain"]), "");
    }

    #[test]
    fn a_silent_failure_still_reads_as_a_sentence() {
        let silent = GitResult {
            stdout: String::new(),
            stderr: "  \n".to_string(),
            code: Some(1),
        };
        assert_eq!(said(&silent), "git said nothing");
    }
}
