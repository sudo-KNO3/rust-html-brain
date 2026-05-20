# Rust HTML Brain

> Static HTML "second brain" generator written in Rust. Turns Obsidian-style
> markdown into a fully self-contained, AI-readable site with a live
> D3.js force-graph view and optional semantic code search.

| | |
|---|---|
| **Build** | `cargo run --release` |
| **Index code** | `cargo run --release -- index-code` |
| **Search server** | `cargo run --release -- serve` (listens on `127.0.0.1:9100`) |
| **Output** | `./output/` — open `index.html` in any browser |
| **Dependencies at runtime** | none — everything (D3, CSS, search index) is bundled into `output/` |

## What you get

A static site with:

- **Per-note HTML pages** with backlink + outgoing-link panels (`output/notes/`)
- **Per-code-chunk HTML pages** with Schema.org `SoftwareSourceCode` JSON-LD,
  microdata `itemprop`s, `ai:*` meta tags, and full source (`output/code/`)
- **Graph view** (`output/graph.html`) — D3 force simulation rendered on
  canvas, continuous gentle motion, project / full toggle, semantic search
  box that talks to a local backend
- **`llms.txt`** at site root listing every note + chunk for AI crawlers
- **Vendored D3** so the graph works fully offline

## Why

- Obsidian's graph view + backlinks are great, but tied to Obsidian.
  This publishes the same model as **portable HTML you can host anywhere
  or open from disk**.
- Modern LLM agents want **structured, semantic** HTML. Every page here
  ships JSON-LD + semantic HTML5 + microdata so an LLM can extract the
  shape of a note or code chunk without DOM-walking.
- The **code-search service** lets you ask "where do we handle CSV
  loading?" and get the right files back, not just keyword hits.

## How it works

```
markdown sources  ─►  parse + extract wikilinks
                      ├─► per-note HTML (notes/)
                      ├─► graph nodes + edges
                      └─► sidebar / index pages

code directories  ─►  walk + chunk (≤1500 chars, 200 overlap)
                      └─► scripts/kb_embed*.py (Python sidecar)
                            └─► embeddings → code_index.json
                                  ├─► per-chunk HTML (code/) with JSON-LD
                                  ├─► graph nodes (per-project colour)
                                  └─► kb serve → /search endpoint
```

Three layers of AI-readable signal per chunk page, in priority order
recommended by Google + Schema.org + the 2026 LLM-crawler best practices:

1. **JSON-LD `SoftwareSourceCode`** in `<head>` — primary structured data.
2. **Semantic HTML5** (`<article itemscope>`, `<header>`, `<section>`,
   `<aside>`, `<dl>`) with **microdata `itemprop`** attributes mirroring
   the JSON-LD fields.
3. **`<meta name="ai:*">`** tags (project, language, file-path, lines,
   char-count) as fallback for parsers that don't read JSON-LD.

## Quick start

```bash
# 1. Configure
cp config.example.toml config.toml
# edit config.toml — add your [[sources]] and (optionally) [[code_sources]]

# 2. Build the site
cargo run --release
# → ./output/  ; open output/index.html

# 3. (Optional) Index your codebases for semantic search
cargo run --release -- index-code
# uses scripts/kb_embed_simple.py by default
# (pip install sentence-transformers first)

# 4. (Optional) Start the search server
cargo run --release -- serve
# leave running while browsing output/graph.html
```

## Embedding backends

Two bundled Python sidecars; configurable in `config.toml`:

| Script | Backend | Setup |
|---|---|---|
| `scripts/kb_embed_simple.py` | sentence-transformers (default) | `pip install sentence-transformers` |
| `scripts/kb_embed.py` | aermod-pipeline `VectorStore` (OpenAI → ST → Ollama) | `KB_AERMOD_PATH=/path/to/aermod-pipeline` |

Either one writes the same JSON contract:
`{ "model": str, "dim": int, "embeddings": [[float, ...], ...] }`

Drop in your own script for a different backend — that's the entire
interface.

## Frontmatter conventions

```yaml
---
name:        kebab-case-slug
description: one-line summary
metadata:
  type:   project | reference | feedback | user
  status: active | paused | done   # projects only
---
```

- Wikilinks `[[slug]]` become hyperlinks; unresolved targets render in red.
- Projects (`type: project`) get larger nodes in the graph and appear in
  the "Projects only" graph filter.

## Output layout

```
output/
├── index.html                  notes grouped by type
├── graph.html                  D3 force graph + search bar
├── llms.txt                    AI crawl manifest (notes + chunks)
├── style.css
├── d3.v7.min.js                vendored
├── code_index.json             chunks + embeddings (after `kb index-code`)
├── notes/<slug>.html           per-note pages
└── code/
    ├── index.html              chunks grouped by project → file
    └── <encoded-id>.html       per-chunk page with JSON-LD + full text
```

## CLI

```
kb [build]          generate the static site (default)
kb index-code       walk code sources, chunk + embed, write code_index.json
kb serve            run the semantic-search HTTP server on 127.0.0.1:9100
                    POST /search { "query": "...", "top_k": 8 }
                    GET  /health
```

Global flags:

- `--config <path>` use a different config (default `config.toml`)
- `--output <path>` override the configured output directory

## Status

Functional, single-binary, no runtime dependencies after build. Active
known debt: no live-reload, no syntax highlighting on chunk pages,
sentence-transformers downloads its model on first run.

## License

MIT
