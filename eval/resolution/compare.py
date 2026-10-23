#!/usr/bin/env python3
"""How well one IR document's calls match another's, taken as the truth.

    compare.py CANDIDATE.json TRUTH.json              precision, recall, top fan-in
    compare.py CANDIDATE.json TRUTH.json names        guessed targets by method name
    compare.py CANDIDATE.json TRUTH.json missed [NAME]  true targets the candidate lacks
    compare.py CANDIDATE.json TRUTH.json wrong NAME     wrong guesses at methods NAME

The truth is usually `codeviz analyze --resolve rust-analyzer`, the candidate the same
source with `--resolve syntax`. Edges are compared per call site (unit id and call key);
call sites where the truth itself fell back to a guess are not judged. Source lines are
read from the candidate's `root`.
"""
import json
import os
import sys
from collections import Counter, defaultdict


def load(path):
    document = json.load(open(path))
    root = document.get("root", "file://")[len("file://"):]
    sites = {}
    for unit in document["units"]:
        for call in unit.get("calls", []):
            targets = [(t["unit"], t.get("certainty", "exact")) for t in call.get("targets", [])]
            sites[(unit["id"], call["key"])] = (call, targets)
    return root, sites


def line(root, call):
    location = call["location"]
    number = location["range"]["start_line"]
    text = open(os.path.join(root, location["uri"])).read().splitlines()[number - 1].strip()
    return f"{location['uri']}:{number}  {text[:100]}"


def method(unit):
    return unit.rsplit("::", 1)[-1]


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    root, candidate = load(argv[0])
    _, truth = load(argv[1])
    mode, name = (argv[2] if len(argv) > 2 else "summary"), (argv[3] if len(argv) > 3 else None)
    # Only call sites the truth resolved itself.
    judged = {k for k, (_, ts) in truth.items() if all(c != "inferred" for _, c in ts)}
    judged &= candidate.keys()
    want = {k: {t for t, _ in truth[k][1]} for k in judged}

    if mode == "summary":
        counts = Counter(c for _, ts in candidate.values() for _, c in ts)
        unresolved = sum(1 for _, ts in candidate.values() if not ts)
        print(f"call sites {len(candidate)}  targets {sum(counts.values())}  "
              + "  ".join(f"{c} {n}" for c, n in sorted(counts.items()))
              + f"  unresolved sites {unresolved}")
        right, wrong, missed = Counter(), Counter(), 0
        for k in judged:
            got = candidate[k][1]
            for target, certainty in got:
                (right if target in want[k] else wrong)[certainty] += 1
            missed += len(want[k] - {t for t, _ in got})
        hits, made = sum(right.values()), sum(right.values()) + sum(wrong.values())
        print(f"judged sites {len(judged)}  precision {hits}/{made} = {hits / max(made, 1):.3f}  "
              f"recall {hits}/{hits + missed} = {hits / max(hits + missed, 1):.3f}")
        for certainty in sorted(set(right) | set(wrong)):
            n = right[certainty] + wrong[certainty]
            print(f"  {certainty:9} precision {right[certainty]}/{n} = {right[certainty] / n:.3f}")
        callers, true_callers = defaultdict(set), defaultdict(set)
        for (unit, _), (_, ts) in candidate.items():
            for target, _ in ts:
                callers[target].add(unit)
        for (unit, _), (_, ts) in truth.items():
            for target, _ in ts:
                true_callers[target].add(unit)
        print("top fan-in (distinct callers: candidate / truth)")
        for target, units in sorted(callers.items(), key=lambda kv: -len(kv[1]))[:12]:
            print(f"  {len(units):4} / {len(true_callers.get(target, ())):4}  {target}")
    elif mode == "names":
        right, wrong = Counter(), Counter()
        for k in judged:
            for target, certainty in candidate[k][1]:
                if certainty == "inferred":
                    (right if target in want[k] else wrong)[method(target)] += 1
        for n in sorted(set(right) | set(wrong), key=lambda n: -(right[n] + wrong[n]))[:40]:
            print(f"{n:28} right {right[n]:4}  wrong {wrong[n]:4}")
    elif mode == "missed":
        names, shown = Counter(), 0
        for k in judged:
            got = {t for t, _ in candidate[k][1]}
            for target in want[k] - got:
                names[method(target)] += 1
                if name and method(target) == name and shown < 12:
                    print(f"{line(root, candidate[k][0])}\n    want {target}  got {sorted(got)}")
                    shown += 1
        if not name:
            print(names.most_common(30))
    elif mode == "wrong":
        shown = 0
        for k in judged:
            bad = [t for t, _ in candidate[k][1] if t not in want[k] and method(t) == name]
            if bad and shown < 12:
                print(f"{line(root, candidate[k][0])}\n    got {bad}  truth {sorted(want[k])}")
                shown += 1
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
