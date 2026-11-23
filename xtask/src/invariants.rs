//! The invariant checks (docs/RUST-PLAN.md §3.2 and §4 item 5; ADR-0041's table).
//!
//! Contract: [`check`] takes a [`Workspace`] and returns every violation, each
//! naming its file, its line and the rule, with a message that says what to do
//! instead. It reads nothing and writes nothing. The rules:
//!
//! - **`lints`**: every member opts into the workspace lints
//!   (`[lints] workspace = true`), so invariants 1 and 7 reach it.
//! - **`dep-shape`**: the crates RUST-PLAN §3.2 constrains depend only on what
//!   it allows (`eph-ui` reaches the core through `eph-api` alone, and so on).
//! - **`no-io`**: those crates' sources name no `std::fs`, `std::net` or
//!   `std::process` path, and `eph-ui` names no `alacritty_terminal::tty`.
//! - **`exemption`**: a disallowed-method exemption is an `#[expect]` (it fails
//!   once the call it excuses is gone), carries a `reason` that begins with the
//!   category it claims (`process:`, `git:` or `fs-write:`), sits on a fn, an
//!   impl, a module or a file so the rule can see what it covers, claims only a
//!   category the crate's policy allows, and covers only calls of that category.
//! - **`git`**: only a crate allowed the `git` category starts git (invariant 4,
//!   the single committer). Seen: a `Command::new` whose program is a string
//!   literal naming git, a same-file `const` or `static` holding one, the binary
//!   by path, a `use … Command as Alias`, and a shell (`sh`, `cmd`, `pwsh`, …)
//!   handed a `git …` string, literally, in an array, or in a macro such as
//!   `vec!` or `format!`. Not seen, and recorded as limits: a program held in a
//!   variable or returned by a function, a program built by `format!`, an
//!   environment value, and code a macro generates.
//! - **`parse`**: a source the rules cannot parse fails. A rule that cannot read
//!   a file must not pass it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::workspace::{DepKind, Member, Source, Workspace};

/// What a disallowed-method exemption may excuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// Starting a process other than git.
    Process,
    /// Starting git: the single committer alone (invariant 4).
    Git,
    /// Writing a file directly instead of through `eph_fs` (invariant 3).
    FsWrite,
}

impl Category {
    /// The tag a `reason` begins with.
    pub fn tag(self) -> &'static str {
        match self {
            Category::Process => "process",
            Category::Git => "git",
            Category::FsWrite => "fs-write",
        }
    }

    fn from_reason(reason: &str) -> Option<Category> {
        let (tag, _) = reason.split_once(':')?;
        [Category::Process, Category::Git, Category::FsWrite]
            .into_iter()
            .find(|category| category.tag() == tag.trim())
    }
}

/// What RUST-PLAN §3.2 allows one crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// The crate.
    pub name: &'static str,
    /// The workspace crates it may depend on (`[dependencies]` and
    /// `[build-dependencies]`); `None` means any.
    pub workspace_deps: Option<&'static [&'static str]>,
    /// The external crates it may depend on (same tables); `None` means any
    /// approved crate (`cargo xtask deps` holds the approved set).
    pub external_deps: Option<&'static [&'static str]>,
    /// Whether its sources may name `std::fs`, `std::net` or `std::process`.
    pub io: bool,
    /// The exemption categories it may claim.
    pub exemptions: &'static [Category],
    /// Path prefixes its sources must not name.
    pub banned_paths: &'static [&'static str],
}

/// The pure crates' external dependencies: data types, no I/O.
const PURE_EXTERNALS: &[&str] = &["serde", "serde_json", "thiserror", "jiff", "uuid"];

/// RUST-PLAN §3.2 as data. A crate that is not listed gets [`DEFAULT_POLICY`].
/// The table names crates before they exist (RUST-PLAN §3.1 creates each at its
/// phase), so a rule is in force from the commit that adds its crate.
pub const POLICIES: &[Policy] = &[
    Policy {
        name: "eph-schema",
        workspace_deps: Some(&[]),
        external_deps: Some(PURE_EXTERNALS),
        io: false,
        exemptions: &[],
        banned_paths: &[],
    },
    Policy {
        name: "eph-api",
        workspace_deps: Some(&["eph-schema"]),
        external_deps: Some(PURE_EXTERNALS),
        io: false,
        exemptions: &[],
        banned_paths: &[],
    },
    Policy {
        name: "eph-ui",
        workspace_deps: Some(&["eph-api"]),
        external_deps: Some(&["egui", "eframe", "alacritty_terminal"]),
        io: false,
        exemptions: &[],
        banned_paths: &["alacritty_terminal::tty"],
    },
    Policy {
        name: "eph-fs",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::FsWrite],
        banned_paths: &[],
    },
    Policy {
        name: "eph-git",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Git, Category::Process],
        banned_paths: &[],
    },
    Policy {
        name: "eph-pty",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Process],
        banned_paths: &[],
    },
    Policy {
        name: "eph-engines",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Process],
        banned_paths: &[],
    },
    Policy {
        name: "eph-library",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Process],
        banned_paths: &[],
    },
    Policy {
        name: "eph-harbor",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Process],
        banned_paths: &[],
    },
    Policy {
        name: "xtask",
        workspace_deps: None,
        external_deps: None,
        io: true,
        exemptions: &[Category::Process],
        banned_paths: &[],
    },
];

/// The policy of a crate RUST-PLAN §3.2 does not name: any dependency the
/// approved set allows, I/O allowed, no exemption.
pub const DEFAULT_POLICY: Policy = Policy {
    name: "",
    workspace_deps: None,
    external_deps: None,
    io: true,
    exemptions: &[],
    banned_paths: &[],
};

/// The policy for `name`.
pub fn policy(name: &str) -> Policy {
    POLICIES
        .iter()
        .copied()
        .find(|policy| policy.name == name)
        .unwrap_or(DEFAULT_POLICY)
}

