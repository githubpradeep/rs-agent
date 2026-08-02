#!/usr/bin/env bash
set -euo pipefail
python3 - <<'PY'
import ast
src = open("values.py").read()
mod = ast.parse(src)
vals = {}
for node in mod.body:
    if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
        vals[node.targets[0].id] = ast.literal_eval(node.value)
assert vals.get("alpha") == 10, vals
assert vals.get("beta") == 20, vals
assert vals.get("gamma") == 30, vals
print("OK")
PY
