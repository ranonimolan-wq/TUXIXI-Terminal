//! Terminal drawing logic shared by all renderers: the [`Palette`],
//! [`Selection`] span and the pure color helpers ([`resolve`],
//! [`blend`], [`pack`], [`unpack`]).
//!
//! The actual GUI renders on the GPU (`crate::gpu`, OpenGL 3.3); the
//! [`paint`] function in this module is the CPU reference renderer
//! that mirrors the exact same cell semantics pixel by pixel — it
//! doubles as the oracle for the renderer tests. The whole frame is a
//! few MB of plain arithmetic, so tests can assert on real pixels.

mod canvas;
mod painter;

pub use canvas::{blend, pack, resolve, unpack, Canvas, Palette, Selection};
pub use painter::paint;

#[cfg(test)]
mod tests;
