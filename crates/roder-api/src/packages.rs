//! Canonical Roder package contracts (roadmap phase 93).
//!
//! A Roder package is an installable bundle of process extensions, skills,
//! slash commands, and themes fetched from npm, git, or a local path with one
//! command (`roder install npm:@foo/pkg`). These types are the canonical
//! model shared by the fetch/store layer (`roder-config`), the activation
//! layer (`roder-extension-host`), and the app-server protocol surface.
//!
//! Plugin authors publish a `roder.toml` manifest at the root of their
//! repository (or a `roder` key in `package.json`) describing what the
//! package provides and how to launch any process extensions.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Settings file name inside a scope directory (`~/.roder/packages.json` or
/// `<workspace>/.roder/packages.json`).
pub const PACKAGES_SETTINGS_FILE: &str = "packages.json";

/// Canonical package manifest file at the package (repository) root.
pub const PACKAGE_MANIFEST_FILE: &str = "roder.toml";

/// npm keyword recommended for discoverability of Roder packages.
pub const PACKAGE_NPM_KEYWORD: &str = "roder-package";

pub const EVENT_PACKAGE_INSTALLED: &str = "package.installed";
pub const EVENT_PACKAGE_UPDATED: &str = "package.updated";
pub const EVENT_PACKAGE_REMOVED: &str = "package.removed";
pub const EVENT_PACKAGE_RESOURCE_TOGGLED: &str = "package.resource_toggled";
pub const EVENT_PACKAGE_EXTENSIONS_APPROVED: &str = "package.extensions_approved";

/// Where a package source was fetched from. The canonical spec string
/// round-trips through [`parse_package_spec`] and [`PackageSource::spec`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PackageSource {
    Npm {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Git {
        url: String,
        #[serde(rename = "refName")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
    },
    LocalPath {
        path: String,
    },
}

impl PackageSource {
    /// Canonical spec string used in CLI output and settings files.
    pub fn spec(&self) -> String {
        match self {
            PackageSource::Npm { name, version } => match version {
                Some(version) => format!("npm:{name}@{version}"),
                None => format!("npm:{name}"),
            },
            PackageSource::Git { url, ref_name } => match ref_name {
                Some(ref_name) => format!("git:{url}@{ref_name}"),
                None => format!("git:{url}"),
            },
            PackageSource::LocalPath { path } => path.clone(),
        }
    }

    /// Stable identity: npm name, git URL without ref, or the path as given.
    /// Local paths are resolved to absolute form by the settings layer, which
    /// knows the resolution base.
    pub fn identity(&self) -> PackageIdentity {
        match self {
            PackageSource::Npm { name, .. } => PackageIdentity(format!("npm:{name}")),
            PackageSource::Git { url, .. } => {
                PackageIdentity(format!("git:{}", normalize_git_identity(url)))
            }
            PackageSource::LocalPath { path } => PackageIdentity(format!("path:{path}")),
        }
    }

    /// Pinned sources are skipped by bulk `roder update`.
    pub fn pinned(&self) -> bool {
        match self {
            PackageSource::Npm { version, .. } => version.is_some(),
            PackageSource::Git { ref_name, .. } => ref_name.is_some(),
            PackageSource::LocalPath { .. } => false,
        }
    }
}

impl fmt::Display for PackageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spec())
    }
}

fn normalize_git_identity(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    trimmed
        .strip_suffix(".git")
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

/// Identity key for scope deduplication (project entry wins over user entry).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageIdentity(pub String);

impl fmt::Display for PackageIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum PackageScope {
    User,
    Project,
}

impl fmt::Display for PackageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageScope::User => f.write_str("user"),
            PackageScope::Project => f.write_str("project"),
        }
    }
}

/// One installed (or configured) package in a scope's `packages.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackageRecord {
    /// Short package id from the manifest (or derived from the source).
    pub package_id: String,
    pub identity: PackageIdentity,
    pub source: PackageSource,
    pub scope: PackageScope,
    /// Materialized install root. `None` for local-path packages, which load
    /// in place from `source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    /// Resolved npm version or git commit after install/update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether `--allow-scripts` was granted at install time.
    #[serde(default)]
    pub allow_scripts: bool,
    /// Process extensions never launch until this is set by an explicit
    /// approval step.
    #[serde(default)]
    pub extensions_approved: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "PackageResourceFilters::is_empty")]
    pub filters: PackageResourceFilters,
    /// Resource ids (see [`PackageResource::id`]) disabled by the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_resources: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum PackageResourceKind {
    Extension,
    Skill,
    Command,
    Theme,
}

