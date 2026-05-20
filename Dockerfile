# syntax=docker/dockerfile:1.7
#
# Multi-stage build for rust-html-brain.
#
#   docker build -t rust-html-brain:slim .                    # ~2.8 GB, downloads model on first use
#   docker build --build-arg BAKE_MODEL=true \
#                -t rust-html-brain:full .                    # ~3.5 GB, fully offline
#
# Usage:
#   docker run --rm -v ./content:/sources/notes:ro \
#                   -v ./output:/output \
#                   rust-html-brain:slim kb              # build site
#
#   docker run --rm -v ./mycode:/sources/code:ro \
#                   -v ./output:/output \
#                   -v kb-cache:/cache \
#                   rust-html-brain:slim kb index-code   # embed code
#
#   docker run -p 9100:9100 -v ./output:/output:ro \
#              rust-html-brain:slim                      # default: kb serve
#
# See docker-compose.yml for an ergonomic multi-service setup.

# ─────────────────────────────────────────────────────────────────────────────
# Stage 1 — build the Rust binary
# ─────────────────────────────────────────────────────────────────────────────
FROM rust:1.95-slim AS builder

WORKDIR /build

# Pre-fetch + compile dependencies with a dummy main so dep layers cache
# even when src/ changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/rust_html_brain* \
                target/release/rust-html-brain* \
                target/release/kb*

COPY src       ./src
COPY templates ./templates
COPY assets    ./assets
RUN cargo build --release && \
    cp target/release/kb /usr/local/bin/kb && \
    strip /usr/local/bin/kb

# ─────────────────────────────────────────────────────────────────────────────
# Stage 2 — runtime: Python + sentence-transformers + the binary
# ─────────────────────────────────────────────────────────────────────────────
FROM python:3.11-slim

ARG BAKE_MODEL=false
ARG EMBED_MODEL=nomic-ai/nomic-embed-text-v1.5

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    KB_EMBED_MODEL=${EMBED_MODEL} \
    HF_HOME=/cache/huggingface \
    SENTENCE_TRANSFORMERS_HOME=/cache/sentence-transformers \
    RUST_BACKTRACE=1

# Minimal Python deps. `einops` is required by the default nomic model.
RUN pip install --no-cache-dir \
        "sentence-transformers>=3.0,<4" \
        einops

WORKDIR /app

COPY --from=builder /usr/local/bin/kb /usr/local/bin/kb
COPY scripts/kb_embed.py /app/scripts/kb_embed.py
COPY content             /app/content
COPY config.example.toml /app/config.example.toml

# Container-native default config: mount your data at /sources/{notes,code}.
RUN cat > /app/config.toml <<'EOF'
title  = "Knowledge Base"
output = "/output"

[[sources]]
name = "content"
path = "/app/content"

[[sources]]
name = "notes"
path = "/sources/notes"

[code_index]
python_bin   = "python"
embed_script = "/app/scripts/kb_embed.py"
index_file   = "code_index.json"
batch_size   = 64

[[code_sources]]
name = "code"
path = "/sources/code"
EOF

# Optionally pre-download the embedding model so the image works fully
# offline (no Hugging Face Hub call on first `kb index-code`).
RUN if [ "$BAKE_MODEL" = "true" ]; then \
        echo "Pre-baking model $EMBED_MODEL …" && \
        python -c "from sentence_transformers import SentenceTransformer; \
                   SentenceTransformer('$EMBED_MODEL', trust_remote_code=True)" && \
        echo "Model cached at $HF_HOME"; \
    else \
        echo "BAKE_MODEL=false — model will download on first use into /cache"; \
    fi

VOLUME ["/sources", "/output", "/cache"]
EXPOSE 9100

# Default: run the semantic-search HTTP server. Override on docker run.
CMD ["kb", "serve", "--bind", "0.0.0.0:9100"]
