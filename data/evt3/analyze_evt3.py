
import numpy as np
from collections import Counter


def analyze_evt3(path: str):
    with open(path, "rb") as f:
        raw_bytes = f.read()

    words = np.frombuffer(raw_bytes[:-1], dtype="<u2")
    types = (words >> 12).astype(np.uint8)

    type_names = {
        0x0: "EVT_ADDR_Y",
        0x2: "EVT_ADDR_X",
        0x3: "VECT_BASE_X",
        0x4: "VECT_12",
        0x5: "VECT_8",
        0x6: "EVT_TIME_LOW",
        0x8: "EVT_TIME_HIGH",
        0xA: "EXT_TRIGGER",
    }

    print(f"Total words: {len(words)}")

    # ── Type frequency ────────────────────────────────────────────────────────
    print("\n=== Type frequency ===")
    freq = Counter(types.tolist())
    for code, count in sorted(freq.items(), key=lambda x: -x[1]):
        name = type_names.get(code, f"UNKNOWN_{code:#x}")
        print(f"  {name:20s} ({code:#04x}): {count:10d}  ({100 * count / len(words):.2f}%)")

    # ── N-gram analysis (n=2 and n=3) ────────────────────────────────────────
    for ngram_n in (2, 3):
        print(f"\n=== {ngram_n}-gram type sequences (top 20) ===")
        ngrams = [
            tuple(types[i : i + ngram_n].tolist())
            for i in range(len(types) - ngram_n + 1)
        ]
        ngram_freq = Counter(ngrams)
        for gram, count in ngram_freq.most_common(20):
            names = " -> ".join(type_names.get(c, f"UNK_{c:#x}") for c in gram)
            print(f"  [{names}]: {count}")

    # ── Popcount helpers ──────────────────────────────────────────────────────
    def popcount_array(arr: np.ndarray) -> np.ndarray:
        return np.array([int(x).bit_count() for x in arr], dtype=np.uint8)

    # ── VECT_12 stats ─────────────────────────────────────────────────────────
    vect12_words = words[types == 0x4] & 0x0FFF
    if len(vect12_words) > 0:
        pc12 = popcount_array(vect12_words)
        print(f"\n=== VECT_12 ({len(vect12_words)} words) ===")
        print(f"  avg bits/word : {pc12.mean():.3f}  (out of 12)")
        print(f"  events emitted: {pc12.sum()}")
        vals, cnts = np.unique(pc12, return_counts=True)
        print("  popcount distribution:")
        for v, c in zip(vals, cnts):
            bar = "#" * int(40 * c / cnts.max())
            print(f"    {v:2d}: {c:8d}  {bar}")

    # ── VECT_8 stats ──────────────────────────────────────────────────────────
    vect8_words = words[types == 0x5] & 0x00FF
    if len(vect8_words) > 0:
        pc8 = popcount_array(vect8_words)
        print(f"\n=== VECT_8 ({len(vect8_words)} words) ===")
        print(f"  avg bits/word : {pc8.mean():.3f}  (out of 8)")
        print(f"  events emitted: {pc8.sum()}")
        vals, cnts = np.unique(pc8, return_counts=True)
        print("  popcount distribution:")
        for v, c in zip(vals, cnts):
            bar = "#" * int(40 * c / cnts.max())
            print(f"    {v:2d}: {c:8d}  {bar}")

    # ── VECT_12 run-length: how many consecutive VECT_12 appear? ─────────────
    print("\n=== VECT_12 run-length distribution ===")
    runs = []
    run = 0
    for t in types:
        if t == 0x4:
            run += 1
        else:
            if run > 0:
                runs.append(run)
            run = 0
    if run > 0:
        runs.append(run)
    if runs:
        runs_arr = np.array(runs)
        print(f"  total runs   : {len(runs_arr)}")
        print(f"  mean run len : {runs_arr.mean():.2f}")
        print(f"  max run len  : {runs_arr.max()}")
        rl_freq = Counter(runs_arr.tolist())
        print("  run-length histogram (top 15):")
        for rl, c in sorted(rl_freq.items(), key=lambda x: -x[1])[:15]:
            bar = "#" * int(40 * c / max(rl_freq.values()))
            print(f"    len {rl:3d}: {c:8d}  {bar}")

    # ── CD event source breakdown ─────────────────────────────────────────────
    n_addrx  = freq.get(0x2, 0)
    n_vect12 = int(popcount_array(vect12_words).sum()) if len(vect12_words) > 0 else 0
    n_vect8  = int(popcount_array(vect8_words).sum())  if len(vect8_words)  > 0 else 0
    total_cd = n_addrx + n_vect12 + n_vect8
    if total_cd > 0:
        print(f"\n=== CD event sources ===")
        print(f"  EVT_ADDR_X   : {n_addrx:10d}  ({100 * n_addrx  / total_cd:.1f}%)")
        print(f"  from VECT_12 : {n_vect12:10d}  ({100 * n_vect12 / total_cd:.1f}%)")
        print(f"  from VECT_8  : {n_vect8:10d}  ({100 * n_vect8  / total_cd:.1f}%)")
        print(f"  Total CD     : {total_cd:10d}")
        print(f"\n  Words/CD ratio: {len(words) / total_cd:.3f}")
        print(f"  CD/word ratio : {total_cd / len(words):.3f}")


analyze_evt3("./aruco_noheader.raw")
