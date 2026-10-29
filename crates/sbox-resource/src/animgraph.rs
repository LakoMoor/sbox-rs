//! `.vanmgrph_c` animation graphs: parsing and a reduced runtime evaluator.
//!
//! Supported: sequence, blend (by parameter), choice/selector (first child), state machines with
//! parameter / time / tag / finished conditions and timed cross-fades, and pass-through of the
//! base input for layering nodes (add, subtract, bone mask, look-at, aim matrix, IK, ...).
//! Not supported yet: additive layers, bone masks, IK, aim/look-at, root motion.
//! Comparison operator ids (0 `==`, 1 `!=`, 2 `<`, 3 `>`, 4 `<=`, 5 `>=`) are inferred from the data.

use crate::anim::BonePose;
use crate::{kv3, Resource, Value};
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum Cond {
    Param { id: i64, op: i64, value: f64 },
    Time { op: i64, value: f64 },
    Tag { id: i64, value: bool },
    Finished,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub dest: i64,
    pub conditions: Vec<Cond>,
    pub blend: f32,
}

#[derive(Clone, Debug)]
pub struct State {
    pub id: i64,
    pub name: String,
    pub input: Option<i64>,
    pub start: bool,
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug)]
pub enum Node {
    Sequence { name: String, speed: f32, looping: bool },
    Blend { param: Option<i64>, children: Vec<(i64, f32)> },
    First { children: Vec<i64> },
    /// Picks `children[value of param]` (bool / enum / int selector).
    Select { param: Option<i64>, children: Vec<i64> },
    /// Directional blend over (sequence, [x, y]) points driven by two parameters.
    Blend2D { px: Option<i64>, py: Option<i64>, items: Vec<(String, [f32; 2])> },
    StateMachine { states: Vec<State> },
    Pass { child: Option<i64> },
    /// Base pose with an additive (delta) pose applied on top.
    Add { base: Option<i64>, additive: Option<i64> },
    /// Generates an additive delta: `base - subtract`.
    Subtract { base: Option<i64>, subtract: Option<i64> },
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub id: i64,
    pub default: f64,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: HashMap<i64, Node>,
    pub root: Option<i64>,
    pub params: Vec<Param>,
}

fn num(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Bool(b)) => *b as i32 as f64,
        Some(Value::Int(i)) => *i as f64,
        Some(Value::UInt(i)) => *i as f64,
        Some(Value::Double(d)) => *d,
        Some(o @ Value::Object(_)) => num(o.get("m_data")),
        _ => 0.0,
    }
}

fn id_of(v: Option<&Value>) -> Option<i64> {
    v.and_then(|v| v.get("m_id")).and_then(Value::as_i64)
}

fn conn(v: &Value, key: &str) -> Option<i64> {
    let c = v.get(key)?;
    let id = id_of(c.get("m_nodeID"))?;
    Some(id)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).unwrap_or(&[])
}

fn parse_cond(c: &Value) -> Option<Cond> {
    let op = num(c.get("m_comparisonOp")) as i64;
    match c.get("_class")?.as_str()? {
        "CParameterAnimCondition" => Some(Cond::Param { id: id_of(c.get("m_paramID"))?, op, value: num(c.get("m_comparisonValue")) }),
        "CTimeCondition" => Some(Cond::Time { op, value: num(c.get("m_comparisonValue")) }),
        "CTagCondition" => Some(Cond::Tag { id: id_of(c.get("m_tagID"))?, value: num(c.get("m_comparisonValue")) != 0.0 }),
        "CFinishedCondition" => Some(Cond::Finished),
        _ => None,
    }
}

const PASS_KEYS: [&str; 6] = ["m_inputConnection", "m_baseInput", "m_baseInputConnection", "m_inputConnection1", "m_input1", "m_pChild"];

