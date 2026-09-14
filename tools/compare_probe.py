"""Compare two width_probe TSV outputs (cp, eaw, gc, w). usage: compare_probe.py A.tsv B.tsv"""
import sys
from collections import Counter


def load(p):
    d = {}
    for line in open(p, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        if len(f) < 4 or f[3] == "":
            continue
        d[int(f[0])] = (f[1], f[2], int(f[3]))
    return d


a, b = load(sys.argv[1]), load(sys.argv[2])
common = sorted(set(a) & set(b))
diff = [(cp, a[cp], b[cp]) for cp in common if a[cp][2] != b[cp][2]]
print(f"common probes: {len(common)}  differing: {len(diff)}")
cls = Counter((x[1][2], x[2][2], x[1][1]) for x in diff)
for (wa, wb, gc), n in sorted(cls.items()):
    print(f"  A={wa} B={wb} gc={gc}: {n}")
# ranges
runs = []
for cp, va, vb in diff:
    key = (va[2], vb[2], va[0], va[1])
    if runs and runs[-1][1] == cp - 1 and runs[-1][2] == key:
        runs[-1][1] = cp
    else:
        runs.append([cp, cp, key])
for s, e, (wa, wb, eaw, gc) in runs:
    r = f"U+{s:04X}" if s == e else f"U+{s:04X}-U+{e:04X} ({e - s + 1})"
    print(f"  {r:26} A={wa} B={wb} eaw={eaw} gc={gc} [{chr(s)}]")
