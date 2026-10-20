//! `cargo xtask coverage`: one line-coverage floor per crate (RUST-PLAN §4 item
//! 7; ENGINEERING-STANDARDS §6 item 7; Architect, 2026-10-20).
//!
//! The Rust half of the seam rule's coverage half: no crate falls below the
//! floor it was measured at, and no production file lands that no test enters.
//! It carries the TypeScript record's rules (`scripts/check-coverage.cjs`), with
//! the two the Architect set for Rust: ONE floor per crate, the lowest line
//! coverage of the three CI platforms, raised only from CI artifacts.
//!
//! Commands:
//! - `cargo xtask coverage`, which is also the gate's last step, runs
//!   `cargo llvm-cov --workspace`, measures this platform and checks the result
//!   against [`FLOORS_FILE`].
//! - `cargo xtask coverage emit` reads the report that run left and prints the
//!   measurement as JSON, with the condition it was taken in. CI redirects it to
//!   a file and uploads it.
//! - `cargo xtask coverage ratchet <dir>` reads the measurements downloaded from
//!   CI and prints the floors file they justify. The developer redirects it and
//!   moves it into place, and the diff is the review.
//!
//! Contract: xtask writes no file here. cargo-llvm-cov writes its lcov report
//! (at [`REPORT`] under the target directory), and this module deletes a
//! previous report before a run so a failed run cannot leave a stale one. What
//! else must land on disk goes to stdout for the shell to write, because RUST-PLAN
//! §3.2 gives the write exemption to eph-fs alone.
//!
//! The rules, each carried from the TypeScript record or decided for Rust:
//!
//! 1. **Production lines only.** A line inside an item compiled only for tests
//!    (`#[cfg(test)]` or a test attribute such as `#[test]`, found in the syntax
//!    tree) is not counted, and nor is a file outside a crate's `src/`. Counted,
//!    inline test modules nearly double eph-schema's lines and cover themselves,
//!    so a production drop would show at about half its size (measured
//!    2026-10-20: 1004 production lines, 915 test lines). The TypeScript floors
//!    likewise measured `src/` without `test/`. Code under `#[cfg(not(test))]`
//!    is refused: no test build compiles it, so no figure could include it.
//! 2. **No production file no test enters**, read in both directions. A file
//!    with production lines and none of them run fails, by name, and so does a
//!    production file the report never names that holds a function body: one no
//!    `mod` declares, one compiled only on another platform, or generics nothing
//!    instantiates. There is no list of known cases: the workspace starts
//!    without one.
//! 3. **Every crate has a floor, and every floor a crate.** A crate without one
//!    fails until it is seeded from CI artifacts, never from one machine and
//!    never with a figure invented for a platform it was not measured on (§6
//!    item 7). A crate that measures no production line fails too: a check over
//!    nothing proves nothing.
//! 4. **A regression fails on the run that measured it**, beyond the recorded
//!    tolerance. Only a raise waits for corroboration.
//! 5. **A record nobody turns fails.** A platform measuring more than the
//!    recorded lag above its own recorded figure must be ratcheted. The lag is
//!    compared per platform, because a crate with platform-specific code sits
//!    above the cross-platform floor on one platform by design.
//! 6. **A raise is corroborated.** [`ratchet`] needs `corroboratingRuns` CI runs
//!    on each platform, every one of the current tree, and takes the lowest. It
//!    raises every crate that drifted up, refuses one that fell, never writes a
//!    lower floor, and counts one artifact once however often it was downloaded.
//! 7. **A figure is compared only under the tool it was taken with.** A
//!    measurement by another cargo-llvm-cov than the floors' fails, naming the
//!    approved version.
//! 8. **The floors file holds only what a ratchet writes.** Its validator
//!    refuses a floor below its own lowest platform figure, a tolerance over
//!    [`MAX_TOLERANCE`], and rows without `corroboratingRuns` recorded runs per
//!    platform, so a hand edit that lowers a floor is refused on read.
//!
//! What it cannot see: the body of a generic function in a file the report
//! does name, when nothing instantiates it (LLVM instruments only what is
//! compiled), and code a macro generates, whose lines the syntax tree does
//! not show as items.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde_json::{Map, Value};
use syn::spanned::Spanned;

use crate::gate::workspace_root;
use crate::workspace::{self, Workspace};

/// The floors file, relative to the workspace root.
pub const FLOORS_FILE: &str = "xtask/coverage-floors.json";

/// Where a run leaves its lcov report, relative to the target directory.
pub const REPORT: &str = "ephesus-coverage/lcov.info";

/// The platforms a floor is the lowest of, as `std::env::consts::OS` names
/// them: the `rust` CI job's matrix (NFR-11).
pub const PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];

/// The install line a missing cargo-llvm-cov is refused with.
pub const INSTALL: &str = "cargo install cargo-llvm-cov --version 0.9.1 --locked";

/// The fewest corroborating runs the floors file may ask for: two runs taken
/// back to back share a machine's mood (the TypeScript record, 2026-09-04).
pub const MIN_CORROBORATING_RUNS: u64 = 3;

/// The widest tolerance the floors file may record, in hundredths of a
/// percent: the TypeScript record's own band was 0.25 points, and a wider one
/// lowers every floor at once without touching a row (review finding F14).
pub const MAX_TOLERANCE: u64 = 100;

/// How CI runs become floors, for every message that needs to say so. A
/// green `rust` job is rerun whole; `--failed` reruns only red jobs (review
/// finding F15).
const HOW_TO_RATCHET: &str = "rerun that push's CI run until each platform has the runs it needs (`gh run rerun <run>`, or `gh run rerun <run> --failed` while the `rust` job is red), download each attempt (`gh run download <run> -p 'rust-coverage-*' -D <dir>/attempt-<n>`), then `cargo xtask coverage ratchet <dir> > floors.new && mv floors.new xtask/coverage-floors.json`";

/// The files outside the members' own manifests and sources whose text decides
/// what a run measures, relative to the root. They are part of the tree hash.
pub const TREE_FILES: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/config.toml",
];

const USAGE: &str = "usage: cargo xtask coverage [emit | ratchet <dir>]";

/// A count of executable lines and how many of them ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lines {
    /// Lines that ran at least once.
    pub covered: u64,
    /// Executable lines.
    pub count: u64,
}

impl Lines {
    /// Coverage in hundredths of a percent, rounded down so a figure never
    /// claims more than was measured. `None` when there is no line to measure.
    pub fn basis_points(self) -> Option<u64> {
        (self.count > 0).then(|| self.covered.saturating_mul(10_000) / self.count)
    }

    fn add(&mut self, other: Lines) {
        self.covered += other.covered;
        self.count += other.count;
    }
}

/// Hundredths of a percent as the record writes them: `9373` is `93.73`.
pub fn percent(basis_points: u64) -> String {
    format!("{}.{:02}", basis_points / 100, basis_points % 100)
}

/// One source file of an lcov report: the hit count of each executable line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFile {
    /// The path the report names.
    pub path: PathBuf,
    /// Hits per 1-based line number.
    pub hits: BTreeMap<usize, u64>,
}

/// Reads an lcov report. `SF:`, `DA:<line>,<hits>` and `end_of_record` are
/// read and every other record is skipped. A line named twice keeps the larger
/// count. This is the validator for the format: the one place it is read.
pub fn parse_lcov(text: &str) -> Result<Vec<ReportFile>, String> {
    let mut files = Vec::new();
    let mut current: Option<ReportFile> = None;
    for (index, line) in text.lines().enumerate() {
        let at = index + 1;
        if let Some(path) = line.strip_prefix("SF:") {
            if current.is_some() {
                return Err(format!(
                    "lcov line {at}: a file starts before the last one ended"
                ));
            }
            current = Some(ReportFile {
                path: PathBuf::from(path),
                hits: BTreeMap::new(),
            });
        } else if let Some(data) = line.strip_prefix("DA:") {
            let Some(file) = current.as_mut() else {
                return Err(format!("lcov line {at}: line data outside a file"));
            };
            let mut fields = data.split(',');
            let number = fields.next().and_then(|field| field.parse::<usize>().ok());
            let hits = fields.next().and_then(|field| field.parse::<u64>().ok());
            let (Some(number), Some(hits)) = (number, hits) else {
                return Err(format!("lcov line {at}: `DA:{data}` is not <line>,<hits>"));
            };
            let entry = file.hits.entry(number).or_insert(0);
            *entry = (*entry).max(hits);
        } else if line == "end_of_record" {
            match current.take() {
                Some(file) => files.push(file),
                None => return Err(format!("lcov line {at}: end_of_record outside a file")),
            }
        }
    }
    if current.is_some() {
        return Err("lcov: the last file has no end_of_record, so the report is cut short".into());
    }
    Ok(files)
}

/// The 1-based lines of `source` inside an item compiled only for tests: one
/// carrying `#[cfg(test)]` or `#[cfg(all(.., test, ..))]`, or a test attribute
/// (`#[test]`, `#[tokio::test]`: any whose path ends in `test`), at any depth of
/// inline modules, impl blocks and traits. Two shapes are refused. An
/// out-of-line `#[cfg(test)] mod x;` would have its file counted as production.
/// An item under `#[cfg(not(test))]` is compiled by no test build, so it is
/// production code no test can run and the report can never show (review
/// finding F11).
pub fn test_only_lines(source: &str) -> Result<BTreeSet<usize>, String> {
    let file = parse_source(source)?;
    let mut lines = BTreeSet::new();
    test_items(&file.items, &mut lines)?;
    Ok(lines)
}

fn parse_source(source: &str) -> Result<syn::File, String> {
    syn::parse_file(source).map_err(|error| format!("line {}: {error}", error.span().start().line))
}

/// Whether `source` holds a function body a test build compiles: a free
/// function, a method, or a trait's default method, outside test-only items.
/// A production file the report never names and that holds one is untested
/// (review finding F11).
pub fn has_body(source: &str) -> Result<bool, String> {
    Ok(items_have_body(&parse_source(source)?.items))
}

fn items_have_body(items: &[syn::Item]) -> bool {
    items.iter().any(|item| {
        if test_only(item_attrs(item)) {
            return false;
        }
        match item {
            syn::Item::Fn(_) => true,
            syn::Item::Impl(block) => block.items.iter().any(|member| {
                matches!(member, syn::ImplItem::Fn(_)) && !test_only(impl_item_attrs(member))
            }),
            syn::Item::Trait(block) => block.items.iter().any(|member| {
                matches!(member, syn::TraitItem::Fn(method) if method.default.is_some())
                    && !test_only(trait_item_attrs(member))
            }),
            syn::Item::Mod(module) => module
                .content
                .as_ref()
                .is_some_and(|(_, inner)| items_have_body(inner)),
            _ => false,
        }
    })
}

fn refuse_hidden(attrs: &[syn::Attribute], span: proc_macro2::Span) -> Result<(), String> {
    let hidden = attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| excludes_test(&meta))
    });
    if hidden {
        return Err(format!(
            "line {}: `#[cfg(not(test))]` keeps this code out of every test build, so no test can run it and coverage cannot see it; give the test build the same code through a seam instead of a cfg",
            span.start().line
        ));
    }
    Ok(())
}

/// Whether a `cfg` predicate is false in every test build: `not(test)`, or an
/// `all` with such a term.
fn excludes_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::List(list) if list.path.is_ident("not") => list
            .parse_args::<syn::Meta>()
            .is_ok_and(|inner| matches!(&inner, syn::Meta::Path(path) if path.is_ident("test"))),
        syn::Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|terms| terms.iter().any(excludes_test)),
        _ => false,
    }
}

