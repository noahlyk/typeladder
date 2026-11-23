//! Works out the version string the binary reports (`typeladder --version`).
//!
//! Priority:
//! 1. `TYPELADDER_VERSION` in the environment (a release build, or the PKGBUILD, sets it).
//! 2. `git describe` on a checkout, against tags named `v<number>...`:
//!    - exactly on a tag:        `1.2.0`
//!    - commits past a tag:      `1.2.0+8.g0587d41`
//!    - uncommitted changes:     `dirty` added to the end of the `+` part (`1.2.0+dirty` on a tag)
//! 3. No tags: Cargo.toml's version and the short hash, `0.1.0+g0587d41` (`0.1.0+g0587d41.dirty`).
//! 4. No git (a tarball, or git missing): `0.1.0+unknown`.
//!
//! Re-runs when HEAD, the current branch's ref, tags, the index or a tracked file changes, so a
//! plain `cargo run` after a commit shows the new version. Git's own files are found with
//! `git rev-parse --git-path`, which is right inside a linked worktree.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// `v1.2.0-8-g0587d41` -> `1.2.0+8.g0587d41`; on the tag itself (`-0-`) just `1.2.0`.
fn tag_version(described: &str, dirty: bool) -> Option<String> {
    let mut parts = described.rsplitn(3, '-');
    let hash = parts.next()?;
    let count: u32 = parts.next()?.parse().ok()?;
    let tag = parts.next()?.strip_prefix('v')?;
    let mut meta = Vec::new();
    if count > 0 {
        meta.push(format!("{count}.{hash}"));
    }
    if dirty {
        meta.push("dirty".to_string());
    }
    Some(if meta.is_empty() {
        tag.to_string()
    } else {
        format!("{tag}+{}", meta.join("."))
    })
}

fn describe(cargo_version: &str) -> String {
    let Some(hash) = git(&["rev-parse", "--short", "HEAD"]).filter(|h| !h.is_empty()) else {
        return format!("{cargo_version}+unknown");
    };
    // `status` refreshes the index first, so a file edited and then restored reads as clean.
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    git(&["describe", "--tags", "--long", "--match", "v[0-9]*"])
        .and_then(|described| tag_version(&described, dirty))
        .unwrap_or_else(|| format!("{cargo_version}+g{hash}{}", if dirty { ".dirty" } else { "" }))
}

fn watch_git() {
    let mut internals = vec![
        "HEAD".to_string(),
        "packed-refs".to_string(),
        "index".to_string(),
        "refs/tags".to_string(),
    ];
    if let Some(branch_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
        internals.push(branch_ref);
    }
    for p in internals {
        if let Some(path) = git(&["rev-parse", "--git-path", &p]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // Tracked files are real paths in the worktree; --git-path would point into .git/ instead.
    if let Some(files) = git(&["ls-files"]) {
        for file in files.lines() {
            println!("cargo:rerun-if-changed={file}");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=TYPELADDER_VERSION");
    watch_git();
    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let version = std::env::var("TYPELADDER_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| describe(&cargo_version));
    println!("cargo:rustc-env=TYPELADDER_VERSION={version}");
}
