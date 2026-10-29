//! Scene / GameObject / Component model, ported from Sandbox.Engine/Scene.
//!
//! Lifecycle matches s&box: `on_awake` on creation, `on_enabled` when the component
//! becomes active in the hierarchy, `on_start` once before the first update,
//! `on_fixed_update` / `on_update` per tick, `on_disabled`, `on_destroy`.
//! Destruction is deferred until the end of the tick (see Scene.Deletes.cs).

mod builtin;
mod scene;
mod serialize;

pub use builtin::*;
pub use scene::*;
pub use serialize::*;
