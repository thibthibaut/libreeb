MV_HAL_PLUGIN_PATH := "{{justfile_dir()}}/openeb/build/openeb/lib/metavision/hal/plugins/"

generate_references:
    #@openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/openeb/claque_doigt_evt21.raw"
    # @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/openeb/gen4_evt3_hand.raw"
    # @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/openeb/gen4_evt2_hand.raw"
    @openeb/build/openeb/bin/TestOpenEB "{{justfile_dir()}}/data/openeb/blinking_leds.raw"

build_openeb:
    cmake -S {{justfile_dir()}}/openeb/ -B {{justfile_dir()}}/openeb/build
    cmake --build {{justfile_dir()}}/openeb/build
