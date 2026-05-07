import numpy as np
from collections import Counter

def analyze_evt3(path: str, ngram_n: int = 2):
    """
    Reads an EVT3 .raw file, extracts 4-bit type codes from each u16 word,
    and reports type frequency + n-gram statistics.
    """
    with open(path, "rb") as f:
        # Skip Prophesee RAW header (lines starting with %)
        raw_bytes = f.read()

    # Parse as little-endian u16
    words = np.frombuffer(raw_bytes[:-1], dtype="<u2")
    types = (words >> 12).astype(np.uint8)

    print(f"Total words: {len(words)}")
    print("\n=== Type frequency ===")
    type_names = {
        0x0: "EVT_ADDR_Y",  0x2: "EVT_ADDR_X",
        0x3: "VECT_BASE_X", 0x4: "VECT_12",
        0x5: "VECT_8",      0x6: "EVT_TIME_LOW",
        0x8: "EVT_TIME_HIGH", 0xA: "EXT_TRIGGER",
    }
    freq = Counter(types)
    for code, count in sorted(freq.items(), key=lambda x: -x[1]):
        name = type_names.get(code, f"UNKNOWN_{code:X}")
        print(f"  {name:20s} ({code:#04x}): {count:10d}  ({100*count/len(words):.2f}%)")

    print(f"\n=== {ngram_n}-gram type sequences ===")
    ngrams = [tuple(types[i:i+ngram_n].tolist()) for i in range(len(types)-ngram_n+1)]
    ngram_freq = Counter(ngrams)
    print("Top 20:")
    for gram, count in ngram_freq.most_common(20):
        names = " → ".join(type_names.get(c, f"UNK_{c:X}") for c in gram)
        print(f"  [{names}]: {count}")

    # VECT density: how many bits set per VECT_12 word on average?
    vect12_mask = words[types == 0x4] & 0x0FFF
    vect8_mask  = words[types == 0x5] & 0x00FF
    if len(vect12_mask) > 0:
        print(f"\nVECT_12 avg bits/word: {np.mean(np.vectorize(bin(0).__class__.count)(vect12_mask, '1')):.2f}")
    if len(vect8_mask) > 0:
        print(f"VECT_8  avg bits/word: {np.mean(np.vectorize(bin(0).__class__.count)(vect8_mask, '1')):.2f}")

analyze_evt3("./driving_sample_noheader.raw", ngram_n=8)
