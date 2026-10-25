"""Trim a vendored crate manifest to what the importer needs.

Drops optional and target-specific dependencies, dev-dependencies, and
[[test]] / [[bench]] / [[example]] targets. Feature names stay, with empty
lists, so `cfg(feature = ...)` code compiles out without unknown-cfg lints.
"""
import re
import sys

path = sys.argv[1]
out, section, features = [], None, []
for line in open(path, encoding="utf-8").read().splitlines():
    header = re.match(r"^\s*\[\[?([^\]]+)\]\]?\s*$", line)
    if header:
        section = header.group(1).strip()
        if section == "features":
            continue
        keep = section in ("package", "dependencies", "lints") or section.startswith("package.")
        if keep:
            out.append(line)
        continue
    if section == "features":
        m = re.match(r"^\s*([A-Za-z0-9_-]+)\s*=", line)
        if m:
            features.append(m.group(1))
        continue
    if section == "dependencies" and "optional = true" in line:
        continue
    if section in ("package", "dependencies", "lints") or (section or "").startswith("package."):
        out.append(line)
if features:
    out.append("")
    out.append("[features]")
    out.extend(f"{f} = []" for f in features)
open(path, "w", encoding="utf-8").write("\n".join(out) + "\n")
