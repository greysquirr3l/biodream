//! Build script — embeds git SHA and commit date at compile time.
//!
//! Env vars set (available via `env!(...)`):
//!   - `BIODREAM_GIT_SHA`  — short 8-char commit hash, "crates.io", or "unknown"
//!   - `BIODREAM_GIT_DATE` — commit date (YYYY-MM-DD), "crates.io", or "unknown"

use std::{env, process::Command};

fn git<const N: usize>(args: [&str; N]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo::rerun-if-changed=.git/HEAD");
    println!("cargo::rerun-if-changed=.git/refs/heads/");

    // Detect crates.io installs (no .git directory present).
    let is_crates_io = env::var("CARGO_MANIFEST_DIR")
        .map(|d| d.contains("/.cargo/registry/src/"))
        .unwrap_or(false);

    let (sha, date) = if is_crates_io {
        ("crates.io".to_owned(), "crates.io".to_owned())
    } else {
        let sha = git(["rev-parse", "--short=8", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
        let date = git(["log", "-1", "--format=%cd", "--date=format:%Y-%m-%d"])
            .unwrap_or_else(|| "unknown".to_owned());
        (sha, date)
    };

    println!("cargo::rustc-env=BIODREAM_GIT_SHA={sha}");
    println!("cargo::rustc-env=BIODREAM_GIT_DATE={date}");
}