fn test_items(items: &[syn::Item], lines: &mut BTreeSet<usize>) -> Result<(), String> {
    for item in items {
        refuse_hidden(item_attrs(item), item.span())?;
        if test_only(item_attrs(item)) {
            if let syn::Item::Mod(module) = item
                && module.content.is_none()
            {
                return Err(format!(
                    "line {}: `#[cfg(test)] mod {};` keeps tests in another file, which coverage would count as production; write the module inline",
                    module.span().start().line,
                    module.ident
                ));
            }
            span_lines(item.span(), lines);
            continue;
        }
        match item {
            syn::Item::Mod(module) => {
                if let Some((_, inner)) = &module.content {
                    test_items(inner, lines)?;
                }
            }
            syn::Item::Impl(block) => {
                for member in &block.items {
                    refuse_hidden(impl_item_attrs(member), member.span())?;
                    if test_only(impl_item_attrs(member)) {
                        span_lines(member.span(), lines);
                    }
                }
            }
            syn::Item::Trait(block) => {
                for member in &block.items {
                    refuse_hidden(trait_item_attrs(member), member.span())?;
                    if test_only(trait_item_attrs(member)) {
                        span_lines(member.span(), lines);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn span_lines(span: proc_macro2::Span, lines: &mut BTreeSet<usize>) {
    lines.extend(span.start().line..=span.end().line);
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn impl_item_attrs(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn trait_item_attrs(item: &syn::TraitItem) -> &[syn::Attribute] {
    match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

/// Whether an item compiles only for tests: a `cfg` that [`requires_test`],
/// or a test attribute, which compiles only under `--test` and would cover
/// itself (review finding F12).
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
            || (path.is_ident("cfg")
                && attr
                    .parse_args::<syn::Meta>()
                    .is_ok_and(|meta| requires_test(&meta)))
    })
}

/// Whether a `cfg` predicate holds only in a test build: `test`, or an `all`
/// with `test` among its terms. `any(test, x)` and `not(test)` do not, since
/// each can hold in a build that ships.
fn requires_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|terms| terms.iter().any(requires_test)),
        _ => false,
    }
}

/// Whether `path` is production code of the crate in `dir`: under its `src/`.
/// A crate's `tests/`, `benches/`, `examples/` and `build.rs` are not.
pub fn is_production(dir: &Path, path: &Path) -> bool {
    path.strip_prefix(dir)
        .is_ok_and(|rest| rest.components().next() == Some(Component::Normal("src".as_ref())))
}

/// What a report says about the workspace's production lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tally {
    /// Production lines per crate. Every member has a row, measured or not.
    pub crates: BTreeMap<String, Lines>,
    /// Production files with lines and none of them run, relative to the root.
    pub untested: Vec<String>,
    /// Production files read.
    pub files: usize,
}

/// Tallies `report` against the workspace's sources, in both directions (the
/// TypeScript record's rule). Pure. A report naming a file that is no source of
/// the workspace is a report of another tree, and fails, as does a production
/// file whose syntax tree cannot be read. A production file the report never
/// names but that holds a function body ([`has_body`]) is untested: declared by
/// no `mod`, compiled only on another platform, or only generics nothing
/// instantiates (review finding F11).
pub fn tally(
    root: &Path,
    workspace: &Workspace,
    report: &[ReportFile],
) -> Result<Tally, Vec<String>> {
    let mut out = Tally {
        crates: workspace
            .members
            .iter()
            .map(|member| (member.name.clone(), Lines::default()))
            .collect(),
        ..Tally::default()
    };
    let mut errors = Vec::new();
    for file in report {
        let found = workspace.members.iter().find_map(|member| {
            member
                .sources
                .iter()
                .find(|source| path_key(&source.path) == path_key(&file.path))
                .map(|source| (member, source))
        });
        let Some((member, source)) = found else {
            errors.push(format!(
                "the report names {}, which is no source of this workspace, so it is a report of another tree; run `cargo xtask coverage` again",
                file.path.display()
            ));
            continue;
        };
        let dir = member.manifest_path.parent().unwrap_or(root);
        if !is_production(dir, &source.path) {
            continue;
        }
        let excluded = match test_only_lines(&source.text) {
            Ok(lines) => lines,
            Err(error) => {
                errors.push(format!("{}: {error}", relative(root, &source.path)));
                continue;
            }
        };
        let mut lines = Lines::default();
        for (number, hits) in &file.hits {
            if !excluded.contains(number) {
                lines.count += 1;
                if *hits > 0 {
                    lines.covered += 1;
                }
            }
        }
        if lines.count > 0 && lines.covered == 0 {
            out.untested.push(relative(root, &source.path));
        }
        out.crates
            .entry(member.name.clone())
            .or_default()
            .add(lines);
        out.files += 1;
    }
    let reported: BTreeSet<String> = report.iter().map(|file| path_key(&file.path)).collect();
    for member in &workspace.members {
        let dir = member.manifest_path.parent().unwrap_or(root);
        for source in &member.sources {
            if !is_production(dir, &source.path) || reported.contains(&path_key(&source.path)) {
                continue;
            }
            match has_body(&source.text) {
                Ok(true) => out.untested.push(relative(root, &source.path)),
                Ok(false) => {}
                Err(error) => errors.push(format!("{}: {error}", relative(root, &source.path))),
            }
        }
    }
    out.untested.sort();
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// A path as a comparison key: `/` separators, and a drive letter in upper
/// case, because `c:` and `C:` name one drive.
fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) => format!(
            "{}{}",
            drive.to_ascii_uppercase(),
            &text[drive.len_utf8()..]
        ),
        _ => text,
    }
}

/// `path` relative to `root` with `/` separators, or the whole path when it is
/// not under `root`.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// A stable hash of the files that decide what a run measures, as `(path,
/// text)` pairs with paths relative to the root. Independent of order and of
/// line endings, so a Windows checkout and a Linux one of a commit agree.
/// FNV-1a (64-bit) is stable across Rust versions, which `DefaultHasher` is not,
/// and tells trees apart; nothing here defends against a tree built to collide.
pub fn tree_hash(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (path, text) in sorted {
        hash = fnv(hash, path.as_bytes());
        hash = fnv(hash, &[0]);
        hash = fnv(hash, text.replace("\r\n", "\n").as_bytes());
        hash = fnv(hash, &[0]);
    }
    format!("fnv1a64:{hash:016x}")
}

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A CI run's identity: what keeps one artifact from counting twice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CiRun {
    /// `GITHUB_RUN_ID`.
    pub id: String,
    /// `GITHUB_RUN_ATTEMPT`.
    pub attempt: String,
    /// `GITHUB_SHA`: the commit it ran.
    pub sha: String,
}

/// What one run measured on one platform, with the condition it ran in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    /// The platform, as `std::env::consts::OS` names it.
    pub os: String,
    /// The tree it measured ([`tree_hash`]).
    pub tree: String,
    /// The coverage tool and its version.
    pub tool: String,
    /// The CI run it came from; `None` for a run on a developer's machine.
    pub run: Option<CiRun>,
    /// Production lines per crate.
    pub crates: BTreeMap<String, Lines>,
    /// Production files no test enters, relative to the root.
    pub untested: Vec<String>,
}

/// The measurement as the artifact CI uploads.
pub fn render_measurement(measurement: &Measurement) -> String {
    let crates: Map<String, Value> = measurement
        .crates
        .iter()
        .map(|(name, lines)| {
            (
                name.clone(),
                serde_json::json!({ "covered": lines.covered, "count": lines.count }),
            )
        })
        .collect();
    let run = measurement.run.as_ref().map_or(
        Value::Null,
        |run| serde_json::json!({ "id": run.id, "attempt": run.attempt, "sha": run.sha }),
    );
    let value = serde_json::json!({
        "schemaVersion": 1,
        "os": measurement.os,
        "tree": measurement.tree,
        "tool": measurement.tool,
        "run": run,
        "crates": crates,
        "untested": measurement.untested,
    });
    format!("{value:#}\n")
}

/// Reads a measurement. This is its validator: every field is required and no
/// other is allowed.
pub fn parse_measurement(text: &str) -> Result<Measurement, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let object = as_object(&value, "the measurement")?;
    exact_keys(
        object,
        &[
            "schemaVersion",
            "os",
            "tree",
            "tool",
            "run",
            "crates",
            "untested",
        ],
        "the measurement",
    )?;
    schema_version(object)?;
    let run = match object.get("run") {
        Some(Value::Object(run)) => {
            exact_keys(run, &["id", "attempt", "sha"], "`run`")?;
            Some(CiRun {
                id: string(run, "id")?,
                attempt: string(run, "attempt")?,
                sha: string(run, "sha")?,
            })
        }
        Some(Value::Null) => None,
        _ => return Err("`run` must be null or an object with id, attempt and sha".into()),
    };
    let mut crates = BTreeMap::new();
    for (name, lines) in as_object(field(object, "crates")?, "`crates`")? {
        let what = format!("`crates.{name}`");
        let lines = as_object(lines, &what)?;
        exact_keys(lines, &["covered", "count"], &what)?;
        let count = |key: &str| {
            lines
                .get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{what}.{key} must be a whole number"))
        };
        let (covered, count) = (count("covered")?, count("count")?);
        if covered > count {
            return Err(format!("{what} covers {covered} of {count} lines"));
        }
        crates.insert(name.clone(), Lines { covered, count });
    }
    let untested = field(object, "untested")?
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .map(|f| f.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| "`untested` must be a list of paths".to_string())?;
    Ok(Measurement {
        os: string(object, "os")?,
        tree: string(object, "tree")?,
        tool: string(object, "tool")?,
        run,
        crates,
        untested,
    })
}

/// A crate's row in the floors file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateFloor {
    /// The floor, in hundredths of a percent: the lowest figure the
    /// corroborating runs measured across the platforms.
    pub floor: u64,
    /// The lowest each platform measured: the gap between platforms, and what
    /// the lag rule compares a platform to.
    pub platforms: BTreeMap<String, u64>,
}

/// Where the current floors came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The tree the runs measured.
    pub tree: String,
    /// The tool they measured with.
    pub tool: String,
    /// Each run, as `<os> <run id>/<attempt>`.
    pub runs: Vec<String>,
}

/// The floors file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floors {
    /// What the file is, for its reader.
    pub about: Vec<String>,
    /// How far below its floor a crate may measure before it fails, in
    /// hundredths of a percent.
    pub tolerance: u64,
    /// Why the tolerance is what it is.
    pub tolerance_reason: String,
    /// How far above its recorded figure a platform may measure before the
    /// record must be ratcheted, in hundredths of a percent.
    pub ratchet_lag: u64,
    /// Why the lag is what it is.
    pub ratchet_lag_reason: String,
    /// The CI runs per platform a raise needs.
    pub corroborating_runs: u64,
    /// Why that many.
    pub corroborating_runs_reason: String,
    /// Where the floors came from; `None` before the first ratchet.
    pub measured_from: Option<Provenance>,
    /// The rows, by crate.
    pub crates: BTreeMap<String, CrateFloor>,
}

const FLOORS_KEYS: [&str; 10] = [
    "schemaVersion",
    "about",
    "tolerance",
    "toleranceReason",
    "ratchetLag",
    "ratchetLagReason",
    "corroboratingRuns",
    "corroboratingRunsReason",
    "measuredFrom",
    "crates",
];

/// Reads the floors file. This is its validator: every field is required, no
/// other is allowed, every row names all three platforms, and the corroborating
/// runs are at least [`MIN_CORROBORATING_RUNS`].
pub fn parse_floors(text: &str) -> Result<Floors, String> {
    let fail = |message: String| format!("{FLOORS_FILE}: {message}");
    let value: Value =
        serde_json::from_str(text).map_err(|error| fail(format!("not JSON: {error}")))?;
    let object = as_object(&value, "the file").map_err(fail)?;
    exact_keys(object, &FLOORS_KEYS, "the file").map_err(fail)?;
    schema_version(object).map_err(fail)?;
    let about = field(object, "about")
        .ok()
        .and_then(Value::as_array)
        .and_then(|lines| {
            lines
                .iter()
                .map(|l| l.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| fail("`about` must be a list of strings".into()))?;
    let corroborating_runs = field(object, "corroboratingRuns")
        .ok()
        .and_then(Value::as_u64)
        .filter(|runs| *runs >= MIN_CORROBORATING_RUNS)
        .ok_or_else(|| {
            fail(format!(
                "`corroboratingRuns` must be a whole number of at least {MIN_CORROBORATING_RUNS}"
            ))
        })?;
    let measured_from = match field(object, "measuredFrom").map_err(fail)? {
        Value::Null => None,
        Value::Object(from) => {
            exact_keys(from, &["tree", "tool", "runs"], "`measuredFrom`").map_err(fail)?;
            let runs = field(from, "runs")
                .ok()
                .and_then(Value::as_array)
                .and_then(|runs| {
                    runs.iter()
                        .map(|r| r.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| fail("`measuredFrom.runs` must be a list of strings".into()))?;
            Some(Provenance {
                tree: string(from, "tree").map_err(fail)?,
                tool: string(from, "tool").map_err(fail)?,
                runs,
            })
        }
        _ => return Err(fail("`measuredFrom` must be null or an object".into())),
    };
    let mut crates = BTreeMap::new();
    for (name, row) in
        as_object(field(object, "crates").map_err(fail)?, "`crates`").map_err(fail)?
    {
        let what = format!("`crates.{name}`");
        let row = as_object(row, &what).map_err(fail)?;
        exact_keys(row, &["floor", "platforms"], &what).map_err(fail)?;
        let floor =
            points(field(row, "floor").map_err(fail)?, &format!("{what}.floor")).map_err(fail)?;
        let listed = as_object(
            field(row, "platforms").map_err(fail)?,
            &format!("{what}.platforms"),
        )
        .map_err(fail)?;
        exact_keys(listed, &PLATFORMS, &format!("{what}.platforms")).map_err(fail)?;
        let mut platforms = BTreeMap::new();
        for (os, figure) in listed {
            platforms.insert(
                os.clone(),
                points(figure, &format!("{what}.platforms.{os}")).map_err(fail)?,
            );
        }
        // A ratchet writes floor = max(old floor, the runs' lowest), so a floor
        // below its own lowest platform figure was lowered by hand (review
        // finding F14).
        if platforms
            .values()
            .min()
            .is_some_and(|lowest| floor < *lowest)
        {
            return Err(fail(format!(
                "{what}.floor is below its lowest platform figure, which no ratchet writes: a floor is never lowered by hand"
            )));
        }
        crates.insert(name.clone(), CrateFloor { floor, platforms });
    }
    let tolerance =
        points(field(object, "tolerance").map_err(fail)?, "`tolerance`").map_err(fail)?;
    if tolerance > MAX_TOLERANCE {
        return Err(fail(format!(
            "`tolerance` may be at most {} points; a wider band lowers every floor at once without touching a row",
            percent(MAX_TOLERANCE)
        )));
    }
    if !crates.is_empty() {
        let runs = measured_from
            .as_ref()
            .map(|from| from.runs.as_slice())
            .unwrap_or_default();
        for os in PLATFORMS {
            let count = runs
                .iter()
                .filter(|run| run.split(' ').next() == Some(os))
                .count() as u64;
            if count < corroborating_runs {
                return Err(fail(format!(
                    "`measuredFrom` records {count} {os} run(s) and the rows need {corroborating_runs}: floors are written only by `cargo xtask coverage ratchet` from CI artifacts"
                )));
            }
        }
    }
    Ok(Floors {
        about,
        tolerance,
        tolerance_reason: string(object, "toleranceReason").map_err(fail)?,
        ratchet_lag: points(field(object, "ratchetLag").map_err(fail)?, "`ratchetLag`")
            .map_err(fail)?,
        ratchet_lag_reason: string(object, "ratchetLagReason").map_err(fail)?,
        corroborating_runs,
        corroborating_runs_reason: string(object, "corroboratingRunsReason").map_err(fail)?,
        measured_from,
        crates,
    })
}

/// Renders the floors file: fixed key order, two-space indent, figures with two
/// decimals, and a trailing newline. [`parse_floors`] reads it back unchanged.
pub fn render_floors(floors: &Floors) -> String {
    let text = |value: &str| Value::String(value.to_string()).to_string();
    let list = |values: &[String], indent: &str| {
        if values.is_empty() {
            return "[]".to_string();
        }
        let items: Vec<String> = values
            .iter()
            .map(|v| format!("{indent}  {}", text(v)))
            .collect();
        format!("[\n{}\n{indent}]", items.join(",\n"))
    };
    let measured_from = floors.measured_from.as_ref().map_or_else(
        || "null".to_string(),
        |from| {
            format!(
                "{{\n    \"tree\": {},\n    \"tool\": {},\n    \"runs\": {}\n  }}",
                text(&from.tree),
                text(&from.tool),
                list(&from.runs, "    ")
            )
        },
    );
    let rows: Vec<String> = floors
        .crates
        .iter()
        .map(|(name, row)| {
            let platforms: Vec<String> = row
                .platforms
                .iter()
                .map(|(os, figure)| format!("{}: {}", text(os), percent(*figure)))
                .collect();
            format!(
                "    {}: {{\n      \"floor\": {},\n      \"platforms\": {{ {} }}\n    }}",
                text(name),
                percent(row.floor),
                platforms.join(", ")
            )
        })
        .collect();
    let crates = if rows.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n  }}", rows.join(",\n"))
    };
    format!(
        "{{\n  \"schemaVersion\": 1,\n  \"about\": {},\n  \"tolerance\": {},\n  \"toleranceReason\": {},\n  \"ratchetLag\": {},\n  \"ratchetLagReason\": {},\n  \"corroboratingRuns\": {},\n  \"corroboratingRunsReason\": {},\n  \"measuredFrom\": {},\n  \"crates\": {}\n}}\n",
        list(&floors.about, "  "),
        percent(floors.tolerance),
        text(&floors.tolerance_reason),
        percent(floors.ratchet_lag),
        text(&floors.ratchet_lag_reason),
        floors.corroborating_runs,
        text(&floors.corroborating_runs_reason),
        measured_from,
        crates
    )
}

fn as_object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{what} must be a JSON object"))
}

