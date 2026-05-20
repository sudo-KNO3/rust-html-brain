//! Local HTTP server for semantic code search.
//!
//! Loads `code_index.json` into memory, pre-normalises every embedding,
//! and serves a single endpoint `POST /search { query, top_k? }` that
//! embeds the query (via the Python sidecar) and returns the top-K
//! nearest chunks by cosine similarity.

use crate::{embed, CodeIndex};
use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

const DEFAULT_TOP_K: usize = 8;
const MAX_TOP_K: usize = 50;

pub struct ServerState {
    pub python_bin: String,
    pub embed_script: PathBuf,
    pub model: String,
    pub dim: usize,
    /// Pre-normalised embeddings parallel to `chunks`.
    pub normalised: Vec<Vec<f32>>,
    pub chunks: Vec<ChunkMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkMeta {
    pub id: String,
    pub project: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default)]
    top_k: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SearchHit {
    #[serde(flatten)]
    chunk: ChunkMeta,
    score: f32,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    model: String,
    n_candidates: usize,
    results: Vec<SearchHit>,
}

pub async fn serve(
    index_path: &Path,
    python_bin: String,
    embed_script: PathBuf,
    bind: &str,
) -> Result<()> {
    let raw = std::fs::read_to_string(index_path)
        .with_context(|| format!("reading code index {}", index_path.display()))?;
    let ci: CodeIndex = serde_json::from_str(&raw).context("parsing code index")?;

    println!(
        "Loaded {} chunks (model={}, dim={})",
        ci.chunks.len(),
        ci.model,
        ci.dim
    );

    let normalised: Vec<Vec<f32>> = ci.chunks.iter().map(|c| normalise(&c.embedding)).collect();
    let chunks: Vec<ChunkMeta> = ci
        .chunks
        .iter()
        .map(|c| ChunkMeta {
            id: c.chunk.id.clone(),
            project: c.chunk.project.clone(),
            path: c.chunk.path.clone(),
            start_line: c.chunk.start_line,
            end_line: c.chunk.end_line,
            snippet: snippet(&c.chunk.text, 160),
        })
        .collect();

    let state = Arc::new(ServerState {
        python_bin,
        embed_script,
        model: ci.model.clone(),
        dim: ci.dim,
        normalised,
        chunks,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health))
        .route("/search", post(search_handler))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding to {}", bind))?;
    let addr = listener.local_addr().context("local addr")?;
    println!("Listening on http://{}", addr);
    println!("  GET  /health");
    println!("  POST /search   body: {{\"query\": \"...\", \"top_k\": 8}}");
    println!("\nPoint your graph.html search box at http://{}/search.", addr);
    println!("Press Ctrl-C to stop.");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn search_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, (StatusCode, String)> {
    if req.query.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty query".into()));
    }
    let top_k = req
        .top_k
        .unwrap_or(DEFAULT_TOP_K)
        .clamp(1, MAX_TOP_K);

    // Embed query in a blocking thread (the sidecar is sync I/O).
    let python_bin = state.python_bin.clone();
    let embed_script = state.embed_script.clone();
    let query = req.query.clone();
    let resp = tokio::task::spawn_blocking(move || {
        embed::embed_batch(&python_bin, &embed_script, &[query])
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("join: {}", e)))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("embed: {}", e)))?;

    let q_emb = resp
        .embeddings
        .first()
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "no embedding returned".into()))?;
    if q_emb.len() != state.dim {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "embedding dim mismatch: query={} index={}",
                q_emb.len(),
                state.dim
            ),
        ));
    }
    let q_norm = normalise(q_emb);

    // Score every chunk by cosine similarity (= dot product on normalised vectors).
    let mut scored: Vec<(usize, f32)> = state
        .normalised
        .iter()
        .enumerate()
        .map(|(i, v)| (i, dot(v, &q_norm)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);

    let results: Vec<SearchHit> = scored
        .into_iter()
        .map(|(i, score)| SearchHit {
            chunk: state.chunks[i].clone(),
            score,
        })
        .collect();

    Ok(Json(SearchResponse {
        query: req.query,
        model: state.model.clone(),
        n_candidates: state.chunks.len(),
        results,
    }))
}

fn normalise(v: &[f32]) -> Vec<f32> {
    let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag <= 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / mag).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn snippet(text: &str, max: usize) -> String {
    let collapsed: String = text.chars().take(max * 2).collect::<String>().replace('\n', " ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let mut s: String = collapsed.chars().take(max).collect();
        s.push('…');
        s
    }
}

// Wrapper so main.rs can call from a non-async context.
pub fn run(
    index_path: PathBuf,
    python_bin: String,
    embed_script: PathBuf,
    bind: String,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(serve(&index_path, python_bin, embed_script, &bind))
}
