use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use clap::{Parser, Subcommand, ValueEnum};
use semver::{BuildMetadata, Prerelease, Version};
use toml_edit::{DocumentMut, value};

#[derive(Parser)]
#[command(name = "xtask", about = "Workspace utility tasks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Bump a crate's version and propagate to downstream path-dep
    /// `version =` specifiers + downstream crate versions.
    Bump {
        /// Target crate name (e.g. `quilt-uri`).
        crate_name: String,
        /// New version for the target crate (e.g. `0.3.0`).
        new_version: String,
        /// How to bump downstream crates that transitively depend on the target.
        #[arg(long, value_enum, default_value_t = BumpKind::Patch)]
        bump_downstream: BumpKind,
        /// Skip the post-bump `cargo check --workspace`.
        #[arg(long)]
        skip_check: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum BumpKind {
    Patch,
    Minor,
    Major,
}

struct Member {
    name: String,
    dir: PathBuf,
    manifest: PathBuf,
    doc: DocumentMut,
    version: Version,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Bump {
            crate_name,
            new_version,
            bump_downstream,
            skip_check,
        } => bump(&crate_name, &new_version, bump_downstream, skip_check),
    }
}

fn bump(target: &str, new_version: &str, kind: BumpKind, skip_check: bool) -> Result<()> {
    let root = workspace_root()?;
    let mut members = load_members(&root)?;

    if !members.iter().any(|m| m.name == target) {
        bail!("`{target}` is not a workspace member");
    }
    let new_target_version =
        Version::parse(new_version).with_context(|| format!("invalid version `{new_version}`"))?;

    let new_versions = compute_cascade(&members, target, &new_target_version, kind);

    for m in &mut members {
        let mut changed = false;
        if let Some(v) = new_versions.get(&m.name) {
            m.doc["package"]["version"] = value(v.to_string());
            changed = true;
        }
        for (dep_name, dep_v) in &new_versions {
            if &m.name == dep_name {
                continue;
            }
            if rewrite_path_dep_version(&mut m.doc, dep_name, &dep_v.to_string()) {
                changed = true;
            }
        }
        if changed {
            fs::write(&m.manifest, m.doc.to_string())
                .with_context(|| format!("writing {}", m.manifest.display()))?;
            println!("updated {}", m.manifest.display());
        }
    }

    let date = Local::now().format("%Y-%m-%d").to_string();
    for (name, ver) in &new_versions {
        let m = members
            .iter()
            .find(|m| &m.name == name)
            .expect("member exists");
        rewrite_alpha_heading(&m.dir.join("CHANGELOG.md"), ver, &date)?;
    }

    if !skip_check {
        let status = Command::new("cargo")
            .args(["check", "--workspace"])
            .current_dir(&root)
            .status()
            .context("running `cargo check --workspace`")?;
        if !status.success() {
            bail!("`cargo check --workspace` failed");
        }
    }

    Ok(())
}

/// Walks downstream from the target: any member with a versioned path dep on
/// a crate already in the cascade gets a new version too (per `kind`).
fn compute_cascade(
    members: &[Member],
    target: &str,
    new_target_version: &Version,
    kind: BumpKind,
) -> BTreeMap<String, Version> {
    let mut new_versions = BTreeMap::new();
    new_versions.insert(target.to_string(), new_target_version.clone());

    let mut frontier = VecDeque::from([target.to_string()]);
    while let Some(upstream) = frontier.pop_front() {
        for m in members {
            if new_versions.contains_key(&m.name) {
                continue;
            }
            if has_versioned_path_dep(&m.doc, &upstream) {
                new_versions.insert(m.name.clone(), bump_version(&m.version, kind));
                frontier.push_back(m.name.clone());
            }
        }
    }
    new_versions
}

fn bump_version(v: &Version, kind: BumpKind) -> Version {
    let mut next = v.clone();
    match kind {
        BumpKind::Patch => next.patch += 1,
        BumpKind::Minor => {
            next.minor += 1;
            next.patch = 0;
        }
        BumpKind::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
        }
    }
    next.pre = Prerelease::EMPTY;
    next.build = BuildMetadata::EMPTY;
    next
}