fn field<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Value, String> {
    object.get(key).ok_or_else(|| format!("`{key}` is missing"))
}

fn string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    field(object, key)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn exact_keys(object: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    if let Some(extra) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "{what} has `{extra}`, which is not one of: {}",
            allowed.join(", ")
        ));
    }
    if let Some(missing) = allowed.iter().find(|key| !object.contains_key(**key)) {
        return Err(format!("{what} has no `{missing}`"));
    }
    Ok(())
}

fn schema_version(object: &Map<String, Value>) -> Result<(), String> {
    match object.get("schemaVersion").and_then(Value::as_u64) {
        Some(1) => Ok(()),
        _ => Err("`schemaVersion` must be 1, the only version this build reads".into()),
    }
}

/// A percentage from 0 to 100 with at most two decimals, in hundredths.
fn points(value: &Value, what: &str) -> Result<u64, String> {
    let bad = || {
        format!("{what} must be a percentage from 0 to 100 with at most two decimals, like 93.73")
    };
    let number = value.as_f64().ok_or_else(bad)?;
    let rounded = (number * 100.0).round();
    // A number has at most two decimals exactly when its hundredths, divided
    // back, are the same double: both sides are the nearest double to one real
    // number. An epsilon test had a boundary no input could reach (xtask
    // mutation round 1, 2026-10-20).
    if !(0.0..=10_000.0).contains(&rounded) || rounded / 100.0 != number {
        return Err(bad());
    }
    // A whole number from 0 to 10000, checked above, so the cast is exact.
    Ok(rounded as u64)
}

/// Checks one platform's measurement against the floors (rules 2 to 5). Returns
/// a row per crate and the failures, each naming its crate or file.
pub fn check(floors: &Floors, measured: &Measurement) -> (Vec<String>, Vec<String>) {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    if !PLATFORMS.contains(&measured.os.as_str()) {
        failures.push(format!(
            "this platform is `{}`, and floors are recorded for {} only",
            measured.os,
            PLATFORMS.join(", ")
        ));
    }
    if let Some(from) = &floors.measured_from
        && from.tool != measured.tool
    {
        failures.push(format!(
            "measured with `{}`, but the floors were measured with `{}`, and a figure is compared only under the tool it was taken with. Install the approved version (docs/RUST-CRATES.md): {INSTALL}",
            measured.tool, from.tool
        ));
    }
    for file in &measured.untested {
        failures.push(format!(
            "{file}: no test runs any of its production lines (or the report never names it: a file no `mod` declares, one compiled only on another platform, or generics nothing instantiates). Test it, or delete it if nothing needs it"
        ));
    }
    for (name, lines) in &measured.crates {
        let Some(figure) = lines.basis_points() else {
            failures.push(format!(
                "{name}: measured no production line, and a check over nothing proves nothing; a crate with no executable code holds no floor"
            ));
            continue;
        };
        let measured_text = format!(
            "{}% ({} of {} lines)",
            percent(figure),
            lines.covered,
            lines.count
        );
        let Some(row) = floors.crates.get(name) else {
            rows.push(format!("{name}: {measured_text}, no floor"));
            failures.push(format!(
                "{name}: has no floor yet. Floors are seeded only from CI, {} runs per platform of one tree: push, then {HOW_TO_RATCHET}",
                floors.corroborating_runs
            ));
            continue;
        };
        rows.push(format!(
            "{name}: {measured_text}, floor {}%",
            percent(row.floor)
        ));
        if figure + floors.tolerance < row.floor {
            failures.push(format!(
                "{name}: line coverage {}% is below its floor of {}% (tolerance {}). A floor is never lowered: test the lines this change left unrun",
                percent(figure),
                percent(row.floor),
                percent(floors.tolerance)
            ));
        }
        if let Some(recorded) = row.platforms.get(&measured.os)
            && figure > recorded + floors.ratchet_lag
        {
            failures.push(format!(
                "{name}: line coverage {}% is more than {} points above the {}% recorded for {}, so the record has fallen behind. Ratchet it from CI artifacts (`cargo xtask coverage ratchet <dir>`)",
                percent(figure),
                percent(floors.ratchet_lag),
                percent(*recorded),
                measured.os
            ));
        }
    }
    for name in floors.crates.keys() {
        if !measured.crates.contains_key(name) {
            failures.push(format!(
                "{name}: has a floor in {FLOORS_FILE} but is no crate of this workspace. Ratchet from CI artifacts to drop the row"
            ));
        }
    }
    (rows, failures)
}

