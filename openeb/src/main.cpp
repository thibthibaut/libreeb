#include "metavision/sdk/stream/file_config_hints.h"
#include <cstdint>
#include <filesystem>
#include <format>
#include <iostream>
#include <metavision/sdk/stream/camera.h>
#include <span>
#include <string>
#include <vector>
#include <xxhash.hpp>

std::pair<std::string, xxh::hash_t<64>>
compute_file_hash(const std::string &filepath, bool time_shift = false) {
  auto hints = Metavision::FileConfigHints();
  hints.set("time_shift", time_shift);

  auto camera = Metavision::Camera::from_file(filepath, hints);
  xxh::hash_state_t<64> hash_stream(0);

  camera.cd().add_callback([&](const Metavision::EventCD *ev_begin,
                               const Metavision::EventCD *ev_end) {
    std::span<const Metavision::EventCD> events(ev_begin, ev_end);

    for (const auto &event : events) {
      std::array<uint16_t, 2> xy = {event.x, event.y};
      std::array<uint8_t, 1> p = {static_cast<uint8_t>(event.p)};
      std::array<uint64_t, 1> t = {static_cast<uint64_t>(event.t)};

      hash_stream.update(xy);
      hash_stream.update(p);
      hash_stream.update(t);
    }
  });

  camera.start();

  while (camera.is_running()) {
  }

  camera.stop();

  return {std::filesystem::path(filepath).filename().string(),
          hash_stream.digest()};
}

std::vector<std::pair<std::string, xxh::hash_t<64>>>
compute_files_hash(const std::vector<std::string> &files,
                   bool time_shift = false) {
  std::vector<std::pair<std::string, xxh::hash_t<64>>> results;
  results.reserve(files.size());

  for (const auto &file : files) {
    results.push_back(compute_file_hash(file, time_shift));
  }

  return results;
}

int main(int argc, char *argv[]) {

  bool time_shift = false;
  auto file_hash = compute_file_hash(argv[1], time_shift);

  std::cout << file_hash.second;

  return 0;
}
