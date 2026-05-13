//! UI modules.

#[cfg(target_arch = "wasm32")]
pub mod bias_controls;

#[cfg(target_arch = "wasm32")]
pub mod recording_controls;

#[cfg(target_arch = "wasm32")]
pub mod info_panel;

#[cfg(target_arch = "wasm32")]
pub mod canvas;

#[cfg(target_arch = "wasm32")]
pub mod selection;

#[cfg(target_arch = "wasm32")]
pub mod timeline_panel;

#[cfg(target_arch = "wasm32")]
pub mod xyt_view;

#[cfg(target_arch = "wasm32")]
pub use bias_controls::{format_bias_file, parse_bias_file, BiasAction};

#[cfg(target_arch = "wasm32")]
pub use canvas::show_canvas;

#[cfg(target_arch = "wasm32")]
pub use info_panel::show_info_panel;

#[cfg(target_arch = "wasm32")]
pub use selection::{show_selection_screen, SelectionAction};

#[cfg(target_arch = "wasm32")]
pub use timeline_panel::show_timeline_panel;