fn has_versioned_path_dep(doc: &DocumentMut, upstream: &str) -> bool {
    DEP_TABLES.iter().any(|t| {
        doc.get(t)
            .and_then(|i| i.as_table_like())
            .and_then(|tbl| tbl.get(upstream))
            .and_then(|d| d.as_table_like())
            .is_some_and(|t| t.contains_key("path") && t.contains_key("version"))
    })
}

fn rewrite_path_dep_version(doc: &mut DocumentMut, upstream: &str, new_version: &str) -> bool {
    let mut changed = false;
    for table in DEP_TABLES {
        let Some(deps) = doc.get_mut(table).and_then(|i| i.as_table_like_mut()) else {
            continue;
        };
        let Some(dep) = deps.get_mut(upstream).and_then(|d| d.as_table_like_mut()) else {
            continue;
        };
        if dep.contains_key("path") && dep.contains_key("version") {
            dep.insert("version", value(new_version));
            changed = true;
        }
    }
    changed
}

const DEP_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

fn workspace_root() -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .context("running `cargo locate-project`")?;
    if !out.status.success() {
        bail!(
            "`cargo locate-project` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let manifest = String::from_utf8(out.stdout)?.trim().to_string();
    Path::new(&manifest)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("workspace manifest has no parent"))
}

fn load_members(root: &Path) -> Result<Vec<Member>> {
    let raw = fs::read_to_string(root.join("Cargo.toml")).context("reading root Cargo.toml")?;
    let root_doc: DocumentMut = raw.parse().context("parsing root Cargo.toml")?;
    let names: Vec<String> = root_doc["workspace"]["members"]
        .as_array()
        .context("[workspace].members not found")?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    let mut out = Vec::with_capacity(names.len());
    for rel in names {
        let dir = root.join(&rel);
        let manifest = dir.join("Cargo.toml");
        let raw = fs::read_to_string(&manifest)
            .with_context(|| format!("reading {}", manifest.display()))?;
        let doc: DocumentMut = raw
            .parse()
            .with_context(|| format!("parsing {}", manifest.display()))?;
        let name = doc["package"]["name"]
            .as_str()
            .with_context(|| format!("[package].name missing in {}", manifest.display()))?
            .to_string();
        // The xtask itself has a fixed `0.0.0` placeholder version; skip it from the cascade.
        if name == "xtask" {
            continue;
        }
        let version_raw = doc["package"]["version"]
            .as_str()
            .with_context(|| format!("[package].version missing in {}", manifest.display()))?
            .to_string();
        let version = Version::parse(&version_raw).with_context(|| {
            format!("invalid version `{version_raw}` in {}", manifest.display())
        })?;
        out.push(Member {
            name,
            dir,
            manifest,
            doc,
            version,
        });
    }
    Ok(out)
}

/// Replaces the topmost `## [v...-alphaN] - DATE` heading with the new
/// `## [vX.Y.Z] - YYYY-MM-DD` heading, preserving the body underneath.
/// Warns and leaves the file untouched if no alpha heading is found —
/// the caller can add one manually before running again.
fn rewrite_alpha_heading(path: &Path, new_version: &Version, date: &str) -> Result<()> {
    if !path.exists() {
        eprintln!("warn: no CHANGELOG.md at {}", path.display());
        return Ok(());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let target_heading = format!("## [v{new_version}] - {date}");

    let mut out = String::with_capacity(content.len() + target_heading.len());
    let mut replaced = false;
    let trailing_newline = content.ends_with('\n');
    for line in content.split('\n') {
        if !replaced && is_alpha_heading(line) {
            out.push_str(&target_heading);
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !trailing_newline && out.ends_with('\n') {
        out.pop();
    }

    if !replaced {
        eprintln!(
            "warn: no `## [v...-alphaN]` heading found in {} — add a `## [vX.Y.Z-alpha1]` block manually and rerun",
            path.display()
        );
        return Ok(());
    }

    fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    println!("updated {} → {}", path.display(), target_heading);
    Ok(())
}

fn is_alpha_heading(line: &str) -> bool {
    line.starts_with("## [v") && line.contains("-alpha")
}