fn parse_node(v: &Value) -> Option<Node> {
    let class = v.get("_class")?.as_str()?;
    Some(match class {
        "CSequenceAnimNode" => Node::Sequence {
            name: v.get("m_sequenceName")?.as_str()?.to_string(),
            speed: v.get("m_playbackSpeed").map_or(1.0, |s| num(Some(s)) as f32),
            looping: num(v.get("m_bLoop")) != 0.0,
        },
        // A frozen pose of a sequence: played at speed 0 (the frame index is ignored, nearly always 0).
        "CSingleFrameAnimNode" => Node::Sequence { name: v.get("m_sequenceName")?.as_str()?.to_string(), speed: 0.0, looping: false },
        "CBlendAnimNode" => Node::Blend {
            param: id_of(v.get("m_param")),
            children: list(v, "m_children")
                .iter()
                .filter_map(|c| Some((conn(c, "m_inputConnection")?, num(c.get("m_blendValue")) as f32)))
                .collect(),
        },
        "CSelectorAnimNode" => {
            let param = ["m_enumParamID", "m_intParamID", "m_boolParamID"]
                .iter()
                .filter_map(|k| id_of(v.get(k)))
                .find(|id| *id != 0xFFFF_FFFF);
            Node::Select { param, children: list(v, "m_children").iter().filter_map(|c| id_of(c.get("m_nodeID"))).collect() }
        }
        "CBlend2DAnimNode" => Node::Blend2D {
            px: id_of(v.get("m_paramX")),
            py: id_of(v.get("m_paramY")),
            items: list(v, "m_items")
                .iter()
                .filter_map(|i| {
                    let b = i.get("m_blendValue").and_then(Value::as_array)?;
                    Some((i.get("m_sequenceName")?.as_str()?.to_string(), [num(b.first()) as f32, num(b.get(1)) as f32]))
                })
                .collect(),
        },
        "CChoiceAnimNode" => Node::First {
            children: list(v, "m_children").iter().filter_map(|c| conn(c, "m_inputConnection")).collect(),
        },
        "CStateMachineAnimNode" => Node::StateMachine {
            states: list(v, "m_states")
                .iter()
                .filter_map(|s| {
                    Some(State {
                        id: id_of(s.get("m_stateID"))?,
                        name: s.get("m_name").and_then(Value::as_str).unwrap_or("").to_string(),
                        input: conn(s, "m_inputConnection"),
                        start: matches!(s.get("m_bIsStartState"), Some(Value::Bool(true))),
                        transitions: list(s, "m_transitions")
                            .iter()
                            .filter(|t| !matches!(t.get("m_bDisabled"), Some(Value::Bool(true))))
                            .filter_map(|t| {
                                Some(Transition {
                                    dest: id_of(t.get("m_destState"))?,
                                    conditions: list(t, "m_conditions").iter().filter_map(parse_cond).collect(),
                                    blend: num(t.get("m_blendDuration")) as f32,
                                })
                            })
                            .collect(),
                    })
                })
                .collect(),
        },
        // Layering / modifier nodes: follow the base input.
        "CAddAnimNode" => Node::Add { base: conn(v, "m_baseInput"), additive: conn(v, "m_additiveInput") },
        "CSubtractAnimNode" => Node::Subtract { base: conn(v, "m_baseInputConnection"), subtract: conn(v, "m_subtractInputConnection") },
        "CRootAnimNode" | "CBoneMaskAnimNode" | "CLookAtAnimNode" | "CAimMatrixAnimNode"
        | "CTwoBoneIKAnimNode" | "CSolveIKChainAnimNode" | "CSpeedScaleAnimNode" | "CHitReactAnimNode" | "CCycleControlAnimNode"
        | "CChoreoAnimNode" | "CDirectPlaybackAnimNode" => Node::Pass { child: PASS_KEYS.iter().find_map(|k| conn(v, k)) },
        _ => return None,
    })
}

impl Graph {
    pub fn parse(file: &[u8]) -> Result<Graph> {
        let res = Resource::parse(file)?;
        let kv = kv3::parse(res.block_data("DATA").ok_or_else(|| anyhow!("no DATA"))?)?;
        let mut g = Graph::default();
        let nodes = kv.get("m_nodeManager").map(|m| list(m, "m_nodes")).unwrap_or(&[]);
        for n in nodes {
            let (Some(id), Some(val)) = (id_of(n.get("key")), n.get("value")) else { continue };
            if val.get("_class").and_then(Value::as_str) == Some("CRootAnimNode") {
                g.root = Some(id);
            }
            if let Some(node) = parse_node(val) {
                g.nodes.insert(id, node);
            }
        }
        for p in kv.get("m_pParameterList").map(|m| list(m, "m_Parameters")).unwrap_or(&[]) {
            let default = ["m_fDefaultValue", "m_bDefaultValue", "m_nDefaultValue", "m_defaultValue"]
                .iter()
                .find_map(|k| p.get(k))
                .map_or(0.0, |v| num(Some(v)));
            if let (Some(name), Some(id)) = (p.get("m_name").and_then(Value::as_str), id_of(p.get("m_id"))) {
                g.params.push(Param { name: name.to_string(), id, default });
            }
        }
        Ok(g)
    }

