//! Drawing the editor into an inline viewport.

pub mod layout;
pub mod render;
pub mod splash;
pub mod view;

pub use render::draw;
pub use view::{View, window_height};

#[cfg(test)]
mod tests;
