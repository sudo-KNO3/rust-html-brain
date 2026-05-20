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
# uses scripts/kb_embed.py by default
# (pip install sentence-transformers first)

# 4. (Optional) Start the search server
cargo run --release -- serve
# leave running while browsing output/graph.html
```

## Embedding backend

Bundled: `scripts/kb_embed.py` — pure sentence-transformers, no external
deps. `pip install sentence-transformers` and you're done.

To swap in a different backend (OpenAI, Cohere, Voyage, Ollama, a local
GGUF model, …), drop in a replacement script that honours the same JSON
contract and update `embed_script` in `config.toml`:

```
stdin / argv[1]: JSON list of strings
stdout / argv[2]: JSON {"model": str, "dim": int, "embeddings": [[float, ...]]}
```

That's the entire interface.

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

## Docker

Two image tags from the same `Dockerfile`:

| Tag | Size | First-run cost | Best for |
|---|---|---|---|
| `rust-html-brain:slim` | ~2.8 GB | downloads model from HF Hub (~500 MB, cached in `kb-cache` volume) | local + development |
| `rust-html-brain:full` | ~3.5 GB | none — model pre-baked | air-gapped servers, distribution |

### Quick start with `docker compose`

```bash
docker compose build slim-image          # ~5 min
docker compose run --rm build            # build site from ./content
docker compose run --rm index            # walk ./code, embed → output/code_index.json
docker compose up serve                  # search server on localhost:9100
```

Edit `docker-compose.yml` to point volume mounts at your real content +
code directories.

### Bare `docker run`

```bash
docker build -t rust-html-brain:slim .

# index
docker run --rm \
    -v "$PWD/content":/sources/notes:ro \
    -v "$PWD/code":/sources/code:ro \
    -v "$PWD/output":/output \
    -v kb-cache:/cache \
    rust-html-brain:slim kb index-code

# serve
docker run -d --name kb \
    -p 9100:9100 \
    -v "$PWD/output":/output:ro \
    -v kb-cache:/cache \
    rust-html-brain:slim
```

### Building the offline image

```bash
docker build --build-arg BAKE_MODEL=true -t rust-html-brain:full .
```

Takes ~10 minutes (downloads + bakes `nomic-embed-text-v1.5`). After
that, `kb index-code` works with no internet.

The container's bundled `config.toml` lives at `/app/config.toml` and
points at `/sources/notes` + `/sources/code` — mount your data there.
Only `kb_embed.py` (sentence-transformers) is used in the image;
nothing leaves your network unless you swap the embed script.

### Published images

Pushed automatically by GitHub Actions on every commit to `main` and on
version tags (`v1.2.3`):

```
ghcr.io/sudo-kno3/rust-html-brain:slim         # latest main, downloads model
ghcr.io/sudo-kno3/rust-html-brain:full         # latest main, model pre-baked
ghcr.io/sudo-kno3/rust-html-brain:latest       # alias for :slim
ghcr.io/sudo-kno3/rust-html-brain:1.2.3        # specific version
ghcr.io/sudo-kno3/rust-html-brain:1.2.3-full   # specific version, baked
```

Also published to Docker Hub if `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`
secrets are configured in the repo.

## Status

Functional. Native single-binary build, or one container if you prefer
isolation. Active known debt: no live-reload, no syntax highlighting
on chunk pages, sentence-transformers downloads its model on first run
(slim image) or pre-bakes it (full image).

## License

MIT
