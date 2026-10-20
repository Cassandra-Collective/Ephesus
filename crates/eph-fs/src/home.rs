//! The harness home (SDD-RUST §2): where it is, whose it is, and what a first
//! boot writes into it.
//!
//! Contract (Architect, 2026-10-20):
//! - **A directory is this build's home only if it holds a valid `home.json`.**
//!   The TypeScript build keeps running from `~/.ephesus` until R14, and six of
//!   its files are formats this build cannot read. So an existing directory
//!   without the marker is never written to, however empty it looks, with one
//!   exception: a directory with nothing in it at all, which no TypeScript home
//!   is, since that build creates its directories at its first boot. A refused
//!   directory is reported once, with the fix, and left exactly as it is.
//! - **A new home is marked first.** `home.json` is written before anything
//!   else, so a boot killed while creating a home resumes it next time instead
//!   of refusing a directory it made itself.
//! - **Seeded only when absent** (SDD-RUST §4.0 rule 8). `gate-policy.json` and
//!   `authority.json` are written from the shipped defaults bundled with this
//!   crate when they are missing, and each is reported. An existing file is the
//!   Architect's: it is never overwritten. When it records an older
//!   `seededFrom` than the shipped default, the drift is reported and nothing is
//!   changed. A file with no `seededFrom` was made by hand and is not compared.
//!   `config.json` is created as `{schemaVersion}` when absent, which means every
//!   default (§4.1).
//! - **`prompts/` holds overrides only** (SDD-RUST §4.4). Nothing is copied
//!   into it; [`prompt`] uses the bundled text unless the home holds a file at
//!   the same path, and reports one that cannot be read.
//!
//! Every condition is returned to the caller, which reports it (invariant 7).
//! Nothing here decides how a boot proceeds on a refused home: that is the
//! application's.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eph_schema::authority::Authority;
use eph_schema::config::Config;
use eph_schema::degradation::{Cause, Source};
use eph_schema::document::{parse, render};
use eph_schema::gates::GatePolicy;
use eph_schema::home::HomeMarker;
use eph_schema::time::Timestamp;
use eph_schema::{SchemaError, Versioned};

use crate::atomic::write_atomic;

/// The marker file at a home's root.
pub const MARKER: &str = "home.json";

/// The directories every home has (today's set, SDD-RUST §2).
pub const HOME_DIRS: [&str; 7] = [
    "agora",
    "engines",
    "index",
    "profiles",
    "prompts",
    "tools",
    "worktrees",
];

/// The shipped gate policy, seeded when `gate-policy.json` is absent.
pub const SHIPPED_GATE_POLICY: &str = include_str!("../shipped/gate-policy.json");

/// The shipped authority table, seeded when `authority.json` is absent.
pub const SHIPPED_AUTHORITY: &str = include_str!("../shipped/authority.json");

/// Names a TypeScript-era home holds, so a refusal can say which kind of
/// directory it found. They choose the message only: a directory without the
/// marker is refused whatever it holds.
const TYPESCRIPT_NAMES: [&str; 12] = [
    "agora",
    "authority.json",
    "config.json",
    "db.sqlite",
    "engines",
    "gate-policy.json",
    "index",
    "known-targets.json",
    "profiles",
    "prompts",
    "secrets.enc",
    "worktrees",
];

/// One thing a boot must report about its home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// What it is, in the degradation channel's grammar (`home/<slug>[:<file>]`).
    pub cause: Cause,
    /// What the Architect reads, with what to do.
    pub detail: String,
}

/// A home this build may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    /// Its root.
    pub root: PathBuf,
    /// Whether this call created it.
    pub created: bool,
    /// What to report: files seeded, policy drift, a policy that cannot be read.
    pub conditions: Vec<Condition>,
}

/// What opening a home found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The home is this build's, created now or found with its marker.
    Ready(Home),
    /// The directory is not this build's home. Nothing was written to it.
    Refused(Condition),
}

