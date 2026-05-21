//! Call the Python `kb_embed.py` sidecar to embed batches of text.
//!
//! The sidecar is a script honouring a tiny JSON-in / JSON-out contract;
//! the default ships sentence-transformers, but you can swap in OpenAI,
//! Cohere, Voyage, Ollama, or any backend by writing a replacement script.
//!
//! When `embed_script` in config is the literal `"@bundled"`, the sidecar
//! script is extracted from the binary itself at first use — so a freshly-
//! initialised codebase needs no Python file on disk to start indexing.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sentinel value for `embed_script` that means "use the script bundled
/// inside the binary, extracted to a stable temp path on first use".
pub const BUNDLED_SENTINEL: &str = "@bundled";

/// Source of the bundled embed script — copied verbatim from
/// `scripts/kb_embed.py` at compile time.
const BUNDLED_EMBED_PY: &str = include_str!("../scripts/kb_embed.py");

/// Resolve `embed_script` from config. If it's the bundled sentinel,
/// extract the embedded script to `<temp>/rust-html-brain/embed.py` and
/// return that path. Otherwise return the original path unchanged.
pub fn resolve_embed_script(configured: &str) -> Result<PathBuf> {
    if configured != BUNDLED_SENTINEL {
        return Ok(PathBuf::from(configured));
    }
    let dir = std::env::temp_dir().join("rust-html-brain");
    std::fs::create_dir_all(&dir).context("creating temp dir for bundled embed script")?;
    let path = dir.join("embed.py");
    // Re-extract only if missing or stale (different content).
    let needs_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != BUNDLED_EMBED_PY,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&path, BUNDLED_EMBED_PY)
            .with_context(|| format!("writing bundled embed script to {}", path.display()))?;
    }
    Ok(path)
}

/// Response shape written by `scripts/kb_embed.py`.
#[derive(Debug, Deserialize)]
pub struct EmbedResponse {
    pub model: String,
    pub dim: usize,
    pub embeddings: Vec<Vec<f32>>,
}

/// Embed a batch of strings by shelling out to the Python sidecar.
///
/// `python_bin` is whichever interpreter has the embedding backend's
/// deps installed (`sentence-transformers` by default). Defaults to
/// `python` on PATH. `script_path` is the path to the embed script
/// (typically `scripts/kb_embed.py`).
pub fn embed_batch(
    python_bin: &str,
    script_path: &Path,
    texts: &[String],
) -> Result<EmbedResponse> {
    if texts.is_empty() {
        return Ok(EmbedResponse {
            model: "none".into(),
            dim: 0,
            embeddings: vec![],
        });
    }

    let input = tempfile_with_extension("json")?;
    let output = tempfile_with_extension("json")?;

    std::fs::write(&input, serde_json::to_string(texts)?)
        .with_context(|| format!("writing embed input {}", input.display()))?;

    let status = Command::new(python_bin)
        .arg(script_path)
        .arg(&input)
        .arg(&output)
        .status()
        .with_context(|| format!("running '{}' {}", python_bin, script_path.display()))?;

    if !status.success() {
        return Err(anyhow!(
            "kb_embed.py failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }

    let raw = std::fs::read_to_string(&output)
        .with_context(|| format!("reading embed output {}", output.display()))?;
    let resp: EmbedResponse = serde_json::from_str(&raw).context("parsing embed output")?;

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);

    if resp.embeddings.len() != texts.len() {
        return Err(anyhow!(
            "embedding count mismatch: sent {} texts, got {} embeddings",
            texts.len(),
            resp.embeddings.len()
        ));
    }

    Ok(resp)
}

fn tempfile_with_extension(ext: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir();
    let stem = format!(
        "kb_embed_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    Ok(dir.join(format!("{stem}.{ext}")))
}
