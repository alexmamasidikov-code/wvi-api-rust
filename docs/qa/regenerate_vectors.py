#!/usr/bin/env python3
"""Regenerate `expected_wvi` for each test vector by running the Python
validator. The validator becomes the source of truth — the Rust calculator
and iOS port must match it. Any divergence is a bug to investigate."""

import json
from pathlib import Path
from wvi_validator import WVIInput, calculate

VECTORS = Path(__file__).parent / "test-vectors" / "wvi_vectors.json"

def main():
    vectors = json.loads(VECTORS.read_text())
    for v in vectors:
        inp = WVIInput(**v["input"])
        result = calculate(inp)
        v["expected_wvi"] = result.wvi_score
        # Keep existing tolerance
    VECTORS.write_text(json.dumps(vectors, indent=2, ensure_ascii=False))
    print(f"Regenerated {len(vectors)} vectors")

if __name__ == "__main__":
    main()
