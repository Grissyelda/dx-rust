#!/usr/bin/env python3
"""AI audit: the original game's pawn AI source next to our Rust port.

Deus Ex's .u packages carry the original UnrealScript source (dxtool extract-src writes it out, see the README), so
nothing has to be decompiled: this tool reads that source, indexes the AI classes (states, the functions each state
overrides, the labels of their state code, the latent calls in them, `ignores` lists and the numbers they use) and
lines it up with the Rust side, where every ported function / label carries a marker comment:

    // uc: ScriptedPawn.Attacking.PickDestination
    // uc: ScriptedPawn.Attacking:Fire          (a label of the state code)
    // uc: ScriptedPawn.SetEnemy                 (a function outside any state)

The marker covers the Rust item that follows it, up to the next marker or the end of the function's braces.

Usage:
    ucaudit.py <extracted source root> <rust source dir[,dir...]> [--out report.md] [--json index.json]
               [--classes ScriptedPawn,HumanMilitary,...] [--state Attacking]

The report lists, per class and state: every function and label with its source lines, whether a marker covers it,
the latent calls it makes, and the numbers in it that the Rust code under the marker never mentions (a quick check
for a constant mistyped or left out). Items without a marker are the port's gaps.

Nothing from the game is written into the repository: the source and the reports stay on your machine.
"""
import json
import os
import re
import sys
from collections import defaultdict

# The pawn AI's classes, in inheritance order. Subclasses only matter where they override something.
DEFAULT_CLASSES = [
    "Engine/Pawn",
    "DeusEx/ScriptedPawn",
    "DeusEx/HumanMilitary",
    "DeusEx/HumanThug",
    "DeusEx/HumanCivilian",
    "DeusEx/Robot",
    "DeusEx/Animal",
    "DeusEx/MJ12Troop",
    "DeusEx/MJ12Commando",
    "DeusEx/Terrorist",
    "DeusEx/UNATCOTroop",
    "DeusEx/Doberman",
    "DeusEx/Karkian",
    "DeusEx/Greasel",
    "DeusEx/Gray",
    "DeusEx/MIB",
    "DeusEx/WIB",
    "DeusEx/SecurityBot2",
    "DeusEx/SecurityBot3",
    "DeusEx/SecurityBot4",
    "DeusEx/MilitaryBot",
    "DeusEx/SpiderBot",
    "DeusEx/SpiderBot2",
    "DeusEx/CleanerBot",
    "DeusEx/MedicalBot",
    "DeusEx/RepairBot",
    "DeusEx/AnnaNavarre",
    "DeusEx/GuntherHermann",
    "DeusEx/WaltonSimons",
]

# UnrealScript's latent (time-taking) natives and the DX ones the AI uses.
LATENT = {
    "MoveTo", "MoveToward", "StrafeTo", "StrafeFacing", "TurnTo", "TurnToward", "FinishAnim", "FinishInterpolation",
    "Sleep", "WaitForLanding", "WaitForInterpolation", "FinishedMoving",
}