impl PackageResourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageResourceKind::Extension => "extension",
            PackageResourceKind::Skill => "skill",
            PackageResourceKind::Command => "command",
            PackageResourceKind::Theme => "theme",
        }
    }
}

impl fmt::Display for PackageResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PackageResourceKind {
    type Err = PackageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "extension" | "extensions" => Ok(Self::Extension),
            "skill" | "skills" => Ok(Self::Skill),
            "command" | "commands" => Ok(Self::Command),
            "theme" | "themes" => Ok(Self::Theme),
            other => Err(PackageError::InvalidResourceKind {
                kind: other.to_string(),
            }),
        }
    }
}

/// One enumerated resource inside an installed package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageResource {
    pub package_id: String,
    pub kind: PackageResourceKind,
    /// Path relative to the package root (slash-separated).
    pub path: String,
    /// Short resource name (skill name, command name, theme id, extension id).
    pub name: String,
    pub enabled: bool,
    /// True for resources that execute code and therefore require approval.
    pub requires_approval: bool,
}

impl PackageResource {
    /// Registry-facing id: `<package-id>:<kind>/<name>`.
    pub fn id(&self) -> String {
        package_resource_id(&self.package_id, self.kind, &self.name)
    }
}

pub fn package_resource_id(package_id: &str, kind: PackageResourceKind, name: &str) -> String {
    format!("{package_id}:{kind}/{name}")
}

/// Parses `<package-id>:<kind>/<name>` back into its parts.
pub fn parse_package_resource_id(
    id: &str,
) -> Result<(String, PackageResourceKind, String), PackageError> {
    let (package_id, rest) = id.split_once(':').ok_or_else(|| invalid_resource_id(id))?;
    let (kind, name) = rest
        .split_once('/')
        .ok_or_else(|| invalid_resource_id(id))?;
    if package_id.is_empty() || name.is_empty() {
        return Err(invalid_resource_id(id));
    }
    Ok((package_id.to_string(), kind.parse()?, name.to_string()))
}

fn invalid_resource_id(id: &str) -> PackageError {
    PackageError::InvalidResourceId { id: id.to_string() }
}

/// Declared resources from `roder.toml` or the `package.json` `roder` key.
/// Entries are package-root-relative paths or globs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifestSpec {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Paths to process-extension manifests (`roder-extension.toml`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<String>,
}

/// Per-type filter patterns layered on top of the manifest by settings.
///
/// Semantics (mirroring the documented contract):
/// - `None` loads everything the manifest allows.
/// - `Some([])` loads nothing of that type (except `+path` force-includes).
/// - Plain patterns are include globs (`*` within a segment, `**` across).
/// - `!pattern` excludes glob matches.
/// - `+path` force-includes an exact path.
/// - `-path` force-excludes an exact path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageResourceFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<String>>,
}

impl PackageResourceFilters {
    pub fn is_empty(&self) -> bool {
        self.extensions.is_none()
            && self.skills.is_none()
            && self.commands.is_none()
            && self.themes.is_none()
    }

    pub fn for_kind(&self, kind: PackageResourceKind) -> Option<&[String]> {
        match kind {
            PackageResourceKind::Extension => self.extensions.as_deref(),
            PackageResourceKind::Skill => self.skills.as_deref(),
            PackageResourceKind::Command => self.commands.as_deref(),
            PackageResourceKind::Theme => self.themes.as_deref(),
        }
    }

    pub fn set_for_kind(&mut self, kind: PackageResourceKind, patterns: Option<Vec<String>>) {
        match kind {
            PackageResourceKind::Extension => self.extensions = patterns,
            PackageResourceKind::Skill => self.skills = patterns,
            PackageResourceKind::Command => self.commands = patterns,
            PackageResourceKind::Theme => self.themes = patterns,
        }
    }

    /// Whether `path` (package-root-relative, slash-separated) passes the
    /// filter configured for `kind`.
    pub fn allows(&self, kind: PackageResourceKind, path: &str) -> bool {
        filter_allows(self.for_kind(kind), path)
    }
}

