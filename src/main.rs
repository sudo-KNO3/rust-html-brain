//! kb — static HTML knowledge-base generator + code embedder.

use anyhow::{Context, Result};
use clap::{Parser as ClapParser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

mod code_index;
mod embed;
mod graph;
mod parser;
mod render;
mod serve;

use code_index::CodeChunk;

#[derive(Debug, Deserialize)]
struct Config {
    title: String,
    output: String,
    #[serde(default)]
    sources: Vec<Source>,
    code_index: Option<CodeIndexConfig>,
    #[serde(default)]
    code_sources: Vec<CodeSource>,
}

#[derive(Debug, Deserialize)]
struct Source {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CodeIndexConfig {
    #[serde(default = "default_python")]
    python_bin: String,
    #[serde(default = "default_embed_script")]
    embed_script: String,
    #[serde(default = "default_index_file")]
    index_file: String,
    #[serde(default = "default_batch_size")]
    batch_size: usize,
}

fn default_python() -> String { "python".into() }
fn default_embed_script() -> String { "@bundled".into() }
fn default_index_file() -> String { "code_index.json".into() }
fn default_batch_size() -> usize { 64 }

#[derive(Debug, Deserialize)]
struct CodeSource {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeIndex {
    pub model: String,
    pub dim: usize,
    pub chunks: Vec<IndexedChunk>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexedChunk {
    #[serde(flatten)]
    pub chunk: CodeChunk,
    pub embedding: Vec<f32>,
}

#[derive(ClapParser, Debug)]
#[command(name = "kb", about = "Static HTML knowledge-base + code embedder")]
struct Cli {
    /// Path to config.toml
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
    /// Override output directory
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Drop the tool into the current directory: write config.toml,
    /// AGENTS.md, and append .kb/ to .gitignore.
    Init {
        /// Overwrite existing config.toml / AGENTS.md if present.
        #[arg(long)]
        force: bool,
    },
    /// Build the static HTML site (default if no subcommand given).
    Build,
    /// Walk code sources, chunk + embed, write code_index.json.
    IndexCode,
    /// Run the local semantic search HTTP server.
    Serve {
        /// Bind address (default 127.0.0.1:9100).
        #[arg(short, long, default_value = "127.0.0.1:9100")]
        bind: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // `kb init` runs before config-loading because the whole point is to
    // create the config in the first place.
    if let Some(Command::Init { force }) = cli.command {
        return init_cmd(force);
    }

    let cfg_raw = fs::read_to_string(&cli.config)
        .with_context(|| format!("reading config {} — run `kb init` to create one", cli.config.display()))?;
    let cfg: Config = toml::from_str(&cfg_raw).context("parsing config.toml")?;
    let output = cli.output.unwrap_or_else(|| PathBuf::from(&cfg.output));

    match cli.command.unwrap_or(Command::Build) {
        Command::Init { .. } => unreachable!(),
        Command::Build => build(&cfg, &output),
        Command::IndexCode => index_code(&cfg, &output),
        Command::Serve { bind } => serve_cmd(&cfg, &output, bind),
    }
}

const AGENTS_MD_TEMPLATE: &str = r#"# AGENTS.md

This repository is indexed for AI agents by
[rust-html-brain](https://github.com/sudo-KNO3/rust-html-brain).

## Where the context lives

The full indexed knowledge of this repo lives in `.kb/` (gitignored).
Start with these files:

- **`.kb/llms.txt`** — flat-text index of every note + code chunk with
  one-line descriptions and links to per-item HTML pages. The canonical
  entry point for AI crawlers.
- **`.kb/index.html`** — human-browsable overview, notes grouped by
  category.
- **`.kb/graph.html`** — interactive D3 force-graph view. Open in a
  browser; type queries into the search box if `kb serve` is running.
- **`.kb/code/<encoded-id>.html`** — one HTML page per indexed code
  chunk. Each page ships Schema.org `SoftwareSourceCode` JSON-LD,
  microdata `itemprop` attributes, `ai:*` meta tags, and the full
  chunk text — enough for an AI agent to extract the code + metadata
  from any single page without crawling further.

## Rebuild the index

```
kb            # rebuild HTML + llms.txt from sources
kb index-code # walk + chunk + embed code (slow first run; needs Python)
kb serve      # semantic-search HTTP server on localhost:9100
```

## Tool

- Repo: https://github.com/sudo-KNO3/rust-html-brain
- Install: `cargo install --git https://github.com/sudo-KNO3/rust-html-brain`
- Or: `docker run -v $PWD:/sources/code:ro ghcr.io/sudo-kno3/rust-html-brain:slim`

<!-- kb:version 1 -->
"#;

fn init_cmd(force: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("reading current dir")?;
    let project_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "my-brain".into());

    println!("Initialising rust-html-brain in {}", cwd.display());

    // 1. config.toml
    let config_path = cwd.join("config.toml");
    if config_path.exists() && !force {
        println!("  config.toml already exists — skipping (use --force to overwrite)");
    } else {
        let content = format!(
            r#"# Auto-generated by `kb init`. Adjust [[sources]] / [[code_sources]] to taste.

title  = "{name}"
output = ".kb"

# Notes (optional): add markdown directories to mix into the brain.
# [[sources]]
# name = "docs"
# path = "docs"

[code_index]
python_bin   = "python"
embed_script = "@bundled"          # use the sentence-transformers script bundled in `kb`
index_file   = "code_index.json"
batch_size   = 64

[[code_sources]]
name = "{name}"
path = "."
"#,
            name = project_name
        );
        fs::write(&config_path, content)
            .with_context(|| format!("writing {}", config_path.display()))?;
        println!("  wrote config.toml");
    }

    // 2. .gitignore — append .kb/ idempotently
    let gitignore_path = cwd.join(".gitignore");
    let mut existing = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let already_ignored = existing
        .lines()
        .any(|line| matches!(line.trim(), ".kb" | ".kb/" | "/.kb" | "/.kb/"));
    if already_ignored {
        println!("  .gitignore already excludes .kb/");
    } else {
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str("\n# rust-html-brain output\n.kb/\n");
        fs::write(&gitignore_path, existing)
            .with_context(|| format!("writing {}", gitignore_path.display()))?;
        println!(
            "  appended .kb/ to {}",
            if gitignore_path.exists() {
                ".gitignore"
            } else {
                "(new) .gitignore"
            }
        );
    }

    // 3. AGENTS.md
    let agents_path = cwd.join("AGENTS.md");
    if agents_path.exists() && !force {
        println!("  AGENTS.md already exists — skipping (use --force to overwrite)");
    } else {
        fs::write(&agents_path, AGENTS_MD_TEMPLATE)
            .with_context(|| format!("writing {}", agents_path.display()))?;
        println!("  wrote AGENTS.md");
    }

    println!("\nNext steps:");
    println!("  kb               # build the HTML index into .kb/");
    println!("  kb index-code    # embed code chunks for semantic search");
    println!("  kb serve         # start the search HTTP server (localhost:9100)");
    Ok(())
}

fn serve_cmd(cfg: &Config, output: &Path, bind: String) -> Result<()> {
    let ci_cfg = cfg
        .code_index
        .as_ref()
        .context("no [code_index] section in config.toml")?;
    let index_path = output.join(&ci_cfg.index_file);
    if !index_path.exists() {
        anyhow::bail!(
            "no code index at {} — run `kb index-code` first",
            index_path.display()
        );
    }
    let script_path = embed::resolve_embed_script(&ci_cfg.embed_script)?;
    let script_path = if script_path.is_absolute() {
        script_path
    } else {
        std::env::current_dir()?.join(&script_path)
    };
    serve::run(
        index_path,
        ci_cfg.python_bin.clone(),
        script_path,
        bind,
    )
}

fn build(cfg: &Config, output: &Path) -> Result<()> {
    let mut notes: Vec<parser::Note> = Vec::new();
    let mut by_slug: HashMap<String, usize> = HashMap::new();

    for src in &cfg.sources {
        if !src.path.exists() {
            eprintln!(
                "  skipping source '{}' (path missing: {})",
                src.name,
                src.path.display()
            );
            continue;
        }
        let mut n = 0;
        for entry in WalkDir::new(&src.path).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let note = match parser::parse_file(path, &src.name) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("  failed to parse {}: {}", path.display(), e);
                    continue;
                }
            };
            if let Some(&existing_idx) = by_slug.get(&note.slug) {
                eprintln!(
                    "  duplicate slug '{}' — keeping {}, skipping {}",
                    note.slug,
                    notes[existing_idx].path.display(),
                    note.path.display(),
                );
                continue;
            }
            by_slug.insert(note.slug.clone(), notes.len());
            notes.push(note);
            n += 1;
        }
        println!("  {} notes from source '{}'", n, src.name);
    }
    if notes.is_empty() {
        eprintln!(
            "  no notes found in any source — site will only show code chunks (if indexed)"
        );
    }

    // Load code index if present.
    let code_index_path = output.join(
        cfg.code_index
            .as_ref()
            .map(|c| c.index_file.as_str())
            .unwrap_or("code_index.json"),
    );
    let code_index: Option<CodeIndex> = if code_index_path.exists() {
        let raw = fs::read_to_string(&code_index_path)
            .with_context(|| format!("reading {}", code_index_path.display()))?;
        match serde_json::from_str(&raw) {
            Ok(ci) => Some(ci),
            Err(e) => {
                eprintln!("  failed to parse code index: {}", e);
                None
            }
        }
    } else {
        None
    };
    if let Some(ci) = &code_index {
        println!(
            "  loaded code index: {} chunks (model={}, dim={})",
            ci.chunks.len(),
            ci.model,
            ci.dim
        );
    } else {
        println!("  no code index found — run `kb index-code` to build one");
    }

    println!(
        "Rendering {} notes{} into {}",
        notes.len(),
        code_index
            .as_ref()
            .map(|ci| format!(" + {} code chunks", ci.chunks.len()))
            .unwrap_or_default(),
        output.display()
    );
    render::render_site(&notes, code_index.as_ref(), &cfg.title, output)?;

    println!("\nDone. Open {}/index.html in a browser.", output.display());
    Ok(())
}

fn index_code(cfg: &Config, output: &Path) -> Result<()> {
    let ci_cfg = cfg
        .code_index
        .as_ref()
        .context("no [code_index] section in config.toml")?;
    if cfg.code_sources.is_empty() {
        anyhow::bail!("no [[code_sources]] configured");
    }

    println!("Walking {} code source(s)…", cfg.code_sources.len());
    let sources: Vec<(String, PathBuf)> = cfg
        .code_sources
        .iter()
        .map(|s| (s.name.clone(), s.path.clone()))
        .collect();
    let chunks = code_index::walk_all(&sources)?;
    if chunks.is_empty() {
        anyhow::bail!("no code chunks discovered");
    }
    println!("Total chunks: {}", chunks.len());

    let script_path = embed::resolve_embed_script(&ci_cfg.embed_script)?;
    let script_path = if script_path.is_absolute() {
        script_path
    } else {
        std::env::current_dir()?.join(&script_path)
    };
    if !script_path.exists() {
        anyhow::bail!(
            "embed script not found: {} — adjust [code_index].embed_script",
            script_path.display()
        );
    }

    println!(
        "Embedding via '{}' {} (batch={}) …",
        ci_cfg.python_bin,
        script_path.display(),
        ci_cfg.batch_size
    );
    let mut all_embeds: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());
    let mut model = String::new();
    let mut dim = 0usize;

    for (i, batch) in chunks.chunks(ci_cfg.batch_size).enumerate() {
        let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
        let resp = embed::embed_batch(&ci_cfg.python_bin, &script_path, &texts)
            .with_context(|| format!("embedding batch {}", i + 1))?;
        if model.is_empty() {
            model = resp.model.clone();
            dim = resp.dim;
        }
        all_embeds.extend(resp.embeddings);
        println!(
            "  batch {}/{} done ({} chunks)",
            i + 1,
            (chunks.len() + ci_cfg.batch_size - 1) / ci_cfg.batch_size,
            batch.len()
        );
    }

    let indexed: Vec<IndexedChunk> = chunks
        .into_iter()
        .zip(all_embeds)
        .map(|(chunk, embedding)| IndexedChunk { chunk, embedding })
        .collect();

    let ci = CodeIndex {
        model: model.clone(),
        dim,
        chunks: indexed,
    };
    fs::create_dir_all(output).context("creating output dir")?;
    let out_path = output.join(&ci_cfg.index_file);
    fs::write(&out_path, serde_json::to_string(&ci)?)
        .with_context(|| format!("writing {}", out_path.display()))?;

    println!(
        "\nWrote {} chunks (model={}, dim={}) to {}",
        ci.chunks.len(),
        model,
        dim,
        out_path.display()
    );
    println!("Now run `kb` (or `kb build`) to regenerate the site with code nodes in the graph.");
    Ok(())
}
