//! Call the Python `kb_embed.py` sidecar to embed batches of text.
//!
//! The sidecar is a script honouring a tiny JSON-in / JSON-out contract;
//! the default ships sentence-transformers, but you can swap in OpenAI,
//! Cohere, Voyage, Ollama, or any backend by writing a replacement script.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

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
