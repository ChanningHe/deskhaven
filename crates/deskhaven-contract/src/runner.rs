//! Discovers fixtures, replays each against a fresh [`TestApp`] and collects
//! every failure (one bad fixture does not hide the others).

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::app::TestApp;
use crate::check::{Reply, check};
use crate::fixture::Fixture;
use crate::seeds;

/// Part of a reply body quoted in a failure.
const BODY_EXCERPT: usize = 512;

/// `tests/contract` at the repository root.
pub fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contract")
}

#[derive(Debug)]
pub struct Failure {
    /// Relative to the fixtures directory.
    pub file: String,
    pub source: Vec<String>,
    pub problems: Vec<String>,
    pub body: Option<String>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub passed: Vec<String>,
    pub failures: Vec<Failure>,
}

/// Runs `<dir>/fixtures/**/*.json` whose relative path contains `filter`.
/// Fixture problems are failures; only an unreadable layout is an `Err`.
pub async fn run_dir(dir: &Path, filter: Option<&str>) -> anyhow::Result<Outcome> {
    let upstream = std::fs::read_to_string(dir.join("UPSTREAM_COMMIT"))
        .with_context(|| format!("cannot read {}/UPSTREAM_COMMIT", dir.display()))?;
    let root = dir.join("fixtures");
    let files = fixture_files(&root)?;
    let mut outcome = Outcome::default();
    for path in files {
        let file = relative(&root, &path);
        if filter.is_some_and(|f| !file.contains(f)) {
            continue;
        }
        match run_file(&path, upstream.trim()).await {
            Ok(()) => outcome.passed.push(file),
            Err(failure) => outcome.failures.push(Failure { file, ..failure }),
        }
    }
    Ok(outcome)
}

async fn run_file(path: &Path, upstream: &str) -> Result<(), Failure> {
    let fail = |source: Vec<String>, problems: Vec<String>, body| Failure {
        file: String::new(),
        source,
        problems,
        body,
    };
    let text =
        std::fs::read_to_string(path).map_err(|err| fail(vec![], vec![err.to_string()], None))?;
    let fixture = Fixture::parse(&text).map_err(|err| fail(vec![], vec![err], None))?;
    let problems = fixture.problems(upstream);
    if !problems.is_empty() {
        return Err(fail(fixture.source, problems, None));
    }
    let reply = replay(&fixture)
        .await
        .map_err(|err| fail(fixture.source.clone(), vec![format!("{err:#}")], None))?;
    let problems = check(&fixture.expect, &reply);
    if problems.is_empty() {
        return Ok(());
    }
    Err(fail(fixture.source, problems, Some(excerpt(&reply))))
}

async fn replay(fixture: &Fixture) -> anyhow::Result<Reply> {
    let app = TestApp::start().await.context("cannot start the app")?;
    for name in &fixture.seed {
        let seed = seeds::lookup(name).with_context(|| format!("unknown seed {name:?}"))?;
        seed(&app.state)
            .await
            .with_context(|| format!("seed {name:?} failed"))?;
    }
    app.send(&fixture.request).await
}

fn fixture_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries =
            std::fs::read_dir(&dir).with_context(|| format!("cannot list {}", dir.display()))?;
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn excerpt(reply: &Reply) -> String {
    let text = String::from_utf8_lossy(&reply.body);
    let cut: String = text.chars().take(BODY_EXCERPT).collect();
    if cut.len() < text.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "FAIL tests/contract/fixtures/{}", self.file)?;
        if !self.source.is_empty() {
            writeln!(f, "  client source: {}", self.source.join(", "))?;
        }
        for problem in &self.problems {
            writeln!(f, "  - {problem}")?;
        }
        if let Some(body) = &self.body {
            writeln!(f, "  body: {body}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for file in &self.passed {
            writeln!(f, "ok   {file}")?;
        }
        for failure in &self.failures {
            write!(f, "{failure}")?;
        }
        write!(
            f,
            "{} passed, {} failed",
            self.passed.len(),
            self.failures.len()
        )
    }
}