fn filter_allows(patterns: Option<&[String]>, path: &str) -> bool {
    let Some(patterns) = patterns else {
        return true;
    };
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in patterns {
        if let Some(exact) = pattern.strip_prefix('-') {
            if exact == path {
                return false;
            }
        } else if let Some(exact) = pattern.strip_prefix('+') {
            if exact == path {
                return true;
            }
        } else if let Some(glob) = pattern.strip_prefix('!') {
            excludes.push(glob);
        } else {
            includes.push(pattern.as_str());
        }
    }
    if excludes.iter().any(|glob| glob_match(glob, path)) {
        return false;
    }
    if includes.is_empty() {
        // `Some([])` (or only force/exclude entries) loads nothing by default.
        return false;
    }
    includes.iter().any(|glob| glob_match(glob, path))
}

/// Segment-wise glob match. `*` matches within one path segment, `**` matches
/// any number of segments. A pattern that matches a leading directory prefix
/// of `path` also counts (so `skills` matches `skills/foo/SKILL.md`).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments_match(&pattern_segments, &path_segments)
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.first() {
        None => true, // pattern exhausted: exact match or directory prefix
        Some(&"**") => {
            if segments_match(&pattern[1..], path) {
                return true;
            }
            if path.is_empty() {
                return false;
            }
            segments_match(pattern, &path[1..])
        }
        Some(first) => {
            let Some(segment) = path.first() else {
                return false;
            };
            segment_match(first, segment) && segments_match(&pattern[1..], &path[1..])
        }
    }
}

fn segment_match(pattern: &str, segment: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == segment;
    }
    let mut rest = segment;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            let Some(after) = rest.strip_prefix(part) else {
                return false;
            };
            rest = after;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else if part.is_empty() {
            continue;
        } else {
            let Some(found) = rest.find(part) else {
                return false;
            };
            rest = &rest[found + part.len()..];
        }
    }
    true
}

/// Parses one package spec string into a [`PackageSource`].
///
/// Accepted forms:
/// - `npm:@scope/pkg`, `npm:@scope/pkg@1.2.3`, `npm:pkg`
/// - `git:github.com/user/repo[@ref]`, `git:git@host:user/repo[@ref]`,
///   `git:<protocol-url>[@ref]`
/// - protocol URLs without prefix: `https://`, `http://`, `ssh://`, `git://`
/// - local paths: absolute, `./relative`, `../relative`, `~/path`
pub fn parse_package_spec(input: &str) -> Result<PackageSource, PackageError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(PackageError::InvalidSpec {
            spec: input.to_string(),
            reason: "spec is empty".to_string(),
        });
    }
    if let Some(rest) = input.strip_prefix("npm:") {
        return parse_npm_spec(input, rest);
    }
    if input.starts_with("git://") {
        return parse_git_spec(input, input);
    }
    if let Some(rest) = input.strip_prefix("git:") {
        return parse_git_spec(input, rest);
    }
    if ["https://", "http://", "ssh://", "file://"]
        .iter()
        .any(|scheme| input.starts_with(scheme))
    {
        return parse_git_spec(input, input);
    }
    if input.starts_with('/')
        || input.starts_with("./")
        || input.starts_with("../")
        || input.starts_with("~/")
        || input == "."
        || input == ".."
    {
        return Ok(PackageSource::LocalPath {
            path: input.to_string(),
        });
    }
    Err(PackageError::InvalidSpec {
        spec: input.to_string(),
        reason: "expected npm:<name>[@version], git:<url>[@ref], a protocol URL, or a local path"
            .to_string(),
    })
}

fn parse_npm_spec(original: &str, rest: &str) -> Result<PackageSource, PackageError> {
    let invalid = |reason: &str| PackageError::InvalidSpec {
        spec: original.to_string(),
        reason: reason.to_string(),
    };
    if rest.is_empty() {
        return Err(invalid("npm spec is missing a package name"));
    }
    let (name, version) = if let Some(scoped) = rest.strip_prefix('@') {
        match scoped.split_once('@') {
            Some((name, version)) => (format!("@{name}"), Some(version.to_string())),
            None => (format!("@{scoped}"), None),
        }
    } else {
        match rest.split_once('@') {
            Some((name, version)) => (name.to_string(), Some(version.to_string())),
            None => (rest.to_string(), None),
        }
    };
    if name == "@" || name.is_empty() {
        return Err(invalid("npm spec is missing a package name"));
    }
    if name.starts_with('@') && !name[1..].contains('/') {
        return Err(invalid("scoped npm names must look like @scope/name"));
    }
    if let Some(version) = &version
        && version.is_empty()
    {
        return Err(invalid("npm version after @ is empty"));
    }
    if name.contains("..") || name.contains(' ') {
        return Err(invalid("npm package name contains invalid characters"));
    }
    Ok(PackageSource::Npm { name, version })
}