/// Where the home is: `$EPH_HOME` when it is set and not empty, else `.ephesus`
/// under the user's home directory. Pure: the two inputs are parameters.
pub fn resolve_root(
    eph_home: Option<OsString>,
    user_home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(root) = eph_home.filter(|root| !root.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    user_home
        .map(|home| home.join(".ephesus"))
        .ok_or_else(|| "there is no user home directory to put .ephesus in; set EPH_HOME to the directory Ephesus should use".to_string())
}

/// [`resolve_root`] from this process's environment.
pub fn root_from_env() -> Result<PathBuf, String> {
    resolve_root(std::env::var_os("EPH_HOME"), std::env::home_dir())
}

fn condition(slug: &str, subject: Option<&str>, detail: String) -> Condition {
    Condition {
        cause: Cause {
            source: Source::Home,
            slug: slug.to_string(),
            subject: subject.map(str::to_string),
        },
        detail,
    }
}

/// Opens the home at `root`, creating it if the path does not exist or is an
/// empty directory. `now` stamps a new marker. See the module contract.
pub fn open(root: &Path, now: Timestamp) -> io::Result<Opened> {
    if root.is_file() {
        return Ok(Opened::Refused(condition(
            "not-a-home",
            None,
            format!(
                "{} is a file, not a directory. Set EPH_HOME to a directory that does not exist yet, and this build makes its home there",
                root.display()
            ),
        )));
    }
    // A no-op for a directory that exists; what is IN it decides, so the only
    // directories adopted are the one made here and an empty one (eph-fs
    // mutation round 1 found a separate not-found guard no test could tell
    // from this, 2026-10-20).
    fs::create_dir_all(root)?;
    let names = entries(root)?;
    let created = if names.is_empty() {
        true
    } else if names.iter().any(|name| name == MARKER) {
        if let Some(refused) = check_marker(root)? {
            return Ok(Opened::Refused(refused));
        }
        false
    } else {
        return Ok(Opened::Refused(foreign(root, &names)));
    };
    if created {
        let marker = render(&HomeMarker::new(now)).map_err(io::Error::other)?;
        write_atomic(&root.join(MARKER), marker.as_bytes())?;
    }
    for dir in HOME_DIRS {
        fs::create_dir_all(root.join(dir))?;
    }
    let config = root.join(Config::FILE);
    if !config.exists() {
        let seed = render(&Config::seed()).map_err(io::Error::other)?;
        write_atomic(&config, seed.as_bytes())?;
    }
    let mut conditions = Vec::new();
    seed(
        root,
        GatePolicy::FILE,
        SHIPPED_GATE_POLICY,
        gate_seeded_from,
        &mut conditions,
    )?;
    seed(
        root,
        Authority::FILE,
        SHIPPED_AUTHORITY,
        authority_seeded_from,
        &mut conditions,
    )?;
    Ok(Opened::Ready(Home {
        root: root.to_path_buf(),
        created,
        conditions,
    }))
}

fn entries(root: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root)? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

fn check_marker(root: &Path) -> io::Result<Option<Condition>> {
    let text = fs::read_to_string(root.join(MARKER))?;
    Ok(parse::<HomeMarker>(&text).err().map(|error| {
        condition(
            "marker-damaged",
            None,
            format!(
                "{} cannot be read ({error}), so this build does not take the directory as its home and wrote nothing to it. Repair home.json, or set EPH_HOME to a directory that does not exist yet",
                root.join(MARKER).display()
            ),
        )
    }))
}

fn foreign(root: &Path, names: &[String]) -> Condition {
    let found: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| TYPESCRIPT_NAMES.contains(name))
        .collect();
    if found.is_empty() {
        condition(
            "not-a-home",
            None,
            format!(
                "{} holds files and no home.json, so it is not a home this build made, and nothing was written to it. Set EPH_HOME to a directory that does not exist yet, or to an empty one",
                root.display()
            ),
        )
    } else {
        condition(
            "typescript-era",
            None,
            format!(
                "{} is a TypeScript-era Ephesus home (it holds {} and no home.json). This build writes nothing into it, because the TypeScript build still runs from it until parity. Set EPH_HOME to a directory that does not exist yet, and this build makes its own home there",
                root.display(),
                found.join(", ")
            ),
        )
    }
}