/// One broken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The rule's name (see the module docs).
    pub rule: &'static str,
    /// The file it is in.
    pub file: PathBuf,
    /// Its 1-based line (1 when the rule concerns the whole file).
    pub line: usize,
    /// What is wrong and what to do instead.
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: [{}] {}",
            self.file.display(),
            self.line,
            self.rule,
            self.message
        )
    }
}

/// Every violation in `workspace`, in member order.
pub fn check(workspace: &Workspace) -> Vec<Violation> {
    let mut out = Vec::new();
    for member in &workspace.members {
        let policy = policy(&member.name);
        lints(member, &mut out);
        dep_shape(member, &policy, &mut out);
        for source in &member.sources {
            match syn::parse_file(&source.text) {
                Ok(file) => sources(member, &policy, source, &file, &mut out),
                Err(error) => out.push(Violation {
                    rule: "parse",
                    file: source.path.clone(),
                    line: error.span().start().line.max(1),
                    message: format!(
                        "cannot parse this file ({error}), so no rule can read it; a file the checks cannot read fails"
                    ),
                }),
            }
        }
    }
    out
}

fn lints(member: &Member, out: &mut Vec<Violation>) {
    let opted_in = match member.manifest.parse::<toml::Table>() {
        Ok(table) => {
            table
                .get("lints")
                .and_then(|lints| lints.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true)
        }
        Err(error) => {
            out.push(Violation {
                rule: "lints",
                file: member.manifest_path.clone(),
                line: 1,
                message: format!("cannot parse this manifest: {error}"),
            });
            return;
        }
    };
    if !opted_in {
        out.push(Violation {
            rule: "lints",
            file: member.manifest_path.clone(),
            line: 1,
            message: format!(
                "`{}` does not opt into the workspace lints, so invariants 1 and 7 do not reach it. \
                 Add `[lints]` with `workspace = true` (ADR-0041's invariant table)",
                member.name
            ),
        });
    }
}

fn dep_shape(member: &Member, policy: &Policy, out: &mut Vec<Violation>) {
    for dep in &member.deps {
        if dep.kind == DepKind::Dev {
            continue;
        }
        let (allowed, what) = if dep.member {
            (policy.workspace_deps, "of this workspace")
        } else {
            (policy.external_deps, "outside this workspace")
        };
        let Some(allowed) = allowed else { continue };
        if allowed.contains(&dep.name.as_str()) {
            continue;
        }
        let list = if allowed.is_empty() {
            "nothing".to_string()
        } else {
            allowed.join(", ")
        };
        out.push(Violation {
            rule: "dep-shape",
            file: member.manifest_path.clone(),
            line: manifest_line(&member.manifest, &dep.name),
            message: format!(
                "`{}` may depend only on {list} {what} (RUST-PLAN §3.2), but {} names `{}`",
                member.name,
                dep.kind.table(),
                dep.name
            ),
        });
    }
}

/// The 1-based line of the manifest that declares `name`, or 1.
pub(crate) fn manifest_line(manifest: &str, name: &str) -> usize {
    manifest
        .lines()
        .position(|line| {
            let line = line.trim_start();
            line.strip_prefix(name)
                .is_some_and(|rest| rest.trim_start().starts_with(['=', '.']))
                || line.ends_with(&format!(".{name}]"))
        })
        .map_or(1, |index| index + 1)
}

fn sources(
    member: &Member,
    policy: &Policy,
    source: &Source,
    file: &syn::File,
    out: &mut Vec<Violation>,
) {
    let path = &source.path;
    let mut banned: Vec<&str> = policy.banned_paths.to_vec();
    if !policy.io {
        banned.extend(["std::fs", "std::net", "std::process"]);
    }
    if !banned.is_empty() {
        let mut paths = Paths::default();
        paths.visit_file(file);
        for (named, line) in paths.found {
            if let Some(prefix) = banned
                .iter()
                .find(|prefix| starts_with_path(&named, prefix))
            {
                let why = if policy.banned_paths.contains(prefix) {
                    "is banned in this crate (RUST-PLAN §3.2)"
                } else {
                    "is I/O, and this crate does none (RUST-PLAN §3.2): the core does it, behind eph-api"
                };
                out.push(Violation {
                    rule: "no-io",
                    file: path.clone(),
                    line,
                    message: format!("`{}` names `{named}`, which {why}", member.name),
                });
            }
        }
    }
    exemptions(member, policy, path, file, out);
    if !policy.exemptions.contains(&Category::Git) {
        let mut calls = Calls::new(file);
        calls.visit_file(file);
        for line in calls.git {
            out.push(Violation {
                rule: "git",
                file: path.clone(),
                line,
                message: format!(
                    "`{}` starts git, and only the single committer may (invariant 4, ADR-0004): \
                     commit through eph-git instead",
                    member.name
                ),
            });
        }
    }
}

fn starts_with_path(named: &str, prefix: &str) -> bool {
    named == prefix || named.starts_with(&format!("{prefix}::"))
}

/// Every path a file names, flattened to `a::b::c`, with its line. Use trees are
/// expanded, so `use std::{fs, io}` yields `std::fs` and `std::io`.
#[derive(Default)]
struct Paths {
    found: Vec<(String, usize)>,
}

impl<'ast> Visit<'ast> for Paths {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let named = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        self.found.push((named, line_of(path.span())));
        syn::visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut flat = Vec::new();
        flatten_use(&item.tree, String::new(), &mut flat);
        let line = line_of(item.span());
        self.found
            .extend(flat.into_iter().map(|(named, _)| (named, line)));
    }
}

