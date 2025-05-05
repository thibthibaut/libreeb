cmake_minimum_required(VERSION 3.10)
project(TestOpenEB)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
 
# Disable some features of openeb
set(BUILD_TESTING OFF CACHE BOOL "Disable testing in openeb" FORCE)
set(COMPILE_PYTHON3_BINDINGS OFF CACHE BOOL "Disable python3 bindings in openeb" FORCE)
set(BUILD_SAMPLES OFF CACHE BOOL "Disable python3 bindings in openeb" FORCE)
set(USE_PROTOBUF OFF CACHE BOOL "Disable protobuf" FORCE)

# download CPM.cmake
file(
  DOWNLOAD
  https://github.com/cpm-cmake/CPM.cmake/releases/download/v0.40.8/CPM.cmake
  ${CMAKE_CURRENT_BINARY_DIR}/cmake/CPM.cmake
  EXPECTED_HASH SHA256=78ba32abdf798bc616bab7c73aac32a17bbd7b06ad9e26a6add69de8f3ae4791
)
include(${CMAKE_CURRENT_BINARY_DIR}/cmake/CPM.cmake)

CPMAddPackage("gh:redspah/xxhash_cpp#0.8.1")

# Add openeb as subdirectory
add_subdirectory(openeb)

add_executable(${PROJECT_NAME} 
    src/main.cpp
)

# # Link against openeb libraries you need
target_link_libraries(${PROJECT_NAME}
    PRIVATE
    metavision_sdk_core
    metavision_sdk_base
    metavision_sdk_stream
    xxhash_cpp
)

target_compile_features(${PROJECT_NAME} PRIVATE cxx_std_20)

# Include directories
# target_include_directories(${PROJECT_NAME}
#     PRIVATE
#     ${CMAKE_CURRENT_SOURCE_DIR}/include
# )
