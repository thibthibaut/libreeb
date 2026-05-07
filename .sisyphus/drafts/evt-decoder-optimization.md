# Draft: EVT2/EVT3 Decoder Optimization to GB/s

## User Requirements (Confirmed)
- **Goal**: Optimize EVT2 and EVT3 decoders to reach GB/s decoding speeds
- **Language**: Rust only (libreeb crate)
- **No unsafe code**: Keep code safe
- **Portable**: No CPU-specific intrinsics (AVX2/AVX-512)
- **Can write new benchmarks**: Allowed to add benchmark code
- **Never modify .xxh files**: Use existing hash files for correctness validation
- **First task**: Trim raw test files to make benchmarks run faster

## Current State Analysis

### Existing Decoders
1. **EVT2** (`src/evt2.rs`): 
   - Simple scalar decoder
   - Uses `Option<u64>` for time_high (branch overhead)
   - Iterator pattern with conditional matching
   - No SIMD optimizations

2. **EVT3** (`src/evt3.rs`):
   - More complex format with vector encoding
   - Manual loop optimization with local variables
   - No SIMD for bit extraction (VECT_12, VECT_8)
   - Already has some branch reduction

3. **EVT21** (`src/evt2_1.rs`):
   - Already optimized with SIMD (wide crate, u64x4)
   - Processes 4 words at a time
   - Good reference for optimization patterns

### Test Data Files
- EVT3: 93MB to 1.2GB files
- EVT21: 256MB file
- EVT2: smaller files

### Current Infrastructure
- Benchmark suite: `benches/benchmark.rs`
- Test validation: xxh64 hashes in `references/` directory
- Buffer size: 262KB aligned to 64 bytes
- Uses `wide` crate for SIMD

## User Decisions (Confirmed)

### 1. Trimmed File Size
- **20MB each** for faster iteration
- **Use multiple slices** from same file to be representative of real data patterns
- Extract slices from different parts (start, middle, end) to capture various event densities

### 2. Target Performance
- **"As fast as possible"** - squeeze every optimization
- **Minimum target**: EVT3 must reach 1 GB/s
- **Stretch goal**: Approach memory bandwidth limits (4-6 GB/s)

### 3. Optimization Scope
- **All three decoders**: EVT2, EVT3, EVT21
- EVT3 is priority (must hit 1 GB/s)
- EVT21 already has SIMD but can improve further
- EVT2 needs complete SIMD rewrite

### 4. SIMD Library Strategy
- **Consider alternatives** to `wide` crate
- Evaluate: `safe_arch`, `pulp`, or stay with `wide`
- Choose based on portable performance benchmarks
- Must remain portable (no AVX2/AVX-512 intrinsics)

### 5. Benchmark Metrics
- **Both metrics required**:
  1. **Throughput**: GB/s of raw bytes decoded
  2. **Event rate**: Millions of events/second output
- Show before/after comparisons

### 6. Documentation
- **Create markdown file** documenting all improvements
- Include: optimization techniques, performance gains, before/after metrics
- Should be educational for future maintainers

## Technical Approach

### Optimization Strategies

**EVT3 Specific:**
- SIMD for VECT_12 and VECT_8 bit expansion (trailing_zeros in loop is slow)
- Batch processing with predictable branches
- Prefetch hints for memory access patterns
- Replace conditional jumps with branchless arithmetic where possible

**EVT2 Specific:**
- Remove `Option<u64>` overhead (use sentinel value or guaranteed initialization)
- SIMD batch processing (similar to EVT21 pattern)
- Vectorize the time_low | time_high operation
- Parallel event extraction

**EVT21 Improvements:**
- Already has u64x4 SIMD, but could use u64x8 (AVX-512 portable via library)
- Optimize the mask expansion inner loop (trailing_zeros)
- Better branch prediction hints
- Prefetching for next chunk

**Cross-Decoder:**
- Increase buffer size if beneficial
- Optimize EventBatch growth strategy
- Consider SIMD-friendly memory layout during decode
- Profile-guided optimization hints

### SIMD Library Evaluation Criteria
1. **Performance**: Actual GB/s achieved in benchmarks
2. **Portability**: Works on ARM and x86 without platform-specific code
3. **API ergonomics**: Easy to use, maintainable
4. **Compile-time dispatch**: Automatically uses best ISA available
5. **Active maintenance**: Recent updates, good documentation

### File Trimming Strategy
For 20MB trimmed files, extract multiple representative slices:
- **Slice 1**: First 7MB (beginning - initialization patterns)
- **Slice 2**: Middle 7MB (steady state - typical event density)
- **Slice 3**: Last 6MB (may have different characteristics)
- Total: ~20MB representative sample

This captures various event densities and patterns throughout recordings.

## Expected Performance Gains

### Baseline (Current)
- EVT3: ~200-300 MB/s (estimated, needs measurement)
- EVT2: ~100-200 MB/s (estimated, needs measurement)
- EVT21: ~400-600 MB/s (has SIMD, needs measurement)

### Target (Optimized)
- EVT3: **≥1.0 GB/s** (3-5x improvement)
- EVT2: **≥800 MB/s** (4-8x improvement)
- EVT21: **≥1.2 GB/s** (2-3x improvement)

### Optimization Phases Impact
1. **SIMD introduction**: 2-4x speedup
2. **Branch elimination**: 10-20% additional
3. **Memory layout optimization**: 5-15% additional
4. **Library selection**: 0-30% based on choice

## Work Plan Structure

### Wave 1: Setup & Baseline (foundation)
- Create file trimming script and trimmed datasets
- Add enhanced benchmark suite (both metrics)
- Establish baseline measurements
- Document current performance

### Wave 2: Research & Analysis (preparation)
- Profile current decoders (flamegraph/perf)
- Evaluate SIMD libraries (create comparison benchmark)
- Analyze EVT21's SIMD patterns (learn from existing)
- Document optimization opportunities

### Wave 3: EVT3 Optimization (priority)
- Implement SIMD vector expansion
- Branch elimination in hot path
- Optimize time handling
- Benchmark and validate (xxh64)

### Wave 4: EVT2 Optimization (secondary)
- Remove Option overhead
- Implement SIMD batch processing
- Optimize time concatenation
- Benchmark and validate (xxh64)

### Wave 5: EVT21 Enhancement (refinement)
- Further SIMD improvements
- Optimize mask expansion
- Memory access optimization
- Benchmark and validate (xxh64)

### Wave 6: Documentation & Finalization
- Create optimization documentation markdown
- Before/after performance comparison
- Techniques and learnings summary
- Commit strategy and cleanup

## Success Criteria
- [ ] All tests pass with original .xxh validation
- [ ] EVT3 achieves ≥1.0 GB/s throughput
- [ ] EVT2 and EVT21 show significant improvements
- [ ] Benchmarks show both GB/s and Mevents/s metrics
- [ ] Comprehensive optimization documentation created
- [ ] Code remains safe Rust (no unsafe)
- [ ] Code remains portable (no CPU-specific intrinsics)
