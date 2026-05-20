"""Embedding sidecar — sentence-transformers backend.

I/O contract:
    python kb_embed.py <input.json> <output.json>

    input.json  : JSON list of strings (chunks to embed)
    output.json : JSON {"model": str, "dim": int, "embeddings": [[float, ...]]}

This is the default backend used by `kb index-code` and `kb serve`. To use
a different model or service (OpenAI, Cohere, Voyage, Ollama, ...), drop in
a replacement script that honours the same JSON contract and update
`embed_script` in `config.toml`.

Install:
    pip install sentence-transformers

Configuration (env vars):
    KB_EMBED_MODEL   any sentence-transformers model id
                     (default: "nomic-ai/nomic-embed-text-v1.5", 768-dim)
"""

import json
import os
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: kb_embed.py <input.json> <output.json>", file=sys.stderr)
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