/// Flattens a use tree into `(full path, local name)` pairs.
fn flatten_use(tree: &syn::UseTree, prefix: String, out: &mut Vec<(String, String)>) {
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}::{name}")
        }
    };
    match tree {
        syn::UseTree::Path(path) => flatten_use(&path.tree, join(&path.ident.to_string()), out),
        syn::UseTree::Name(name) => {
            out.push((join(&name.ident.to_string()), name.ident.to_string()))
        }
        syn::UseTree::Rename(rename) => {
            out.push((join(&rename.ident.to_string()), rename.rename.to_string()));
        }
        syn::UseTree::Glob(_) => out.push((join("*"), String::new())),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix.clone(), out);
            }
        }
    }
}

fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line.max(1)
}

/// The disallowed-lint exemptions in an attribute list.
struct Exemption {
    line: usize,
    /// `true` for `expect`, `false` for `allow`.
    expect: bool,
    /// `disallowed_methods` (true) or `disallowed_types` (false).
    methods: bool,
    reason: Option<String>,
}

fn exemptions_in(attrs: &[syn::Attribute]) -> Vec<Exemption> {
    let mut out = Vec::new();
    for attr in attrs {
        exemptions_in_meta(&attr.meta, line_of(attr.span()), &mut out);
    }
    out
}

/// The exemptions one attribute's meta declares. `cfg_attr(pred, a, b)` is
/// opened, so `#[cfg_attr(test, allow(clippy::disallowed_methods))]` is seen.
/// A meta that does not parse is the compiler's to refuse; it cannot hide an
/// exemption from this rule, because rustc rejects the attribute first.
fn exemptions_in_meta(meta: &syn::Meta, line: usize, out: &mut Vec<Exemption>) {
    let syn::Meta::List(list) = meta else { return };
    let Ok(nested) = list.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    ) else {
        return;
    };
    if list.path.is_ident("cfg_attr") {
        for inner in nested.iter().skip(1) {
            exemptions_in_meta(inner, line, out);
        }
        return;
    }
    let expect = list.path.is_ident("expect");
    if !expect && !list.path.is_ident("allow") {
        return;
    }
    let reason = nested.iter().find_map(|inner| match inner {
        syn::Meta::NameValue(pair) if pair.path.is_ident("reason") => string_literal(&pair.value),
        _ => None,
    });
    for inner in &nested {
        let syn::Meta::Path(path) = inner else {
            continue;
        };
        let methods = match segments(path).join("::").as_str() {
            "clippy::disallowed_methods" => true,
            "clippy::disallowed_types" => false,
            _ => continue,
        };
        out.push(Exemption {
            line,
            expect,
            methods,
            reason: reason.clone(),
        });
    }
}