def strip_comments(text):
    """Comments out, line structure kept (so line numbers stay right); strings left alone."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * text.count("\n", i, j))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def line_of(text, pos):
    return text.count("\n", 0, pos) + 1


def match_brace(text, open_pos):
    depth = 0
    i = open_pos
    n = len(text)
    while i < n:
        c = text[i]
        if c == '"':
            i = text.find('"', i + 1) + 1
            if i == 0:
                return n
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n


FUNC_RE = re.compile(
    r"^[ \t]*((?:(?:static|final|native|simulated|singular|latent|iterator|private|protected|exec|"
    r"function|event|operator|preoperator|postoperator)\b[ \t]*(?:\(\d+\))?[ \t]*)+)"
    r"([\w<>]+[ \t]+)?(\w+)[ \t]*\(",
    re.M | re.I,
)
STATE_RE = re.compile(r"^[ \t]*(?:auto[ \t]+|simulated[ \t]+)*state[ \t]*(?:\(\))?[ \t]+(\w+)(?:[ \t]+extends[ \t]+(\w+))?",
                      re.M | re.I)
LABEL_RE = re.compile(r"^[ \t]*(\w+):[ \t]*$", re.M)
NUM_RE = re.compile(r"(?<![\w.])(\d+\.\d*|\.\d+|\d+)(?![\w.])")
CALL_RE = re.compile(r"\b(\w+)[ \t]*\(")
IGNORES_RE = re.compile(r"^[ \t]*ignores\b([^;]*);", re.M | re.I)


def numbers(body):
    found = set()
    for m in NUM_RE.finditer(body):
        try:
            v = float(m.group(1))
        except ValueError:
            continue
        if v in (0.0, 1.0):
            continue
        found.add(round(v, 4))
    return found


def parse_functions(text, start, end, base_line_text):
    """Functions defined between start and end (a class body or a state body)."""
    funcs = []
    pos = start
    while True:
        m = FUNC_RE.search(text, pos, end)
        if not m:
            break
        name = m.group(3)
        # A declaration ends at ';' (native, no body) or '{' (body).
        semi = text.find(";", m.end(), end)
        brace = text.find("{", m.end(), end)
        if brace < 0 or (0 <= semi < brace and "native" in m.group(1).lower()):
            funcs.append({"name": name, "line": line_of(base_line_text, m.start()), "native": True, "body": ""})
            pos = (semi if semi >= 0 else m.end()) + 1
            continue
        close = match_brace(text, brace)
        body = text[brace + 1:close]
        funcs.append({
            "name": name,
            "line": line_of(base_line_text, m.start()),
            "end": line_of(base_line_text, close),
            "native": "native" in m.group(1).lower(),
            "body": body,
        })
        pos = close + 1
    return funcs


def summarize_body(body):
    calls = [c for c in CALL_RE.findall(body) if c not in ("if", "while", "for", "switch", "return", "foreach")]
    return {
        "latent": sorted({c for c in calls if c in LATENT}),
        "calls": sorted(set(calls)),
        "numbers": sorted(numbers(body)),
        "lines": body.count("\n") + 1,
    }


def parse_class(path):
    raw = open(path, encoding="latin-1").read()
    text = strip_comments(raw)
    cls = os.path.splitext(os.path.basename(path))[0]
    ext = re.search(r"\bclass\s+\w+\s+extends\s+(\w+)", text, re.I)
    out = {"class": cls, "extends": ext.group(1) if ext else None, "file": path, "functions": {}, "states": {}}
    # States first: their spans are excluded from the class-level function scan.
    spans = []
    for m in STATE_RE.finditer(text):
        brace = text.find("{", m.end())
        if brace < 0:
            continue
        close = match_brace(text, brace)
        spans.append((m.start(), close))
        sbody = text[brace + 1:close]
        st = {
            "name": m.group(1),
            "extends": m.group(2),
            "line": line_of(text, m.start()),
            "end": line_of(text, close),
            "ignores": [],
            "functions": {},
            "labels": {},
        }
        ig = IGNORES_RE.search(sbody)
        if ig:
            st["ignores"] = [s.strip() for s in ig.group(1).split(",") if s.strip()]
        for f in parse_functions(text, brace + 1, close, text):
            st["functions"][f["name"]] = {k: v for k, v in f.items() if k != "body"} | summarize_body(f["body"])
        # State code: whatever follows the last function, split by labels.
        last_end = brace + 1
        for f in parse_functions(text, brace + 1, close, text):
            if not f.get("native"):
                # find the closing brace of this function again by line
                pass
        code_start = brace + 1
        # The labels' code: blank out function bodies, then cut at labels.
        masked = list(text[brace + 1:close])
        for fm in FUNC_RE.finditer(text, brace + 1, close):
            b = text.find("{", fm.end(), close)
            s = text.find(";", fm.end(), close)
            if b < 0 or (0 <= s < b):
                continue
            e = match_brace(text, b)
            for k in range(fm.start() - (brace + 1), min(e - (brace + 1) + 1, len(masked))):
                if masked[k] != "\n":
                    masked[k] = " "
        code = "".join(masked)
        labels = list(LABEL_RE.finditer(code))
        for i, lm in enumerate(labels):
            body = code[lm.end(): labels[i + 1].start() if i + 1 < len(labels) else len(code)]
            st["labels"][lm.group(1)] = {
                "line": line_of(text, brace + 1 + lm.start()),
            } | summarize_body(body)
        out["states"][st["name"]] = st
    # Class-level functions (outside states).
    mask = list(text)
    for a, b in spans:
        for k in range(a, b + 1):
            if mask[k] != "\n":
                mask[k] = " "
    ctext = "".join(mask)
    for f in parse_functions(ctext, 0, len(ctext), ctext):
        out["functions"][f["name"]] = {k: v for k, v in f.items() if k != "body"} | summarize_body(f["body"])
    # Default properties (numbers worth comparing).
    dp = re.search(r"defaultproperties\s*\{(.*?)\}", text, re.S | re.I)
    out["defaults"] = {}
    if dp:
        for line in dp.group(1).splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                out["defaults"][k.strip()] = v.strip()
    return out


# ---------------------------------------------------------------------------------------------------------------
# The Rust side

MARK_RE = re.compile(r"//[/!]?\s*uc:\s*([\w.:]+(?:\s*,\s*[\w.:]+)*)")


def scan_rust(src_dir):
    """uc markers -> list of (file, line, rust text under the marker)."""
    marks = defaultdict(list)
    for root, _, files in os.walk(src_dir):
        for fn in files:
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(root, fn)
            lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
            hits = [(i, m) for i, l in enumerate(lines) for m in [MARK_RE.search(l)] if m]
            for k, (i, m) in enumerate(hits):
                # The covered text: from the marker to the next marker, at most 600 lines.
                end = hits[k + 1][0] if k + 1 < len(hits) else min(len(lines), i + 600)
                end = min(end, i + 600)
                chunk = "\n".join(lines[i:end])
                for key in re.split(r"\s*,\s*", m.group(1)):
                    marks[key.strip()].append({"file": os.path.relpath(path, src_dir), "line": i + 1, "text": chunk})
    return marks


def rust_numbers(text):
    vals = set()
    for m in re.finditer(r"(?<![\w.])(\d+(?:_\d+)*(?:\.\d*)?(?:e-?\d+)?)(?:f32|f64|u\d+|i\d+|usize)?(?![\w])", text):
        try:
            vals.add(round(float(m.group(1).replace("_", "")), 4))
        except ValueError:
            pass
    return vals


def key_variants(cls, state, item):
    """Marker spellings that count for an item (a subclass's marker may name the defining class)."""
    if state:
        return [f"{cls}.{state}.{item}", f"{cls}.{state}:{item}"]
    return [f"{cls}.{item}"]


def audit(index, marks, only_state=None):
    rows = []
    for cls in index:
        c = index[cls]
        for fname, f in sorted(c["functions"].items(), key=lambda kv: kv[1]["line"]):
            if f.get("native"):
                continue
            rows.append(("function", cls, None, fname, f, marks_for(marks, key_variants(cls, None, fname))))
        for sname, st in sorted(c["states"].items(), key=lambda kv: kv[1]["line"]):
            if only_state and sname != only_state:
                continue
            for fname, f in sorted(st["functions"].items(), key=lambda kv: kv[1]["line"]):
                rows.append(("state-fn", cls, sname, fname, f, marks_for(marks, [f"{cls}.{sname}.{fname}"])))
            for lname, l in sorted(st["labels"].items(), key=lambda kv: kv[1]["line"]):
                rows.append(("label", cls, sname, lname, l, marks_for(marks, [f"{cls}.{sname}:{lname}"])))
    return rows


def marks_for(marks, keys):
    out = []
    for k in keys:
        out.extend(marks.get(k, []))
    return out


def report(index, rows, marks, out):
    w = out.write
    total = len(rows)
    covered = sum(1 for r in rows if r[5])
    w("# AI port audit\n\n")
    w(f"{covered} of {total} original functions / labels carry a `uc:` marker in the Rust code.\n\n")
    known = set()
    for r in rows:
        kind, cls, st, name = r[0], r[1], r[2], r[3]
        known.update(key_variants(cls, st, name))
    stray = sorted(k for k in marks if k not in known)
    if stray:
        w("## Markers that name nothing in the source\n\n")
        for k in stray:
            locs = ", ".join(f"{m['file']}:{m['line']}" for m in marks[k])
            w(f"- `{k}` ({locs})\n")
        w("\n")
    by_group = defaultdict(list)
    for r in rows:
        by_group[(r[1], r[2])].append(r)
    for (cls, st), items in by_group.items():
        c = index[cls]
        title = f"{cls}" + (f" - state {st}" if st else " - functions")
        done = sum(1 for r in items if r[5])
        w(f"## {title} ({done}/{len(items)})\n\n")
        if st:
            s = c["states"][st]
            if s["ignores"]:
                w(f"ignores: {', '.join(s['ignores'])}\n\n")
        w("| item | source line | lines | latent calls | ported at | numbers not in the port |\n|---|---|---|---|---|---|\n")
        for kind, cls_, st_, name, info, ms in items:
            label = f"`{name}:`" if kind == "label" else f"`{name}()`"
            where = ", ".join(f"{m['file']}:{m['line']}" for m in ms) or "**missing**"
            missing_nums = ""
            if ms:
                rn = set()
                for m in ms:
                    rn |= rust_numbers(m["text"])
                miss = [n for n in info.get("numbers", []) if n not in rn and (n * 65536 / 360) not in rn]
                missing_nums = ", ".join(f"{n:g}" for n in miss[:12])
            w(f"| {label} | {info['line']} | {info.get('lines', '')} | {', '.join(info.get('latent', []))} | {where} | {missing_nums} |\n")
        w("\n")


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    src_root, rust_dir = argv[1], argv[2]
    out_path, json_path, only_state = "ai_audit.md", None, None
    classes = DEFAULT_CLASSES
    i = 3
    while i < len(argv):
        a = argv[i]
        if a == "--out":
            out_path = argv[i + 1]; i += 2
        elif a == "--json":
            json_path = argv[i + 1]; i += 2
        elif a == "--state":
            only_state = argv[i + 1]; i += 2
        elif a == "--classes":
            names = argv[i + 1].split(",")
            classes = [c for c in DEFAULT_CLASSES if c.split("/")[1] in names] or [f"DeusEx/{n}" for n in names]
            i += 2
        else:
            print("unknown option", a); return 2
    index = {}
    for c in classes:
        pkg, name = c.split("/")
        path = os.path.join(src_root, pkg, "Classes", name + ".uc")
        if not os.path.exists(path):
            print("not found:", path, file=sys.stderr)
            continue
        index[name] = parse_class(path)
    # Several Rust folders may be given, separated by commas (crates/dxview/src,crates/imsim-core/src).
    marks = defaultdict(list)
    for d in rust_dir.split(","):
        for k, v in scan_rust(d).items():
            marks[k].extend(v)
    rows = audit(index, marks, only_state)
    with open(out_path, "w", encoding="utf-8") as f:
        report(index, rows, marks, f)
    if json_path:
        with open(json_path, "w", encoding="utf-8") as f:
            json.dump(index, f, indent=1)
    covered = sum(1 for r in rows if r[5])
    print(f"{covered}/{len(rows)} items ported -> {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
