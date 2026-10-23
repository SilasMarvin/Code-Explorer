#!/usr/bin/env python3
"""Measures how much of a codeviz view a reader gets right from its screenshot alone.

`render` draws each scenario in scenarios.json bare (only the graph and the journey),
reads what the view means to show from the `.truth.json` beside the screenshot, and
writes a quiz for it: questions with answer keys, and a prompt per condition for a
reader who sees only the image. `grade` scores the answers readers wrote and compares
the run with an earlier one. The `readability` workflow in .claude/workflows runs both,
with agents as the readers.

    python3 eval/readability/readability.py render --run NAME [--only A,B]
    python3 eval/readability/readability.py grade --run NAME [--baseline NAME]

Each run lives in target/readability/NAME: shots/ and prompts/ for readers, keys/ for
the grader, answers/ for what readers wrote, report.json for the scores.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
RUNS = ROOT / "target" / "readability"

# A screenshot that takes more memory or time than this is a bug in the viewer: kill
# it before it takes the machine with it.
LIMIT_MB = 2000
LIMIT_SECONDS = 120
# What the window measures in points: what a person sees at 1x.
IMAGE_SIZE = 1600

ASPECTS = {
    "focus": "identity",
    "focus_change": "diff",
    "calls": "order",
    "callees": "nesting",
    "hidden": "folding",
    "fails": "failure",
    "conditional": "condition",
    "callers": "callers",
    "module": "link",
    "added": "diff",
    "removed": "diff",
    "modified": "diff",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    render_args = sub.add_parser("render", help="screenshot each scenario and write its quiz")
    render_args.add_argument("--run", default="latest")
    render_args.add_argument("--only", default="", help="comma-separated scenario names")
    render_args.add_argument("--conditions", default="cold", help="cold, primed, or both")
    grade_args = sub.add_parser("grade", help="score a run's answers")
    grade_args.add_argument("--run", default="latest")
    grade_args.add_argument("--baseline", help="an earlier run to compare with")
    args = parser.parse_args()
    if args.command == "render":
        render(args.run, [s for s in args.only.split(",") if s], args.conditions.split(","))
    else:
        grade(args.run, args.baseline)


# Rendering ----------------------------------------------------------------------------


def render(run, only, conditions):
    scenarios = json.loads((HERE / "scenarios.json").read_text())
    names = only or list(scenarios)
    unknown = [n for n in names if n not in scenarios]
    if unknown:
        sys.exit(f"unknown scenarios: {', '.join(unknown)}")
    out = RUNS / run
    if out.exists():
        shutil.rmtree(out)
    for part in ("shots", "prompts", "keys", "answers"):
        (out / part).mkdir(parents=True)
    build()
    listed = []
    for name in names:
        shot = out / "keys" / f"{name}.png"
        command = [str(ROOT / "target" / "debug" / "codeviz"), *scenarios[name]["command"]]
        command += ["--bare", "--screenshot", str(shot)]
        guarded(command, out / "keys" / f"{name}.log")
        truth = json.loads(shot.with_suffix(".truth.json").read_text())
        image = out / "shots" / f"{name}.png"
        shrink(shot, image)
        questions = quiz(truth)
        (out / "keys" / f"{name}.questions.json").write_text(json.dumps(questions, indent=2))
        prompts = {}
        for condition in conditions:
            path = out / "prompts" / f"{name}.{condition}.txt"
            path.write_text(prompt(image, questions, condition))
            prompts[condition] = str(path)
        listed.append({"name": name, "questions": len(questions), "prompts": prompts})
        print(f"{name}: {len(questions)} questions", file=sys.stderr)
    print(json.dumps({"run": run, "dir": str(out), "scenarios": listed}))


def build():
    cargo = ["mise", "exec", "--", "cargo"] if shutil.which("mise") else ["cargo"]
    subprocess.run([*cargo, "build", "-q", "-p", "codeviz"], cwd=ROOT, check=True)


def guarded(command, log):
    """Runs `command`, killing its process tree past LIMIT_MB or LIMIT_SECONDS."""
    with open(log, "w") as out:
        process = subprocess.Popen(
            command, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT, start_new_session=True
        )
        start = time.monotonic()
        while process.poll() is None:
            used = tree_rss_mb(process.pid)
            if used > LIMIT_MB or time.monotonic() - start > LIMIT_SECONDS:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                sys.exit(f"killed {command[1]} at {used} MB after {time.monotonic() - start:.0f}s")
            time.sleep(0.2)
    if process.returncode != 0:
        sys.exit(f"{' '.join(command)} failed; see {log}")


def tree_rss_mb(pid):
    rows = subprocess.run(
        ["ps", "-A", "-o", "pid=,ppid=,rss="], capture_output=True, text=True
    ).stdout.split()
    table = [tuple(map(int, rows[i : i + 3])) for i in range(0, len(rows) - 2, 3)]
    tree, total = {pid}, 0
    for _ in range(4):
        tree |= {p for p, parent, _ in table if parent in tree}
    total = sum(rss for p, _, rss in table if p in tree)
    return total // 1024


def shrink(source, target):
    if shutil.which("sips"):
        subprocess.run(
            ["sips", "-Z", str(IMAGE_SIZE), str(source), "--out", str(target)],
            capture_output=True,
            check=True,
        )
    else:
        shutil.copy(source, target)


# Questions ----------------------------------------------------------------------------


def quiz(truth):
    """Questions about what the view means to show, each with its answer key."""
    calls = truth["calls"]
    questions = []

    def ask(id, text, kind, key):
        questions.append({"id": id, "aspect": ASPECTS[id.split(":")[0]], "q": text, "kind": kind, "key": key})

    ask("focus", "Which function is this view examining: the one at the center of attention?", "one", [truth["focus"]])
    if truth["changes"] is not None:
        ask(
            "focus_change",
            "Did the examined function change? Answer one of: added, modified, unchanged.",
            "one",
            [truth["focus_change"] or "unchanged"],
        )
    direct = [c for c in calls if c["parent"] == ""]
    if direct:
        ask("calls", "List the functions the examined function calls directly, in the order it calls them.", "order", names(direct))
    # Nesting: what the shown calls with the most shown calls of their own call.
    parents = sorted(
        (c for c in calls if any(k["parent"] == c["number"] for k in calls)),
        key=lambda c: -sum(k["parent"] == c["number"] for k in calls),
    )
    for call in parents[:2]:
        ask(
            f"callees:{call['number']}",
            f"What does {call['name']} call, in order?",
            "order",
            names(k for k in calls if k["parent"] == call["number"]),
        )
    folded = sorted((c for c in calls if c["hidden_calls"]), key=lambda c: -c["hidden_calls"])
    if folded:
        call = folded[len(folded) // 2]
        ask(
            "hidden",
            f"How many of the calls {call['name']} makes are not shown individually? Answer with a number.",
            "count",
            [str(call["hidden_calls"])],
        )
    ask(
        "fails",
        "Which of the functions shown in the journey can end with an error themselves? Answer an empty list if none.",
        "set",
        ([truth["focus"]] if truth["focus_fails"] else []) + names(c for c in calls if c["fails"]),
    )
    ask(
        "conditional",
        "Which of the calls shown only happen under some condition (or in a loop), rather than exactly once every time? Answer an empty list if none.",
        "set",
        names(c for c in calls if c["conditional"]),
    )
    # Linking the journey to the graph: a call's number leads to its node, and its node
    # sits in a module's ring. One call per module, deepest first, up to three.
    seen = set()
    for call in sorted(calls, key=lambda c: (-c["number"].count("."), c["number"])):
        if len(seen) == 3 or not call.get("module") or call["module"] in seen:
            continue
        seen.add(call["module"])
        ask(
            f"module:{call['number']}",
            f"In which module (the capitalized name on a ring) does call {call['number']} of the journey live?",
            "one",
            [call["module"]],
        )
    if truth["callers"] and len(truth["callers"]) <= 8:
        ask("callers", "What calls the examined function?", "set", truth["callers"])
    changes = truth["changes"]
    if changes is not None:
        ask("added", "Which functions are new in this change? Answer an empty list if none.", "set", changes["added"])
        ask("removed", "Which functions were removed? Answer an empty list if none.", "set", changes["removed"])
        ask(
            "modified",
            "Which functions that already existed were modified? Answer an empty list if none.",
            "set",
            changes["modified"] + changes["moved"],
        )
    return questions


def names(calls):
    return [c["name"] for c in calls]


PRIMER = """How to read this tool: each dot is a function. Rings group functions by module, with the module name in capitals; a node is filled with its module's color. The white ringed node is the function being examined. Blue arrows run from it to the functions it calls, violet arrows into it from its callers ("calls it #k of n"), with dots flowing in the direction of the call. The number on a colored badge before a name is the order of the call (1 runs first; 3.2 is the second call made inside call 3). The panel at the bottom right is a flame graph: the top bar is the examined function, each row below is one level deeper, left to right is the order in time, and each bar is filled with the same color as its node. "+N" on a bar means N more calls inside it are hidden. A red stripe at a bar's right end means that call can fail, splitting off an error outcome; a dashed outline means the call only happens under a condition. In a diff, a node and its bar are filled by what changed: "+" and green mean added, "~" and yellow mean changed, "−" and red, struck through, mean removed, gray means unchanged."""


def prompt(image, questions, condition):
    lines = [
        f"Look at the screenshot at {image} with the Read tool. It shows a code visualization tool "
        "displaying part of a codebase. Answer only from what the image shows: do not read any "
        "other file, run commands, or search the codebase. The point is to measure what the "
        "image alone communicates.",
    ]
    if condition == "primed":
        lines += ["", PRIMER]
    lines += ["", "Questions:"]
    lines += [f"- {q['id']}: {q['q']}" for q in questions]
    lines += [
        "",
        "Answer every question. Give each answer as a list of strings: one item for a single "
        "name, number or word; items in order where order is asked; an empty list for none. "
        "Name functions as the image names them. If the image does not let you tell, set "
        "cannot_tell and leave the list empty rather than guessing. Give your confidence "
        "from 0 to 1 that the answer is right.",
    ]
    return "\n".join(lines) + "\n"


# Grading ------------------------------------------------------------------------------


def grade(run, baseline):
    out = RUNS / run
    keys = {p.name.removesuffix(".questions.json"): json.loads(p.read_text()) for p in (out / "keys").glob("*.questions.json")}
    results = []
    for path in sorted((out / "answers").glob("*.json")):
        scenario, condition, reader = path.stem.rsplit(".", 2)
        answers = {a["id"]: a for a in json.loads(path.read_text())["answers"]}
        for q in keys[scenario]:
            a = answers.get(q["id"], {"answer": [], "cannot_tell": True, "confidence": 0})
            results.append(
                {
                    "scenario": scenario,
                    "condition": condition,
                    "reader": reader,
                    "id": q["id"],
                    "aspect": q["aspect"],
                    "score": 0.0 if a.get("cannot_tell") else score(q, a.get("answer") or []),
                    "abstained": bool(a.get("cannot_tell")),
                    "confidence": float(a.get("confidence", 0)),
                    "answer": a.get("answer"),
                    "key": q["key"],
                }
            )
    if not results:
        sys.exit(f"no answers in {out / 'answers'}")
    report = summarize(results)
    (out / "report.json").write_text(json.dumps({"summary": report, "results": results}, indent=2))
    before = load_baseline(run, baseline)
    print_report(run, report, results, before)


def norm(name):
    name = re.sub(r"\(.*?\)", "", str(name)).strip().lower()
    return re.sub(r"^[\d.]*\s*[+~\-−]?\s*", "", name).strip()


def same(answer, key):
    answer, key = norm(answer), norm(key)
    if answer == key:
        return True
    if "/" in key or " " in key:
        return False
    tail = lambda n: re.split(r"\.|::", n)[-1]
    return tail(answer) == tail(key)


def score(q, answer):
    kind, key = q["kind"], q["key"]
    if kind == "one":
        return float(bool(answer) and same(answer[0], key[0]))
    if kind == "count":
        digits = re.findall(r"\d+", " ".join(map(str, answer)))
        return float(bool(digits) and digits[0] == key[0])
    if kind == "order":
        return lcs(answer, key) / max(len(answer), len(key)) if answer else 0.0
    if not key:
        return float(not answer)
    hits = sum(any(same(a, k) for a in answer) for k in key)
    if not answer or not hits:
        return 0.0
    precision = sum(any(same(a, k) for k in key) for a in answer) / len(answer)
    recall = hits / len(key)
    return 2 * precision * recall / (precision + recall)


def lcs(a, b):
    table = [[0] * (len(b) + 1) for _ in range(len(a) + 1)]
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            table[i + 1][j + 1] = table[i][j] + 1 if same(x, y) else max(table[i][j + 1], table[i + 1][j])
    return table[-1][-1]


def mean(values):
    values = list(values)
    return sum(values) / len(values) if values else 0.0


def summarize(results):
    def group(field):
        out = {}
        for r in results:
            out.setdefault(r[field], []).append(r)
        return {k: {"score": mean(r["score"] for r in rs), "n": len(rs)} for k, rs in out.items()}

    runs = {}
    for r in results:
        runs.setdefault((r["scenario"], r["condition"], r["reader"]), []).append(r["score"])
    return {
        "score": mean(r["score"] for r in results),
        "brier": mean((r["confidence"] - r["score"]) ** 2 for r in results),
        "abstained": mean(r["abstained"] for r in results),
        "by_scenario": group("scenario"),
        "by_aspect": group("aspect"),
        "by_condition": group("condition"),
        "by_reader": group("reader"),
        # How far one reader's score on a scenario strays from all readers' mean there:
        # differences smaller than this are noise.
        "spread": mean(
            abs(mean(s) - mean(mean(t) for k2, t in runs.items() if k2[0] == k[0])) for k, s in runs.items()
        ),
    }


def load_baseline(run, baseline):
    if baseline:
        path = RUNS / baseline / "report.json"
    else:
        earlier = [p for p in RUNS.glob("*/report.json") if p.parent.name != run]
        path = max(earlier, key=lambda p: p.stat().st_mtime, default=None)
    if path is None or not path.exists():
        return None
    return path.parent.name, json.loads(path.read_text())["summary"]


def print_report(run, report, results, before):
    def row(label, value, previous):
        delta = "" if previous is None else f"  ({value - previous:+.2f})"
        print(f"  {label:28}{value:5.2f}{delta}")

    old = before[1] if before else {}
    print(f"run {run}" + (f", compared with {before[0]}" if before else ""))
    row("score", report["score"], old.get("score"))
    row("brier (lower is better)", report["brier"], old.get("brier"))
    row("abstained", report["abstained"], old.get("abstained"))
    print(f"  {'reader spread (noise)':28}{report['spread']:5.2f}")
    for section in ("by_scenario", "by_aspect", "by_condition", "by_reader"):
        print(section.replace("_", " ") + ":")
        for name, value in sorted(report[section].items()):
            previous = old.get(section, {}).get(name, {}).get("score")
            row(f"{name} (n={value['n']})", value["score"], previous)
    misses = [r for r in results if r["score"] < 1]
    if misses:
        print("misses:")
        for r in misses:
            got = "cannot tell" if r["abstained"] else r["answer"]
            print(f"  {r['scenario']}.{r['condition']}.{r['reader']} {r['id']}: {r['score']:.2f}  got {got}  key {r['key']}")


if __name__ == "__main__":
    main()
