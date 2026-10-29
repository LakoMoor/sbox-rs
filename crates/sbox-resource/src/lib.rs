//! Readers for Source 2 compiled resources (`*_c`) as used by s&box.
//! Format knowledge is derived from inspecting the shipped files.

pub mod anim;
pub mod container;
pub mod kv3;
pub mod mesh;
pub mod model;
pub mod vbib;
pub mod vmat;
pub mod vtex;

pub use container::{Block, Resource};
pub use kv3::Value;
