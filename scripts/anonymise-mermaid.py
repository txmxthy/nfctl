#!/usr/bin/env python3
"""Anonymise Mermaid `graph LR` pipeline diagrams into a structure-preserving corpus.

Input: directories of `.mmd` files in the dialect
    graph LR / subgraph id["label"] ... end / node shapes ([ ]) [[ ]] [ ] (( )) ( ) /
    edges `a --> b`, `a -->|t1, t2| b`, `a -.->|topic| b` / `style ...` / `%% ...`
Output: one file per input, renamed, with every pipeline, vertex, tag, topic and
environment replaced by a numbered token. Structure (shard indices, role suffixes,
tag-to-vertex references) is preserved so layouts stay representative.

Usage:
    anonymise-mermaid.py OUT_DIR IN_DIR...      # IN_DIR's basename is the environment
    anonymise-mermaid.py --check OUT_DIR IN_DIR...   # anonymise, then fail if any input token survives

Stdlib only. Run by hand; the output directory is git-ignored.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROLE_SUFFIXES = (
    "dlq-source", "source", "sink", "forwarder", "transform", "transformer", "processor",
    "assigner", "splitter", "decoder", "publisher", "dlq", "logger", "enricher", "router",
)
SHAPES = (('(["', '"])', "source"), ('[["', '"]]', "sink"), ('(("', '"))', "ghost"), ('("', '")', "ext"), ('["', '"]', "map"))
NODE_RE = re.compile(r'^\s*([A-Za-z0-9_]+)(\(\["|\[\["|\(\("|\("|\[")(.*?)("\]\)|"\]\]|"\)\)|"\)|"\])\s*$')
EDGE_RE = re.compile(r'^\s*(\S+?)\s+(-->|-\.->)(?:\|([^|]*)\|)?\s+(\S+?)\s*$')
SUBGRAPH_RE = re.compile(r'^\s*subgraph\s+([A-Za-z0-9_]+)\["(.*)"\]\s*$')
SHARD_RE = re.compile(r"^(.*)-(\d+)$")


class Names:
    """Deterministic first-seen numbering per category."""

    def __init__(self) -> None:
        self.maps: dict[str, dict[str, str]] = {k: {} for k in ("pipeline", "vertex", "tag", "topic", "env", "ext", "prefix")}
        self.prefixes = {"pipeline": "p", "vertex": "v", "tag": "t", "topic": "topic", "env": "ns", "ext": "ext", "prefix": "x"}

    def get(self, cat: str, key: str) -> str:
        m = self.maps[cat]
        if key not in m:
            m[key] = f"{self.prefixes[cat]}{len(m) + 1}"
        return m[key]

    def env(self, name: str) -> str:
        return self.get("env", name)

    def pipeline(self, name: str) -> str:
        # `foo-<env>` keeps its environment suffix, mapped through the env table.
        for env in list(self.maps["env"]):
            if name.endswith("-" + env):
                return f"{self.get('pipeline', name[: -len(env) - 1])}-{self.env(env)}"
        return self.get("pipeline", name)

    def vertex(self, name: str) -> str:
        stem, idx = name, None
        m = SHARD_RE.match(stem)
        if m:
            stem, idx = m.group(1), m.group(2)
        role = next((r for r in ROLE_SUFFIXES if stem == r or stem.endswith("-" + r)), None)
        if role and stem != role:
            stem = stem[: -len(role) - 1]
            out = f"{self.get('vertex', stem)}-{role}"
        elif role:
            out = role  # a bare role word carries nothing
        else:
            out = self.get("vertex", stem)
        return f"{out}-{idx}" if idx is not None else out

    def tag(self, raw: str) -> str:
        raw = raw.strip()
        if ":" in raw:  # composite `prefix:inner`, inner often names a vertex
            prefix, inner = raw.split(":", 1)
            inner_out = self.vertex(inner) if inner in self.maps_seen_vertices else self.tag(inner)
            return f"{self.get('prefix', prefix)}:{inner_out}"
        stem, idx = raw, None
        m = SHARD_RE.match(raw)
        if m:
            stem, idx = m.group(1), m.group(2)
        out = self.get("tag", stem)
        return f"{out}-{idx}" if idx is not None else out

    maps_seen_vertices: set[str] = set()


def mermaid_id(pipeline: str, vertex: str) -> str:
    return f"{pipeline}__{vertex}".replace("-", "_")


def anonymise_file(text: str, env: str, names: Names) -> tuple[str, str]:
    """Returns (output text, suggested file stem)."""
    out: list[str] = ["graph LR"]
    id_map: dict[str, str] = {}  # old node id -> new node id
    id_pipeline: dict[str, str] = {}  # old node id -> new pipeline name
    cur_pipeline: str | None = None
    first_pipeline: str | None = None
    pipelines = 0
    pending_edges: list[tuple[str, str, str | None, str, int]] = []

    # Pass 1: names. Subgraphs give pipelines; node declarations give vertices.
    lines = text.splitlines()
    for line in lines:
        m = SUBGRAPH_RE.match(line)
        if m:
            cur_pipeline = names.pipeline(m.group(2))
            first_pipeline = first_pipeline or cur_pipeline
            pipelines += 1
            id_map[m.group(1)] = mermaid_id(cur_pipeline, "cluster")
            continue
        if line.strip() == "end":
            cur_pipeline = None
            continue
        m = NODE_RE.match(line)
        if m and cur_pipeline is not None:
            label = m.group(3)
            names.maps_seen_vertices.add(label)
            v = names.vertex(label)
            id_map[m.group(1)] = mermaid_id(cur_pipeline, v)
            id_pipeline[m.group(1)] = cur_pipeline

    # Pass 2: emit.
    cur_pipeline = None
    depth = 0
    for line in lines:
        s = line.strip()
        if not s or s.startswith("%%") or s.startswith("style ") or s.startswith("graph ") or s.startswith("flowchart "):
            continue
        m = SUBGRAPH_RE.match(line)
        if m:
            cur_pipeline = id_map[m.group(1)].rsplit("__", 1)[0].replace("_", "-")
            out.append(f'  subgraph {id_map[m.group(1)]}["{names.pipeline(m.group(2))}"]')
            depth += 1
            continue
        if s == "end":
            out.append("  end")
            depth -= 1
            cur_pipeline = None
            continue
        m = NODE_RE.match(line)
        if m:
            old, open_, label, close = m.groups()
            if old in id_map:
                new_label = names.vertex(label)
                out.append(f"    {id_map[old]}{open_}{new_label}{close}")
            continue
        m = EDGE_RE.match(line)
        if m:
            src, op, label, tgt = m.groups()
            src_id, tgt_id = _edge_endpoint(src, id_map, names, out), _edge_endpoint(tgt, id_map, names, out)
            if label is not None:
                if op == "-.->":
                    new_label = names.get("topic", label.strip())
                else:
                    new_label = ", ".join(names.tag(t) for t in label.split(","))
                out.append(f"{'    ' if depth else '  '}{src_id} {op}|{new_label}| {tgt_id}")
            else:
                out.append(f"{'    ' if depth else '  '}{src_id} {op} {tgt_id}")
            continue
        raise SystemExit(f"unrecognised line: {line!r}")
    stem = f"composite-{names.env(env)}" if pipelines > 1 else (first_pipeline or f"empty-{names.env(env)}")
    return "\n".join(out) + "\n", stem


def _edge_endpoint(token: str, id_map: dict[str, str], names: Names, out: list[str]) -> str:
    """Bare id, or an inline declaration like `ext__foo("label")` / `id(("p/v"))`."""
    m = NODE_RE.match(token)
    if not m:
        if token not in id_map:
            # Unknown bare id: keep it structurally but anonymised.
            id_map[token] = "n" + names.get("ext", token)
        return id_map[token]
    old, open_, label, close = m.groups()
    if old not in id_map:
        if open_ == '(("' and "/" in label:  # ghost `pipeline/vertex`
            p, v = label.split("/", 1)
            names.maps_seen_vertices.add(v)
            id_map[old] = mermaid_id(names.pipeline(p), names.vertex(v)) + "_ghost"
            new_label = f"{names.pipeline(p)}/{names.vertex(v)}"
        else:  # external source: label is infrastructure detail, replace whole
            id_map[old] = "ext__" + names.get("ext", label)
            new_label = names.get("ext", label)
        out.append(f"  {id_map[old]}{open_}{new_label}{close}")
    return id_map[old]


def check(out_dir: Path, names: Names) -> int:
    """Fail if any original name (pipeline, vertex stem, tag, topic, env, external) survives."""
    tokens = set()
    for cat, m in names.maps.items():
        for k in m:
            for part in re.split(r"[^A-Za-z0-9]+", k):
                if len(part) >= 4 and part.lower() not in ROLE_SUFFIXES:
                    tokens.add(part.lower())
    bad = 0
    for f in sorted(out_dir.glob("*.mmd")):
        low = f.read_text().lower()
        for t in sorted(tokens):
            if re.search(rf"(?<![a-z0-9]){re.escape(t)}(?![a-z0-9])", low):
                print(f"LEAK {f.name}: {t}")
                bad += 1
    return bad


def main(argv: list[str]) -> int:
    do_check = "--check" in argv
    argv = [a for a in argv if a != "--check"]
    if len(argv) < 2:
        print(__doc__)
        return 2
    out_dir, in_dirs = Path(argv[0]), [Path(a) for a in argv[1:]]
    out_dir.mkdir(parents=True, exist_ok=True)
    names = Names()
    for d in in_dirs:
        names.env(d.name)
    written = 0
    for d in in_dirs:
        for f in sorted(d.glob("*.mmd")):
            text, stem = anonymise_file(f.read_text(), d.name, names)
            target = out_dir / f"{stem}.mmd"
            if target.exists() and target.read_text() != text:
                target = out_dir / f"{stem}-{names.env(d.name)}.mmd"
            target.write_text(text)
            written += 1
    (out_dir / "mapping.json").write_text(json.dumps(names.maps, indent=1, sort_keys=True))
    print(f"wrote {written} files to {out_dir}")
    if do_check:
        bad = check(out_dir, names)
        print("check:", "clean" if bad == 0 else f"{bad} leaks")
        return 1 if bad else 0
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
