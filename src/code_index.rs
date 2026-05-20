//! Code-base walker + per-file chunker.
//!
//! Walks every code-bearing file under the configured root(s), strips
//! obvious noise (build artefacts, vendor dirs, dot-dirs), and emits a
//! list of CodeChunks: a chunk per file when the file fits in
//! `max_chunk_chars`, otherwise sliding-window chunks with overlap so
//! semantic boundaries aren't lost at the seams.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Default per-chunk character budget. ~1500 chars ≈ 300-400 tokens.
pub const DEFAULT_MAX_CHARS: usize = 1500;
/// Sliding-window overlap so a function split across chunks isn't lost.
pub const DEFAULT_OVERLAP_CHARS: usize = 200;

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "md", "toml", "yaml", "yml",
    "html", "css", "go", "java", "cpp", "c", "h", "hpp", "rb", "sh", "bash", "ps1",
];

/// Directory names to skip entirely (build outputs, vendored deps, VCS).
const SKIP_DIRS: &[&str] = &[
    "target", "output", "node_modules", ".git", ".venv", "venv",
    "__pycache__", "dist", "build", ".next", ".cargo", "vendor",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    /// Stable id: `{project}/{relative_path}#{chunk_index}`.
    pub id: String,
    pub project: String,
    /// POSIX-style relative path from `project_root`.
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

/// Walk a single project root and return all chunks.
pub fn walk_project(project_name: &str, project_root: &Path) -> Result<Vec<CodeChunk>> {
    let mut chunks = Vec::new();

    for entry in WalkDir::new(project_root)
        .into_iter()
        .filter_entry(|e| !is_skipped_dir(e))
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|s| s.to_str()) {
            Some(e) => e.to_lowercase(),
            None => continue,
        };
        if !CODE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let body = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue, // skip binaries / files we can't read as text
        };
        if body.trim().is_empty() {
            continue;
        }

        let rel = relative_posix(path, project_root);
        chunk_file(project_name, &rel, &body, DEFAULT_MAX_CHARS, DEFAULT_OVERLAP_CHARS, &mut chunks);
    }

    Ok(chunks)
}

/// Walk many roots and merge.
pub fn walk_all(sources: &[(String, PathBuf)]) -> Result<Vec<CodeChunk>> {
    let mut all = Vec::new();
    for (name, root) in sources {
        if !root.exists() {
            eprintln!(
                "  skipping code source '{}' (path missing: {})",
                name,
                root.display()
            );
            continue;
        }
        let n_before = all.len();
        all.extend(walk_project(name, root)?);
        println!("  {} chunks from '{}'", all.len() - n_before, name);
    }
    Ok(all)
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if !entry.file_type().is_dir() {
        return false;
    }
    SKIP_DIRS.iter().any(|skip| *skip == name)
}

fn relative_posix(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn chunk_file(
    project: &str,
    rel_path: &str,
    body: &str,
    max_chars: usize,
    overlap: usize,
    out: &mut Vec<CodeChunk>,
) {
    let chars: Vec<char> = body.chars().collect();
    if chars.len() <= max_chars {
        let total_lines = body.lines().count().max(1);
        out.push(CodeChunk {
            id: format!("{}/{}#0", project, rel_path),
            project: project.to_string(),
            path: rel_path.to_string(),
            start_line: 1,
            end_line: total_lines,
            text: body.to_string(),
        });
        return;
    }

    // Sliding-window. Find chunk boundaries by char count, then snap to
    // newlines so chunks start/end on whole lines when possible.
    let step = max_chars.saturating_sub(overlap).max(1);
    let mut start = 0usize;
    let mut idx = 0usize;
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        let raw: String = chars[start..end].iter().collect();
        // Snap end to last newline if it's not at the EOF boundary.
        let snapped_end = if end < chars.len() {
            raw.rfind('\n').map(|p| p + 1).unwrap_or(raw.len())
        } else {
            raw.len()
        };
        let chunk_text = raw[..snapped_end].to_string();

        let start_line = body[..body.char_indices().nth(start).map(|(b, _)| b).unwrap_or(0)]
            .matches('\n')
            .count()
            + 1;
        let end_line = start_line + chunk_text.matches('\n').count();

        out.push(CodeChunk {
            id: format!("{}/{}#{}", project, rel_path, idx),
            project: project.to_string(),
            path: rel_path.to_string(),
            start_line,
            end_line,
            text: chunk_text,
        });

        // Advance by step counted in chars (approximate; good enough for chunking).
        start += step;
        idx += 1;
        if start >= chars.len() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_yields_one_chunk() {
        let mut out = Vec::new();
        chunk_file("p", "src/main.rs", "fn main() {}\n", 1500, 200, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text.trim(), "fn main() {}");
        assert_eq!(out[0].start_line, 1);
    }

    #[test]
    fn large_file_yields_overlapping_chunks() {
        let body: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let mut out = Vec::new();
        chunk_file("p", "f.rs", &body, 200, 50, &mut out);
        assert!(out.len() > 1);
        // First chunk should start at line 1.
        assert_eq!(out[0].start_line, 1);
        // Each chunk should be non-empty.
        for c in &out {
            assert!(!c.text.is_empty());
        }
    }
}
