#!/usr/bin/env bash
set -euo pipefail
# Build a padded corpus with a buried token + a real section to summarize.
python3 - <<'PY'
from pathlib import Path
pad = "\n".join(
    f"## Pad {i}\n" + ("lorem " * 40)
    for i in range(1, 80)
)
body = f"""{pad}

## Cargo
Rust ships with cargo, rustc, and crates.io. Depend on libraries with Cargo.toml.
Build with cargo build --release. Test with cargo test.

## Noise
The secret token for this eval is RLM-EVAL-91C2. Do not invent another.

## More noise
alpha bravo charlie
"""
Path("corpus.md").write_text(body)
print(f"wrote corpus.md ({Path('corpus.md').stat().st_size} bytes)")
PY
