MV_HAL_PLUGIN_PATH := "{{justfile_dir()}}/openeb/build/openeb/lib/metavision/hal/plugins/"

test:
    cargo nextest r

generate_references:
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/gen4_evt3_hand.raw" > references/gen4_evt3_hand.raw.no_time_shift.xxh
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/claque_doigt_evt21.raw" >  references/claque_doigt_evt21.raw.no_time_shift.xxh
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/blinking_leds.raw" > references/blinking_leds.raw.no_time_shift.xxh

build_openeb:
    cmake -S {{justfile_dir()}}/openeb/ -B {{justfile_dir()}}/openeb/build
    cmake --build {{justfile_dir()}}/openeb/build
