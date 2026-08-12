//! Opt-in oracle-source acquisition for the model-catalog workflow.
//!
//! This is the opt-in regeneration path (A.3: "separate opt-in oracle
//! regeneration from normal offline verification"). Normal `nix flake check`
//! only consumes the committed, hash-pinned canonical outputs; this module is
//! only invoked explicitly (e.g. `nix run .#update-model-catalog -- --local`)
//! to re-hydrate a fresh catalog from Pi's published endpoint or a pinned git
//! revision, then normalize it through the same core.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model_catalog::ModelCatalogError;

#[derive(Debug)]
pub struct Acquired {
    /// Directory containing the materialized source (cleaned up on drop by the
    /// caller via [`TempDir`]).
    pub root: PathBuf,
    /// Path to the source file (`catalog.json` for published, or the `.ts`
    /// generator inside a git checkout).
    pub file: PathBuf,
    /// Revision label (published: header/fallback sha256; git: rev-parse HEAD).
    pub revision: String,
}

impl Acquired {
    fn new(root: PathBuf, file: PathBuf, revision: String) -> Self {
        Acquired { root, file, revision }
    }
}

pub struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<TempDir, ModelCatalogError> {
        let base = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base)?;
        Ok(TempDir(base))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Acquire from `--source` (a local `.ts`/`.js`/`.json` file or a checkout dir).
pub fn from_source(source: &Path, source_path: &str, revision: Option<&str>) -> Acquired {
    let is_file = matches!(
        source.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("js") | Some("json")
    );
    if is_file {
        let root = source.parent().unwrap_or(Path::new(".")).to_path_buf();
        Acquired::new(
            root.clone(),
            source.to_path_buf(),
            revision.unwrap_or("local-fixture").to_owned(),
        )
    } else {
        Acquired::new(
            source.to_path_buf(),
            source.join(source_path),
            revision.unwrap_or("local-fixture").to_owned(),
        )
    }
}

/// Acquire from the published catalog endpoint.
pub async fn from_catalog(
    catalog_url: &str,
) -> Result<(TempDir, Value, String), ModelCatalogError> {
    let tmp = TempDir::new("pi-model-catalog-remote")?;
    let client = reqwest::Client::new();
    let response = client
        .get(catalog_url)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| ModelCatalogError::Message(format!("published catalog request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ModelCatalogError::Message(format!(
            "published catalog request failed: {}",
            response.status()
        )));
    }
    let revision = response
        .headers()
        .get("x-pi-model-catalog-revision")
        .or_else(|| response.headers().get("etag"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_owned())
        .unwrap_or_else(|| {
            // Fallback: sha256 of the body (computed later). We read headers
            // before moving the body; the fallback is filled after the fetch.
            String::default()
        })
        .to_owned();
    let body = response.text().await.map_err(|e| {
        ModelCatalogError::Message(format!("published catalog read failed: {e}"))
    })?;
    let revision = if revision.is_empty() {
        format!("sha256-{}", sha256_hex(body.as_bytes()))
    } else {
        revision
    };
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| ModelCatalogError::Message("published catalog is not valid JSON".into()))?;
    let file = tmp.path().join("catalog.json");
    std::fs::write(&file, body.as_bytes())?;
    Ok((tmp, value, revision))
}

/// Acquire from a git revision (`--local`) by cloning into a temp dir.
pub fn from_git(
    repository: &str,
    revision: &str,
    source_path: &str,
) -> Result<(TempDir, PathBuf, String), ModelCatalogError> {
    let tmp = TempDir::new("pi-model-catalog-remote")?;
    run(&[
        "git", "init", "--quiet", tmp.path().to_str().unwrap_or_default(),
    ])?;
    run(&[
        "git", "-C", tmp.path().to_str().unwrap_or_default(), "fetch", "--quiet",
        "--depth", "1", repository, revision,
    ])?;
    run(&[
        "git", "-C", tmp.path().to_str().unwrap_or_default(), "checkout", "--quiet", "FETCH_HEAD",
    ])?;
    let head = run(&[
        "git", "-C", tmp.path().to_str().unwrap_or_default(), "rev-parse", "HEAD",
    ])?;
    let file = tmp.path().join(source_path);
    if !file.exists() {
        return Err(ModelCatalogError::Message(format!(
            "generated catalog not found: {}",
            file.display()
        )));
    }
    Ok((tmp, file, head))
}

fn run(args: &[&str]) -> Result<String, ModelCatalogError> {
    let output = std::process::Command::new(args[0])
        .args(&args[1..])
        .output()
        .map_err(|e| ModelCatalogError::Message(format!("command failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ModelCatalogError::Message(format!(
            "command failed: {}: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}