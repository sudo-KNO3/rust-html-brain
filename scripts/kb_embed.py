"""Embedding sidecar — aermod-pipeline VectorStore backend.

Routes embeddings through the AERMOD pipeline's `VectorStore`, which has a
graceful fallback chain (OpenAI → sentence-transformers → Ollama). Use this
when you already have aermod-pipeline checked out and configured; otherwise
use `kb_embed_simple.py` which uses sentence-transformers directly.

Usage:
    python kb_embed.py <input.json> <output.json>

input.json  : JSON list of strings (chunks to embed)
output.json : JSON {"model": str, "dim": int, "embeddings": [[float]]}

Configuration:
    KB_AERMOD_PATH   path to your aermod-pipeline checkout (required)
    OPENAI_API_KEY   if set, VectorStore uses OpenAI text-embedding-3-small
    EMBEDDING_BACKEND  force `openai` | `sentence-transformers` | `ollama`

Exit codes:
    0  success
    1  bad arguments
    2  import error (KB_AERMOD_PATH unset or path wrong)
    3  embedding call failed
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

    aermod_path_str = os.environ.get("KB_AERMOD_PATH")
    if not aermod_path_str:
        print(
            "KB_AERMOD_PATH env var not set — point it at your aermod-pipeline\n"
            "checkout (the directory containing agents/core/vector_store.py).\n"
            "Or switch to scripts/kb_embed_simple.py for a pure sentence-\n"
            "transformers backend with no aermod dependency.",
            file=sys.stderr,
        )
        return 2

    aermod_path = Path(aermod_path_str)
    if not aermod_path.exists():
        print(f"KB_AERMOD_PATH does not exist: {aermod_path}", file=sys.stderr)
        return 2
    if str(aermod_path) not in sys.path:
        sys.path.insert(0, str(aermod_path))

    try:
        from agents.core.vector_store import VectorStore  # type: ignore
    except Exception as e:
        print(f"failed to import VectorStore: {e}", file=sys.stderr)
        return 2

    try:
        texts = json.loads(input_path.read_text(encoding="utf-8"))
    except Exception as e:
        print(f"bad input file: {e}", file=sys.stderr)
        return 1
    if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
        print("input must be a JSON list of strings", file=sys.stderr)
        return 1

    try:
        vs = VectorStore(api_key=os.environ.get("OPENAI_API_KEY", ""))
        embeddings = vs._embed_texts(texts)
    except Exception as e:
        print(f"embedding failed: {e}", file=sys.stderr)
        return 3

    if not embeddings:
        print("no embeddings returned", file=sys.stderr)
        return 3

    dim = len(embeddings[0]) if embeddings and embeddings[0] else 0
    model = getattr(vs, "embedding_model", None) or os.environ.get("EMBEDDING_BACKEND", "unknown")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps({"model": str(model), "dim": dim, "embeddings": embeddings}),
        encoding="utf-8",
    )
    print(
        f"embedded {len(texts)} chunks (model={model}, dim={dim}) -> {output_path}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
