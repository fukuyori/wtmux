#!/usr/bin/env python3
"""Measure how the *attached console* (conhost / ConPTY) advances the cursor
for each East Asian Ambiguous code point, using DSR (ESC[6n) / CPR.

usage: width_probe.py <EastAsianWidth.txt> <out-prefix> [codepage] [eaw-console.json]

Writes <out-prefix>.jsonl (one {"cp","cat","w"} per line) and
<out-prefix>.summary.txt.  Talks to CONOUT$/CONIN$ directly so it works even
when stdin/stdout are pipes (e.g. spawned from a cargo test harness).
"""
import ctypes
import json
import sys
import time
from ctypes import wintypes, byref

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.CreateFileW.restype = wintypes.HANDLE
k32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                            wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
k32.GetConsoleMode.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
k32.SetConsoleMode.argtypes = [wintypes.HANDLE, wintypes.DWORD]
k32.WriteConsoleW.argtypes = [wintypes.HANDLE, wintypes.LPCWSTR, wintypes.DWORD,
                              ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
k32.ReadConsoleW.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                             ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
k32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
k32.WaitForSingleObject.restype = wintypes.DWORD
k32.FlushConsoleInputBuffer.argtypes = [wintypes.HANDLE]
k32.SetConsoleOutputCP.argtypes = [wintypes.UINT]
k32.SetConsoleCP.argtypes = [wintypes.UINT]
k32.GetConsoleOutputCP.restype = wintypes.UINT

GENERIC_RW = 0x80000000 | 0x40000000
ENABLE_PROCESSED_OUTPUT = 0x1
ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x4
ENABLE_LINE_INPUT = 0x2
ENABLE_ECHO_INPUT = 0x4
ENABLE_VIRTUAL_TERMINAL_INPUT = 0x200


def con(name):
    h = k32.CreateFileW(name, GENERIC_RW, 3, None, 3, 0, None)
    if h == wintypes.HANDLE(-1).value or h is None:
        raise OSError(f"cannot open {name}: {ctypes.get_last_error()}")
    return h


def write(hout, s):
    n = wintypes.DWORD()
    buf = ctypes.create_unicode_buffer(s)
    k32.WriteConsoleW(hout, buf, len(buf) - 1, byref(n), None)


def read_cpr(hin, deadline_s=2.0):
    """Return (row, col) from the next CPR, or None on timeout."""
    acc = ""
    end = time.monotonic() + deadline_s
    buf = ctypes.create_unicode_buffer(256)
    n = wintypes.DWORD()
    while time.monotonic() < end:
        remaining = max(1, int((end - time.monotonic()) * 1000))
        if k32.WaitForSingleObject(hin, remaining) != 0:
            break
        if not k32.ReadConsoleW(hin, buf, 255, byref(n), None):
            break
        acc += buf.value[: n.value]
        i = acc.rfind("\x1b[")
        if i >= 0:
            j = acc.find("R", i)
            if j > i:
                body = acc[i + 2 : j]
                if ";" in body:
                    r, c = body.split(";", 1)
                    try:
                        return int(r), int(c)
                    except ValueError:
                        return None
    return None


def load_eaw(path):
    rows = []
    for line in open(path, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if not line:
            continue
        r, cat = [x.strip() for x in line.split(";")]
        a, *b = r.split("..")
        a = int(a, 16)
        b = int(b[0], 16) if b else a
        rows.append((a, b, cat))
    return rows


def load_gc(unicodedata_path):
    """General category per code point (ranges expanded), from UnicodeData.txt."""
    gc = {}
    first = None
    for line in open(unicodedata_path, encoding="utf-8"):
        row = line.split(";")
        cp, name, cat = int(row[0], 16), row[1], row[2]
        if name.endswith(", First>"):
            first = cp
            continue
        if name.endswith(", Last>"):
            for c in range(first, cp + 1):
                gc[c] = cat
            continue
        gc[cp] = cat
    return gc


def targets_all(eaw_rows, gc):
    """Every assigned, non-control, non-surrogate code point; PUA sampled."""
    def lookup(cp):
        for a, b, cat in eaw_rows:
            if a <= cp <= b:
                return cat
        return "N"
    out = []
    for cp, cat in sorted(gc.items()):
        if cat in ("Cc", "Cs"):
            continue
        if cat == "Co":
            if cp not in (0xE000, 0xE0B0, 0xEC7F, 0xEE00, 0xEE0B, 0xF8FF,
                          0xF0000, 0xF00B0, 0xFFFFD, 0x100000, 0x10FFFD):
                continue
        out.append((cp, lookup(cp), cat))
    return out


def targets(eaw_rows):
    cps = []
    for a, b, cat in eaw_rows:
        if cat != "A":
            continue
        if b - a + 1 > 300:
            # PUA / plane 15-16: sample only
            for cp in (a, a + 0xB0, (a + b) // 2, b):
                cps.append((cp, cat))
        else:
            cps.extend((cp, cat) for cp in range(a, b + 1))
    # reference points with unambiguous categories
    refs = [0x41, 0x65E5, 0x1F600, 0x2603, 0x24EA, 0x3251, 0x1F004, 0x1F0CF,
            0x2022, 0x2219, 0xFFE9, 0x2B29, 0xE0B0, 0xEE00, 0xEE0B,
            # representatives of the classes where inbox conhost != wtmux
            0x0300, 0x0301, 0x20DD, 0xFE0E, 0xFE0F, 0x200B, 0x200D, 0x00AD, 0xFEFF,
            0x1F1E6, 0x1F1EF, 0x1161, 0x11A8, 0xD7B0, 0xFF9E, 0xFFA0, 0x3099, 0x302A,
            0x3164, 0x2FFC, 0x31EF, 0x17A4, 0x17D8, 0x0D4E, 0xA8FA, 0x2764, 0x1F468]
    cat_of = {}

    def lookup(cp):
        for a, b, cat in eaw_rows:
            if a <= cp <= b:
                return cat
        return "N"

    for cp in refs:
        cps.append((cp, lookup(cp)))
    seen = set()
    out = []
    for cp, cat in cps:
        if cp in seen or 0xD800 <= cp <= 0xDFFF:
            continue
        seen.add(cp)
        out.append((cp, cat))
    return out


def main():
    eaw_path, out_prefix = sys.argv[1], sys.argv[2]
    codepage = int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3] != "-" else None
    console_json = sys.argv[4] if len(sys.argv) > 4 and sys.argv[4] != "-" else None
    mode = sys.argv[5] if len(sys.argv) > 5 else "amb"
    import os
    gc = load_gc(os.path.join(os.path.dirname(eaw_path), "UnicodeData.txt"))

    hout, hin = con("CONOUT$"), con("CONIN$")
    if codepage:
        k32.SetConsoleOutputCP(codepage)
        k32.SetConsoleCP(codepage)
    cp_now = k32.GetConsoleOutputCP()

    cmode = wintypes.DWORD()
    k32.GetConsoleMode(hout, byref(cmode))
    k32.SetConsoleMode(hout, cmode.value | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING)
    k32.GetConsoleMode(hin, byref(cmode))
    k32.SetConsoleMode(hin, (cmode.value | ENABLE_VIRTUAL_TERMINAL_INPUT) & ~(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT))
    k32.FlushConsoleInputBuffer(hin)

    # sanity: DSR must work at all
    write(hout, "\r\x1b[K\x1b[6n")
    base = read_cpr(hin)
    if base is None:
        with open(out_prefix + ".summary.txt", "w", encoding="utf-8") as f:
            f.write("ERROR: no CPR response from console (DSR unsupported here)\n")
        return 2

    if mode == "seq":
        # Multi-code-point sequences: what matters in practice (emoji presentation,
        # ZWJ families, flags, composed Hangul, dakuten), measured after a base 'a'.
        seqs = [
            ("heart+VS16", "❤️"), ("heart", "❤"),
            ("check+VS16", "✔️"), ("ballot+VS16", "☑️"),
            ("warning+VS16", "⚠️"), ("warning", "⚠"),
            ("copyright+VS16", "©️"), ("digit1+VS16+keycap", "1️⃣"),
            ("grinning", "\U0001F600"), ("grinning+VS16", "\U0001F600️"),
            ("family-zwj", "\U0001F468‍\U0001F469‍\U0001F467"),
            ("flag-JP", "\U0001F1EF\U0001F1F5"), ("RI-single", "\U0001F1EF"),
            ("thumbs+skin", "\U0001F44D\U0001F3FD"),
            ("a+grave", "à"), ("e-acute-precomposed", "é"),
            ("hangul-jamo-seq", "가"), ("hangul-syllable", "가"),
            ("halfwidth-ka+dakuten", "ｶﾞ"), ("hiragana-ka+dakuten", "が"),
            ("cjk+VS", "辻\U000E0100"), ("text-VS15-heart", "❤︎"),
            ("nerd-powerline", ""), ("tab-fullwidth", "　"),
        ]
        lines = [f"mode: seq", f"console output codepage: {cp_now}"]
        for name, sq in seqs:
            write(hout, "\r\x1b[Ka" + sq + "\x1b[6n")
            rc = read_cpr(hin)
            w = None if rc is None else rc[1] - 2
            hexs = " ".join(f"{ord(c):04X}" for c in sq)
            lines.append(f"seq {name:24} {hexs:28} -> {w}")
        write(hout, "\r\x1b[K")
        with open(out_prefix + ".summary.txt", "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        return 0

    results = []
    failures = 0
    t0 = time.monotonic()
    eaw_rows = load_eaw(eaw_path)
    plan = targets_all(eaw_rows, gc) if mode == "all" else [(cp, cat, gc.get(cp, "Cn")) for cp, cat in targets(eaw_rows)]
    for cp, cat, g in plan:
        # A base char precedes the probe so combining marks / VS are measured in
        # context (attached => 0), not as orphans; width = column - 2.
        write(hout, "\r\x1b[Ka" + chr(cp) + "\x1b[6n")
        rc = read_cpr(hin)
        if rc is None:
            failures += 1
            results.append({"cp": cp, "cat": cat, "gc": g, "w": None})
            k32.FlushConsoleInputBuffer(hin)
            continue
        results.append({"cp": cp, "cat": cat, "gc": g, "w": rc[1] - 2})
    elapsed = time.monotonic() - t0
    write(hout, "\r\x1b[K")

    with open(out_prefix + ".jsonl", "w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r) + "\n")
    with open(out_prefix + ".tsv", "w", encoding="utf-8") as f:
        for r in results:
            f.write(f"{r['cp']}\t{r['cat']}\t{r['gc']}\t{'' if r['w'] is None else r['w']}\n")

    # ---- summary ----
    lines = []
    lines.append(f"mode: {mode}")
    lines.append(f"console output codepage: {cp_now}  probes: {len(results)}  failures: {failures}  elapsed: {elapsed:.2f}s")
    amb = [r for r in results if r["cat"] == "A" and r["w"] is not None]
    from collections import Counter
    cnt = Counter(r["w"] for r in amb)
    lines.append(f"EAW=A measured widths: {dict(sorted(cnt.items()))}")
    for r in results:
        if r["cat"] != "A" and mode != "all":
            lines.append(f"  ref U+{r['cp']:04X} ({r['cat']}) -> {r['w']}")

    def ranges(items):
        out = []
        for cp in items:
            if out and cp == out[-1][1] + 1:
                out[-1][1] = cp
            else:
                out.append([cp, cp])
        return out

    for w in sorted(cnt):
        cps = sorted(r["cp"] for r in amb if r["w"] == w)
        rs = ranges(cps)
        lines.append(f"EAW=A -> {w} : {len(cps)} code points in {len(rs)} ranges")
        if mode != "all" or w != 1:
            lines.append("  " + " ".join(f"{a:04X}" if a == b else f"{a:04X}-{b:04X}" for a, b in rs))

    if console_json:
        table = json.load(open(console_json, encoding="utf-8"))

        def eaw_console(cp):
            for a, b, w in table:
                if a <= cp <= b:
                    return w
            return None

        mism = [(r["cp"], r["w"], eaw_console(r["cp"])) for r in amb if eaw_console(r["cp"]) not in (None, r["w"])]
        lines.append(f"mismatch vs locale-eaw EAW-CONSOLE: {len(mism)} of {len(amb)}")
        by = Counter((m[1], m[2]) for m in mism)
        for (c, e), n in sorted(by.items()):
            lines.append(f"  conhost={c} eaw-console={e}: {n}")
        rs = ranges(sorted(m[0] for m in mism))
        lines.append("  " + " ".join(f"{a:04X}" if a == b else f"{a:04X}-{b:04X}" for a, b in rs))

    with open(out_prefix + ".summary.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
