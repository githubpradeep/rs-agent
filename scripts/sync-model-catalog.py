#!/usr/bin/env python3
"""Regenerate data/models.catalog.json from reference/pi *.models.ts catalogs.

Usage (from repo root):
  python3 scripts/sync-model-catalog.py
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROVIDERS = ROOT / "reference" / "pi" / "packages" / "ai" / "src" / "providers"
OUT = ROOT / "data" / "models.catalog.json"


def main() -> int:
    if not PROVIDERS.is_dir():
        print(f"error: missing {PROVIDERS} (clone reference/pi first)", file=sys.stderr)
        return 1

    entries = []
    for path in sorted(PROVIDERS.glob("*.models.ts")):
        text = path.read_text(encoding="utf-8")
        for m in re.finditer(
            r'"([^"]+)":\s*\{([^}]*(?:\{[^}]*\}[^}]*)*)\}', text, re.S
        ):
            block = m.group(2)

            def field(name: str):
                mm = re.search(rf'{name}:\s*"([^"]*)"', block)
                return mm.group(1) if mm else None

            def field_num(name: str):
                mm = re.search(rf"{name}:\s*(\d+)", block)
                return int(mm.group(1)) if mm else None

            mid = field("id") or m.group(1)
            provider = field("provider")
            if not provider or not mid:
                continue
            entries.append(
                {
                    "id": mid,
                    "name": field("name") or mid,
                    "provider": provider,
                    "api": field("api") or "openai-completions",
                    "base_url": field("baseUrl") or "",
                    "reasoning": bool(re.search(r"reasoning:\s*true", block)),
                    "context_window": field_num("contextWindow") or 128000,
                    "max_tokens": field_num("maxTokens") or 8192,
                }
            )

    seen = set()
    out = []
    for e in entries:
        key = (e["provider"], e["id"])
        if key in seen:
            continue
        seen.add(key)
        out.append(e)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        json.dumps({"version": 1, "models": out}, separators=(",", ":")),
        encoding="utf-8",
    )
    providers = {e["provider"] for e in out}
    print(f"wrote {OUT} ({len(out)} models, {len(providers)} providers)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