fn gate_seeded_from(text: &str) -> Result<Option<u32>, SchemaError> {
    parse::<GatePolicy>(text).map(|policy| policy.seeded_from)
}

fn authority_seeded_from(text: &str) -> Result<Option<u32>, SchemaError> {
    parse::<Authority>(text).map(|table| table.seeded_from)
}

/// Seeds `file` from `shipped` when absent, or reports its drift from it.
fn seed(
    root: &Path,
    file: &str,
    shipped: &str,
    seeded_from: fn(&str) -> Result<Option<u32>, SchemaError>,
    conditions: &mut Vec<Condition>,
) -> io::Result<()> {
    let path = root.join(file);
    let ships = seeded_from(shipped)
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other(format!("the shipped {file} carries no seededFrom")))?;
    if !path.exists() {
        write_atomic(&path, shipped.as_bytes())?;
        conditions.push(condition(
            "seeded-config",
            Some(file),
            format!(
                "{file} was missing and has been created with the shipped default (version {ships}); review it at {}",
                path.display()
            ),
        ));
        return Ok(());
    }
    match seeded_from(&fs::read_to_string(&path)?) {
        Ok(Some(have)) if have < ships => conditions.push(condition(
            "policy-behind",
            Some(file),
            format!(
                "{file} was seeded from shipped version {have}, and this build ships version {ships}. The file is yours and was not changed; compare it with the shipped default to take what you want"
            ),
        )),
        Ok(_) => {}
        Err(error) => conditions.push(condition(
            "unreadable-policy",
            Some(file),
            format!(
                "{} cannot be read ({error}). It was left as it is; repair it, or move it aside to have the shipped default seeded at the next start",
                path.display()
            ),
        )),
    }
    Ok(())
}

/// A shipped prompt as this home uses it (SDD-RUST §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The override when there is one that can be read, else the bundled text.
    pub text: String,
    /// Set when an override exists and cannot be read.
    pub condition: Option<Condition>,
}