fn exemptions(
    member: &Member,
    policy: &Policy,
    path: &Path,
    file: &syn::File,
    out: &mut Vec<Violation>,
) {
    // Every exemption anywhere in the file, by line.
    let mut all = AllExemptions::default();
    all.visit_file(file);
    // The ones at a position the rule can judge, each with what it covers.
    let mut placed = PlacedExemptions::new(file);
    placed.visit_file(file);
    let placed_lines: BTreeSet<usize> = placed.found.iter().map(|(e, _)| e.line).collect();
    for line in all.lines.difference(&placed_lines) {
        out.push(Violation {
            rule: "exemption",
            file: path.to_path_buf(),
            line: *line,
            message: "put a disallowed-lint exemption on the enclosing fn, impl, module or file, \
                      where this rule can see every call it covers"
                .to_string(),
        });
    }
    for (exemption, covered) in &placed.found {
        let mut fail = |message: String| {
            out.push(Violation {
                rule: "exemption",
                file: path.to_path_buf(),
                line: exemption.line,
                message,
            });
        };
        if !exemption.expect {
            fail("use `#[expect(..., reason = \"<category>: ...\")]`, not `#[allow]`: an expect fails \
                  the build once the call it excuses is gone, so the exemption cannot outlive it"
                .to_string());
            continue;
        }
        let Some(reason) = &exemption.reason else {
            fail(
                "an exemption carries a `reason` naming the category it claims and why".to_string(),
            );
            continue;
        };
        if !exemption.methods {
            continue;
        }
        let Some(category) = Category::from_reason(reason) else {
            fail(format!(
                "the reason must begin with the category it claims (`process:`, `git:` or `fs-write:`); it reads \"{reason}\""
            ));
            continue;
        };
        if !policy.exemptions.contains(&category) {
            fail(format!(
                "`{}` may not claim `{}:` (RUST-PLAN §3.2 allows it: {})",
                member.name,
                category.tag(),
                if policy.exemptions.is_empty() {
                    "none".to_string()
                } else {
                    policy
                        .exemptions
                        .iter()
                        .map(|c| format!("`{}:`", c.tag()))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
            continue;
        }
        if covered.fs_write && category != Category::FsWrite {
            fail(format!(
                "this `{}:` exemption also covers a direct file write; a write belongs in eph-fs under `fs-write:`",
                category.tag()
            ));
        }
        if covered.process && category == Category::FsWrite {
            fail(
                "this `fs-write:` exemption also covers a process start, which needs `process:`"
                    .to_string(),
            );
        }
    }
}

/// The lines of every disallowed-lint exemption attribute in a file.
#[derive(Default)]
struct AllExemptions {
    lines: BTreeSet<usize>,
}

impl<'ast> Visit<'ast> for AllExemptions {
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.lines.extend(
            exemptions_in(std::slice::from_ref(attr))
                .into_iter()
                .map(|exemption| exemption.line),
        );
    }
}

/// What the item under an exemption does that the disallowed lints refuse.
#[derive(Debug, Default, Clone, Copy)]
struct Covered {
    process: bool,
    fs_write: bool,
}

/// Exemptions on fns, impls, modules and the file, each with what it covers.
struct PlacedExemptions {
    found: Vec<(Exemption, Covered)>,
    aliases: BTreeSet<String>,
}

impl PlacedExemptions {
    fn new(file: &syn::File) -> Self {
        PlacedExemptions {
            found: Vec::new(),
            aliases: command_aliases(file),
        }
    }

    fn record(&mut self, attrs: &[syn::Attribute], visit: impl FnOnce(&mut Coverage)) {
        let found = exemptions_in(attrs);
        if found.is_empty() {
            return;
        }
        let mut coverage = Coverage {
            covered: Covered::default(),
            aliases: &self.aliases,
        };
        visit(&mut coverage);
        let covered = coverage.covered;
        self.found
            .extend(found.into_iter().map(|exemption| (exemption, covered)));
    }
}

impl<'ast> Visit<'ast> for PlacedExemptions {
    fn visit_file(&mut self, file: &'ast syn::File) {
        self.record(&file.attrs, |c| c.visit_file(file));
        for item in &file.items {
            self.visit_item(item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.record(&item.attrs, |c| c.visit_item_fn(item));
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.record(&item.attrs, |c| c.visit_item_impl(item));
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.record(&item.attrs, |c| c.visit_impl_item_fn(item));
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.record(&item.attrs, |c| c.visit_item_mod(item));
        syn::visit::visit_item_mod(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.record(&item.attrs, |c| c.visit_trait_item_fn(item));
        syn::visit::visit_trait_item_fn(self, item);
    }
}

/// Finds the calls the disallowed lints refuse, by name.
struct Coverage<'a> {
    covered: Covered,
    aliases: &'a BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Coverage<'_> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(callee) = &*call.func {
            let segments = segments(&callee.path);
            if is_command_new(&segments, self.aliases) {
                self.covered.process = true;
            }
            if [
                &["fs", "write"][..],
                &["File", "create"],
                &["File", "create_new"],
                &["fs", "copy"],
            ]
            .iter()
            .any(|tail| ends_with(&segments, tail))
            {
                self.covered.fs_write = true;
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    /// `.truncate(..)` and any `.open(..)` method count as writes. `File::open`
    /// is a path call, not a method, so a reading open is not caught here; an
    /// `OpenOptions` held in a variable is, which a check on the builder chain
    /// would miss. A false refusal costs a message; a false pass costs
    /// invariant 3.
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "truncate" || call.method == "open" {
            self.covered.fs_write = true;
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn ends_with(segments: &[String], tail: &[&str]) -> bool {
    segments.len() >= tail.len()
        && segments[segments.len() - tail.len()..]
            .iter()
            .zip(tail)
            .all(|(a, b)| a == b)
}

/// `Command::new`, `std::process::Command::new`, `tokio::process::Command::new`,
/// or `Alias::new` for a `use … Command as Alias` in the same file.
fn is_command_new(segments: &[String], aliases: &BTreeSet<String>) -> bool {
    match segments {
        [.., ty, new] if new == "new" => ty == "Command" || aliases.contains(ty),
        _ => false,
    }
}

/// The local names `Command` is imported under (`use … Command as Alias`).
fn command_aliases(file: &syn::File) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in &file.items {
        if let syn::Item::Use(item) = item {
            let mut flat = Vec::new();
            flatten_use(&item.tree, String::new(), &mut flat);
            for (path, local) in flat {
                if path.ends_with("::Command") {
                    out.insert(local);
                }
            }
        }
    }
    out
}

/// Whether a program name is git: `git` or `git.exe`, by path or not, in any
/// case (Windows resolves `GIT` to git).
fn names_git(program: &str) -> bool {
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    base.eq_ignore_ascii_case("git") || base.eq_ignore_ascii_case("git.exe")
}

/// Whether a shell command string starts git as a word.
fn shell_runs_git(text: &str) -> bool {
    text.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | '`'))
        .any(names_git)
}

const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "cmd",
    "cmd.exe",
    "powershell",
    "pwsh",
];

/// The git starts in a file (see the module docs for what is and is not seen).
struct Calls {
    aliases: BTreeSet<String>,
    /// Same-file `const`/`static` string values by name.
    consts: Vec<(String, String)>,
    git: Vec<usize>,
}

impl Calls {
    fn new(file: &syn::File) -> Self {
        let mut consts = Vec::new();
        for item in &file.items {
            let (ident, expr) = match item {
                syn::Item::Const(item) => (&item.ident, &*item.expr),
                syn::Item::Static(item) => (&item.ident, &*item.expr),
                _ => continue,
            };
            if let Some(value) = string_literal(expr) {
                consts.push((ident.to_string(), value));
            }
        }
        Calls {
            aliases: command_aliases(file),
            consts,
            git: Vec::new(),
        }
    }

    /// The program a `Command::new` argument names, when the rule can see it.
    fn program(&self, arg: &syn::Expr) -> Option<String> {
        if let Some(value) = string_literal(arg) {
            return Some(value);
        }
        let syn::Expr::Path(path) = strip_ref(arg) else {
            return None;
        };
        let name = path.path.get_ident()?.to_string();
        self.consts
            .iter()
            .find(|(ident, _)| *ident == name)
            .map(|(_, value)| value.clone())
    }

    /// The program of the `Command::new` at the root of a builder chain.
    fn root_program(&self, mut expr: &syn::Expr) -> Option<String> {
        loop {
            match expr {
                syn::Expr::MethodCall(call) => expr = &call.receiver,
                syn::Expr::Paren(inner) => expr = &inner.expr,
                syn::Expr::Call(call) => {
                    let syn::Expr::Path(callee) = &*call.func else {
                        return None;
                    };
                    if !is_command_new(&segments(&callee.path), &self.aliases) {
                        return None;
                    }
                    return call.args.first().and_then(|arg| self.program(arg));
                }
                _ => return None,
            }
        }
    }
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(callee) = &*call.func
            && is_command_new(&segments(&callee.path), &self.aliases)
            && call
                .args
                .first()
                .and_then(|arg| self.program(arg))
                .is_some_and(|program| names_git(&program))
        {
            self.git.push(line_of(call.span()));
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if (call.method == "arg" || call.method == "args")
            && self
                .root_program(&call.receiver)
                .is_some_and(|program| SHELLS.contains(&program.as_str()))
        {
            let mut texts = Vec::new();
            for arg in &call.args {
                collect_strings(arg, &mut texts);
            }
            if texts.iter().any(|text| shell_runs_git(text)) {
                self.git.push(line_of(call.span()));
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn strip_ref(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Reference(reference) => strip_ref(&reference.expr),
        syn::Expr::Paren(paren) => strip_ref(&paren.expr),
        other => other,
    }
}

fn string_literal(expr: &syn::Expr) -> Option<String> {
    match strip_ref(expr) {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}

/// String literals in an argument: the literal itself, or those in an array or
/// in a macro's comma-separated body (`vec!`, `format!`).
fn collect_strings(expr: &syn::Expr, out: &mut Vec<String>) {
    match strip_ref(expr) {
        syn::Expr::Array(array) => {
            for element in &array.elems {
                collect_strings(element, out);
            }
        }
        // Any macro whose body is a comma list: `vec![..]`, and `format!(..)`,
        // whose literal is the command's text.
        syn::Expr::Macro(mac) => {
            if let Ok(array) = mac.mac.parse_body_with(
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
            ) {
                for element in &array {
                    collect_strings(element, out);
                }
            }
        }
        other => out.extend(string_literal(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Dep;

    fn member(name: &str, manifest: &str, files: &[(&str, &str)]) -> Member {
        Member {
            name: name.to_string(),
            manifest_path: PathBuf::from(format!("crates/{name}/Cargo.toml")),
            manifest: manifest.to_string(),
            deps: Vec::new(),
            sources: files
                .iter()
                .map(|(path, text)| Source {
                    path: PathBuf::from(path),
                    text: (*text).to_string(),
                })
                .collect(),
        }
    }

    const OPTED_IN: &str = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n";

    fn run(members: Vec<Member>) -> Vec<Violation> {
        check(&Workspace { members })
    }

    fn rules(violations: &[Violation]) -> Vec<(&'static str, usize)> {
        violations.iter().map(|v| (v.rule, v.line)).collect()
    }

    // ---- lints ----------------------------------------------------------

    #[test]
    fn a_crate_that_opts_into_the_workspace_lints_passes() {
        assert_eq!(run(vec![member("eph-agora", OPTED_IN, &[])]), vec![]);
        let dotted = "lints.workspace = true\n\n[package]\nname = \"x\"\n";
        assert_eq!(run(vec![member("eph-agora", dotted, &[])]), vec![]);
    }

    #[test]
    fn a_crate_without_the_workspace_lints_fails_naming_the_fix() {
        for manifest in [
            "[package]\nname = \"x\"\n",
            "[package]\nname = \"x\"\n[lints]\nworkspace = false\n",
            "[package]\nname = \"x\"\n[lints.clippy]\nunwrap_used = \"allow\"\n",
            // Under [package] this key is `package.lints.workspace`, which cargo
            // ignores: the line reads right and opts into nothing. A text match
            // would pass it (found when this test's first fixture did exactly that).
            "[package]\nname = \"x\"\nlints.workspace = true\n",
        ] {
            let violations = run(vec![member("eph-agora", manifest, &[])]);
            assert_eq!(rules(&violations), [("lints", 1)], "{manifest}");
            assert!(violations[0].message.contains("workspace = true"));
        }
    }

    #[test]
    fn an_unparseable_manifest_fails() {
        let violations = run(vec![member("eph-agora", "[package\n", &[])]);
        assert_eq!(rules(&violations), [("lints", 1)]);
        assert!(violations[0].message.contains("cannot parse"));
    }

    // ---- dep-shape ------------------------------------------------------

    fn dep(name: &str, kind: DepKind, member: bool) -> Dep {
        Dep {
            name: name.to_string(),
            kind,
            member,
            path: member,
        }
    }

    #[test]
    fn the_window_may_reach_only_eph_api_and_its_ui_crates() {
        let manifest = format!(
            "{OPTED_IN}\n[dependencies]\neph-api.workspace = true\negui = \"0.37\"\neph-fs = {{ path = \"../eph-fs\" }}\ntokio = \"1\"\n\n[dev-dependencies]\negui_kittest = \"0.37\"\n"
        );
        let mut ui = member("eph-ui", &manifest, &[]);
        ui.deps = vec![
            dep("eph-api", DepKind::Normal, true),
            dep("egui", DepKind::Normal, false),
            dep("eph-fs", DepKind::Normal, true),
            dep("tokio", DepKind::Normal, false),
            dep("egui_kittest", DepKind::Dev, false),
        ];
        let violations = run(vec![ui]);
        assert_eq!(rules(&violations), [("dep-shape", 10), ("dep-shape", 11)]);
        assert!(violations[0].message.contains("`eph-fs`"));
        assert!(violations[0].message.contains("eph-api of this workspace"));
        assert!(violations[1].message.contains("`tokio`"));
    }

    #[test]
    fn the_schema_depends_on_no_workspace_crate_and_a_build_dependency_counts() {
        let mut schema = member("eph-schema", OPTED_IN, &[]);
        schema.deps = vec![
            dep("serde", DepKind::Normal, false),
            dep("eph-fs", DepKind::Build, true),
        ];
        let violations = run(vec![schema]);
        assert_eq!(rules(&violations), [("dep-shape", 1)]);
        assert!(
            violations[0]
                .message
                .contains("depend only on nothing of this workspace")
        );
        assert!(violations[0].message.contains("[build-dependencies]"));
    }

    #[test]
    fn an_unconstrained_crate_may_depend_on_anything_approved() {
        let mut agora = member("eph-agora", OPTED_IN, &[]);
        agora.deps = vec![
            dep("eph-fs", DepKind::Normal, true),
            dep("tokio", DepKind::Normal, false),
        ];
        assert_eq!(run(vec![agora]), vec![]);
    }

    #[test]
    fn manifest_line_finds_each_declaration_form() {
        let manifest = "[dependencies]\nserde = \"1\"\nsyn.workspace = true\nserde_json = { version = \"1\" }\n\n[dependencies.toml]\nversion = \"1\"\n";
        assert_eq!(manifest_line(manifest, "serde"), 2);
        assert_eq!(manifest_line(manifest, "syn"), 3);
        assert_eq!(manifest_line(manifest, "serde_json"), 4);
        assert_eq!(manifest_line(manifest, "toml"), 6);
        assert_eq!(manifest_line(manifest, "absent"), 1);
    }

    // ---- no-io ----------------------------------------------------------

    #[test]
    fn a_pure_crate_naming_io_fails_at_each_line() {
        let src = "use std::{fs, io};\nuse std::net::TcpStream as T;\nfn f() {\n    let _ = ::std::process::id();\n    let _ = std::io::stdout();\n}\n";
        let violations = run(vec![member(
            "eph-api",
            OPTED_IN,
            &[("crates/eph-api/src/lib.rs", src)],
        )]);
        assert_eq!(
            rules(&violations),
            [("no-io", 1), ("no-io", 2), ("no-io", 4)]
        );
        assert!(violations[0].message.contains("`std::fs`"));
        assert!(violations[1].message.contains("`std::net::TcpStream`"));
        assert!(violations[2].message.contains("`std::process::id`"));
    }

    #[test]
    fn the_window_may_not_use_the_emulators_own_pty() {
        let src = "use alacritty_terminal::tty::Pty;\nuse alacritty_terminal::term::Term;\n";
        let violations = run(vec![member(
            "eph-ui",
            OPTED_IN,
            &[("crates/eph-ui/src/lib.rs", src)],
        )]);
        assert_eq!(rules(&violations), [("no-io", 1)]);
        assert!(violations[0].message.contains("banned in this crate"));
    }

    #[test]
    fn an_io_crate_may_name_io() {
        let src = "use std::fs;\nfn f() { let _ = std::process::id(); }\n";
        assert_eq!(
            run(vec![member(
                "eph-agora",
                OPTED_IN,
                &[("crates/eph-agora/src/lib.rs", src)]
            )]),
            vec![]
        );
    }

    // ---- exemption ------------------------------------------------------

    fn exemption_violations(crate_name: &str, src: &str) -> Vec<Violation> {
        run(vec![member(crate_name, OPTED_IN, &[("src/lib.rs", src)])])
    }

    #[test]
    fn a_tagged_expect_in_an_allowed_crate_passes() {
        let src = "#[expect(clippy::disallowed_methods, reason = \"process: runs cargo\")]\nfn f() { let _ = std::process::Command::new(\"cargo\"); }\n";
        assert_eq!(exemption_violations("xtask", src), vec![]);
        let fs = "#[expect(clippy::disallowed_methods, reason = \"fs-write: the one write path\")]\nfn w() { let _ = std::fs::write(\"a\", \"b\"); }\n";
        assert_eq!(exemption_violations("eph-fs", fs), vec![]);
    }

    #[test]
    fn an_allow_is_refused_because_it_can_outlive_its_call() {
        let src = "#[allow(clippy::disallowed_methods)]\nfn f() {}\n";
        let violations = exemption_violations("xtask", src);
        assert_eq!(rules(&violations), [("exemption", 1)]);
        assert!(violations[0].message.contains("not `#[allow]`"));
    }

    #[test]
    fn an_expect_without_a_reason_or_without_a_category_is_refused() {
        let bare = "#[expect(clippy::disallowed_methods)]\nfn f() {}\n";
        let v = exemption_violations("xtask", bare);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("carries a `reason`"));
        let untagged =
            "#[expect(clippy::disallowed_methods, reason = \"runs cargo\")]\nfn f() {}\n";
        let v = exemption_violations("xtask", untagged);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("must begin with the category"));
    }

    #[test]
    fn a_crate_may_claim_only_the_categories_its_policy_allows() {
        let src = "#[expect(clippy::disallowed_methods, reason = \"process: runs a tool\")]\nfn f() { let _ = std::process::Command::new(\"tool\"); }\n";
        let v = exemption_violations("eph-agora", src);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("may not claim `process:`"));
        assert!(v[0].message.contains("none"));
        let git = src.replace("process:", "git:");
        let v = exemption_violations("eph-pty", &git);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("`process:`"));
    }

    #[test]
    fn a_process_exemption_may_not_also_cover_a_file_write() {
        let src = "#[expect(clippy::disallowed_methods, reason = \"git: commits\")]\nfn f() {\n    let _ = std::process::Command::new(\"git\");\n    let _ = std::fs::write(\"x\", \"y\");\n}\n";
        let v = exemption_violations("eph-git", src);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("direct file write"));
        let truncating = "#[expect(clippy::disallowed_methods, reason = \"fs-write: w\")]\nfn f(o: &mut std::fs::OpenOptions) { o.truncate(true); let _ = std::process::Command::new(\"x\"); }\n";
        let v = exemption_violations("eph-fs", truncating);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("needs `process:`"));
    }

    #[test]
    fn an_exemption_on_a_statement_is_refused_because_its_reach_is_unseen() {
        let src = "fn f() {\n    #[expect(clippy::disallowed_methods, reason = \"process: x\")]\n    let _c = std::process::Command::new(\"x\");\n}\n";
        let v = exemption_violations("xtask", src);
        assert_eq!(rules(&v), [("exemption", 2)]);
        assert!(v[0].message.contains("enclosing fn"));
    }

    #[test]
    fn a_file_level_and_an_impl_level_exemption_are_judged_too() {
        let file_level = "#![expect(clippy::disallowed_methods, reason = \"fs-write: x\")]\nfn f() { let _ = std::process::Command::new(\"x\"); }\n";
        let v = exemption_violations("eph-fs", file_level);
        assert_eq!(rules(&v), [("exemption", 1)]);
        let impl_level = "struct S;\n#[expect(clippy::disallowed_methods, reason = \"process: x\")]\nimpl S { fn f() { let _ = std::fs::File::create(\"x\"); } }\n";
        let v = exemption_violations("xtask", impl_level);
        assert_eq!(rules(&v), [("exemption", 2)]);
        assert!(v[0].message.contains("direct file write"));
    }

    #[test]
    fn an_exemption_inside_cfg_attr_is_still_judged() {
        let src = "#[cfg_attr(all(test, unix), allow(clippy::disallowed_methods))]\nfn f() {}\n";
        let v = exemption_violations("xtask", src);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("not `#[allow]`"));
        let nested = "#[cfg_attr(test, cfg_attr(unix, expect(clippy::disallowed_methods, reason = \"process: x\")))]\nfn f() { let _ = std::fs::write(\"a\", \"b\"); }\n";
        let v = exemption_violations("xtask", nested);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("direct file write"));
    }

    #[test]
    fn an_unrelated_allow_is_not_an_exemption() {
        let src = "#[allow(dead_code)]\nfn f() {}\n#[expect(clippy::too_many_lines)]\nfn g() {}\n";
        assert_eq!(exemption_violations("eph-agora", src), vec![]);
    }

    // ---- git: GYM-009's shape catalogue, translated ---------------------

    fn git_lines(crate_name: &str, src: &str) -> Vec<usize> {
        run(vec![member(crate_name, OPTED_IN, &[("src/lib.rs", src)])])
            .into_iter()
            .filter(|v| v.rule == "git")
            .map(|v| v.line)
            .collect()
    }

    #[test]
    fn git_shapes_the_rule_sees() {
        let cases: &[(&str, &str)] = &[
            (
                "a literal",
                "fn f() {\n    let _ = std::process::Command::new(\"git\");\n}\n",
            ),
            (
                "an imported Command",
                "use std::process::Command;\nfn f() {\n    let _ = Command::new(\"git\");\n}\n",
            ),
            (
                "tokio's Command",
                "fn f() {\n    let _ = tokio::process::Command::new(\"git\");\n}\n",
            ),
            (
                "a reference",
                "fn f() {\n    let _ = std::process::Command::new(&\"git\");\n}\n",
            ),
            (
                "a same-file const",
                "const GIT: &str = \"git\";\nfn f() {\n    let _ = std::process::Command::new(GIT);\n}\n",
            ),
            (
                "a same-file static",
                "static GIT: &str = \"git\";\nfn f() {\n    let _ = std::process::Command::new(GIT);\n}\n",
            ),
            (
                "the binary by POSIX path",
                "fn f() {\n    let _ = std::process::Command::new(\"/usr/bin/git\");\n}\n",
            ),
            (
                "the binary by Windows path",
                "fn f() {\n    let _ = std::process::Command::new(r\"C:\\Program Files\\Git\\cmd\\git.exe\");\n}\n",
            ),
            (
                "git.exe",
                "fn f() {\n    let _ = std::process::Command::new(\"GIT.EXE\");\n}\n",
            ),
            (
                "git in capitals",
                "fn f() {\n    let _ = std::process::Command::new(\"GIT\");\n}\n",
            ),
            (
                "an import alias",
                "use std::process::Command as Cmd;\nfn f() {\n    let _ = Cmd::new(\"git\");\n}\n",
            ),
            (
                "a grouped alias",
                "use std::{io, process::{Command as C}};\nfn f() {\n    let _ = C::new(\"git\");\n}\n",
            ),
            (
                "sh -c",
                "fn f() {\n    let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"git commit -m x\");\n}\n",
            ),
            (
                "cmd /C via args",
                "fn f() {\n    let _ = std::process::Command::new(\"cmd\").args([\"/C\", \"git status\"]);\n}\n",
            ),
            (
                "pwsh via vec!",
                "fn f() {\n    let _ = std::process::Command::new(\"pwsh\").args(vec![\"-c\", \"cd x; git push\"]);\n}\n",
            ),
            (
                "a wrapped builder",
                "fn f() {\n    let _ = std::process::Command::new(\n        \"git\",\n    )\n    .arg(\"status\");\n}\n",
            ),
        ];
        for (shape, src) in cases {
            let lines = git_lines("eph-agora", src);
            assert_eq!(
                lines.len(),
                1,
                "{shape}: expected one git start, got {lines:?}"
            );
            let line = lines[0];
            let expected = src
                .lines()
                .position(|l| l.contains("new(") || l.contains("arg"))
                .map(|i| i + 1)
                .unwrap();
            assert!(
                line >= expected - 1 && line <= expected + 1,
                "{shape}: line {line}, expected about {expected}"
            );
        }
    }

    #[test]
    fn git_shapes_the_rule_does_not_see_are_pinned_as_its_limits() {
        // Recorded in the module docs and the implementation doc. If one of
        // these starts being seen, move it to the test above.
        let unseen: &[(&str, &str)] = &[
            (
                "a variable",
                "fn f() {\n    let program = \"git\";\n    let _ = std::process::Command::new(program);\n}\n",
            ),
            (
                "format!",
                "fn f() {\n    let _ = std::process::Command::new(format!(\"g{}\", \"it\"));\n}\n",
            ),
            (
                "a function result",
                "fn program() -> &'static str { \"git\" }\nfn f() {\n    let _ = std::process::Command::new(program());\n}\n",
            ),
            (
                "an environment value",
                "fn f() {\n    let _ = std::process::Command::new(std::env::var(\"GIT\").unwrap());\n}\n",
            ),
        ];
        for (shape, src) in unseen {
            assert_eq!(git_lines("eph-agora", src), Vec::<usize>::new(), "{shape}");
        }
    }

    #[test]
    fn non_git_processes_are_not_git() {
        for src in [
            "fn f() { let _ = std::process::Command::new(\"cargo\"); }\n",
            "fn f() { let _ = std::process::Command::new(\"gitk-viewer\"); }\n",
            "fn f() { let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"echo digit\"); }\n",
            "fn f() { let _ = std::process::Command::new(\"cargo\").arg(\"git\"); }\n",
            "struct Command;\nimpl Command { fn new(_: &str) -> Self { Command } }\nfn f() { let _ = Command::new(\"cargo\"); }\n",
        ] {
            assert_eq!(git_lines("eph-agora", src), Vec::<usize>::new(), "{src}");
        }
    }

    #[test]
    fn the_single_committer_may_start_git() {
        let src = "#[expect(clippy::disallowed_methods, reason = \"git: the single committer\")]\nfn f() { let _ = std::process::Command::new(\"git\"); }\n";
        assert_eq!(
            run(vec![member("eph-git", OPTED_IN, &[("src/lib.rs", src)])]),
            vec![]
        );
    }

    // ---- the round-1 survivors (2026-10-19), each a test that was missing --

    #[test]
    fn only_a_reason_name_value_counts_as_the_reason() {
        let src = "#[expect(clippy::disallowed_methods, note = \"process: x\")]\nfn f() {}\n";
        let v = exemption_violations("xtask", src);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("carries a `reason`"));
    }

    #[test]
    fn a_disallowed_types_exemption_is_judged_too() {
        let allow = "#[allow(clippy::disallowed_types)]\nfn f() {}\n";
        let v = exemption_violations("eph-agora", allow);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("not `#[allow]`"));
        let bare = "#[expect(clippy::disallowed_types)]\nfn f() {}\n";
        assert_eq!(
            rules(&exemption_violations("eph-agora", bare)),
            [("exemption", 1)]
        );
        let reasoned =
            "#[expect(clippy::disallowed_types, reason = \"the validator\")]\nfn f() {}\n";
        assert_eq!(exemption_violations("eph-agora", reasoned), vec![]);
    }

    #[test]
    fn an_exemption_on_a_module_or_a_trait_fn_is_placed() {
        let module = "#[expect(clippy::disallowed_methods, reason = \"process: x\")]\nmod m {\n    fn f() { let _ = std::process::Command::new(\"x\"); }\n}\n";
        assert_eq!(exemption_violations("xtask", module), vec![]);
        let trait_fn = "trait T {\n    #[expect(clippy::disallowed_methods, reason = \"process: x\")]\n    fn f() { let _ = std::process::Command::new(\"x\"); }\n}\n";
        assert_eq!(exemption_violations("xtask", trait_fn), vec![]);
        // And what they cover is judged: a module-wide process exemption may
        // not cover a write.
        let writes = "#[expect(clippy::disallowed_methods, reason = \"process: x\")]\nmod m {\n    fn f() { let _ = std::fs::write(\"a\", \"b\"); }\n}\n";
        assert!(
            exemption_violations("xtask", writes)[0]
                .message
                .contains("direct file write")
        );
    }

    #[test]
    fn every_kind_of_write_under_a_process_exemption_is_refused() {
        for body in [
            "o.truncate(true);",
            "let _ = o.open(\"p\");",
            "let _ = std::fs::copy(\"a\", \"b\");",
            "let _ = fs::write(\"a\", \"b\");",
            "let _ = File::create(\"a\");",
            "let _ = File::create_new(\"a\");",
        ] {
            let src = format!(
                "use std::fs;\nuse std::fs::File;\n#[expect(clippy::disallowed_methods, reason = \"process: x\")]\nfn f(o: &mut fs::OpenOptions) {{ {body} }}\n"
            );
            let v = exemption_violations("xtask", &src);
            assert_eq!(rules(&v), [("exemption", 3)], "{body}");
            assert!(v[0].message.contains("direct file write"), "{body}");
        }
    }

    #[test]
    fn a_process_start_inside_a_builder_chain_is_covered() {
        let src = "#[expect(clippy::disallowed_methods, reason = \"fs-write: w\")]\nfn f() { let _ = std::process::Command::new(\"x\").arg(\"y\").status(); }\n";
        let v = exemption_violations("eph-fs", src);
        assert_eq!(rules(&v), [("exemption", 1)]);
        assert!(v[0].message.contains("needs `process:`"));
    }

    #[test]
    fn git_through_a_parenthesised_builder_or_format_is_seen() {
        for src in [
            "fn f() {\n    let _ = (std::process::Command::new(\"sh\")).arg(\"-c\").arg(\"git push\");\n}\n",
            "fn f(x: &str) {\n    let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"git {}\", x));\n}\n",
        ] {
            assert_eq!(git_lines("eph-agora", src), [2], "{src}");
        }
    }

    #[test]
    fn a_new_or_an_alias_that_is_not_commands_is_not_git() {
        for src in [
            "struct Command;\nimpl Command { fn describe(_: &str) {} }\nfn f() { Command::describe(\"git\"); }\n",
            "use std::fs::File;\nfn f() { let _ = File::new(\"git\"); }\n",
        ] {
            assert_eq!(git_lines("eph-agora", src), Vec::<usize>::new(), "{src}");
        }
    }

    // ---- parse ----------------------------------------------------------

    #[test]
    fn a_file_the_rules_cannot_parse_fails() {
        let violations = run(vec![member(
            "eph-agora",
            OPTED_IN,
            &[("src/lib.rs", "fn f( {\n")],
        )]);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, "parse");
        assert!(violations[0].message.contains("cannot parse"));
    }

    #[test]
    fn a_violation_renders_as_file_line_rule_message() {
        let v = Violation {
            rule: "git",
            file: PathBuf::from("src/lib.rs"),
            line: 7,
            message: "m".to_string(),
        };
        assert_eq!(v.to_string(), "src/lib.rs:7: [git] m");
    }

    #[test]
    fn every_policy_names_a_distinct_crate() {
        let names: BTreeSet<&str> = POLICIES.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), POLICIES.len());
        assert_eq!(policy("not-in-the-table"), DEFAULT_POLICY);
    }
}
