# Decoder Performance Optimisation

This document summarises the performance analysis of the EVT3 and EVT2.1 decoders and tracks optimisation efforts.

## Current Performance Baseline (May 2026)

| Decoder | File Size | Events | Time | Throughput (Raw) | IPC | Branch Miss |
|---------|-----------|--------|------|------------------|-----|-------------|
| **EVT3** | 350 MiB | 111.5M | ~1.32s | **265 MiB/s** | 1.36 | 5.78% |
| **EVT2.1** | 256 MiB | 16.3M | ~0.22s | **1.08 GiB/s** | 2.11 | 2.62% |

## Bottleneck Analysis

### EVT3

1. **Branch Misprediction (5.78%, 101M misses)**
   - Main decode loop dispatches on 9+ event types via jump table
   - Unpredictable event sequence causes pipeline stalls
   - ~57 cycles/event average

2. **Stage Buffer Overhead**
   - Every event goes through: `is_full()` check → `push()` → periodic `flush()`
   - Adds ~3-5 cycles overhead per event
   - Fixed-size 256-element buffer causes frequent flushes

3. **IPC of 1.36** - Moderate, room for ~30% improvement

### EVT2.1

1. **Already Highly Optimised** - IPC of 2.11 is near-optimal
2. **Mask Expansion** - `extend_masked_xs` compiles to efficient `tzcnt`+`blsr` sequence
3. **2.62% branch misprediction** - Much better than EVT3

## Optimisation Plan

### Phase 1: Tested (May 2026)

- [ ] ~~**Remove Stage buffer from EVT3**~~ - Tested, regressed performance. The Stage buffer amortises Vec::push overhead.
- [ ] ~~**Profile-Guided Optimisation (PGO)**~~ - Tested with LLVM 22, no improvement. See notes below.

### Phase 2: Future Considerations

| Optimisation | Complexity | Expected Gain | Status |
|--------------|------------|---------------|--------|
| Prefetch lookahead | Low | 2-5% | Not started |
| SIMD time_high detection | Medium | 3-5% | Not started |
| AVX-512 scatter for EVT2.1 | High | 5-10% | Not recommended (diminishing returns) |
| `push_within_capacity` | Low | TBD | Worth revisiting for bounds check elimination |

### Not Recommended (Tested, Regressed or No Gain)

- **Manual SIMD for EVT3 VECT events** - Benchmarked worse than sequential
- **SIMD classification fast paths** - No gain
- **Remove Stage buffer from EVT3** - Regressed ~6% due to increased Vec::push overhead
- **PGO (Profile-Guided Optimisation)** - No measurable improvement, possibly slight regression

## PGO Training Data

Using all available recordings to avoid overfitting:

**EVT3 files:**
- `data/evt3/gen4_evt3_hand.raw` (92 MiB)
- `data/evt3/driving_sample.raw` (350 MiB)
- `data/evt3/aruco.raw` (1.2 GiB)
- `data/evt3/iron_fines_demo.raw` (143 MiB)
- `data/evt3/laser.raw` (325 MiB)
- `data/evt3/record-master.raw` (209 MiB)
- `data/evt3/record-slave-left.raw` (60 MiB)
- `data/evt3/record-slave-right.raw` (83 MiB)

**EVT2.1 files:**
- `data/evt21/claque_doigt_evt21.raw` (256 MiB)

## Build Instructions

### Standard Release Build
```bash
cargo build --release
```

### PGO Build
```bash
# Step 1: Build with instrumentation
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release --bin bench_decoder

# Step 2: Run training workload (all test files)
for f in data/evt3/*.raw data/evt21/*.raw; do
    ./target/release/bench_decoder "$f"
done

# Step 3: Merge profile data
llvm-profdata merge -o /tmp/merged.profdata /tmp/pgo-data

# Step 4: Build with profile data
RUSTFLAGS="-Cprofile-use=/tmp/merged.profdata" cargo build --release
```

## Measurements Log

| Date | Change | EVT3 Time | EVT2.1 Time | Notes |
|------|--------|-----------|-------------|-------|
| Pre-refactor | Baseline | ~246ms | ~282ms | With SIMD/unsafe |
| Post-refactor | Clean sequential | ~249ms | ~226ms | No SIMD, clean code |
| May 2026 | Stage removal test | ~1.40s | - | Regressed ~6%, reverted |
| May 2026 | PGO test | ~1.52s | ~0.23s | No improvement vs baseline |
| May 2026 | Current baseline | ~1.48s | ~0.22s | driving_sample.raw (350 MiB) |

**Note:** The EVT3 times vary between ~1.3s and ~1.5s due to different test files and system conditions.
Using `driving_sample.raw` (350 MiB, 111M events) as the standard benchmark file.

## PGO Analysis

PGO was tested with:
- LLVM 22.1.2 (matching Rust's bundled LLVM version)
- Training data: ~3.5 GiB of EVT3 files + 256 MiB EVT2.1
- ~785M total events decoded during training

**Results:** No measurable improvement. Possible reasons:
1. The compiler's default heuristics are already good for this workload
2. Branch patterns are inherently unpredictable (event type varies per-word)
3. The bottleneck is memory/cache bound, not instruction selection
4. LTO (`lto = "fat"`) already provides similar optimisations

## Notes on `push_within_capacity`

The `push_within_capacity` API was rejected in earlier testing due to:
1. Nightly-only feature friction
2. No measurable gain in benchmarks at the time

However, this deserves revisiting because:
- We do many `Vec::push` calls with bounds checks
- The EVT3 decoder especially pushes millions of events
- Modern nightly may have better codegen

To test:
```rust
#![feature(vec_push_within_capacity)]
// In hot loop:
if output.x.push_within_capacity(x).is_err() {
    output.reserve(256);
    output.x.push(x);
}
```
