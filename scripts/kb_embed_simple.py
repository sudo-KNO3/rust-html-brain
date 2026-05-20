"""Embedding sidecar — sentence-transformers backend (no external deps).

Same I/O contract as `kb_embed.py` but pulls in `sentence-transformers`
directly, with no dependency on aermod-pipeline. Use this if you don't
have aermod-pipeline checked out, or if you want a fully self-contained
embedding backend.

Install:
    pip install sentence-transformers

Usage:
    python kb_embed_simple.py <input.json> <output.json>

Configuration:
    KB_EMBED_MODEL   any sentence-transformers model id
                     (default: "nomic-ai/nomic-embed-text-v1.5", 768-dim)
"""

import json
import os
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: kb_embed_simple.py <input.json> <output.json>", file=sys.stderr)
        return 1

    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])

    try:
        from sentence_transformers import SentenceTransformer
    except ImportError:
        print(
            "sentence-transformers not installed. Run:\n"
            "    pip install sentence-transformers",
            file=sys.stderr,
        )
        return 2

    try:
        texts = json.loads(input_path.read_text(encoding="utf-8"))
    except Exception as e:
        print(f"bad input file: {e}", file=sys.stderr)
        return 1
    if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
        print("input must be a JSON list of strings", file=sys.stderr)
        return 1

    model_id = os.environ.get("KB_EMBED_MODEL", "nomic-ai/nomic-embed-text-v1.5")
    try:
        model = SentenceTransformer(model_id, trust_remote_code=True)
        embeddings = model.encode(texts, normalize_embeddings=False).tolist()
    except Exception as e:
        print(f"embedding failed: {e}", file=sys.stderr)
        return 3

    dim = len(embeddings[0]) if embeddings and embeddings[0] else 0
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps({"model": model_id, "dim": dim, "embeddings": embeddings}),
        encoding="utf-8",
    )
    print(
        f"embedded {len(texts)} chunks (model={model_id}, dim={dim}) -> {output_path}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
