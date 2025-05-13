#include "metavision/sdk/stream/file_config_hints.h"
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <metavision/sdk/stream/camera.h>
#include <span>
#include <string>

uint64_t benchmark(const std::string &filepath, bool time_shift = false) {

  auto hints = Metavision::FileConfigHints();
  hints.set("time_shift", time_shift);
  hints.set("real_time_playback", false);

  auto camera = Metavision::Camera::from_file(filepath, hints);

  uint64_t total = 0;
  camera.cd().add_callback([&](const Metavision::EventCD *ev_begin,
                               const Metavision::EventCD *ev_end) {
    std::span<const Metavision::EventCD> events(ev_begin, ev_end);
    total += events.size();
  });

  camera.start();

  while (camera.is_running()) {
  }

  camera.stop();

  return total;
}

int main() {
  const auto filename = "/home/tvercueil/ws/libreeb/data/openeb/gen4_evt3_hand.raw";

  auto result = benchmark(filename);

  std::cout << result << '\n';

  return 0;
}
