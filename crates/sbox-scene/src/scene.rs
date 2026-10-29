use sbox_math::Transform;
use slotmap::{new_key_type, SlotMap};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::collections::HashSet;

new_key_type! {
    /// Handle to a `GameObject` (Guid in C#).
    pub struct GoId;
    /// Handle to a `Component`.
    pub struct CompId;
}

/// Identifies a component together with its owning object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    pub go: GoId,
    pub comp: CompId,
}

/// Port of `Sandbox.Component`. All hooks default to no-ops.
pub trait Component: Any {
    fn on_awake(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_enabled(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_start(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_update(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_fixed_update(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_pre_render(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_disabled(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn on_destroy(&mut self, _scene: &mut Scene, _h: Handle) {}
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Implements the `as_any` boilerplate for a component type.
#[macro_export]
macro_rules! impl_component_any {
    () => {
        fn as_any(&self) -> &dyn ::std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn ::std::any::Any {
            self
        }
    };
}

pub struct GameObject {
    pub name: String,
    pub tags: HashSet<String>,
    enabled: bool,
    active: bool,
    parent: Option<GoId>,
    children: Vec<GoId>,
    components: Vec<CompId>,
    local: Transform,
    pending_destroy: bool,
}

impl GameObject {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// True when this object and all ancestors are enabled (`Active` in C#).
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn parent(&self) -> Option<GoId> {
        self.parent
    }
    pub fn children(&self) -> &[GoId] {
        &self.children
    }
    pub fn components(&self) -> &[CompId] {
        &self.components
    }
    pub fn local_transform(&self) -> Transform {
        self.local
    }
    pub fn has_tag(&self, t: &str) -> bool {
        self.tags.contains(t)
    }
}

struct Slot {
    go: GoId,
    enabled: bool,
    /// Currently between on_enabled and on_disabled.
    running: bool,
    started: bool,
    pending_destroy: bool,
    /// `None` while the component is being invoked (taken out of the slot).
    comp: Option<Box<dyn Component>>,
}

/// A system runs once per fixed step (after component `on_fixed_update`), like `GameObjectSystem`.
pub type System = Box<dyn FnMut(&mut Scene)>;

#[derive(Default)]
pub struct Scene {
    resources: HashMap<TypeId, Box<dyn Any>>,
    fixed_systems: Vec<System>,
    pub name: String,
    gos: SlotMap<GoId, GameObject>,
    comps: SlotMap<CompId, Slot>,
    roots: Vec<GoId>,
    /// Seconds since the scene started.
    pub time: f64,
    pub delta: f32,
    pub fixed_delta: f32,
    fixed_accum: f64,
    pub fixed_rate: f32,
    pub max_fixed_steps: u32,
    destroy_queue: Vec<GoId>,
    comp_destroy_queue: Vec<CompId>,
}

impl Scene {
    pub fn new() -> Self {
        Self { name: "scene".into(), fixed_rate: 50.0, fixed_delta: 1.0 / 50.0, max_fixed_steps: 5, ..Default::default() }
    }

    // -------------------------------------------------------------- resources

    /// Store a singleton (physics world, input state, ...) in the scene.
    pub fn insert_resource<T: Any>(&mut self, v: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(v));
    }
    pub fn resource<T: Any>(&self) -> Option<&T> {
        self.resources.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref())
    }
    pub fn resource_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.resources.get_mut(&TypeId::of::<T>()).and_then(|b| b.downcast_mut())
    }
    /// Run `f` with the resource temporarily removed so it can also receive `&mut Scene`.
    pub fn with_resource<T: Any, R>(&mut self, f: impl FnOnce(&mut T, &mut Scene) -> R) -> Option<R> {
        let mut b = self.resources.remove(&TypeId::of::<T>())?;
        let r = f(b.downcast_mut::<T>()?, self);
        self.resources.insert(TypeId::of::<T>(), b);
        Some(r)
    }
    pub fn add_fixed_system(&mut self, sys: impl FnMut(&mut Scene) + 'static) {
        self.fixed_systems.push(Box::new(sys));
    }

    // ---------------------------------------------------------------- objects

    pub fn create_object(&mut self, name: &str) -> GoId {
        self.create_object_in(None, name)
    }

    pub fn create_object_in(&mut self, parent: Option<GoId>, name: &str) -> GoId {
        let id = self.gos.insert(GameObject {
            name: name.to_string(),
            tags: HashSet::new(),
            enabled: true,
            active: false,
            parent: None,
            children: vec![],
            components: vec![],
            local: Transform::ZERO,
            pending_destroy: false,
        });
        self.attach(id, parent);
        let active = parent.and_then(|p| self.gos.get(p)).map_or(true, |p| p.active);
        self.gos[id].active = active;
        id
    }

    fn attach(&mut self, id: GoId, parent: Option<GoId>) {
        match parent {
            Some(p) if self.gos.contains_key(p) => {
                self.gos[p].children.push(id);
                self.gos[id].parent = Some(p);
            }
            _ => {
                self.roots.push(id);
                self.gos[id].parent = None;
            }
        }
    }

    fn detach(&mut self, id: GoId) {
        match self.gos[id].parent {
            Some(p) => self.gos[p].children.retain(|c| *c != id),
            None => self.roots.retain(|c| *c != id),
        }
    }

    pub fn get(&self, id: GoId) -> Option<&GameObject> {
        self.gos.get(id)
    }
    pub fn get_mut(&mut self, id: GoId) -> Option<&mut GameObject> {
        self.gos.get_mut(id)
    }
    pub fn roots(&self) -> &[GoId] {
        &self.roots
    }
    pub fn object_count(&self) -> usize {
        self.gos.len()
    }
    pub fn all_objects(&self) -> impl Iterator<Item = GoId> + '_ {
        self.gos.keys()
    }

    pub fn find_by_name(&self, name: &str) -> Option<GoId> {
        self.gos.iter().find(|(_, g)| g.name == name && !g.pending_destroy).map(|(i, _)| i)
    }

    /// Reparent; with `keep_world` the world transform is preserved (`SetParent` in C#).
    pub fn set_parent(&mut self, id: GoId, parent: Option<GoId>, keep_world: bool) {
        if !self.gos.contains_key(id) || parent == self.gos[id].parent {
            return;
        }
        if let Some(p) = parent {
            if p == id || self.is_descendant(id, p) {
                log::warn!("set_parent: would create a cycle");
                return;
            }
        }
        let world = self.world_transform(id);
        self.detach(id);
        self.attach(id, parent);
        if keep_world {
            self.set_world_transform(id, world);
        }
        self.refresh_active(id);
    }

    /// True if `d` is below `ancestor`.
    pub fn is_descendant(&self, ancestor: GoId, d: GoId) -> bool {
        let mut cur = self.gos.get(d).and_then(|g| g.parent);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.gos.get(c).and_then(|g| g.parent);
        }
        false
    }

    pub fn set_enabled(&mut self, id: GoId, enabled: bool) {
        let Some(g) = self.gos.get_mut(id) else { return };
        if g.enabled == enabled {
            return;
        }
        g.enabled = enabled;
        self.refresh_active(id);
    }

    /// Recompute `active` for a subtree and fire enable/disable transitions.
    fn refresh_active(&mut self, id: GoId) {
        let parent_active = self.gos[id].parent.and_then(|p| self.gos.get(p)).map_or(true, |p| p.active);
        let active = parent_active && self.gos[id].enabled;
        let changed = self.gos[id].active != active;
        self.gos[id].active = active;
        if changed {
            let comps = self.gos[id].components.clone();
            for c in comps {
                self.refresh_component(c);
            }
        }
        // children always re-evaluated: parent link may have changed
        for child in self.gos[id].children.clone() {
            self.refresh_active(child);
        }
    }

    // ------------------------------------------------------------- transforms

    pub fn local_transform(&self, id: GoId) -> Transform {
        self.gos.get(id).map_or(Transform::ZERO, |g| g.local)
    }
    pub fn set_local_transform(&mut self, id: GoId, t: Transform) {
        if let Some(g) = self.gos.get_mut(id) {
            g.local = t;
        }
    }

    pub fn world_transform(&self, id: GoId) -> Transform {
        let Some(g) = self.gos.get(id) else { return Transform::ZERO };
        match g.parent {
            Some(p) => self.world_transform(p).to_world(&g.local),
            None => g.local,
        }
    }

    pub fn set_world_transform(&mut self, id: GoId, world: Transform) {
        let local = match self.gos.get(id).and_then(|g| g.parent) {
            Some(p) => self.world_transform(p).to_local(&world),
            None => world,
        };
        self.set_local_transform(id, local);
    }

    // ------------------------------------------------------------- components

    pub fn add_component<T: Component>(&mut self, go: GoId, comp: T) -> Handle {
        self.add_component_boxed(go, Box::new(comp))
    }

    pub fn add_component_boxed(&mut self, go: GoId, comp: Box<dyn Component>) -> Handle {
        let comp_id = self.comps.insert(Slot {
            go,
            enabled: true,
            running: false,
            started: false,
            pending_destroy: false,
            comp: Some(comp),
        });
        self.gos[go].components.push(comp_id);
        let h = Handle { go, comp: comp_id };
        self.with_component(h, |c, s| c.on_awake(s, h));
        self.refresh_component(comp_id);
        h
    }

    pub fn set_component_enabled(&mut self, h: Handle, enabled: bool) {
        if let Some(s) = self.comps.get_mut(h.comp) {
            s.enabled = enabled;
            self.refresh_component(h.comp);
        }
    }

    fn refresh_component(&mut self, id: CompId) {
        let Some(s) = self.comps.get(id) else { return };
        let go_active = self.gos.get(s.go).map_or(false, |g| g.active);
        let want = s.enabled && go_active && !s.pending_destroy;
        if want == s.running {
            return;
        }
        let h = Handle { go: s.go, comp: id };
        self.comps[id].running = want;
        if want {
            self.with_component(h, |c, s| c.on_enabled(s, h));
        } else {
            self.with_component(h, |c, s| c.on_disabled(s, h));
        }
    }

    /// Temporarily take the component out of its slot so it can receive `&mut Scene`.
    fn with_component(&mut self, h: Handle, f: impl FnOnce(&mut dyn Component, &mut Scene)) {
        let Some(mut c) = self.comps.get_mut(h.comp).and_then(|s| s.comp.take()) else { return };
        f(c.as_mut(), self);
        if let Some(s) = self.comps.get_mut(h.comp) {
            s.comp = Some(c);
        }
    }

    /// Borrow a component of a given type immutably.
    pub fn component<T: Component>(&self, go: GoId) -> Option<&T> {
        let g = self.gos.get(go)?;
        g.components.iter().find_map(|c| self.comps.get(*c)?.comp.as_ref()?.as_any().downcast_ref::<T>())
    }

    pub fn component_mut<T: Component>(&mut self, go: GoId) -> Option<&mut T> {
        let ids = self.gos.get(go)?.components.clone();
        for c in ids {
            // Check type first to avoid holding a mutable borrow across iterations.
            let is_t = self.comps.get(c).and_then(|s| s.comp.as_ref()).map_or(false, |b| b.as_any().is::<T>());
            if is_t {
                return self.comps.get_mut(c)?.comp.as_mut()?.as_any_mut().downcast_mut::<T>();
            }
        }
        None
    }

    pub fn component_by_handle<T: Component>(&self, h: Handle) -> Option<&T> {
        self.comps.get(h.comp)?.comp.as_ref()?.as_any().downcast_ref::<T>()
    }

    pub fn component_by_handle_mut<T: Component>(&mut self, h: Handle) -> Option<&mut T> {
        self.comps.get_mut(h.comp)?.comp.as_mut()?.as_any_mut().downcast_mut::<T>()
    }

    /// Iterate all live, running components of type `T` together with their object.
    pub fn iter_components<T: Component>(&self) -> impl Iterator<Item = (GoId, &T)> + '_ {
        self.comps.values().filter(|s| s.running).filter_map(|s| Some((s.go, s.comp.as_ref()?.as_any().downcast_ref::<T>()?)))
    }

    // ---------------------------------------------------------------- destroy

    /// True while the object has not been destroyed.
    pub fn exists(&self, id: GoId) -> bool {
        self.gos.contains_key(id)
    }

    /// Deferred destruction of an object and its subtree.
    pub fn destroy_object(&mut self, id: GoId) {
        if let Some(g) = self.gos.get_mut(id) {
            if !g.pending_destroy {
                g.pending_destroy = true;
                self.destroy_queue.push(id);
            }
        }
    }

    pub fn destroy_component(&mut self, h: Handle) {
        if let Some(s) = self.comps.get_mut(h.comp) {
            if !s.pending_destroy {
                s.pending_destroy = true;
                self.comp_destroy_queue.push(h.comp);
                self.refresh_component(h.comp);
            }
        }
    }

    fn flush_destroys(&mut self) {
        while !self.comp_destroy_queue.is_empty() || !self.destroy_queue.is_empty() {
            for c in std::mem::take(&mut self.comp_destroy_queue) {
                self.destroy_component_now(c);
            }
            for g in std::mem::take(&mut self.destroy_queue) {
                self.destroy_object_now(g);
            }
        }
    }

    fn destroy_component_now(&mut self, id: CompId) {
        let Some(s) = self.comps.get(id) else { return };
        let h = Handle { go: s.go, comp: id };
        if self.comps[id].running {
            self.comps[id].running = false;
            self.with_component(h, |c, s| c.on_disabled(s, h));
        }
        self.with_component(h, |c, s| c.on_destroy(s, h));
        self.comps.remove(id);
        if let Some(g) = self.gos.get_mut(h.go) {
            g.components.retain(|c| *c != id);
        }
    }

    fn destroy_object_now(&mut self, id: GoId) {
        if !self.gos.contains_key(id) {
            return;
        }
        for child in self.gos[id].children.clone() {
            self.destroy_object_now(child);
        }
        for c in self.gos[id].components.clone() {
            self.destroy_component_now(c);
        }
        self.detach(id);
        self.gos.remove(id);
    }

    // ------------------------------------------------------------------- tick

    /// Full frame update: pending starts, fixed steps, update, deferred deletes.
    pub fn tick(&mut self, delta: f32) {
        self.delta = delta;
        self.time += delta as f64;
        self.run_pending_starts();

        self.fixed_accum += delta as f64;
        let step = 1.0 / self.fixed_rate.max(1.0) as f64;
        self.fixed_delta = step as f32;
        let mut n = 0;
        while self.fixed_accum >= step && n < self.max_fixed_steps {
            self.fixed_accum -= step;
            n += 1;
            self.for_each_running(|c, s, h| c.on_fixed_update(s, h));
            let mut systems = std::mem::take(&mut self.fixed_systems);
            for sys in &mut systems {
                sys(self);
            }
            // systems added during the step are kept after the existing ones
            systems.append(&mut self.fixed_systems);
            self.fixed_systems = systems;
        }
        if n == self.max_fixed_steps {
            self.fixed_accum = 0.0;
        }

        self.for_each_running(|c, s, h| c.on_update(s, h));
        self.for_each_running(|c, s, h| c.on_pre_render(s, h));
        self.flush_destroys();
        if let Some(i) = self.resource_mut::<crate::Input>() {
            i.end_frame();
        }
    }

    fn run_pending_starts(&mut self) {
        // Starting may spawn more components, so loop until stable.
        loop {
            let pending: Vec<Handle> = self
                .comps
                .iter()
                .filter(|(_, s)| s.running && !s.started)
                .map(|(id, s)| Handle { go: s.go, comp: id })
                .collect();
            if pending.is_empty() {
                break;
            }
            for h in pending {
                if let Some(s) = self.comps.get_mut(h.comp) {
                    if s.started {
                        continue;
                    }
                    s.started = true;
                }
                self.with_component(h, |c, s| c.on_start(s, h));
            }
        }
    }

    fn for_each_running(&mut self, f: impl Fn(&mut dyn Component, &mut Scene, Handle)) {
        let list: Vec<Handle> = self
            .comps
            .iter()
            .filter(|(_, s)| s.running && s.started)
            .map(|(id, s)| Handle { go: s.go, comp: id })
            .collect();
        for h in list {
            // component may have been disabled/destroyed by an earlier callback
            if self.comps.get(h.comp).map_or(true, |s| !s.running) {
                continue;
            }
            self.with_component(h, |c, s| f(c, s, h));
        }
        // anything spawned during callbacks gets its on_start before the next phase
        self.run_pending_starts();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbox_math::Vec3;

    #[derive(Default)]
    struct Probe {
        log: Vec<&'static str>,
    }
    impl Component for Probe {
        fn on_awake(&mut self, _: &mut Scene, _: Handle) { self.log.push("awake"); }
        fn on_enabled(&mut self, _: &mut Scene, _: Handle) { self.log.push("enabled"); }
        fn on_start(&mut self, _: &mut Scene, _: Handle) { self.log.push("start"); }
        fn on_update(&mut self, _: &mut Scene, _: Handle) { self.log.push("update"); }
        fn on_disabled(&mut self, _: &mut Scene, _: Handle) { self.log.push("disabled"); }
        fn on_destroy(&mut self, _: &mut Scene, _: Handle) { self.log.push("destroy"); }
        impl_component_any!();
    }

    #[test]
    fn lifecycle_order() {
        let mut s = Scene::new();
        let go = s.create_object("a");
        s.add_component(go, Probe::default());
        s.tick(0.016);
        s.set_enabled(go, false);
        s.set_enabled(go, true);
        let log = s.component::<Probe>(go).unwrap().log.clone();
        assert_eq!(log, ["awake", "enabled", "start", "update", "disabled", "enabled"]);
    }

    #[test]
    fn destroy_is_deferred_and_recursive() {
        let mut s = Scene::new();
        let a = s.create_object("a");
        let b = s.create_object_in(Some(a), "b");
        s.destroy_object(a);
        assert_eq!(s.object_count(), 2);
        s.tick(0.016);
        assert_eq!(s.object_count(), 0);
        assert!(s.get(b).is_none());
    }

    #[test]
    fn reparent_keeps_world() {
        let mut s = Scene::new();
        let p = s.create_object("p");
        s.set_local_transform(p, Transform::from_position(Vec3::new(10.0, 0.0, 0.0)));
        let c = s.create_object("c");
        s.set_local_transform(c, Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
        s.set_parent(c, Some(p), true);
        assert!((s.world_transform(c).position - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5);
        assert!((s.local_transform(c).position - Vec3::new(-9.0, 2.0, 3.0)).length() < 1e-5);
    }

    #[test]
    fn disabled_parent_disables_children() {
        let mut s = Scene::new();
        let p = s.create_object("p");
        let c = s.create_object_in(Some(p), "c");
        s.add_component(c, Probe::default());
        s.set_enabled(p, false);
        assert!(!s.get(c).unwrap().active());
        s.tick(0.016);
        assert_eq!(s.component::<Probe>(c).unwrap().log, ["awake", "enabled", "disabled"]);
    }
}