    pub fn param_id(&self, name: &str) -> Option<i64> {
        self.params.iter().find(|p| p.name == name).map(|p| p.id)
    }
}

/// One clip contributing to the current output.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub sequence: String,
    pub weight: f32,
    pub time: f32,
}

#[derive(Default)]
struct MachineState {
    state: i64,
    time: f32,
    prev: Option<(i64, f32, f32, f32)>, // state, its time, blend elapsed, blend duration
}

pub struct Instance {
    pub params: HashMap<i64, f64>,
    pub tags: HashSet<i64>,
    machines: HashMap<i64, MachineState>,
    time: f32,
}

fn compare(op: i64, a: f64, b: f64) -> bool {
    match op {
        0 => (a - b).abs() < 1e-6,
        1 => (a - b).abs() >= 1e-6,
        2 => a < b,
        3 => a > b,
        4 => a <= b,
        5 => a >= b,
        _ => false,
    }
}

impl Instance {
    pub fn new(graph: &Graph) -> Self {
        Self { params: graph.params.iter().map(|p| (p.id, p.default)).collect(), tags: HashSet::new(), machines: HashMap::new(), time: 0.0 }
    }

    pub fn set(&mut self, graph: &Graph, name: &str, value: f64) {
        if let Some(id) = graph.param_id(name) {
            self.params.insert(id, value);
        }
    }

    /// Advance by `dt` and return the weighted clips that make up the pose. `clip_len` gives a clip's duration
    /// in seconds (used by `Finished` conditions). Weights sum to 1.
    pub fn evaluate(&mut self, graph: &Graph, dt: f32, clip_len: &mut dyn FnMut(&str) -> f32) -> Vec<Sample> {
        self.time += dt;
        let mut out = Vec::new();
        if let Some(root) = graph.root {
            self.eval(graph, root, 1.0, self.time, dt, clip_len, &mut out, 0);
        }
        let total: f32 = out.iter().map(|s| s.weight).sum();
        if total > 0.0 {
            for s in &mut out {
                s.weight /= total;
            }
        }
        out
    }