fn parse_git_spec(original: &str, rest: &str) -> Result<PackageSource, PackageError> {
    let invalid = |reason: &str| PackageError::InvalidSpec {
        spec: original.to_string(),
        reason: reason.to_string(),
    };
    if rest.is_empty() {
        return Err(invalid("git spec is missing a repository"));
    }
    let (location, ref_name) = split_git_ref(rest);
    if location.is_empty() {
        return Err(invalid("git spec is missing a repository"));
    }
    if let Some(ref_name) = &ref_name
        && ref_name.is_empty()
    {
        return Err(invalid("git ref after @ is empty"));
    }
    let has_protocol = location.contains("://");
    let is_scp_form = !has_protocol && location.contains('@') && location.contains(':');
    let is_shorthand = !has_protocol && !is_scp_form && location.contains('/');
    if !has_protocol && !is_scp_form && !is_shorthand {
        return Err(invalid(
            "git spec must be host/user/repo shorthand, user@host:path, or a protocol URL",
        ));
    }
    let url = if is_shorthand {
        format!("https://{location}")
    } else {
        location.to_string()
    };
    Ok(PackageSource::Git {
        url,
        ref_name: ref_name.map(str::to_string),
    })
}

/// Splits a trailing `@ref` when the `@` appears after the last `/` and the
/// last `:`, so user-info `@` in ssh and scp forms is never mistaken for a
/// ref separator.
fn split_git_ref(input: &str) -> (&str, Option<&str>) {
    let Some(at) = input.rfind('@') else {
        return (input, None);
    };
    let last_slash = input.rfind('/');
    let last_colon = input.rfind(':');
    let boundary = last_slash.max(last_colon);
    match boundary {
        Some(boundary) if at > boundary => (&input[..at], Some(&input[at + 1..])),
        _ => (input, None),
    }
}

/// Package ids are lowercase alphanumeric with `-`, `_`, and `.` separators.
pub fn validate_package_id(id: &str) -> Result<(), PackageError> {
    let valid = !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(PackageError::InvalidPackageId { id: id.to_string() })
    }
}

/// Derives a fallback package id from the source when the manifest does not
/// declare one (npm name tail, git repo name, or directory name).
pub fn derive_package_id(source: &PackageSource) -> String {
    let raw = match source {
        PackageSource::Npm { name, .. } => name.rsplit('/').next().unwrap_or(name).to_string(),
        PackageSource::Git { url, .. } => {
            let trimmed = url.trim_end_matches('/');
            let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
            trimmed
                .rsplit(['/', ':'])
                .next()
                .unwrap_or(trimmed)
                .to_string()
        }
        PackageSource::LocalPath { path } => {
            let trimmed = path.trim_end_matches('/');
            trimmed
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or("package")
                .to_string()
        }
    };
    let mut id: String = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while id.starts_with(['-', '_', '.']) {
        id.remove(0);
    }
    if id.is_empty() {
        id = "package".to_string();
    }
    id
}

/// Launch description for a process extension shipped inside a package; also
/// usable by hand-written `roder-extension.toml` manifests. Relative paths in
/// `args` and `cwd` resolve against the manifest's directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PackageExtensionLaunch {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub event_filter_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    InvalidSpec { spec: String, reason: String },
    InvalidPackageId { id: String },
    InvalidResourceKind { kind: String },
    InvalidResourceId { id: String },
    DuplicatePackage { identity: String, scope: String },
    PackageNotFound { spec: String },
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::InvalidSpec { spec, reason } => {
                write!(f, "invalid package spec {spec:?}: {reason}")
            }
            PackageError::InvalidPackageId { id } => write!(
                f,
                "invalid package id {id:?}: ids are lowercase alphanumeric plus '-', '_', '.'"
            ),
            PackageError::InvalidResourceKind { kind } => write!(
                f,
                "invalid resource kind {kind:?}: expected extension, skill, command, or theme"
            ),
            PackageError::InvalidResourceId { id } => write!(
                f,
                "invalid resource id {id:?}: expected <package-id>:<kind>/<name>"
            ),
            PackageError::DuplicatePackage { identity, scope } => {
                write!(
                    f,
                    "package {identity} is already installed in {scope} scope"
                )
            }
            PackageError::PackageNotFound { spec } => {
                write!(f, "package {spec} is not installed")
            }
        }
    }
}

impl Error for PackageError {}