/// The prompt at `path` (`/`-separated, as under the repository's `prompts/`)
/// with the home's override applied (SDD-RUST §4.4): `<prompts>/<path>`
/// replaces `bundled` when it exists. One that exists and cannot be read as
/// text is reported, and `bundled` is used. A link to nothing exists, so it is
/// reported too, never read as no override.
pub fn prompt(prompts: &Path, path: &str, bundled: &str) -> Prompt {
    let file = path
        .split('/')
        .fold(prompts.to_path_buf(), |dir, part| dir.join(part));
    let unreadable = match fs::symlink_metadata(&file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => Some(error),
        Ok(_) => match fs::read_to_string(&file) {
            Ok(text) => {
                return Prompt {
                    text,
                    condition: None,
                };
            }
            Err(error) => Some(error),
        },
    };
    Prompt {
        text: bundled.to_string(),
        condition: unreadable.map(|error| {
            condition(
                "prompt-unreadable",
                Some(path),
                format!(
                    "the override {} cannot be read ({error}); the bundled prompt is used until it is repaired or removed",
                    file.display()
                ),
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-fs-home-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        dir
    }

    fn at() -> Timestamp {
        Timestamp::from_millis(1_000).unwrap()
    }

    fn ready(opened: Opened) -> Home {
        match opened {
            Opened::Ready(home) => home,
            Opened::Refused(why) => panic!("refused: {why:?}"),
        }
    }

    fn refused(opened: Opened) -> Condition {
        match opened {
            Opened::Refused(why) => why,
            Opened::Ready(home) => panic!("adopted: {home:?}"),
        }
    }

    /// Every file and directory under `root`, with each file's bytes.
    fn snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if path.is_dir() {
                    out.push((name, None));
                    stack.push(path);
                } else {
                    out.push((name, Some(fs::read(&path).unwrap())));
                }
            }
        }
        out.sort();
        out
    }

    fn causes(home: &Home) -> Vec<String> {
        home.conditions
            .iter()
            .map(|c| c.cause.to_string())
            .collect()
    }

    #[test]
    fn a_missing_path_becomes_a_marked_seeded_home() {
        let root = scratch("missing");
        let home = ready(open(&root, at()).unwrap());
        assert!(home.created);
        assert_eq!(
            causes(&home),
            [
                "home/seeded-config:gate-policy.json",
                "home/seeded-config:authority.json"
            ]
        );
        assert!(
            home.conditions[0].detail.contains("review it at"),
            "{:?}",
            home.conditions[0]
        );
        let marker = parse::<HomeMarker>(&fs::read_to_string(root.join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker, HomeMarker::new(at()));
        for dir in HOME_DIRS {
            assert!(root.join(dir).is_dir(), "{dir}");
        }
        assert_eq!(
            fs::read_to_string(root.join("config.json")).unwrap(),
            "{\n  \"schemaVersion\": 2\n}\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("gate-policy.json")).unwrap(),
            SHIPPED_GATE_POLICY
        );
        assert_eq!(
            fs::read_to_string(root.join("authority.json")).unwrap(),
            SHIPPED_AUTHORITY
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_second_open_seeds_nothing_and_never_overwrites() {
        let root = scratch("again");
        ready(open(&root, at()).unwrap());
        write_atomic(
            &root.join("config.json"),
            b"{\"schemaVersion\":2,\"mode\":\"improving\"}",
        )
        .unwrap();
        let before = snapshot(&root);
        let home = ready(open(&root, Timestamp::from_millis(9_000).unwrap()).unwrap());
        assert!(!home.created);
        assert_eq!(
            causes(&home),
            Vec::<String>::new(),
            "nothing to report on a second boot"
        );
        assert_eq!(
            snapshot(&root),
            before,
            "not a byte changed, the marker's time included"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_empty_directory_is_adopted_and_anything_else_without_the_marker_is_not() {
        let root = scratch("empty");
        fs::create_dir_all(&root).unwrap();
        assert!(ready(open(&root, at()).unwrap()).created);
        fs::remove_dir_all(&root).unwrap();

        // What the TypeScript build makes at its first boot: directories first.
        let root = scratch("typescript");
        fs::create_dir_all(root.join("prompts")).unwrap();
        fs::create_dir_all(root.join("agora")).unwrap();
        let before = snapshot(&root);
        let why = refused(open(&root, at()).unwrap());
        assert_eq!(why.cause.to_string(), "home/typescript-era");
        assert!(
            why.detail
                .contains("it holds agora, prompts and no home.json"),
            "{why:?}"
        );
        assert!(
            why.detail
                .contains("Set EPH_HOME to a directory that does not exist yet"),
            "{why:?}"
        );
        assert_eq!(snapshot(&root), before, "nothing was written");
        fs::remove_dir_all(&root).unwrap();

        let root = scratch("foreign");
        fs::create_dir_all(&root).unwrap();
        write_atomic(&root.join(".DS_Store"), b"x").unwrap();
        let before = snapshot(&root);
        let why = refused(open(&root, at()).unwrap());
        assert_eq!(why.cause.to_string(), "home/not-a-home");
        assert!(why.detail.contains("or to an empty one"), "{why:?}");
        assert_eq!(snapshot(&root), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_or_a_damaged_marker_is_refused_and_left_alone() {
        let parent = scratch("file");
        fs::create_dir_all(&parent).unwrap();
        let file = parent.join("home-file");
        write_atomic(&file, b"not a directory").unwrap();
        let why = refused(open(&file, at()).unwrap());
        assert_eq!(why.cause.to_string(), "home/not-a-home");
        assert!(why.detail.contains("is a file, not a directory"), "{why:?}");
        assert_eq!(fs::read(&file).unwrap(), b"not a directory");
        fs::remove_dir_all(&parent).unwrap();

        let root = scratch("damaged");
        fs::create_dir_all(&root).unwrap();
        write_atomic(&root.join(MARKER), b"{\"schemaVersion\":1}").unwrap();
        let before = snapshot(&root);
        let why = refused(open(&root, at()).unwrap());
        assert_eq!(why.cause.to_string(), "home/marker-damaged");
        assert!(why.detail.contains("Repair home.json"), "{why:?}");
        assert_eq!(snapshot(&root), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_home_killed_after_its_marker_resumes_instead_of_refusing_itself() {
        let root = scratch("resume");
        fs::create_dir_all(&root).unwrap();
        write_atomic(
            &root.join(MARKER),
            render(&HomeMarker::new(at())).unwrap().as_bytes(),
        )
        .unwrap();
        let home = ready(open(&root, at()).unwrap());
        assert!(
            !home.created,
            "it was created by the boot that wrote the marker"
        );
        assert_eq!(home.conditions.len(), 2, "the seeding finishes now");
        assert!(root.join("agora").is_dir());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_older_seeded_policy_is_reported_and_kept() {
        let root = scratch("drift");
        ready(open(&root, at()).unwrap());
        let older = SHIPPED_GATE_POLICY.replace("\"seededFrom\": 1", "\"seededFrom\": 0");
        write_atomic(&root.join("gate-policy.json"), older.as_bytes()).unwrap();
        let home = ready(open(&root, at()).unwrap());
        assert_eq!(causes(&home), ["home/policy-behind:gate-policy.json"]);
        assert!(
            home.conditions[0]
                .detail
                .contains("shipped version 0, and this build ships version 1"),
            "{:?}",
            home.conditions[0]
        );
        assert_eq!(
            fs::read_to_string(root.join("gate-policy.json")).unwrap(),
            older,
            "the Architect's file is kept"
        );

        let by_hand = SHIPPED_AUTHORITY.replace("  \"seededFrom\": 1,\n", "");
        write_atomic(&root.join("authority.json"), by_hand.as_bytes()).unwrap();
        write_atomic(
            &root.join("gate-policy.json"),
            SHIPPED_GATE_POLICY.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            causes(&ready(open(&root, at()).unwrap())),
            Vec::<String>::new(),
            "a hand-made file is not compared"
        );

        write_atomic(
            &root.join("gate-policy.json"),
            b"{\"schemaVersion\":1,\"autonomy\":\"manual\",\"rules\":[]}",
        )
        .unwrap();
        let home = ready(open(&root, at()).unwrap());
        assert_eq!(causes(&home), ["home/unreadable-policy:gate-policy.json"]);
        assert!(
            home.conditions[0].detail.contains("older Ephesus"),
            "{:?}",
            home.conditions[0]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_shipped_defaults_parse_at_this_build_s_versions_and_say_what_they_grant() {
        let policy = parse::<GatePolicy>(SHIPPED_GATE_POLICY).unwrap();
        assert_eq!(policy.schema_version, 2);
        assert_eq!(policy.seeded_from, Some(1));
        assert_eq!(policy.rules.len(), 6);
        let table = parse::<Authority>(SHIPPED_AUTHORITY).unwrap();
        assert_eq!(table.schema_version, 1);
        assert_eq!(table.seeded_from, Some(1));
        assert_eq!(table.grants.len(), 3);
    }

    #[test]
    fn the_root_is_eph_home_or_dot_ephesus_under_the_user_home() {
        assert_eq!(
            resolve_root(Some("/x/home".into()), Some("/users/a".into())),
            Ok(PathBuf::from("/x/home"))
        );
        assert_eq!(
            resolve_root(Some(OsString::new()), Some("/users/a".into())),
            Ok(PathBuf::from("/users/a").join(".ephesus")),
            "an empty EPH_HOME is unset"
        );
        assert_eq!(
            resolve_root(None, Some("/users/a".into())),
            Ok(PathBuf::from("/users/a").join(".ephesus"))
        );
        assert!(
            resolve_root(None, None)
                .unwrap_err()
                .contains("set EPH_HOME")
        );
        let from_env = root_from_env().unwrap();
        assert_eq!(
            Ok(from_env.clone()),
            resolve_root(std::env::var_os("EPH_HOME"), std::env::home_dir()),
            "the process's own environment, read through the pure rule"
        );
        assert!(!from_env.as_os_str().is_empty());
    }

    #[test]
    fn an_older_seeded_authority_is_reported_like_the_policy() {
        // Mutation round 1 (2026-10-20): drift was tested on gate-policy.json
        // only, so authority.json's reading of `seededFrom` was unchecked.
        let root = scratch("authority-drift");
        let home = ready(open(&root, at()).unwrap());
        assert!(
            home.conditions[1].detail.contains("(version 1)"),
            "{:?}",
            home.conditions[1]
        );
        let older = SHIPPED_AUTHORITY.replace("\"seededFrom\": 1", "\"seededFrom\": 0");
        write_atomic(&root.join("authority.json"), older.as_bytes()).unwrap();
        let home = ready(open(&root, at()).unwrap());
        assert_eq!(causes(&home), ["home/policy-behind:authority.json"]);
        assert!(
            home.conditions[0]
                .detail
                .contains("shipped version 0, and this build ships version 1"),
            "{:?}",
            home.conditions[0]
        );
        write_atomic(&root.join("authority.json"), b"{}").unwrap();
        assert_eq!(
            causes(&ready(open(&root, at()).unwrap())),
            ["home/unreadable-policy:authority.json"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_prompt_override_replaces_the_bundled_text_and_a_bad_one_is_reported() {
        let dir = scratch("prompts");
        let file = dir.join("agora").join("PROTOCOL.md");
        let bundled = "the bundled protocol\n";
        assert_eq!(
            prompt(&dir, "agora/PROTOCOL.md", bundled),
            Prompt {
                text: bundled.to_string(),
                condition: None
            },
            "no override, and no prompts directory at all"
        );
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_atomic(&file, b"the Architect's protocol\n").unwrap();
        assert_eq!(
            prompt(&dir, "agora/PROTOCOL.md", bundled),
            Prompt {
                text: "the Architect's protocol\n".to_string(),
                condition: None
            }
        );
        write_atomic(&file, &[0xff, 0xfe]).unwrap();
        let bad = prompt(&dir, "agora/PROTOCOL.md", bundled);
        assert_eq!(bad.text, bundled, "bytes that are not text");
        let why = bad.condition.unwrap();
        assert_eq!(
            why.cause.to_string(),
            "home/prompt-unreadable:agora/PROTOCOL.md"
        );
        assert!(
            why.detail
                .ends_with("the bundled prompt is used until it is repaired or removed"),
            "{}",
            why.detail
        );
        fs::remove_file(&file).unwrap();
        fs::create_dir_all(&file).unwrap();
        assert!(
            prompt(&dir, "agora/PROTOCOL.md", bundled)
                .condition
                .is_some(),
            "a directory where the override goes"
        );
        // A path the OS refuses to look up is an error other than "not found".
        let refused = prompt(&dir, "agora/PRO\0TOCOL.md", bundled);
        assert_eq!(refused.text, bundled);
        assert!(refused.condition.is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_prompt_override_that_links_to_nothing_is_reported() {
        let dir = scratch("prompt-link");
        fs::create_dir_all(dir.join("agora")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("agora").join("board.md")).unwrap();
        let linked = prompt(&dir, "agora/board.md", "seed\n");
        assert_eq!(linked.text, "seed\n");
        assert!(
            linked.condition.is_some(),
            "a link to nothing is not absent"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
