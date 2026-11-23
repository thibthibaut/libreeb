MV_HAL_PLUGIN_PATH := "{{justfile_dir()}}/openeb/build/openeb/lib/metavision/hal/plugins/"

# Build 
build:
    cargo build --release

# Run tests
test:
    cargo nextest r

# Fetch the patched OpenEB used to generate reference hashes (kept out of the repo:
# as a submodule, every crate depending on libreeb via git would download it)
fetch_openeb:
    git clone --branch libreeb-patches https://github.com/thibthibaut/openeb.git openeb/openeb
    git -C openeb/openeb checkout eb6d22f9c5d9c1caba3ec770262d8a096b3260cc

# Generates references hashes (needs `just fetch_openeb` and an OpenEB build in openeb/build)
generate_references:
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/gen4_evt3_hand.raw" > references/gen4_evt3_hand.raw.no_time_shift.xxh
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/claque_doigt_evt21.raw" >  references/claque_doigt_evt21.raw.no_time_shift.xxh
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/blinking_leds.raw" > references/blinking_leds.raw.no_time_shift.xxh

# Build OpenEB
build_openeb:
    git submodule update --init
    cmake -S {{justfile_dir()}}/openeb/ -B {{justfile_dir()}}/openeb/build
    cmake --build {{justfile_dir()}}/openeb/build

# Run comparative benchmarks with openeb
bench_cmp:
    hyperfine \
    --warmup 3 \
    -L prog 'target/release/bench_decoder','openeb/build/openeb/bin/bench_decoder' \
    '{prog} data/blinking_leds.raw'

    hyperfine \
    --warmup 3 \
    -L prog 'target/release/bench_decoder','openeb/build/openeb/bin/bench_decoder' \
    '{prog} data/claque_doigt_evt21.raw'

    hyperfine \
    --warmup 3 \
    -L prog 'target/release/bench_decoder','openeb/build/openeb/bin/bench_decoder' \
    '{prog} data/gen4_evt3_hand.raw'
   