    fn cond_true(&self, c: &Cond, state_time: f32, finished: bool) -> bool {
        match *c {
            Cond::Param { id, op, value } => compare(op, self.params.get(&id).copied().unwrap_or(0.0), value),
            Cond::Time { op, value } => compare(op, state_time as f64, value),
            Cond::Tag { id, value } => self.tags.contains(&id) == value,
            Cond::Finished => finished,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn eval(&mut self, g: &Graph, id: i64, w: f32, t: f32, dt: f32, len: &mut dyn FnMut(&str) -> f32, out: &mut Vec<Sample>, depth: u32) {
        if depth > 512 || w <= 0.0 {
            return;
        }
        let Some(node) = g.nodes.get(&id) else { return };
        match node {
            Node::Sequence { name, speed, .. } => out.push(Sample { sequence: name.clone(), weight: w, time: t * speed }),
            Node::Pass { child } | Node::Add { base: child, .. } | Node::Subtract { base: child, .. } => {
                if let Some(c) = child {
                    self.eval(g, *c, w, t, dt, len, out, depth + 1);
                }
            }
            Node::First { children } => {
                if let Some(c) = children.first() {
                    self.eval(g, *c, w, t, dt, len, out, depth + 1);
                }
            }
            Node::Select { param, children } => {
                let i = param.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0).max(0.0) as usize;
                if let Some(c) = children.get(i.min(children.len().saturating_sub(1))) {
                    self.eval(g, *c, w, t, dt, len, out, depth + 1);
                }
            }
            Node::Blend2D { px, py, items } => {
                let x = px.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let y = py.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let mut near: Vec<(f32, &String)> = items.iter().map(|(n, p)| ((p[0] - x).powi(2) + (p[1] - y).powi(2), n)).collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.truncate(3);
                if let Some(first) = near.first() {
                    if first.0 < 1e-4 {
                        near.truncate(1);
                    }
                    let inv: Vec<f32> = near.iter().map(|(d, _)| 1.0 / (d + 1e-3)).collect();
                    let total: f32 = inv.iter().sum();
                    for ((_, name), wi) in near.iter().zip(inv) {
                        out.push(Sample { sequence: (*name).clone(), weight: w * wi / total, time: t });
                    }
                }
            }
            Node::Blend { param, children } => {
                let v = param.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let mut sorted = children.clone();
                sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
                match sorted.iter().position(|c| c.1 >= v) {
                    None => {
                        if let Some(l) = sorted.last() {
                            self.eval(g, l.0, w, t, dt, len, out, depth + 1);
                        }
                    }
                    Some(0) => self.eval(g, sorted[0].0, w, t, dt, len, out, depth + 1),
                    Some(i) => {
                        let (a, b) = (sorted[i - 1], sorted[i]);
                        let f = ((v - a.1) / (b.1 - a.1).max(1e-6)).clamp(0.0, 1.0);
                        self.eval(g, a.0, w * (1.0 - f), t, dt, len, out, depth + 1);
                        self.eval(g, b.0, w * f, t, dt, len, out, depth + 1);
                    }
                }
            }
            Node::StateMachine { states } => {
                let Some(start) = states.iter().find(|s| s.start).or_else(|| states.first()) else { return };
                let mut m = self.machines.remove(&id).unwrap_or(MachineState { state: start.id, ..Default::default() });
                m.time += dt;
                if let Some(p) = &mut m.prev {
                    p.1 += dt;
                    p.2 += dt;
                    if p.2 >= p.3 {
                        m.prev = None;
                    }
                }
                if let Some(cur) = states.iter().find(|s| s.id == m.state) {
                    let needs_finished = cur.transitions.iter().any(|tr| tr.conditions.iter().any(|c| matches!(c, Cond::Finished)));
                    let finished = needs_finished && {
                        let mut first = Vec::new();
                        if let Some(inp) = cur.input {
                            // Probe only the state's first clip for its duration.
                            let mut probe = Instance { params: self.params.clone(), tags: self.tags.clone(), machines: HashMap::new(), time: 0.0 };
                            probe.eval(g, inp, 1.0, 0.0, 0.0, len, &mut first, depth + 1);
                        }
                        first.first().map_or(false, |s| m.time >= len(&s.sequence))
                    };
                    let next = cur.transitions.iter().find(|tr| tr.dest != cur.id && tr.conditions.iter().all(|c| self.cond_true(c, m.time, finished)));
                    if let Some(tr) = next {
                        if tr.blend > 0.0 {
                            m.prev = Some((m.state, m.time, 0.0, tr.blend));
                        }
                        m.state = tr.dest;
                        m.time = 0.0;
                    }
                }
                let cur_w = m.prev.map_or(1.0, |p| (p.2 / p.3).clamp(0.0, 1.0));
                let cur_input = states.iter().find(|s| s.id == m.state).and_then(|s| s.input);
                let prev = m.prev;
                let cur_time = m.time;
                self.machines.insert(id, m);
                if let Some(inp) = cur_input {
                    self.eval(g, inp, w * cur_w, cur_time, 0.0, len, out, depth + 1);
                }
                if let Some((ps, pt, _, _)) = prev {
                    if let Some(inp) = states.iter().find(|s| s.id == ps).and_then(|s| s.input) {
                        self.eval(g, inp, w * (1.0 - cur_w), pt, 0.0, len, out, depth + 1);
                    }
                }
            }
        }
    }
}

/// A bone pose set; `additive` marks a delta produced by `Subtract` or an additive clip.
#[derive(Clone, Debug)]
pub struct Pose {
    pub bones: Vec<BonePose>,
    pub additive: bool,
}

/// Source of clip poses and durations for [`Instance::evaluate_pose`].
pub struct Clips<'a> {
    /// Pose of a sequence at a time in seconds, plus whether the clip is additive.
    pub sample: &'a mut dyn FnMut(&str, f32) -> Option<(Vec<BonePose>, bool)>,
    pub len: &'a mut dyn FnMut(&str) -> f32,
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn qinv(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// `base - sub` as a local-space delta: positions subtract, rotation is `inv(sub) * base`.
pub fn subtract_poses(base: &[BonePose], sub: &[BonePose]) -> Vec<BonePose> {
    base.iter()
        .zip(sub)
        .map(|(b, s)| BonePose {
            position: std::array::from_fn(|i| b.position[i] - s.position[i]),
            rotation: qmul(qinv(s.rotation), b.rotation),
            scale: b.scale,
        })
        .collect()
}

/// Apply a delta on top of `base`: positions add, rotation is `base * delta` (both scaled by `weight`).
pub fn apply_additive(base: &mut [BonePose], delta: &[BonePose], weight: f32) {
    for (b, d) in base.iter_mut().zip(delta) {
        for i in 0..3 {
            b.position[i] += d.position[i] * weight;
        }
        let mut q = [d.rotation[0] * weight, d.rotation[1] * weight, d.rotation[2] * weight, 1.0 + (d.rotation[3] - 1.0) * weight];
        let l = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        q = q.map(|x| x / l);
        b.rotation = qmul(b.rotation, q);
    }
}

fn first_sequence(g: &Graph, id: i64, depth: u32) -> Option<String> {
    if depth > 128 {
        return None;
    }
    match g.nodes.get(&id)? {
        Node::Sequence { name, .. } => Some(name.clone()),
        Node::Pass { child } | Node::Add { base: child, .. } | Node::Subtract { base: child, .. } => first_sequence(g, (*child)?, depth + 1),
        Node::First { children } | Node::Select { children, .. } => first_sequence(g, *children.first()?, depth + 1),
        Node::Blend { children, .. } => first_sequence(g, children.first()?.0, depth + 1),
        Node::Blend2D { items, .. } => items.first().map(|i| i.0.clone()),
        Node::StateMachine { states } => first_sequence(g, states.iter().find(|s| s.start).or_else(|| states.first())?.input?, depth + 1),
    }
}

impl Instance {
    /// Advance by `dt` and return the final pose, computed node by node (so additive layers work).
    pub fn evaluate_pose(&mut self, graph: &Graph, dt: f32, clips: &mut Clips) -> Option<Pose> {
        self.time += dt;
        let root = graph.root?;
        self.pose(graph, root, self.time, dt, clips, 0)
    }

    fn pose(&mut self, g: &Graph, id: i64, t: f32, dt: f32, clips: &mut Clips, depth: u32) -> Option<Pose> {
        if depth > 512 {
            return None;
        }
        let node = g.nodes.get(&id)?;
        let seq = |clips: &mut Clips, name: &str, time: f32| (clips.sample)(name, time).map(|(bones, additive)| Pose { bones, additive });
        match node {
            Node::Sequence { name, speed, .. } => seq(clips, name, t * speed),
            Node::Pass { child } => self.pose(g, (*child)?, t, dt, clips, depth + 1),
            Node::First { children } => self.pose(g, *children.first()?, t, dt, clips, depth + 1),
            Node::Select { param, children } => {
                let i = param.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0).max(0.0) as usize;
                let c = *children.get(i.min(children.len().saturating_sub(1)))?;
                self.pose(g, c, t, dt, clips, depth + 1)
            }
            Node::Add { base, additive } => {
                let mut b = self.pose(g, (*base)?, t, dt, clips, depth + 1)?;
                if let Some(a) = additive {
                    if let Some(d) = self.pose(g, *a, t, dt, clips, depth + 1) {
                        apply_additive(&mut b.bones, &d.bones, 1.0);
                    }
                }
                Some(b)
            }
            Node::Subtract { base, subtract } => {
                let b = self.pose(g, (*base)?, t, dt, clips, depth + 1)?;
                let s = self.pose(g, (*subtract)?, t, dt, clips, depth + 1)?;
                Some(Pose { bones: subtract_poses(&b.bones, &s.bones), additive: true })
            }
            Node::Blend2D { px, py, items } => {
                let x = px.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let y = py.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let mut near: Vec<(f32, &String)> = items.iter().map(|(n, p)| ((p[0] - x).powi(2) + (p[1] - y).powi(2), n)).collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.truncate(3);
                if near.first().map_or(false, |f| f.0 < 1e-4) {
                    near.truncate(1);
                }
                let mut parts = Vec::new();
                for (d, name) in near {
                    if let Some(p) = seq(clips, name, t) {
                        parts.push((p.bones, 1.0 / (d + 1e-3)));
                    }
                }
                (!parts.is_empty()).then(|| Pose { bones: blend_poses(&parts), additive: false })
            }
            Node::Blend { param, children } => {
                let v = param.and_then(|p| self.params.get(&p).copied()).unwrap_or(0.0) as f32;
                let mut sorted = children.clone();
                sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
                match sorted.iter().position(|c| c.1 >= v) {
                    None => self.pose(g, sorted.last()?.0, t, dt, clips, depth + 1),
                    Some(0) => self.pose(g, sorted[0].0, t, dt, clips, depth + 1),
                    Some(i) => {
                        let (a, b) = (sorted[i - 1], sorted[i]);
                        let f = ((v - a.1) / (b.1 - a.1).max(1e-6)).clamp(0.0, 1.0);
                        let pa = self.pose(g, a.0, t, dt, clips, depth + 1);
                        let pb = self.pose(g, b.0, t, dt, clips, depth + 1);
                        match (pa, pb) {
                            (Some(pa), Some(pb)) => Some(Pose { additive: pa.additive, bones: blend_poses(&[(pa.bones, 1.0 - f), (pb.bones, f)]) }),
                            (p, None) | (None, p) => p,
                        }
                    }
                }
            }
            Node::StateMachine { states } => {
                let start = states.iter().find(|s| s.start).or_else(|| states.first())?;
                let mut m = self.machines.remove(&id).unwrap_or(MachineState { state: start.id, ..Default::default() });
                m.time += dt;
                if let Some(p) = &mut m.prev {
                    p.1 += dt;
                    p.2 += dt;
                    if p.2 >= p.3 {
                        m.prev = None;
                    }
                }
                if let Some(cur) = states.iter().find(|s| s.id == m.state) {
                    let needs_finished = cur.transitions.iter().any(|tr| tr.conditions.iter().any(|c| matches!(c, Cond::Finished)));
                    let finished = needs_finished && cur.input.and_then(|i| first_sequence(g, i, 0)).map_or(false, |n| m.time >= (clips.len)(&n));
                    let next = cur.transitions.iter().find(|tr| tr.dest != cur.id && tr.conditions.iter().all(|c| self.cond_true(c, m.time, finished)));
                    if let Some(tr) = next {
                        if tr.blend > 0.0 {
                            m.prev = Some((m.state, m.time, 0.0, tr.blend));
                        }
                        m.state = tr.dest;
                        m.time = 0.0;
                    }
                }
                let cur_w = m.prev.map_or(1.0, |p| (p.2 / p.3).clamp(0.0, 1.0));
                let cur_input = states.iter().find(|s| s.id == m.state).and_then(|s| s.input);
                let (prev, cur_time) = (m.prev, m.time);
                self.machines.insert(id, m);
                let cur = cur_input.and_then(|i| self.pose(g, i, cur_time, 0.0, clips, depth + 1));
                let old = prev.and_then(|(ps, pt, _, _)| {
                    let inp = states.iter().find(|s| s.id == ps)?.input?;
                    self.pose(g, inp, pt, 0.0, clips, depth + 1)
                });
                match (cur, old) {
                    (Some(c), Some(o)) => Some(Pose { additive: c.additive, bones: blend_poses(&[(c.bones, cur_w), (o.bones, 1.0 - cur_w)]) }),
                    (p, None) | (None, p) => p,
                }
            }
        }
    }
}

/// Weighted blend of poses (positions lerp, rotations nlerp with sign alignment).
pub fn blend_poses(poses: &[(Vec<BonePose>, f32)]) -> Vec<BonePose> {
    let Some((first, _)) = poses.first() else { return Vec::new() };
    let n = poses.iter().map(|p| p.0.len()).min().unwrap_or(0).min(first.len());
    let total: f32 = poses.iter().map(|p| p.1).sum::<f32>().max(1e-6);
    (0..n)
        .map(|b| {
            let mut pos = [0.0f32; 3];
            let mut rot = [0.0f32; 4];
            let mut scale = 0.0;
            let reference = first[b].rotation;
            for (p, w) in poses {
                let w = w / total;
                let q = p[b].rotation;
                let dot: f32 = (0..4).map(|i| q[i] * reference[i]).sum();
                let sign = if dot < 0.0 { -1.0 } else { 1.0 };
                for i in 0..3 {
                    pos[i] += p[b].position[i] * w;
                }
                for i in 0..4 {
                    rot[i] += q[i] * sign * w;
                }
                scale += p[b].scale * w;
            }
            let len = rot.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
            BonePose { position: pos, rotation: rot.map(|x| x / len), scale }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = "E:/sbox-ref/game/addons/citizen/Assets/models/citizen/citizen.vanmgrph_c";

    #[test]
    fn citizen_graph_parses_and_evaluates() {
        let Ok(bytes) = std::fs::read(GRAPH) else { return };
        let g = Graph::parse(&bytes).unwrap();
        assert!(g.root.is_some() && g.nodes.len() > 500 && g.params.len() > 40, "{} nodes {} params", g.nodes.len(), g.params.len());
        let mut inst = Instance::new(&g);
        let mut len = |_: &str| 1.0;
        let mut out = Vec::new();
        for _ in 0..30 {
            out = inst.evaluate(&g, 1.0 / 30.0, &mut len);
        }
        if out.is_empty() {
            let mut id = g.root;
            for _ in 0..40 {
                let Some(i) = id else { println!("chain ended (no child id)"); break };
                let Some(n) = g.nodes.get(&i) else { println!("chain {i}: node missing/unparsed"); break };
                let d = match n {
                    Node::Pass { child } | Node::Add { base: child, .. } | Node::Subtract { base: child, .. } => format!("Pass {child:?}"),
                    Node::First { children } | Node::Select { children, .. } => format!("First {children:?}"),
                    Node::Blend2D { items, .. } => format!("Blend2D {}", items.len()),
                    Node::Blend { children, .. } => format!("Blend {children:?}"),
                    Node::StateMachine { states } => format!("SM {:?}", states.iter().map(|s| (&s.name, s.start, s.input)).collect::<Vec<_>>()),
                    Node::Sequence { name, .. } => format!("Seq {name}"),
                };
                println!("chain {i}: {d}");
                id = match n {
                    Node::Pass { child } | Node::Add { base: child, .. } | Node::Subtract { base: child, .. } => *child,
                    Node::First { children } | Node::Select { children, .. } => children.first().copied(),
                    Node::Blend { children, .. } => children.first().map(|c| c.0),
                    Node::StateMachine { states } => states.iter().find(|s| s.start).and_then(|s| s.input),
                    _ => None,
                };
            }
        }
        assert!(!out.is_empty(), "default parameters should produce at least one clip");
        let sum: f32 = out.iter().map(|s| s.weight).sum();
        assert!((sum - 1.0).abs() < 1e-3, "weights sum to {sum}");
        println!("default pose: {out:?}");
    }

    #[test]
    fn subtract_then_apply_reproduces_the_target() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let reference = vec![BonePose { position: [1.0, 1.0, 0.0], rotation: [0.0, 0.0, s, s], scale: 1.0 }];
        let target = vec![BonePose { position: [3.0, 0.0, 2.0], rotation: [s, 0.0, 0.0, s], scale: 1.0 }];
        let delta = subtract_poses(&target, &reference);
        let mut out = reference.clone();
        apply_additive(&mut out, &delta, 1.0);
        for i in 0..3 {
            assert!((out[0].position[i] - target[0].position[i]).abs() < 1e-5);
        }
        let dot: f32 = (0..4).map(|i| out[0].rotation[i] * target[0].rotation[i]).sum();
        assert!(dot.abs() > 0.9999, "rotation mismatch, dot {dot}");
    }

    #[test]
    fn citizen_graph_produces_a_pose() {
        let Ok(bytes) = std::fs::read(GRAPH) else { return };
        let g = Graph::parse(&bytes).unwrap();
        let mut inst = Instance::new(&g);
        let id = vec![BonePose { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0 }; 4];
        let mut requested = Vec::new();
        let mut sample = |n: &str, _t: f32| {
            requested.push(n.to_string());
            Some((id.clone(), n.ends_with("_delta")))
        };
        let mut len = |_: &str| 1.0;
        let mut pose = None;
        for _ in 0..10 {
            pose = inst.evaluate_pose(&g, 1.0 / 30.0, &mut Clips { sample: &mut sample, len: &mut len });
        }
        assert!(pose.is_some(), "default parameters should produce a pose");
        assert!(requested.iter().any(|n| n == "IdlePose_Default"), "{requested:?}");
    }

    #[test]
    fn blend_two_poses_midpoint() {
        let a = vec![BonePose { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0 }];
        let b = vec![BonePose { position: [10.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, -1.0], scale: 1.0 }];
        let r = blend_poses(&[(a, 0.5), (b, 0.5)]);
        assert!((r[0].position[0] - 5.0).abs() < 1e-5);
        assert!((r[0].rotation[3].abs() - 1.0).abs() < 1e-5, "{:?}", r[0].rotation);
    }
}
