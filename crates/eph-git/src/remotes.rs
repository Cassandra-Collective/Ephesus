//! `read_remotes`: a checkout's remotes (M8.5's reader).
//!
//! It lives in this crate for the reason the worktrees do: this is the one
//! crate that runs git, and that rule is checkable only while every git start
//! is here. It reads, commits nothing, and never touches the Agora.
//!
//! Contract: never panics. A directory that is not a repository, a git that
//! could not start, and a repository with no remote are three different
//! answers, because the Architect is told which one it was.

use std::path::Path;

use crate::runner::Git;

/// One remote, as `git remote -v` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// Its name, such as `origin`.
    pub name: String,
    /// Its URL, as configured.
    pub url: String,
}

/// What reading a checkout's remotes found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemotesRead {
    /// The remotes, each name and URL once, in git's order.
    Read(Vec<Remote>),
    /// Why git could not read them, in git's words where it had any.
    Failed(String),
}

/// Reads the remotes of the checkout at `cwd` with `git`.
pub fn read_remotes(git: &Git, cwd: &Path) -> RemotesRead {
    let result = git.run(cwd, &["remote", "-v"]);
    if result.ok() {
        RemotesRead::Read(parse_remotes(&result.stdout))
    } else {
        RemotesRead::Failed(because(&result.stderr))
    }
}

/// The remotes in `git remote -v` output: one line per remote per direction
/// (`origin\thttps://… (fetch)`), so each name and URL is kept once. A line of
/// any other shape is skipped.
pub fn parse_remotes(stdout: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in stdout.lines() {
        let mut words = line.split_whitespace();
        let (Some(name), Some(url), Some("(fetch)" | "(push)"), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        let remote = Remote {
            name: name.to_string(),
            url: url.to_string(),
        };
        if !remotes.contains(&remote) {
            remotes.push(remote);
        }
    }
    remotes
}

/// The sentence for a failed read: git's first line names the problem better
/// than a guess at it ("not a git repository", "could not be started").
fn because(stderr: &str) -> String {
    match stderr.lines().next().map(str::trim) {
        Some(first) if !first.is_empty() => {
            format!("git could not read the target's remotes: {first}")
        }
        _ => "git could not read the target's remotes".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::runner::tests::{git, must, repo_with_commit, scratch};

    fn remote(name: &str, url: &str) -> Remote {
        Remote {
            name: name.to_string(),
            url: url.to_string(),
        }
    }

    #[test]
    fn one_remote_is_read_once_not_once_per_direction() {
        let dir = scratch("remotes-one");
        repo_with_commit(&dir);
        must(
            &dir,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );
        assert_eq!(
            read_remotes(&git(), &dir),
            RemotesRead::Read(vec![remote("origin", "https://github.com/owner/repo.git")])
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_forks_two_remotes_and_a_separate_push_url_are_each_read() {
        let dir = scratch("remotes-fork");
        repo_with_commit(&dir);
        must(
            &dir,
            &["remote", "add", "origin", "https://github.com/me/repo.git"],
        );
        must(
            &dir,
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/them/repo.git",
            ],
        );
        must(
            &dir,
            &[
                "remote",
                "set-url",
                "--push",
                "upstream",
                "git@github.com:them/repo.git",
            ],
        );
        assert_eq!(
            read_remotes(&git(), &dir),
            RemotesRead::Read(vec![
                remote("origin", "https://github.com/me/repo.git"),
                remote("upstream", "https://github.com/them/repo.git"),
                remote("upstream", "git@github.com:them/repo.git"),
            ])
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repository_with_no_remote_reads_as_an_empty_list() {
        let dir = scratch("remotes-none");
        repo_with_commit(&dir);
        assert_eq!(read_remotes(&git(), &dir), RemotesRead::Read(Vec::new()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_is_said_in_gits_words() {
        let dir = scratch("remotes-not-repo");
        let RemotesRead::Failed(said) = read_remotes(&git(), &dir) else {
            panic!("a directory that is no repository has no remotes to read");
        };
        assert!(
            said.starts_with("git could not read the target's remotes: "),
            "{said}"
        );
        assert!(said.contains("not a git repository"), "{said}");
        let RemotesRead::Failed(said) = read_remotes(&git(), &dir.join("absent")) else {
            panic!("a directory that does not exist has no remotes");
        };
        assert!(said.contains("could not be started"), "{said}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_with_nothing_said_is_still_a_sentence() {
        assert_eq!(because(""), "git could not read the target's remotes");
        assert_eq!(
            because("  \nsecond"),
            "git could not read the target's remotes"
        );
        assert_eq!(
            because("fatal: no\nmore"),
            "git could not read the target's remotes: fatal: no"
        );
    }

    #[test]
    fn a_line_of_any_other_shape_is_skipped() {
        let stdout = "hint: something\norigin\thttps://a (fetch)\norigin\thttps://a (push)\nodd\thttps://b (fetchx)\nshort (fetch)\nlong https://c (push) extra\n\n";
        assert_eq!(parse_remotes(stdout), vec![remote("origin", "https://a")]);
        assert_eq!(parse_remotes(""), Vec::new());
    }
}