/// The floors a set of CI measurements justify (rule 6). `measurements` pairs
/// each with the file it was read from, for messages. `tree` is this tree's
/// hash and `crates` its members. Returns the new floors and a note per change,
/// or every reason the runs cannot raise anything.
pub fn ratchet(
    floors: &Floors,
    measurements: &[(String, Measurement)],
    tree: &str,
    crates: &BTreeSet<String>,
) -> Result<(Floors, Vec<String>), Vec<String>> {
    let mut errors = Vec::new();
    let mut runs: BTreeMap<(String, String, String), &Measurement> = BTreeMap::new();
    for (name, measurement) in measurements {
        let Some(run) = &measurement.run else {
            errors.push(format!(
                "{name}: was measured outside CI, and floors rise only from CI artifacts"
            ));
            continue;
        };
        if measurement.tree != tree {
            errors.push(format!(
                "{name}: measured tree {}, but this tree is {tree}; download the artifacts of a run of these sources, or remove the .rs files git does not track under the crates (`git status --short --ignored -- crates xtask`)",
                measurement.tree
            ));
            continue;
        }
        if !PLATFORMS.contains(&measurement.os.as_str()) {
            errors.push(format!(
                "{name}: platform `{}` holds no floor",
                measurement.os
            ));
            continue;
        }
        let measured: BTreeSet<&String> = measurement.crates.keys().collect();
        if measured != crates.iter().collect() {
            errors.push(format!(
                "{name}: measured the crates {measured:?}, but this workspace's are {crates:?}"
            ));
            continue;
        }
        if !measurement.untested.is_empty() {
            errors.push(format!(
                "{name}: no test enters {}; an untested file is not ratcheted in",
                measurement.untested.join(", ")
            ));
            continue;
        }
        let key = (measurement.os.clone(), run.id.clone(), run.attempt.clone());
        match runs.get(&key) {
            Some(seen) if *seen != measurement => errors.push(format!(
                "{name}: run {}/{} on {} appears twice with different figures",
                run.id, run.attempt, measurement.os
            )),
            Some(_) => {}
            None => {
                runs.insert(key, measurement);
            }
        }
    }
    let tools: BTreeSet<&str> = runs.values().map(|m| m.tool.as_str()).collect();
    if tools.len() > 1 {
        errors.push(format!(
            "the runs measured with different tools ({tools:?})"
        ));
    }
    for os in PLATFORMS {
        let count = runs
            .keys()
            .filter(|(platform, _, _)| platform == os)
            .count() as u64;
        if count < floors.corroborating_runs {
            errors.push(format!(
                "{os}: {count} of the {} CI runs a raise needs; {HOW_TO_RATCHET}",
                floors.corroborating_runs
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut next = floors.clone();
    next.crates = BTreeMap::new();
    let mut notes = Vec::new();
    for name in crates {
        let mut platforms = BTreeMap::new();
        for os in PLATFORMS {
            let lowest = runs
                .iter()
                .filter(|((platform, _, _), _)| platform == os)
                .filter_map(|(_, m)| m.crates.get(name).and_then(|lines| lines.basis_points()))
                .min();
            match lowest {
                Some(lowest) => {
                    platforms.insert(os.to_string(), lowest);
                }
                None => errors.push(format!(
                    "{name}: no {os} run measured a production line of it"
                )),
            }
        }
        let Some(lowest) = platforms
            .values()
            .copied()
            .min()
            .filter(|_| platforms.len() == PLATFORMS.len())
        else {
            continue;
        };
        let floor = match floors.crates.get(name) {
            Some(old) if lowest + floors.tolerance < old.floor => {
                errors.push(format!(
                    "{name}: the runs' lowest is {}%, below its floor of {}%. A regression is not ratcheted, and a floor is never lowered",
                    percent(lowest),
                    percent(old.floor)
                ));
                continue;
            }
            Some(old) if old.floor >= lowest => old.floor,
            Some(old) => {
                notes.push(format!(
                    "{name}: floor {}% -> {}%",
                    percent(old.floor),
                    percent(lowest)
                ));
                lowest
            }
            None => {
                notes.push(format!("{name}: seeded at {}%", percent(lowest)));
                lowest
            }
        };
        next.crates
            .insert(name.clone(), CrateFloor { floor, platforms });
    }
    for name in floors.crates.keys() {
        if !crates.contains(name) {
            notes.push(format!(
                "{name}: row dropped; it is no crate of this workspace"
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    next.measured_from = Some(Provenance {
        tree: tree.to_string(),
        tool: tools.into_iter().next().unwrap_or_default().to_string(),
        runs: runs
            .keys()
            .map(|(os, id, attempt)| format!("{os} {id}/{attempt}"))
            .collect(),
    });
    Ok((next, notes))
}

/// What the commands touch outside their logic. Abstracted so the order of
/// steps and every refusal are testable without running cargo-llvm-cov.
pub trait Harness {
    /// `cargo llvm-cov --version`'s first line, or `None` when it is not
    /// installed.
    fn tool(&mut self) -> Option<String>;
    /// The workspace's target directory.
    fn target_dir(&mut self, root: &Path) -> Result<PathBuf, String>;
    /// Creates the report's directory and deletes a previous report.
    fn prepare(&mut self, report: &Path) -> Result<(), String>;
    /// Runs `cargo llvm-cov` over the workspace at `root`, writing its lcov
    /// report to `report`, with its output shown. Returns the exit code, or
    /// `None` if it did not finish with one.
    fn llvm_cov(&mut self, root: &Path, report: &Path) -> Option<i32>;
    /// The workspace.
    fn workspace(&mut self, root: &Path) -> Result<Workspace, String>;
    /// A file's text.
    fn read(&mut self, path: &Path) -> Result<String, String>;
    /// When a file was last modified, if it exists.
    fn modified(&mut self, path: &Path) -> Option<SystemTime>;
    /// Every `.json` file under `dir`, at any depth, with its text.
    fn json_files(&mut self, dir: &Path) -> Result<Vec<(PathBuf, String)>, String>;
    /// An environment variable, when it is set and not empty.
    fn env(&mut self, name: &str) -> Option<String>;
}

/// The real harness: cargo, the file system, `cargo metadata`, the environment.
pub struct Real {
    /// The cargo to run: the one that launched xtask (`CARGO`), or `cargo`.
    pub cargo: OsString,
}

impl Real {
    /// The harness over the cargo that launched xtask.
    pub fn from_env() -> Real {
        Real {
            cargo: std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
        }
    }
}

impl Harness for Real {
    #[expect(
        clippy::disallowed_methods,
        reason = "process: RUST-PLAN §3.2 allowlists xtask; the coverage step asks cargo-llvm-cov its version"
    )]
    fn tool(&mut self) -> Option<String> {
        let out = Command::new(&self.cargo)
            .args(["llvm-cov", "--version"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // The version line as the tool prints it; a tool that prints nothing is
        // still the tool, and its empty name is compared like any other.
        let text = String::from_utf8_lossy(&out.stdout);
        Some(text.lines().next().unwrap_or_default().trim().to_string())
    }

    fn target_dir(&mut self, root: &Path) -> Result<PathBuf, String> {
        cargo_metadata::MetadataCommand::new()
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .other_options(vec!["--locked".to_string()])
            .exec()
            .map(|metadata| metadata.target_directory.into_std_path_buf())
            .map_err(|error| format!("cargo metadata failed: {error}"))
    }

    fn prepare(&mut self, report: &Path) -> Result<(), String> {
        if let Some(dir) = report.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        }
        match std::fs::remove_file(report) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(format!(
                "cannot delete the previous report {}: {error}",
                report.display()
            )),
            _ => Ok(()),
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "process: RUST-PLAN §3.2 allowlists xtask; the coverage step runs cargo-llvm-cov"
    )]
    fn llvm_cov(&mut self, root: &Path, report: &Path) -> Option<i32> {
        Command::new(&self.cargo)
            .args([
                "llvm-cov",
                "--workspace",
                "--locked",
                "--lcov",
                "--output-path",
            ])
            .arg(report)
            .current_dir(root)
            .status()
            .ok()?
            .code()
    }

    fn workspace(&mut self, root: &Path) -> Result<Workspace, String> {
        workspace::load(root).map_err(|error| error.to_string())
    }

    fn read(&mut self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
    }

    fn modified(&mut self, path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    }

    fn json_files(&mut self, dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(dir)
            .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
                .path();
            if path.is_dir() {
                out.extend(self.json_files(&path)?);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let text = self.read(&path)?;
                out.push((path, text));
            }
        }
        out.sort();
        Ok(out)
    }

    fn env(&mut self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
}

/// The `(path, text)` pairs [`tree_hash`] hashes: every member's manifest and
/// sources, and the [`TREE_FILES`] that exist.
fn tree_files(
    harness: &mut dyn Harness,
    root: &Path,
    workspace: &Workspace,
) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for member in &workspace.members {
        files.push((
            relative(root, &member.manifest_path),
            member.manifest.clone(),
        ));
        for source in &member.sources {
            files.push((relative(root, &source.path), source.text.clone()));
        }
    }
    for name in TREE_FILES {
        if let Ok(text) = harness.read(&root.join(name)) {
            files.push((name.to_string(), text));
        }
    }
    files
}

/// Reads the report a run left and measures it, with the condition it ran in.
/// `fresh` refuses a report older than any member source: the target directory
/// is cached in CI, so a report can outlive the tree it measured.
fn measure(
    harness: &mut dyn Harness,
    root: &Path,
    report: &Path,
    os: &str,
    tool: String,
    fresh: bool,
) -> Result<(Measurement, usize), Vec<String>> {
    let workspace = harness
        .workspace(root)
        .map_err(|error| vec![format!("cannot read the workspace: {error}")])?;
    if fresh {
        let Some(made) = harness.modified(report) else {
            return Err(vec![format!(
                "there is no report at {}; run `cargo xtask coverage` first",
                report.display()
            )]);
        };
        let newer = workspace
            .members
            .iter()
            .flat_map(|member| member.sources.iter().map(|source| source.path.clone()))
            .find(|path| harness.modified(path).is_some_and(|time| time > made));
        if let Some(path) = newer {
            return Err(vec![format!(
                "the report at {} is older than {}, so it measured another tree; run `cargo xtask coverage` again",
                report.display(),
                relative(root, &path)
            )]);
        }
    }
    let text = harness.read(report).map_err(|error| vec![error])?;
    let files = parse_lcov(&text).map_err(|error| vec![error])?;
    let tally = tally(root, &workspace, &files)?;
    let run = match (
        harness.env("GITHUB_RUN_ID"),
        harness.env("GITHUB_RUN_ATTEMPT"),
        harness.env("GITHUB_SHA"),
    ) {
        (Some(id), Some(attempt), Some(sha)) => Some(CiRun { id, attempt, sha }),
        _ => None,
    };
    let tree = tree_hash(&tree_files(harness, root, &workspace));
    Ok((
        Measurement {
            os: os.to_string(),
            tree,
            tool,
            run,
            crates: tally.crates,
            untested: tally.untested,
        },
        tally.files,
    ))
}

fn report_path(harness: &mut dyn Harness, root: &Path) -> Result<PathBuf, String> {
    harness.target_dir(root).map(|dir| dir.join(REPORT))
}

fn not_installed() -> String {
    format!(
        "cargo-llvm-cov is not installed. Install the approved version (docs/RUST-CRATES.md):\n    {INSTALL}\nand the toolchain's llvm-tools (rust-toolchain.toml lists it; `rustup toolchain install` adds it)"
    )
}

/// `cargo xtask coverage`: runs the measurement on this platform and checks it
/// against the floors. Returns the exit code (0 green, 1 red) and the report.
pub fn run(harness: &mut dyn Harness, root: &Path, os: &str) -> (u8, String) {
    let mut report = String::new();
    let mut say = |line: &str| {
        report.push_str("coverage: ");
        report.push_str(line);
        report.push('\n');
    };
    let Some(tool) = harness.tool() else {
        say(&not_installed());
        return (1, report);
    };
    let path = match report_path(harness, root) {
        Ok(path) => path,
        Err(error) => {
            say(&error);
            return (1, report);
        }
    };
    if let Err(error) = harness.prepare(&path) {
        say(&error);
        return (1, report);
    }
    match harness.llvm_cov(root, &path) {
        Some(0) => {}
        code => {
            say(&format!(
                "cargo llvm-cov exited {}; its output is above",
                code.map_or_else(
                    || "without an exit code".to_string(),
                    |code| code.to_string()
                )
            ));
            return (1, report);
        }
    }
    let (measured, files) = match measure(harness, root, &path, os, tool, false) {
        Ok(found) => found,
        Err(errors) => {
            for error in errors {
                say(&error);
            }
            return (1, report);
        }
    };
    let floors = match harness
        .read(&root.join(FLOORS_FILE))
        .and_then(|text| parse_floors(&text))
    {
        Ok(floors) => floors,
        Err(error) => {
            say(&error);
            return (1, report);
        }
    };
    let (rows, mut failures) = check(&floors, &measured);
    if crate::read_nothing(measured.crates.len(), files) {
        failures.push(format!(
            "read {} crate(s) and {files} production file(s); a check over nothing proves nothing",
            measured.crates.len()
        ));
    }
    for row in &rows {
        say(row);
    }
    for failure in &failures {
        say(failure);
    }
    if failures.is_empty() {
        say(&format!(
            "ok ({} crate(s), {files} production file(s), {})",
            measured.crates.len(),
            measured.os
        ));
        (0, report)
    } else {
        say(&format!("{} failure(s)", failures.len()));
        (1, report)
    }
}

/// `cargo xtask coverage emit`: the measurement of the report the last run left,
/// as the JSON CI uploads.
pub fn emit(harness: &mut dyn Harness, root: &Path, os: &str) -> Result<String, Vec<String>> {
    let tool = harness.tool().ok_or_else(|| vec![not_installed()])?;
    let path = report_path(harness, root).map_err(|error| vec![error])?;
    let (measured, _) = measure(harness, root, &path, os, tool, true)?;
    Ok(render_measurement(&measured))
}

/// `cargo xtask coverage ratchet <dir>`: the floors file the measurements under
/// `dir` justify, and a note per change.
pub fn ratchet_dir(
    harness: &mut dyn Harness,
    root: &Path,
    dir: &Path,
) -> Result<(String, Vec<String>), Vec<String>> {
    let found = harness.json_files(dir).map_err(|error| vec![error])?;
    let mut measurements = Vec::new();
    let mut errors = Vec::new();
    for (path, text) in found {
        let name = relative(dir, &path);
        match parse_measurement(&text) {
            Ok(measurement) => measurements.push((name, measurement)),
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let workspace = harness
        .workspace(root)
        .map_err(|error| vec![format!("cannot read the workspace: {error}")])?;
    let crates: BTreeSet<String> = workspace
        .members
        .iter()
        .map(|member| member.name.clone())
        .collect();
    let tree = tree_hash(&tree_files(harness, root, &workspace));
    let floors = harness
        .read(&root.join(FLOORS_FILE))
        .and_then(|text| parse_floors(&text))
        .map_err(|error| vec![error])?;
    let (next, notes) = ratchet(&floors, &measurements, &tree, &crates)?;
    Ok((render_floors(&next), notes))
}

/// What `cargo xtask coverage` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// Measure this platform and check the floors.
    Run,
    /// Print the last measurement for CI to upload.
    Emit,
    /// Print the floors the measurements under the directory justify.
    Ratchet(PathBuf),
}

/// Reads the command's arguments, or `None` for anything else, which is a
/// usage error. Pure, so every form is tested without running cargo-llvm-cov
/// (xtask mutation round 1 found `main`'s arms untested, 2026-10-20).
pub fn parse_args(args: &[String]) -> Option<Verb> {
    match args {
        [] => Some(Verb::Run),
        [verb] if verb == "emit" => Some(Verb::Emit),
        [verb, dir] if verb == "ratchet" => Some(Verb::Ratchet(PathBuf::from(dir))),
        _ => None,
    }
}

/// `cargo xtask coverage [emit | ratchet <dir>]`. Exit 0: green. 1: red. 2: usage.
pub fn main(args: &[String]) -> u8 {
    let Some(verb) = parse_args(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let root = workspace_root();
    let os = std::env::consts::OS;
    let mut harness = Real::from_env();
    match verb {
        Verb::Run => {
            let (code, report) = run(&mut harness, &root, os);
            print!("{report}");
            code
        }
        Verb::Emit => match emit(&mut harness, &root, os) {
            Ok(json) => {
                print!("{json}");
                0
            }
            Err(errors) => {
                for error in errors {
                    eprintln!("coverage emit: {error}");
                }
                1
            }
        },
        Verb::Ratchet(dir) => match ratchet_dir(&mut harness, &root, &dir) {
            Ok((floors, notes)) => {
                for note in notes {
                    eprintln!("coverage ratchet: {note}");
                }
                print!("{floors}");
                0
            }
            Err(errors) => {
                for error in errors {
                    eprintln!("coverage ratchet: {error}");
                }
                1
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Member, Source};

    /// Lines 1-6 are production (1-3 run, 4-6 do not); 7-11 are its tests.
    const LIB: &str = "\
pub fn used() -> u8 {
    1
}
pub fn unused() -> u8 {
    2
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() { assert_eq!(super::used(), 1); }
}
";

    const TREE: &str = "fnv1a64:0000000000000001";

    fn root() -> PathBuf {
        PathBuf::from("/w")
    }

    fn member(name: &str, files: &[(&str, &str)]) -> Member {
        Member {
            name: name.to_string(),
            manifest_path: root().join(format!("crates/{name}/Cargo.toml")),
            manifest: format!("[package]\nname = \"{name}\"\n"),
            deps: Vec::new(),
            sources: files
                .iter()
                .map(|(path, text)| Source {
                    path: root().join(format!("crates/{name}/{path}")),
                    text: (*text).to_string(),
                })
                .collect(),
        }
    }

    fn workspace() -> Workspace {
        Workspace {
            members: vec![
                member(
                    "a",
                    &[
                        ("src/lib.rs", LIB),
                        ("src/dead.rs", "pub fn d() {}\n"),
                        ("tests/it.rs", "fn x() {}\n"),
                    ],
                ),
                member("b", &[("src/lib.rs", "pub fn b() {}\n")]),
            ],
        }
    }

    fn lcov(files: &[(&str, &[(usize, u64)])]) -> String {
        let mut out = String::from("TN:\n");
        for (path, hits) in files {
            out.push_str(&format!("SF:/w/crates/{path}\nFN:1,f\n"));
            for (line, count) in *hits {
                out.push_str(&format!("DA:{line},{count}\n"));
            }
            out.push_str("LF:9\nLH:9\nend_of_record\n");
        }
        out
    }

    /// The report a run over [`workspace`] leaves: `a` at 4 of 7 production
    /// lines (3 of 6 in lib.rs, 1 of 1 in dead.rs), `b` at 1 of 1.
    fn good_lcov() -> String {
        lcov(&[
            (
                "a/src/lib.rs",
                &[(1, 1), (2, 1), (3, 1), (4, 0), (5, 0), (6, 0), (10, 4)],
            ),
            ("a/src/dead.rs", &[(1, 1)]),
            ("a/tests/it.rs", &[(1, 0)]),
            ("b/src/lib.rs", &[(1, 2)]),
        ])
    }

    /// The rules every test runs under, stated here and not read from the
    /// committed file. Its rows change at every ratchet, and a test that leaned
    /// on them broke the moment a floor was planted. Reading it at all made an
    /// invalid committed file fail every test here instead of the one test whose
    /// job that is (both found by the refutation passes, 2026-10-20).
    fn base() -> Floors {
        Floors {
            about: vec!["the test floors".to_string()],
            tolerance: 0,
            tolerance_reason: "measured".to_string(),
            ratchet_lag: 500,
            ratchet_lag_reason: "carried".to_string(),
            corroborating_runs: 3,
            corroborating_runs_reason: "carried".to_string(),
            measured_from: None,
            crates: BTreeMap::new(),
        }
    }

    /// Three CI runs per platform, as a ratchet records them.
    fn nine_run_names() -> Vec<String> {
        PLATFORMS
            .iter()
            .flat_map(|os| (1..=3).map(move |attempt| format!("{os} 77/{attempt}")))
            .collect()
    }

    /// [`base`] with these rows: (crate, floor, [linux, macos, windows]), and
    /// the provenance a ratchet writes beside them.
    fn with_rows(rows: &[(&str, u64, [u64; 3])]) -> Floors {
        let mut floors = base();
        floors.measured_from = Some(Provenance {
            tree: TREE.to_string(),
            tool: "cargo-llvm-cov 0.9.1".to_string(),
            runs: nine_run_names(),
        });
        for (name, floor, figures) in rows {
            let platforms = PLATFORMS
                .iter()
                .map(|os| os.to_string())
                .zip(figures.iter().copied())
                .collect();
            floors.crates.insert(
                name.to_string(),
                CrateFloor {
                    floor: *floor,
                    platforms,
                },
            );
        }
        floors
    }

    fn measured(os: &str, run: Option<(&str, &str)>, crates: &[(&str, u64, u64)]) -> Measurement {
        Measurement {
            os: os.to_string(),
            tree: TREE.to_string(),
            tool: "cargo-llvm-cov 0.9.1".to_string(),
            run: run.map(|(id, attempt)| CiRun {
                id: id.into(),
                attempt: attempt.into(),
                sha: "abc".into(),
            }),
            crates: crates
                .iter()
                .map(|(name, covered, count)| {
                    (
                        name.to_string(),
                        Lines {
                            covered: *covered,
                            count: *count,
                        },
                    )
                })
                .collect(),
            untested: Vec::new(),
        }
    }

    #[test]
    fn a_figure_rounds_down_and_prints_two_decimals() {
        let lines = |covered, count| Lines { covered, count };
        assert_eq!(lines(941, 1004).basis_points(), Some(9372));
        assert_eq!(lines(1, 1).basis_points(), Some(10_000));
        assert_eq!(lines(0, 3).basis_points(), Some(0));
        assert_eq!(lines(0, 0).basis_points(), None);
        assert_eq!(percent(9372), "93.72");
        assert_eq!(percent(5), "0.05");
        assert_eq!(percent(10_000), "100.00");
        let mut sum = lines(1, 2);
        sum.add(lines(3, 4));
        assert_eq!(sum, lines(4, 6));
    }

    #[test]
    fn lcov_is_read_for_its_files_and_line_hits_only() {
        let text = "TN:\nSF:/x/a.rs\nFN:1,f\nFNDA:3,f\nDA:1,3\nDA:2,0\nDA:2,5\nDA:3,7,abc\nBRDA:1,0,0,1\nLF:2\nLH:1\nend_of_record\nSF:/x/b.rs\nend_of_record\n";
        let files = parse_lcov(text).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("/x/a.rs"));
        assert_eq!(
            files[0].hits,
            BTreeMap::from([(1, 3), (2, 5), (3, 7)]),
            "a line named twice keeps the larger count; a checksum is ignored"
        );
        assert!(files[1].hits.is_empty());
        assert_eq!(parse_lcov("").unwrap(), Vec::new());
    }

    #[test]
    fn a_malformed_lcov_report_is_refused_naming_its_line() {
        for (text, needle) in [
            ("DA:1,1\n", "line 1: line data outside a file"),
            (
                "SF:a\nDA:x,1\nend_of_record\n",
                "line 2: `DA:x,1` is not <line>,<hits>",
            ),
            ("SF:a\nDA:1\nend_of_record\n", "`DA:1` is not"),
            ("SF:a\nDA:1,-1\nend_of_record\n", "`DA:1,-1` is not"),
            (
                "SF:a\nSF:b\n",
                "line 2: a file starts before the last one ended",
            ),
            ("end_of_record\n", "line 1: end_of_record outside a file"),
            ("SF:a\nDA:1,1\n", "cut short"),
        ] {
            let error = parse_lcov(text).unwrap_err();
            assert!(error.contains(needle), "{text:?}: {error}");
        }
    }

    #[test]
    fn items_compiled_only_for_tests_are_found_in_the_syntax_tree() {
        assert_eq!(test_only_lines(LIB).unwrap(), (7..=11).collect());
        let source = "\
pub struct S;
impl S {
    pub fn real(&self) {}
    #[cfg(test)]
    fn helper(&self) {}
}
pub trait T {
    #[cfg(test)]
    fn probe(&self) {}
}
mod inner {
    #[cfg(all(unix, test))]
    pub fn only_tests_on_unix() {}
    #[cfg(any(test, feature = \"x\"))]
    pub fn ships_with_x() {}
    #[cfg(feature = \"y\")]
    pub fn ships() {}
}
#[cfg(unix)]
pub fn ships_on_unix() {}
#[cfg(all(unix, windows))]
pub fn never() {}
#[cfg_attr(test, allow(dead_code))]
pub fn ships_too() {}
";
        assert_eq!(
            test_only_lines(source).unwrap(),
            BTreeSet::from([4, 5, 8, 9, 12, 13])
        );
    }

    #[test]
    fn every_kind_of_item_can_be_test_only() {
        let kinds = [
            "const C: u8 = 1;",
            "enum E { A }",
            "extern crate core;",
            "fn f() {}",
            "extern \"C\" {}",
            "impl S {}",
            "m!();",
            "mod m {}",
            "static S: u8 = 1;",
            "struct S;",
            "trait T {}",
            "trait A = Clone;",
            "type A = u8;",
            "union U { a: u8 }",
            "use std::fmt;",
        ];
        for item in kinds {
            let source = format!("#[cfg(test)]\n{item}\n");
            assert_eq!(
                test_only_lines(&source).unwrap(),
                BTreeSet::from([1, 2]),
                "{item}"
            );
        }
        let members = "impl S {\n#[cfg(test)]\nconst C: u8 = 1;\n#[cfg(test)]\ntype A = u8;\n#[cfg(test)]\nm!();\n}\n\
                       trait T {\n#[cfg(test)]\nconst C: u8;\n#[cfg(test)]\ntype A;\n#[cfg(test)]\nm!();\n}\n";
        assert_eq!(
            test_only_lines(members).unwrap(),
            BTreeSet::from([2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15])
        );
        // An attribute on another item with the same path is not a cfg.
        // A test attribute compiles only under `--test` and would cover itself
        // (review finding F12), whatever crate it comes from.
        for attribute in ["#[test]", "#[tokio::test]"] {
            assert_eq!(
                test_only_lines(&format!("{attribute}\nfn t() {{}}\n")).unwrap(),
                BTreeSet::from([1, 2]),
                "{attribute}"
            );
        }
        assert_eq!(
            test_only_lines("#[inline]\nfn t() {}\n").unwrap(),
            BTreeSet::new()
        );
        assert_eq!(
            test_only_lines("#[cfg = \"test\"]\nfn t() {}\n").unwrap(),
            BTreeSet::new()
        );
    }

    #[test]
    fn an_out_of_line_test_module_is_refused_and_a_broken_file_names_its_line() {
        let error = test_only_lines("pub fn a() {}\n#[cfg(test)]\nmod tests;\n").unwrap_err();
        assert!(
            error.contains("line 2: `#[cfg(test)] mod tests;`"),
            "{error}"
        );
        assert!(error.contains("write the module inline"), "{error}");
        let nested = test_only_lines("mod outer {\n#[cfg(test)]\nmod tests;\n}\n").unwrap_err();
        assert!(nested.starts_with("line 2:"), "{nested}");
        assert_eq!(test_only_lines("mod other;\n").unwrap(), BTreeSet::new());
        assert!(
            test_only_lines("\nfn x( {\n")
                .unwrap_err()
                .starts_with("line 2: ")
        );
    }

    #[test]
    fn only_a_crates_src_is_production() {
        let dir = Path::new("/w/crates/a");
        assert!(is_production(dir, &dir.join("src/lib.rs")));
        assert!(is_production(
            dir,
            &dir.join("src").join("deep").join("x.rs")
        ));
        for other in ["tests/it.rs", "benches/b.rs", "examples/e.rs", "build.rs"] {
            assert!(!is_production(dir, &dir.join(other)), "{other}");
        }
        assert!(!is_production(dir, Path::new("/elsewhere/src/lib.rs")));
    }

    #[test]
    fn a_tally_counts_production_lines_and_names_an_untested_file() {
        let report = parse_lcov(&lcov(&[
            (
                "a/src/lib.rs",
                &[(1, 1), (2, 1), (3, 1), (4, 0), (5, 0), (6, 0), (10, 4)],
            ),
            ("a/src/dead.rs", &[(1, 0)]),
            ("a/tests/it.rs", &[(1, 1)]),
        ]))
        .unwrap();
        let counted = tally(&root(), &workspace(), &report).unwrap();
        assert_eq!(
            counted.crates["a"],
            Lines {
                covered: 3,
                count: 7
            },
            "line 10 is a test's, and tests/it.rs is not production"
        );
        assert_eq!(
            counted.crates["b"],
            Lines::default(),
            "every member has a row, measured or not"
        );
        assert_eq!(
            counted.untested,
            ["crates/a/src/dead.rs", "crates/b/src/lib.rs"],
            "b's lib.rs holds a body and the report never names it"
        );
        assert_eq!(counted.files, 2);
        let empty = parse_lcov(&lcov(&[
            ("a/src/lib.rs", &[(1, 1)]),
            ("a/src/dead.rs", &[(1, 1)]),
            ("b/src/lib.rs", &[]),
        ]))
        .unwrap();
        assert_eq!(
            tally(&root(), &workspace(), &empty).unwrap().untested,
            Vec::<String>::new(),
            "a reported file with no executable line is not untested"
        );
        let mut only_mods = workspace();
        only_mods.members[1].sources[0].text = "pub mod x;\npub use x::*;\n".to_string();
        let without_b = parse_lcov(&lcov(&[
            ("a/src/lib.rs", &[(1, 1)]),
            ("a/src/dead.rs", &[(1, 1)]),
        ]))
        .unwrap();
        assert_eq!(
            tally(&root(), &only_mods, &without_b).unwrap().untested,
            Vec::<String>::new(),
            "an unreported file of declarations only holds nothing to run"
        );
        only_mods.members[1].sources[0].text = "fn x( {".to_string();
        let errors = tally(&root(), &only_mods, &without_b).unwrap_err();
        assert!(
            errors[0].starts_with("crates/b/src/lib.rs: line 1"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_function_body_is_found_wherever_a_test_build_compiles_one() {
        for (source, body) in [
            ("pub fn f() {}", true),
            ("pub struct S;\nimpl S { pub fn m(&self) {} }", true),
            ("pub trait T { fn d(&self) {} }", true),
            ("pub trait T { fn r(&self); }", false),
            ("mod inner { pub fn g() {} }", true),
            ("pub mod x;\npub use x::*;\npub struct S;", false),
            ("#[cfg(test)]\nmod tests { fn t() {} }", false),
            ("#[test]\nfn t() {}", false),
            (
                "pub struct S;\nimpl S {\n#[cfg(test)]\nfn h(&self) {}\n}",
                false,
            ),
            ("pub trait T {\n#[cfg(test)]\nfn h(&self) {}\n}", false),
            ("pub fn generic<T: Clone>(t: &T) -> T { t.clone() }", true),
        ] {
            assert_eq!(has_body(source).unwrap(), body, "{source}");
        }
        assert!(has_body("fn (").unwrap_err().starts_with("line 1"));
    }

    #[test]
    fn code_no_test_build_compiles_is_refused_by_line() {
        // Review finding F11: an item under `cfg(not(test))` is production code
        // the report can never show.
        for (source, line) in [
            ("pub fn a() {}\n#[cfg(not(test))]\npub fn b() {}\n", 2),
            ("#[cfg(all(unix, not(test)))]\npub fn b() {}\n", 1),
            ("mod m {\n#[cfg(not(test))]\npub fn b() {}\n}\n", 2),
            (
                "pub struct S;\nimpl S {\n#[cfg(not(test))]\nfn b(&self) {}\n}\n",
                3,
            ),
            ("pub trait T {\n#[cfg(not(test))]\nfn b(&self) {}\n}\n", 2),
        ] {
            let error = test_only_lines(source).unwrap_err();
            assert!(
                error.starts_with(&format!("line {line}: `#[cfg(not(test))]`")),
                "{source}: {error}"
            );
            assert!(error.contains("through a seam"), "{error}");
        }
        for ships in [
            "#[cfg(not(feature = \"x\"))]\npub fn b() {}\n",
            "#[cfg(any(not(test), unix))]\npub fn b() {}\n",
            "#[cfg(not(unix))]\npub fn b() {}\n",
        ] {
            assert!(test_only_lines(ships).is_ok(), "{ships}");
        }
    }

    #[test]
    fn a_report_of_another_tree_is_refused() {
        let report = parse_lcov(&lcov(&[("gone/src/lib.rs", &[(1, 1)])])).unwrap();
        let errors = tally(&root(), &workspace(), &report).unwrap_err();
        assert!(
            errors[0].contains("is no source of this workspace"),
            "{errors:?}"
        );
        let mut broken = workspace();
        broken.members[1].sources[0].text = "fn x( {".to_string();
        let report = parse_lcov(&lcov(&[("b/src/lib.rs", &[(1, 1)])])).unwrap();
        let errors = tally(&root(), &broken, &report).unwrap_err();
        assert!(
            errors[0].starts_with("crates/b/src/lib.rs: line 1"),
            "{errors:?}"
        );
        // A test file that does not parse is not read at all.
        let mut broken_test = workspace();
        broken_test.members[0].sources[2].text = "fn x( {".to_string();
        let report = parse_lcov(&lcov(&[("a/tests/it.rs", &[(1, 1)])])).unwrap();
        assert!(tally(&root(), &broken_test, &report).is_ok());
    }

    #[test]
    fn a_path_compares_across_separators_and_drive_letter_case() {
        assert_eq!(path_key(Path::new(r"c:\w\a.rs")), "C:/w/a.rs");
        assert_eq!(path_key(Path::new("C:/w/a.rs")), "C:/w/a.rs");
        assert_eq!(path_key(Path::new("/w/a.rs")), "/w/a.rs");
        assert_eq!(path_key(Path::new("c")), "c");
        assert_eq!(
            path_key(Path::new("é:x")),
            "é:x",
            "a non-ASCII first letter is kept whole"
        );
        assert_eq!(
            relative(
                Path::new("/w"),
                &Path::new("/w").join("crates").join("a.rs")
            ),
            "crates/a.rs"
        );
        assert_eq!(relative(Path::new("/w"), Path::new("/x/a.rs")), "/x/a.rs");
    }

    #[test]
    fn the_tree_hash_is_fnv1a_and_blind_only_to_order_and_line_endings() {
        // FNV-1a 64's published test vectors: the empty input, "a" and "foobar".
        assert_eq!(tree_hash(&[]), "fnv1a64:cbf29ce484222325");
        assert_eq!(fnv(0xcbf2_9ce4_8422_2325, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv(0xcbf2_9ce4_8422_2325, b"foobar"), 0x8594_4171_f739_67e8);
        let file = |path: &str, text: &str| (path.to_string(), text.to_string());
        let base = tree_hash(&[file("a.rs", "x\ny\n"), file("b.rs", "z")]);
        assert_eq!(base.len(), "fnv1a64:".len() + 16);
        assert_eq!(
            tree_hash(&[file("b.rs", "z"), file("a.rs", "x\r\ny\r\n")]),
            base
        );
        assert_ne!(
            tree_hash(&[file("a.rs", "x\ny!\n"), file("b.rs", "z")]),
            base
        );
        assert_ne!(
            tree_hash(&[file("c.rs", "x\ny\n"), file("b.rs", "z")]),
            base
        );
        assert_ne!(
            tree_hash(&[file("a.rs", "x\ny\nz"), file("b.rs", "")]),
            base,
            "the separator keeps a file's end from moving into the next"
        );
        assert_ne!(
            tree_hash(&[file("a.rsx", "\ny\n"), file("b.rs", "z")]),
            base,
            "the separator keeps a path from running into its text"
        );
    }

    #[test]
    fn a_measurement_round_trips_and_its_validator_refuses_each_fault() {
        let mut m = measured("linux", Some(("9", "2")), &[("a", 3, 6)]);
        m.untested = vec!["crates/a/src/dead.rs".into()];
        let text = render_measurement(&m);
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"schemaVersion\": 1"));
        assert_eq!(parse_measurement(&text).unwrap(), m);
        let local = measured("windows", None, &[]);
        assert_eq!(
            parse_measurement(&render_measurement(&local)).unwrap(),
            local
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut v = value.clone();
            edit(&mut v);
            parse_measurement(&v.to_string()).unwrap_err()
        };
        for (error, needle) in [
            (
                with(&|v| v["extra"] = 1.into()),
                "`extra`, which is not one of",
            ),
            (
                with(&|v| {
                    v.as_object_mut().unwrap().remove("tool");
                }),
                "has no `tool`",
            ),
            (with(&|v| v["schemaVersion"] = 2.into()), "must be 1"),
            (
                with(&|v| v["crates"]["a"]["covered"] = 7.into()),
                "covers 7 of 6",
            ),
            (
                with(&|v| v["crates"]["a"]["count"] = "6".into()),
                "`crates.a`.count must be a whole number",
            ),
            (
                with(&|v| v["crates"]["a"]["extra"] = 1.into()),
                "`crates.a` has `extra`",
            ),
            (
                with(&|v| v["crates"]["a"] = 1.into()),
                "`crates.a` must be a JSON object",
            ),
            (
                with(&|v| v["crates"] = 1.into()),
                "`crates` must be a JSON object",
            ),
            (with(&|v| v["run"] = 5.into()), "`run` must be null or"),
            (with(&|v| v["run"]["extra"] = 1.into()), "`run` has `extra`"),
            (
                with(&|v| v["run"]["id"] = 1.into()),
                "`id` must be a string",
            ),
            (
                with(&|v| v["untested"] = 1.into()),
                "`untested` must be a list",
            ),
            (
                with(&|v| v["untested"] = serde_json::json!([1])),
                "`untested` must be a list",
            ),
            (with(&|v| v["os"] = 1.into()), "`os` must be a string"),
            (with(&|v| v["tree"] = 1.into()), "`tree` must be a string"),
            (with(&|v| v["tool"] = 1.into()), "`tool` must be a string"),
        ] {
            assert!(error.contains(needle), "{needle}: {error}");
        }
        assert!(parse_measurement("{").unwrap_err().starts_with("not JSON"));
        assert!(
            parse_measurement("[]")
                .unwrap_err()
                .contains("the measurement must be a JSON object")
        );
    }

    #[test]
    fn the_committed_floors_file_reads_and_renders_back_unchanged() {
        // Whatever rows it holds: the ratchet rewrites them, and this test must
        // pass before and after.
        let text = include_str!("../coverage-floors.json");
        let floors = parse_floors(text).unwrap();
        assert_eq!(render_floors(&floors), text.replace("\r\n", "\n"));
        let seeded = with_rows(&[("a", 9372, [9372, 9400, 9500]), ("b", 0, [0, 0, 0])]);
        let mut from = seeded.clone();
        from.measured_from = Some(Provenance {
            tree: "t".into(),
            tool: "x".into(),
            runs: PLATFORMS
                .iter()
                .flat_map(|os| (1..=4).map(move |attempt| format!("{os} 9/{attempt}")))
                .collect(),
        });
        from.about = Vec::new();
        for f in [seeded, from] {
            assert_eq!(parse_floors(&render_floors(&f)).unwrap(), f);
        }
    }

    #[test]
    fn the_floors_validator_refuses_each_fault_naming_the_file() {
        let base: Value = serde_json::from_str(&render_floors(&with_rows(&[(
            "a",
            9372,
            [9372, 9400, 9500],
        )])))
        .unwrap();
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            edit(&mut v);
            parse_floors(&v.to_string()).unwrap_err()
        };
        for (error, needle) in [
            (with(&|v| v["corroboratingRuns"] = 2.into()), "at least 3"),
            (with(&|v| v["corroboratingRuns"] = "3".into()), "at least 3"),
            (
                with(&|v| {
                    v["crates"]["a"]["platforms"]
                        .as_object_mut()
                        .unwrap()
                        .remove("macos");
                }),
                "has no `macos`",
            ),
            (
                with(&|v| v["crates"]["a"]["platforms"]["solaris"] = 1.into()),
                "`solaris`, which is not one of",
            ),
            (
                with(&|v| v["crates"]["a"]["platforms"]["linux"] = "x".into()),
                "platforms.linux must be a percentage",
            ),
            (
                with(&|v| v["crates"]["a"]["platforms"] = 1.into()),
                "platforms must be a JSON object",
            ),
            (
                with(&|v| v["crates"]["a"]["floor"] = 93.725.into()),
                "at most two decimals",
            ),
            (
                with(&|v| v["crates"]["a"]["floor"] = 100.01.into()),
                "from 0 to 100",
            ),
            (
                with(&|v| v["crates"]["a"]["floor"] = (-1).into()),
                "from 0 to 100",
            ),
            (
                with(&|v| v["crates"]["a"]["extra"] = 1.into()),
                "`crates.a` has `extra`",
            ),
            (
                with(&|v| v["crates"]["a"] = 1.into()),
                "`crates.a` must be a JSON object",
            ),
            (
                with(&|v| v["crates"] = 1.into()),
                "`crates` must be a JSON object",
            ),
            (
                with(&|v| v["tolerance"] = "0".into()),
                "`tolerance` must be a percentage",
            ),
            (
                with(&|v| v["ratchetLag"] = "5".into()),
                "`ratchetLag` must be a percentage",
            ),
            (with(&|v| v["about"] = 1.into()), "`about` must be a list"),
            (
                with(&|v| v["about"] = serde_json::json!([1])),
                "`about` must be a list",
            ),
            (
                with(&|v| v["measuredFrom"] = 1.into()),
                "`measuredFrom` must be null or an object",
            ),
            (
                with(&|v| {
                    v["measuredFrom"] = serde_json::json!({"tree": "t", "tool": "x", "runs": [1]})
                }),
                "`measuredFrom.runs` must be a list",
            ),
            (
                with(&|v| {
                    v["measuredFrom"] = serde_json::json!({"tree": 1, "tool": "x", "runs": []})
                }),
                "`tree` must be a string",
            ),
            (
                with(&|v| {
                    v["measuredFrom"] = serde_json::json!({"tree": "t", "tool": 1, "runs": []})
                }),
                "`tool` must be a string",
            ),
            (
                with(&|v| v["measuredFrom"] = serde_json::json!({"tree": "t", "runs": []})),
                "`measuredFrom` has no `tool`",
            ),
            (with(&|v| v["schemaVersion"] = 2.into()), "must be 1"),
            (
                with(&|v| v["surprise"] = 1.into()),
                "`surprise`, which is not one of",
            ),
            (
                with(&|v| v["toleranceReason"] = 1.into()),
                "`toleranceReason` must be a string",
            ),
            (
                with(&|v| v["ratchetLagReason"] = 1.into()),
                "`ratchetLagReason` must be a string",
            ),
            (
                with(&|v| v["corroboratingRunsReason"] = 1.into()),
                "`corroboratingRunsReason` must be a string",
            ),
        ] {
            assert!(error.starts_with(FLOORS_FILE), "{error}");
            assert!(error.contains(needle), "{needle}: {error}");
        }
        assert!(parse_floors("{").unwrap_err().contains("not JSON"));
        assert!(
            parse_floors("[]")
                .unwrap_err()
                .contains("the file must be a JSON object")
        );
    }

    #[test]
    fn a_crate_at_or_above_its_floor_passes_and_says_so() {
        let (rows, failures) = check(
            &with_rows(&[("a", 5000, [5000, 5000, 5000])]),
            &measured("linux", None, &[("a", 3, 6)]),
        );
        assert_eq!(failures, Vec::<String>::new());
        assert_eq!(rows, ["a: 50.00% (3 of 6 lines), floor 50.00%"]);
    }

    #[test]
    fn a_regression_fails_beyond_the_tolerance_only() {
        let mut floors = with_rows(&[("a", 5001, [5001, 5001, 5001])]);
        let (_, failures) = check(&floors, &measured("linux", None, &[("a", 3, 6)]));
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0]
                .contains("a: line coverage 50.00% is below its floor of 50.01% (tolerance 0.00)"),
            "{failures:?}"
        );
        assert!(failures[0].contains("never lowered"));
        floors.tolerance = 1;
        assert_eq!(
            check(&floors, &measured("linux", None, &[("a", 3, 6)])).1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_record_left_behind_on_this_platform_fails() {
        let floors = with_rows(&[("a", 4000, [4500, 4400, 6000])]);
        // 50.00 is 5.00 above linux's 45.00: at the lag, not beyond it.
        assert_eq!(
            check(&floors, &measured("linux", None, &[("a", 3, 6)])).1,
            Vec::<String>::new()
        );
        let (_, failures) = check(&floors, &measured("macos", None, &[("a", 3, 6)]));
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].contains("more than 5.00 points above the 44.00% recorded for macos"),
            "{failures:?}"
        );
        // Windows sits above the floor by design: compared to itself, it is current.
        assert_eq!(
            check(&floors, &measured("windows", None, &[("a", 3, 6)])).1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_measurement_under_another_tool_fails_naming_the_approved_one() {
        // Review finding F13: the tool was recorded and never compared.
        let floors = with_rows(&[("a", 5000, [5000, 5000, 5000])]);
        let mut m = measured("linux", None, &[("a", 3, 6)]);
        m.tool = "cargo-llvm-cov 0.9.2".into();
        let (_, failures) = check(&floors, &m);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].contains("measured with `cargo-llvm-cov 0.9.2`, but the floors were measured with `cargo-llvm-cov 0.9.1`"),
            "{failures:?}"
        );
        assert!(failures[0].contains(INSTALL));
        assert!(
            check(&base(), &m)
                .1
                .iter()
                .all(|f| !f.contains("measured with")),
            "no provenance, nothing to compare"
        );
    }

    #[test]
    fn the_floors_file_refuses_what_only_a_hand_could_write() {
        // Review finding F14: a floor below its lowest platform figure, a band
        // wide enough to lower every floor, and rows with no runs behind them.
        let lowered = render_floors(&with_rows(&[("a", 8999, [9000, 9100, 9200])]));
        let error = parse_floors(&lowered).unwrap_err();
        assert!(
            error.contains("`crates.a`.floor is below its lowest platform figure"),
            "{error}"
        );
        assert!(
            parse_floors(&render_floors(&with_rows(&[(
                "a",
                9000,
                [9000, 9100, 9200]
            )])))
            .is_ok()
        );
        assert!(
            parse_floors(&render_floors(&with_rows(&[(
                "a",
                9050,
                [9000, 9100, 9200]
            )])))
            .is_ok(),
            "a kept floor sits above a platform"
        );
        let mut wide = with_rows(&[]);
        wide.tolerance = MAX_TOLERANCE + 1;
        assert!(
            parse_floors(&render_floors(&wide))
                .unwrap_err()
                .contains("`tolerance` may be at most 1.00 points")
        );
        wide.tolerance = MAX_TOLERANCE;
        assert!(parse_floors(&render_floors(&wide)).is_ok());
        let mut unsourced = with_rows(&[("a", 9000, [9000, 9000, 9000])]);
        unsourced.measured_from = None;
        let error = parse_floors(&render_floors(&unsourced)).unwrap_err();
        assert!(
            error.contains("`measuredFrom` records 0 linux run(s) and the rows need 3"),
            "{error}"
        );
        let mut short = with_rows(&[("a", 9000, [9000, 9000, 9000])]);
        if let Some(from) = short.measured_from.as_mut() {
            from.runs.retain(|run| run != "windows 77/3");
        }
        assert!(
            parse_floors(&render_floors(&short))
                .unwrap_err()
                .contains("2 windows run(s)")
        );
    }

    #[test]
    fn a_crate_without_a_floor_or_a_floor_without_a_crate_fails() {
        let floors = with_rows(&[("gone", 5000, [5000, 5000, 5000])]);
        let (rows, failures) = check(&floors, &measured("linux", None, &[("a", 3, 6)]));
        assert_eq!(rows, ["a: 50.00% (3 of 6 lines), no floor"]);
        assert!(
            failures[0].contains("a: has no floor yet. Floors are seeded only from CI"),
            "{failures:?}"
        );
        assert!(failures[0].contains("3 runs per platform of one tree"));
        // A green job reruns whole; `--failed` reruns only a red one (F15).
        assert!(failures[0].contains("`gh run rerun <run>`, or `gh run rerun <run> --failed`"));
        assert!(failures[0].contains("gh run download <run> -p 'rust-coverage-*'"));
        assert!(failures[0].contains("cargo xtask coverage ratchet <dir>"));
        assert!(
            failures[1].contains("gone: has a floor in xtask/coverage-floors.json but is no crate")
        );
    }

    #[test]
    fn an_untested_file_an_empty_crate_and_an_unknown_platform_fail() {
        let mut m = measured("plan9", None, &[("a", 3, 6), ("empty", 0, 0)]);
        m.untested = vec!["crates/a/src/dead.rs".into()];
        let (_, failures) = check(&with_rows(&[("a", 5000, [5000, 5000, 5000])]), &m);
        assert_eq!(failures.len(), 3, "{failures:?}");
        assert!(
            failures[0].contains("`plan9`, and floors are recorded for linux, macos, windows only")
        );
        assert!(
            failures[1]
                .starts_with("crates/a/src/dead.rs: no test runs any of its production lines")
        );
        assert!(failures[2].starts_with("empty: measured no production line"));
    }

    /// Nine CI runs of crate `a` (`x` of 100 lines): three per platform, in
    /// PLATFORMS order.
    fn nine_runs(figures: [(u64, u64, u64); 3]) -> Vec<(String, Measurement)> {
        let mut out = Vec::new();
        for (os, (first, second, third)) in PLATFORMS.iter().zip(figures) {
            for (attempt, covered) in [("1", first), ("2", second), ("3", third)] {
                out.push((
                    format!("{os}-{attempt}"),
                    measured(os, Some(("77", attempt)), &[("a", covered, 100)]),
                ));
            }
        }
        out
    }

    fn crates(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_ratchet_seeds_a_crate_at_the_lowest_of_nine_runs_and_records_each_platform() {
        let runs = nine_runs([(90, 88, 91), (93, 94, 93), (95, 96, 97)]);
        let (next, notes) = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap();
        let row = &next.crates["a"];
        assert_eq!(row.floor, 8800);
        assert_eq!(
            row.platforms,
            BTreeMap::from([
                ("linux".into(), 8800),
                ("macos".into(), 9300),
                ("windows".into(), 9500)
            ])
        );
        assert_eq!(notes, ["a: seeded at 88.00%"]);
        let from = next.measured_from.unwrap();
        assert_eq!(from.tree, TREE);
        assert_eq!(from.tool, "cargo-llvm-cov 0.9.1");
        assert_eq!(from.runs.len(), 9);
        assert_eq!(from.runs[0], "linux 77/1");
        let mut rules = base();
        rules.tolerance = 7;
        rules.ratchet_lag = 300;
        rules.about = vec!["kept".into()];
        let (next, _) = ratchet(&rules, &runs, TREE, &crates(&["a"])).unwrap();
        assert_eq!(
            (next.tolerance, next.ratchet_lag, next.about),
            (7, 300, vec!["kept".to_string()]),
            "the rules carry over"
        );
    }

    #[test]
    fn a_ratchet_raises_a_floor_and_never_lowers_one() {
        let old = with_rows(&[("a", 8500, [8500, 8500, 8500])]);
        let (next, notes) =
            ratchet(&old, &nine_runs([(90, 90, 90); 3]), TREE, &crates(&["a"])).unwrap();
        assert_eq!(next.crates["a"].floor, 9000);
        assert_eq!(notes, ["a: floor 85.00% -> 90.00%"]);
        let (same, notes) =
            ratchet(&next, &nine_runs([(90, 90, 90); 3]), TREE, &crates(&["a"])).unwrap();
        assert_eq!(same.crates, next.crates);
        assert!(
            notes.is_empty(),
            "an unchanged floor needs no note: {notes:?}"
        );
        let mut tolerant = with_rows(&[("a", 9001, [9001, 9001, 9001])]);
        tolerant.tolerance = 1;
        let (next, notes) = ratchet(
            &tolerant,
            &nine_runs([(90, 90, 90); 3]),
            TREE,
            &crates(&["a"]),
        )
        .unwrap();
        assert_eq!(
            next.crates["a"].floor, 9001,
            "within the tolerance below, the floor stays"
        );
        assert_eq!(
            next.crates["a"].platforms["linux"], 9000,
            "the platform figure records what was measured"
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn a_ratchet_refuses_a_regression() {
        let old = with_rows(&[("a", 9100, [9100, 9100, 9100])]);
        let errors =
            ratchet(&old, &nine_runs([(90, 90, 90); 3]), TREE, &crates(&["a"])).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("a: the runs' lowest is 90.00%, below its floor of 91.00%"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_ratchet_needs_three_ci_runs_per_platform_of_this_tree() {
        let mut runs = nine_runs([(90, 90, 90); 3]);
        runs.remove(0);
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert_eq!(
            errors,
            [format!(
                "linux: 2 of the 3 CI runs a raise needs; {HOW_TO_RATCHET}"
            )]
        );
        let errors = ratchet(&base(), &[], TREE, &crates(&["a"])).unwrap_err();
        assert_eq!(errors.len(), 3, "every platform is short: {errors:?}");

        let mut runs = nine_runs([(90, 90, 90); 3]);
        runs[0].1.run = None;
        runs[1].1.tree = "fnv1a64:00000000000000ff".into();
        runs[2].1.os = "plan9".into();
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert!(
            errors[0].contains("linux-1: was measured outside CI"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("linux-2: measured tree fnv1a64:00000000000000ff, but this tree is"),
            "{errors:?}"
        );
        assert!(
            errors[2].contains("linux-3: platform `plan9` holds no floor"),
            "{errors:?}"
        );
        assert!(errors[3].starts_with("linux: 0 of the 3"), "{errors:?}");
        assert_eq!(errors.len(), 4, "{errors:?}");
    }

    #[test]
    fn one_artifact_counts_once_and_a_conflicting_copy_is_refused() {
        let mut runs = nine_runs([(90, 90, 90); 3]);
        // The linux attempts all become run 77/1: one run, downloaded three times.
        for (_, m) in runs.iter_mut().take(3) {
            m.run = Some(CiRun {
                id: "77".into(),
                attempt: "1".into(),
                sha: "abc".into(),
            });
        }
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("linux: 1 of the 3"), "{errors:?}");
        runs[1].1.crates.insert(
            "a".into(),
            Lines {
                covered: 89,
                count: 100,
            },
        );
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert!(
            errors[0].contains("linux-2: run 77/1 on linux appears twice with different figures"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_ratchet_refuses_other_crates_untested_files_and_mixed_tools() {
        let errors = ratchet(
            &base(),
            &nine_runs([(90, 90, 90); 3]),
            TREE,
            &crates(&["a", "b"]),
        )
        .unwrap_err();
        assert!(
            errors[0].contains("linux-1: measured the crates"),
            "{errors:?}"
        );
        let mut runs = nine_runs([(90, 90, 90); 3]);
        runs[0].1.untested = vec!["crates/a/src/dead.rs".into()];
        runs[8].1.tool = "cargo-llvm-cov 0.9.2".into();
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert!(
            errors[0].contains("linux-1: no test enters crates/a/src/dead.rs"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("the runs measured with different tools"),
            "{errors:?}"
        );
        assert!(errors[2].starts_with("linux: 2 of the 3"), "{errors:?}");
    }

    #[test]
    fn a_ratchet_drops_a_row_whose_crate_is_gone_and_needs_a_line_per_platform() {
        let old = with_rows(&[("gone", 5000, [5000, 5000, 5000])]);
        let (next, notes) =
            ratchet(&old, &nine_runs([(90, 90, 90); 3]), TREE, &crates(&["a"])).unwrap();
        assert!(!next.crates.contains_key("gone"));
        assert_eq!(
            notes,
            [
                "a: seeded at 90.00%",
                "gone: row dropped; it is no crate of this workspace"
            ]
        );
        let mut runs = nine_runs([(90, 90, 90); 3]);
        for (_, m) in runs.iter_mut().filter(|(_, m)| m.os == "macos") {
            m.crates.insert("a".into(), Lines::default());
        }
        let errors = ratchet(&base(), &runs, TREE, &crates(&["a"])).unwrap_err();
        assert_eq!(errors, ["a: no macos run measured a production line of it"]);
    }

    /// A scripted harness: every answer is a field, and the calls that change
    /// something are recorded in order.
    struct Fake {
        tool: Option<String>,
        target: Result<PathBuf, String>,
        prepare: Result<(), String>,
        llvm_cov: Option<i32>,
        workspace: Result<Workspace, String>,
        files: BTreeMap<PathBuf, String>,
        times: BTreeMap<PathBuf, SystemTime>,
        json: Result<Vec<(PathBuf, String)>, String>,
        env: BTreeMap<String, String>,
        calls: Vec<String>,
    }

    fn target() -> PathBuf {
        PathBuf::from("/t")
    }

    fn report() -> PathBuf {
        target().join(REPORT)
    }

    impl Fake {
        fn new(floors_text: &str) -> Fake {
            let mut files = BTreeMap::new();
            files.insert(root().join(FLOORS_FILE), floors_text.to_string());
            files.insert(report(), good_lcov());
            files.insert(root().join("Cargo.lock"), "lock".to_string());
            Fake {
                tool: Some("cargo-llvm-cov 0.9.1".into()),
                target: Ok(target()),
                prepare: Ok(()),
                llvm_cov: Some(0),
                workspace: Ok(workspace()),
                files,
                times: BTreeMap::new(),
                json: Ok(Vec::new()),
                env: BTreeMap::new(),
                calls: Vec::new(),
            }
        }
    }

    impl Harness for Fake {
        fn tool(&mut self) -> Option<String> {
            self.calls.push("tool".into());
            self.tool.clone()
        }
        fn target_dir(&mut self, _root: &Path) -> Result<PathBuf, String> {
            self.target.clone()
        }
        fn prepare(&mut self, report: &Path) -> Result<(), String> {
            self.calls
                .push(format!("prepare {}", relative(&target(), report)));
            self.prepare.clone()
        }
        fn llvm_cov(&mut self, _root: &Path, report: &Path) -> Option<i32> {
            self.calls
                .push(format!("llvm-cov {}", relative(&target(), report)));
            self.llvm_cov
        }
        fn workspace(&mut self, _root: &Path) -> Result<Workspace, String> {
            self.workspace.clone()
        }
        fn read(&mut self, path: &Path) -> Result<String, String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("cannot read {}", path.display()))
        }
        fn modified(&mut self, path: &Path) -> Option<SystemTime> {
            self.times.get(path).copied()
        }
        fn json_files(&mut self, _dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
            self.json.clone()
        }
        fn env(&mut self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
    }

    /// Floors that [`good_lcov`] meets exactly.
    fn seeded_text() -> String {
        render_floors(&with_rows(&[
            ("a", 5714, [5714, 5714, 5714]),
            ("b", 10_000, [10_000, 10_000, 10_000]),
        ]))
    }

    #[test]
    fn a_run_prepares_measures_and_checks_in_that_order() {
        let mut fake = Fake::new(&seeded_text());
        let (code, text) = run(&mut fake, &root(), "linux");
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            fake.calls,
            [
                "tool",
                "prepare ephesus-coverage/lcov.info",
                "llvm-cov ephesus-coverage/lcov.info"
            ]
        );
        assert!(
            text.contains("coverage: a: 57.14% (4 of 7 lines), floor 57.14%\n"),
            "{text}"
        );
        assert!(
            text.contains("coverage: b: 100.00% (1 of 1 lines), floor 100.00%\n"),
            "{text}"
        );
        assert!(
            text.ends_with("coverage: ok (2 crate(s), 3 production file(s), linux)\n"),
            "{text}"
        );
    }

    #[test]
    fn a_run_fails_and_says_why_at_each_step() {
        type Edit = Box<dyn Fn(&mut Fake)>;
        let cases: Vec<(Edit, &str)> = vec![
            (
                Box::new(|f| f.tool = None),
                "cargo-llvm-cov is not installed",
            ),
            (
                Box::new(|f| f.target = Err("no metadata".into())),
                "coverage: no metadata",
            ),
            (
                Box::new(|f| f.prepare = Err("cannot delete".into())),
                "coverage: cannot delete",
            ),
            (
                Box::new(|f| f.llvm_cov = Some(101)),
                "cargo llvm-cov exited 101; its output is above",
            ),
            (
                Box::new(|f| f.llvm_cov = None),
                "cargo llvm-cov exited without an exit code",
            ),
            (
                Box::new(|f| f.workspace = Err("bad".into())),
                "cannot read the workspace: bad",
            ),
            (
                Box::new(|f| {
                    f.files.remove(&report());
                }),
                "coverage: cannot read",
            ),
            (
                Box::new(|f| {
                    f.files.insert(report(), "DA:1,1\n".into());
                }),
                "line data outside a file",
            ),
            (
                Box::new(|f| {
                    f.files
                        .insert(report(), lcov(&[("x/src/lib.rs", &[(1, 1)])]));
                }),
                "no source of this workspace",
            ),
            (
                Box::new(|f| {
                    f.files.remove(&root().join(FLOORS_FILE));
                }),
                "coverage: cannot read",
            ),
            (
                Box::new(|f| {
                    f.files.insert(root().join(FLOORS_FILE), "{".into());
                }),
                "not JSON",
            ),
            (
                Box::new(|f| {
                    f.files
                        .insert(root().join(FLOORS_FILE), render_floors(&base()));
                }),
                "coverage: a: has no floor yet",
            ),
        ];
        for (edit, needle) in cases {
            let mut fake = Fake::new(&seeded_text());
            edit(&mut fake);
            let (code, text) = run(&mut fake, &root(), "linux");
            assert_eq!(code, 1, "{needle}: {text}");
            assert!(text.contains(needle), "{needle}: {text}");
            assert!(!text.contains("coverage: ok"), "{needle}: {text}");
        }
        let mut fake = Fake::new(&seeded_text());
        fake.tool = None;
        run(&mut fake, &root(), "linux");
        assert_eq!(fake.calls, ["tool"], "nothing runs without the tool");
        let mut fake = Fake::new(&seeded_text());
        fake.prepare = Err("cannot delete".into());
        run(&mut fake, &root(), "linux");
        assert_eq!(
            fake.calls.last().map(String::as_str),
            Some("prepare ephesus-coverage/lcov.info"),
            "a failed prepare runs nothing"
        );
    }

    #[test]
    fn a_failing_run_ends_with_its_count_of_failures() {
        let mut fake = Fake::new(&render_floors(&base()));
        let (_, text) = run(&mut fake, &root(), "linux");
        assert!(text.ends_with("coverage: 2 failure(s)\n"), "{text}");
    }

    #[test]
    fn a_run_that_read_no_production_file_proves_nothing() {
        let mut fake = Fake::new(&render_floors(&with_rows(&[])));
        fake.files.insert(report(), String::new());
        fake.workspace = Ok(Workspace::default());
        let (code, text) = run(&mut fake, &root(), "linux");
        assert_eq!(code, 1);
        assert!(
            text.contains(
                "read 0 crate(s) and 0 production file(s); a check over nothing proves nothing"
            ),
            "{text}"
        );
        let mut fake = Fake::new(&seeded_text());
        fake.files
            .insert(report(), lcov(&[("a/tests/it.rs", &[(1, 1)])]));
        let (code, text) = run(&mut fake, &root(), "linux");
        assert_eq!(code, 1);
        assert!(
            text.contains("read 2 crate(s) and 0 production file(s)"),
            "{text}"
        );
    }

    #[test]
    fn emit_prints_the_measurement_with_its_ci_run_and_tree() {
        let mut fake = Fake::new(&seeded_text());
        fake.times.insert(
            report(),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
        );
        for (name, value) in [
            ("GITHUB_RUN_ID", "77"),
            ("GITHUB_RUN_ATTEMPT", "2"),
            ("GITHUB_SHA", "abc"),
        ] {
            fake.env.insert(name.into(), value.into());
        }
        let measurement = parse_measurement(&emit(&mut fake, &root(), "macos").unwrap()).unwrap();
        assert_eq!(measurement.os, "macos");
        assert_eq!(measurement.tool, "cargo-llvm-cov 0.9.1");
        assert_eq!(
            measurement.run,
            Some(CiRun {
                id: "77".into(),
                attempt: "2".into(),
                sha: "abc".into()
            })
        );
        assert_eq!(
            measurement.crates["a"],
            Lines {
                covered: 4,
                count: 7
            }
        );
        let files = tree_files(&mut fake, &root(), &workspace());
        assert_eq!(measurement.tree, tree_hash(&files));
        assert!(
            files.contains(&("Cargo.lock".to_string(), "lock".to_string())),
            "a tree file is hashed"
        );
        assert!(
            files.contains(&(
                "crates/b/src/lib.rs".to_string(),
                "pub fn b() {}\n".to_string()
            )),
            "a source is hashed"
        );
        assert!(
            files.iter().any(|(path, _)| path == "crates/a/Cargo.toml"),
            "a manifest is hashed"
        );
        assert!(
            !files.iter().any(|(path, _)| path == "rust-toolchain.toml"),
            "an absent tree file is skipped"
        );
        for name in ["GITHUB_SHA", "GITHUB_RUN_ATTEMPT", "GITHUB_RUN_ID"] {
            let mut partial = Fake::new(&seeded_text());
            partial.times = fake.times.clone();
            partial.env = fake.env.clone();
            partial.env.remove(name);
            assert_eq!(
                parse_measurement(&emit(&mut partial, &root(), "macos").unwrap())
                    .unwrap()
                    .run,
                None,
                "without {name} it is no CI run"
            );
        }
    }

    #[test]
    fn emit_refuses_a_missing_or_stale_report() {
        let mut fake = Fake::new(&seeded_text());
        let errors = emit(&mut fake, &root(), "linux").unwrap_err();
        assert!(errors[0].contains("there is no report at"), "{errors:?}");
        let made = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        fake.times.insert(report(), made);
        fake.times.insert(root().join("crates/b/src/lib.rs"), made);
        assert!(
            emit(&mut fake, &root(), "linux").is_ok(),
            "a source as old as the report is not newer"
        );
        fake.times.insert(
            root().join("crates/b/src/lib.rs"),
            made + std::time::Duration::from_secs(1),
        );
        let errors = emit(&mut fake, &root(), "linux").unwrap_err();
        assert!(
            errors[0].contains("is older than crates/b/src/lib.rs, so it measured another tree"),
            "{errors:?}"
        );
        fake.tool = None;
        assert!(emit(&mut fake, &root(), "linux").unwrap_err()[0].contains("not installed"));
        let mut fake = Fake::new(&seeded_text());
        fake.target = Err("no metadata".into());
        assert_eq!(
            emit(&mut fake, &root(), "linux").unwrap_err(),
            ["no metadata"]
        );
    }

    #[test]
    fn ratchet_dir_reads_every_measurement_and_prints_the_floors() {
        let mut fake = Fake::new(&render_floors(&base()));
        let tree = tree_hash(&tree_files(&mut fake, &root(), &workspace()));
        let mut found = Vec::new();
        for (name, mut m) in nine_runs([(90, 90, 90); 3]) {
            m.tree = tree.clone();
            m.crates.insert(
                "b".into(),
                Lines {
                    covered: 1,
                    count: 1,
                },
            );
            found.push((
                PathBuf::from(format!("/dl/{name}/rust-coverage.json")),
                render_measurement(&m),
            ));
        }
        fake.json = Ok(found.clone());
        let (text, notes) = ratchet_dir(&mut fake, &root(), Path::new("/dl")).unwrap();
        let next = parse_floors(&text).unwrap();
        assert_eq!(next.crates["a"].floor, 9000);
        assert_eq!(next.crates["b"].floor, 10_000);
        assert_eq!(notes, ["a: seeded at 90.00%", "b: seeded at 100.00%"]);

        let mut stale = Fake::new(&render_floors(&base()));
        stale
            .files
            .insert(root().join("Cargo.lock"), "another lock".into());
        stale.json = Ok(found.clone());
        let errors = ratchet_dir(&mut stale, &root(), Path::new("/dl")).unwrap_err();
        assert!(
            errors[0].contains("but this tree is"),
            "the tree is this one's: {errors:?}"
        );

        found[0].1 = "{".into();
        fake.json = Ok(found);
        let errors = ratchet_dir(&mut fake, &root(), Path::new("/dl")).unwrap_err();
        assert!(
            errors[0].starts_with("linux-1/rust-coverage.json: not JSON"),
            "{errors:?}"
        );
        fake.json = Err("cannot read /dl".into());
        assert_eq!(
            ratchet_dir(&mut fake, &root(), Path::new("/dl")).unwrap_err(),
            ["cannot read /dl"]
        );
        fake.json = Ok(Vec::new());
        fake.workspace = Err("bad".into());
        assert!(
            ratchet_dir(&mut fake, &root(), Path::new("/dl")).unwrap_err()[0]
                .contains("cannot read the workspace")
        );
        let mut fake = Fake::new("{");
        assert!(
            ratchet_dir(&mut fake, &root(), Path::new("/dl")).unwrap_err()[0].contains("not JSON")
        );
    }

    #[test]
    fn every_command_form_parses_and_anything_else_is_usage() {
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&args(&[])), Some(Verb::Run));
        assert_eq!(parse_args(&args(&["emit"])), Some(Verb::Emit));
        assert_eq!(
            parse_args(&args(&["ratchet", "dl"])),
            Some(Verb::Ratchet(PathBuf::from("dl")))
        );
        for bad in [
            &["bogus"][..],
            &["emit", "x"],
            &["ratchet"],
            &["ratchet", "a", "b"],
            &["Emit"],
        ] {
            assert_eq!(parse_args(&args(bad)), None, "{bad:?}");
        }
    }

    /// The gate's coverage step needs cargo-llvm-cov wherever the suite runs
    /// (CI installs it before the gate), so the real probe is tested against
    /// the real tool: a stand-in can only show the failing half (xtask
    /// mutation round 1, 2026-10-20).
    #[test]
    fn the_real_probe_names_the_installed_tool() {
        let tool = Real::from_env().tool();
        assert!(
            tool.as_deref()
                .is_some_and(|line| line.starts_with("cargo-llvm-cov ")),
            "cargo-llvm-cov must be installed for this suite: {INSTALL} (got {tool:?})"
        );
    }

    #[test]
    fn the_real_harness_prepares_reads_and_walks_the_disk() {
        let scratch =
            std::env::temp_dir().join(format!("ephesus-coverage-test-{}", std::process::id()));
        let report = scratch.join("nested").join("lcov.info");
        let mut real = Real::from_env();
        real.prepare(&report).unwrap();
        assert!(
            report.parent().unwrap().is_dir(),
            "the report's directory is created"
        );
        std::fs::create_dir_all(&report).unwrap();
        assert!(
            real.prepare(&report)
                .unwrap_err()
                .contains("cannot delete the previous report")
        );
        // Not `fixtures/coverage/`: the TypeScript tree's `.gitignore` ignores
        // every directory named `coverage`, so a fixture there is never committed
        // (found by the Linux run, which copies only what git would commit).
        let fixtures = workspace_root()
            .join("xtask")
            .join("tests")
            .join("fixtures")
            .join("json-walk");
        let json = fixtures.join("deep").join("m.json");
        assert!(real.modified(&json).is_some());
        assert_eq!(real.modified(&fixtures.join("absent")), None);
        assert_eq!(real.read(&json).unwrap().trim(), "{}");
        assert!(
            real.read(&fixtures.join("absent"))
                .unwrap_err()
                .starts_with("cannot read")
        );
        let found = real.json_files(&fixtures).unwrap();
        assert_eq!(
            found.len(),
            1,
            "only the .json file, found in its subdirectory: {found:?}"
        );
        assert_eq!(found[0].0, json);
        assert!(
            real.json_files(&fixtures.join("absent"))
                .unwrap_err()
                .starts_with("cannot read")
        );
        assert_eq!(real.env("EPHESUS_NO_SUCH_VARIABLE"), None);
        assert!(real.env("PATH").is_some());
        let target = real.target_dir(&workspace_root()).unwrap();
        assert!(target.is_absolute(), "{}", target.display());
        assert!(
            real.target_dir(&scratch)
                .unwrap_err()
                .starts_with("cargo metadata failed")
        );
        let members = real.workspace(&workspace_root()).unwrap().members;
        assert!(members.iter().any(|m| m.name == "xtask"));
        assert!(real.workspace(&scratch).is_err());
        assert!(std::fs::remove_dir_all(&scratch).is_ok());
    }
}
