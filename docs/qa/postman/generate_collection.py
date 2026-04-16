#!/usr/bin/env python3
"""Generate a Postman v2.1 collection from the Rust router.

Extracts every .route("...") entry from src/main.rs and infers HTTP methods
from the surrounding handler chain (get/post/put/delete/patch).
"""

import json
import re
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent.parent
MAIN_RS = ROOT / "src" / "main.rs"
OUT = Path(__file__).parent / "WVI-API.postman_collection.json"


def parse_routes():
    text = MAIN_RS.read_text()
    # Matches .route("/path", get(h).post(h))  — multiline OK
    pattern = re.compile(r'\.route\(\s*"([^"]+)"\s*,([\s\S]*?)\)\s*(?=\.route|\.with_state|\.layer|$)')
    routes = []
    for m in pattern.finditer(text):
        path = m.group(1)
        rest = m.group(2).lower()
        methods = []
        for method in ("get", "post", "put", "patch", "delete"):
            if re.search(rf"\b{method}\s*\(", rest):
                methods.append(method.upper())
        if not methods:
            methods = ["GET"]
        for method in methods:
            routes.append((method, path))
    # Deduplicate
    return sorted(set(routes), key=lambda x: (x[1], x[0]))


def group(path):
    parts = path.strip("/").split("/")
    # skip /api/v1 prefix for grouping
    if len(parts) >= 3 and parts[0] == "api" and parts[1] == "v1":
        return parts[2]
    if parts and parts[0]:
        return parts[0]
    return "other"


def make_request(method, path):
    name = f"{method} {path}"
    has_body = method in ("POST", "PUT", "PATCH")
    body = None
    if has_body:
        body = {
            "mode": "raw",
            "raw": json.dumps({}, indent=2),
            "options": {"raw": {"language": "json"}},
        }
    return {
        "name": name,
        "request": {
            "method": method,
            "header": [
                {"key": "Authorization", "value": "Bearer {{token}}", "type": "text"},
                {"key": "Content-Type", "value": "application/json", "type": "text"},
            ],
            "url": {
                "raw": "{{baseUrl}}" + path,
                "host": ["{{baseUrl}}"],
                "path": [p for p in path.strip("/").split("/") if p],
            },
            **({"body": body} if body else {}),
        },
        "response": [],
    }


def main():
    routes = parse_routes()

    folders = {}
    for method, path in routes:
        folder = group(path)
        folders.setdefault(folder, []).append(make_request(method, path))

    collection = {
        "info": {
            "name": "WVI Health API",
            "description": (
                f"Auto-generated from src/main.rs router. "
                f"{len(routes)} endpoints grouped by module.\n\n"
                "Set `baseUrl` to https://6ssssdj5s38h.share.zrok.io "
                "and `token` to a valid Privy bearer token."
            ),
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
        },
        "item": [
            {
                "name": f"{name.capitalize()} ({len(items)})",
                "item": items,
            }
            for name, items in sorted(folders.items())
        ],
        "variable": [
            {"key": "baseUrl", "value": "https://6ssssdj5s38h.share.zrok.io"},
            {"key": "token", "value": "dev-token"},
        ],
    }

    OUT.write_text(json.dumps(collection, indent=2, ensure_ascii=False))
    print(f"Wrote {OUT} with {len(routes)} requests across {len(folders)} folders")


if __name__ == "__main__":
    main()
