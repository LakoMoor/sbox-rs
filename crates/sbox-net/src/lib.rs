//! Authoritative client-server networking over UDP. The server owns the world and sends transform snapshots; clients
//! send their name and input. This is a small protocol of our own, not s&box's (whose networking is closed C# / Steam
//! Networking). Packets are tiny and unreliable except for the connect handshake, which the client retries.

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const MAGIC: [u8; 2] = *b"sb";
const MAX_PACKET: usize = 1400;
/// Bytes one entity takes in a snapshot: id + 3 position + 4 rotation floats.
const ENTITY_BYTES: usize = 32;

/// State of one networked object in a snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityState {
    pub id: u32,
    pub position: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Packet {
    Connect { name: String },
    Accepted { client_id: u32 },
    /// Per-tick client input: movement axes in -1..1, look angles in degrees and a button bit set.
    /// `dt` is the client frame time the movement was held for; the server integrates it exactly once.
    Input { seq: u32, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32, dt: f32 },
    /// `ack` is the last input of the receiving client that the server has applied. A tick's entities may span several
    /// packets (`part` of `parts`); they hold only what changed since the last tick the client acknowledged
    /// ([`Packet::SnapshotAck`]), the client keeps the rest as it was.
    Snapshot { tick: u32, ack: u32, part: u8, parts: u8, entities: Vec<EntityState> },
    /// The client has received every packet of the snapshot of this tick.
    SnapshotAck { tick: u32 },
    Disconnect,
    /// Reliable remote call: delivered at least once by retransmission, executed once (the receiver drops duplicates).
    Rpc { id: u32, name: String, args: Vec<u8> },
    RpcAck { id: u32 },
}

/// Largest RPC name and argument sizes; a call must fit one packet.
pub const MAX_RPC_NAME: usize = 64;
pub const MAX_RPC_ARGS: usize = 1000;

struct Writer(Vec<u8>);
impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes())
    }
    fn f32(&mut self, v: f32) {
        self.0.extend(v.to_le_bytes())
    }
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.0.len() < n {
            bail!("truncated packet")
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(MAGIC.to_vec());
        match self {
            Packet::Connect { name } => {
                w.0.push(1);
                let b = name.as_bytes();
                let b = &b[..b.len().min(64)];
                w.0.push(b.len() as u8);
                w.0.extend(b);
            }
            Packet::Accepted { client_id } => {
                w.0.push(2);
                w.u32(*client_id);
            }
            Packet::Input { seq, movement, yaw, pitch, buttons, dt } => {
                w.0.push(3);
                w.u32(*seq);
                for v in [movement[0], movement[1], *yaw, *pitch, *dt] {
                    w.f32(v);
                }
                w.u32(*buttons);
            }
            Packet::Snapshot { tick, ack, part, parts, entities } => {
                w.0.push(4);
                w.u32(*tick);
                w.u32(*ack);
                w.0.push(*part);
                w.0.push(*parts);
                w.u32(entities.len() as u32);
                for e in entities {
                    w.u32(e.id);
                    for v in e.position.iter().chain(&e.rotation) {
                        w.f32(*v);
                    }
                }
            }
            Packet::Disconnect => w.0.push(5),
            Packet::Rpc { id, name, args } => {
                w.0.push(6);
                w.u32(*id);
                w.0.push(name.len().min(MAX_RPC_NAME) as u8);
                w.0.extend(&name.as_bytes()[..name.len().min(MAX_RPC_NAME)]);
                w.0.extend((args.len().min(MAX_RPC_ARGS) as u16).to_le_bytes());
                w.0.extend(&args[..args.len().min(MAX_RPC_ARGS)]);
            }
            Packet::RpcAck { id } => {
                w.0.push(7);
                w.u32(*id);
            }
            Packet::SnapshotAck { tick } => {
                w.0.push(8);
                w.u32(*tick);
            }
        }
        w.0
    }

    pub fn decode(data: &[u8]) -> Result<Packet> {
        let mut r = Reader(data);
        if r.take(2)? != MAGIC {
            bail!("bad magic")
        }
        Ok(match r.u8()? {
            1 => {
                let n = r.u8()? as usize;
                Packet::Connect { name: String::from_utf8_lossy(r.take(n)?).into_owned() }
            }
            2 => Packet::Accepted { client_id: r.u32()? },
            3 => {
                let seq = r.u32()?;
                let (m0, m1, yaw, pitch, dt) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                Packet::Input { seq, movement: [m0, m1], yaw, pitch, buttons: r.u32()?, dt }
            }
            4 => {
                let tick = r.u32()?;
                let ack = r.u32()?;
                let (part, parts) = (r.u8()?, r.u8()?);
                let n = r.u32()? as usize;
                // Reject counts the packet cannot hold before allocating.
                if n > MAX_PACKET / ENTITY_BYTES {
                    bail!("too many entities")
                }
                let mut entities = Vec::with_capacity(n);
                for _ in 0..n {
                    entities.push(EntityState { id: r.u32()?, position: [r.f32()?, r.f32()?, r.f32()?], rotation: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] });
                }
                Packet::Snapshot { tick, ack, part, parts, entities }
            }
            5 => Packet::Disconnect,
            6 => {
                let id = r.u32()?;
                let n = r.u8()? as usize;
                let name = String::from_utf8_lossy(r.take(n)?).into_owned();
                let len = u16::from_le_bytes(r.take(2)?.try_into()?) as usize;
                Packet::Rpc { id, name, args: r.take(len)?.to_vec() }
            }
            7 => Packet::RpcAck { id: r.u32()? },
            8 => Packet::SnapshotAck { tick: r.u32()? },
            t => bail!("unknown packet type {t}"),
        })
    }
}

const RESEND_AFTER: Duration = Duration::from_millis(200);

/// Sending side of the reliable channel: remembers unacknowledged calls and re-sends them.
#[derive(Default)]
struct Outbox {
    next: u32,
    pending: HashMap<u32, (Vec<u8>, Instant)>,
}

impl Outbox {
    fn push(&mut self, name: &str, args: &[u8]) -> Result<Vec<u8>> {
        if name.len() > MAX_RPC_NAME || args.len() > MAX_RPC_ARGS {
            bail!("rpc too large ({} byte name, {} bytes of arguments)", name.len(), args.len());
        }
        self.next += 1;
        let data = Packet::Rpc { id: self.next, name: name.to_string(), args: args.to_vec() }.encode();
        self.pending.insert(self.next, (data.clone(), Instant::now()));
        Ok(data)
    }

    fn ack(&mut self, id: u32) {
        self.pending.remove(&id);
    }

    /// Packets that were last sent more than [`RESEND_AFTER`] ago.
    fn due(&mut self) -> Vec<Vec<u8>> {
        let now = Instant::now();
        self.pending.values_mut().filter(|(_, t)| now.duration_since(*t) >= RESEND_AFTER).map(|(d, t)| {
            *t = now;
            d.clone()
        }).collect()
    }
}

/// How far ahead of the next expected call an out-of-order one is still held (and acknowledged).
const RPC_WINDOW: u32 = 256;

/// Receiving side: drops retransmitted calls and releases the rest in the order they were made, holding early arrivals
/// until the ones before them have come.
struct Inbox {
    next: u32,
    held: std::collections::BTreeMap<u32, (String, Vec<u8>)>,
}

impl Default for Inbox {
    fn default() -> Self {
        Self { next: 1, held: Default::default() }
    }
}

impl Inbox {
    /// Whether to acknowledge call `id`: everything already delivered or within the window (a call too far ahead is left
    /// unacknowledged, so the sender keeps retrying it until the gap has closed).
    fn acceptable(&self, id: u32) -> bool {
        id < self.next.saturating_add(RPC_WINDOW)
    }

    /// Take call `id`; returns the calls that are now deliverable, in order.
    fn accept(&mut self, id: u32, name: String, args: Vec<u8>) -> Vec<(String, Vec<u8>)> {
        if id >= self.next && self.acceptable(id) {
            self.held.insert(id, (name, args));
        }
        let mut out = Vec::new();
        while let Some(call) = self.held.remove(&self.next) {
            out.push(call);
            self.next += 1;
        }
        out
    }
}

/// Events the server reports from [`Server::poll`].
#[derive(Clone, Debug, PartialEq)]
pub enum ServerEvent {
    Connected { client_id: u32, name: String },
    Input { client_id: u32, seq: u32, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32, dt: f32 },
    Disconnected { client_id: u32 },
    Rpc { client_id: u32, name: String, args: Vec<u8> },
}

pub struct Server {
    socket: UdpSocket,
    clients: HashMap<SocketAddr, u32>,
    reliable: HashMap<u32, (Outbox, Inbox)>,
    /// Last input number applied per client, echoed in snapshots.
    acked: HashMap<u32, u32>,
    /// Newest tick each client has fully received; snapshots to it only carry what changed since then.
    baseline: HashMap<u32, u32>,
    /// Full world states of the last ticks, the bases deltas are computed against.
    history: std::collections::VecDeque<(u32, HashMap<u32, EntityState>)>,
    next_id: u32,
    pub tick: u32,
}

/// How many past ticks the server remembers; a client whose acknowledged tick fell out of it gets the full state.
const HISTORY_TICKS: usize = 64;

impl Server {
    pub fn bind(addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, clients: HashMap::new(), reliable: HashMap::new(), acked: HashMap::new(), baseline: HashMap::new(), history: Default::default(), next_id: 1, tick: 0 })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Read every waiting packet. Malformed packets and input from unknown addresses are dropped.
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut events = Vec::new();
        let mut buf = [0u8; MAX_PACKET + 64];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            let Ok(p) = Packet::decode(&buf[..n]) else { continue };
            match p {
                Packet::Connect { name } => {
                    // A retried Connect from a known client just gets the same id back.
                    let id = match self.clients.get(&from) {
                        Some(&id) => id,
                        None => {
                            let id = self.next_id;
                            self.next_id += 1;
                            self.clients.insert(from, id);
                            self.reliable.insert(id, Default::default());
                            events.push(ServerEvent::Connected { client_id: id, name });
                            id
                        }
                    };
                    let _ = self.socket.send_to(&Packet::Accepted { client_id: id }.encode(), from);
                }
                Packet::Input { seq, movement, yaw, pitch, buttons, dt } => {
                    if let Some(&client_id) = self.clients.get(&from) {
                        // Inputs are applied in arrival order; a reordered older one still counts as applied.
                        let a = self.acked.entry(client_id).or_default();
                        *a = (*a).max(seq);
                        events.push(ServerEvent::Input { client_id, seq, movement, yaw, pitch, buttons, dt });
                    }
                }
                Packet::SnapshotAck { tick } => {
                    if let Some(&client_id) = self.clients.get(&from) {
                        // Only ticks the server really sent count; the baseline never moves backwards.
                        if self.history.iter().any(|(t, _)| *t == tick) {
                            let b = self.baseline.entry(client_id).or_default();
                            *b = (*b).max(tick);
                        }
                    }
                }
                Packet::Disconnect => {
                    if let Some(client_id) = self.clients.remove(&from) {
                        self.reliable.remove(&client_id);
                        self.acked.remove(&client_id);
                        self.baseline.remove(&client_id);
                        events.push(ServerEvent::Disconnected { client_id });
                    }
                }
                Packet::Rpc { id, name, args } => {
                    if let Some(&client_id) = self.clients.get(&from) {
                        if let Some((_, inbox)) = self.reliable.get_mut(&client_id) {
                            // Acknowledge duplicates too (the earlier ack may have been lost), but not calls beyond the window.
                            if inbox.acceptable(id) {
                                let _ = self.socket.send_to(&Packet::RpcAck { id }.encode(), from);
                            }
                            for (name, args) in inbox.accept(id, name, args) {
                                events.push(ServerEvent::Rpc { client_id, name, args });
                            }
                        }
                    }
                }
                Packet::RpcAck { id } => {
                    if let Some((out, _)) = self.clients.get(&from).and_then(|c| self.reliable.get_mut(c)) {
                        out.ack(id);
                    }
                }
                _ => {}
            }
        }
        // Retransmit calls the clients have not acknowledged yet.
        let addrs: HashMap<u32, SocketAddr> = self.clients.iter().map(|(a, c)| (*c, *a)).collect();
        for (id, (out, _)) in &mut self.reliable {
            for data in out.due() {
                if let Some(addr) = addrs.get(id) {
                    let _ = self.socket.send_to(&data, addr);
                }
            }
        }
        events
    }

    /// Reliable call on one client. Fails for unknown clients and calls that do not fit a packet.
    pub fn call(&mut self, client_id: u32, name: &str, args: &[u8]) -> Result<()> {
        let Some(addr) = self.clients.iter().find(|(_, c)| **c == client_id).map(|(a, _)| *a) else { bail!("unknown client {client_id}") };
        let data = self.reliable.get_mut(&client_id).expect("outbox per client").0.push(name, args)?;
        let _ = self.socket.send_to(&data, addr);
        Ok(())
    }

    /// Send the world state to every client as the entities that changed since the tick that client last acknowledged
    /// (all of them for a new client), split into packets that fit the MTU. Advances the tick. A lost packet is repaired
    /// by itself: the client does not acknowledge the tick, so the changes stay in the next delta.
    pub fn broadcast(&mut self, entities: &[EntityState]) {
        self.tick += 1;
        self.history.push_back((self.tick, entities.iter().map(|e| (e.id, *e)).collect()));
        while self.history.len() > HISTORY_TICKS {
            self.history.pop_front();
        }
        let per_packet = MAX_PACKET / ENTITY_BYTES - 1;
        for (addr, id) in &self.clients {
            let base = self.baseline.get(id).and_then(|t| self.history.iter().find(|(h, _)| h == t)).map(|(_, m)| m);
            let changed: Vec<EntityState> = entities.iter().filter(|e| base.map_or(true, |b| b.get(&e.id) != Some(*e))).copied().collect();
            let ack = self.acked.get(id).copied().unwrap_or(0);
            let chunks: Vec<&[EntityState]> = if changed.is_empty() { vec![&[]] } else { changed.chunks(per_packet).collect() };
            let parts = chunks.len().min(255) as u8;
            for (i, chunk) in chunks.into_iter().take(255).enumerate() {
                let data = Packet::Snapshot { tick: self.tick, ack, part: i as u8, parts, entities: chunk.to_vec() }.encode();
                let _ = self.socket.send_to(&data, addr);
            }
        }
    }
}

pub struct Client {
    socket: UdpSocket,
    pub client_id: Option<u32>,
    pub last_tick: u32,
    /// Last input the server reported as applied.
    pub ack: u32,
    /// Number of snapshot packets applied so far.
    pub snapshots: u32,
    /// Latest known state of every entity, by id.
    pub entities: HashMap<u32, EntityState>,
    /// Entities received in snapshot packets so far (deltas make this much smaller than snapshots x entities).
    pub received_entities: u64,
    /// Parts of the newest snapshot tick seen so far, to acknowledge it once it is complete.
    partial: (u32, std::collections::HashSet<u8>),
    seq: u32,
    out: Outbox,
    inbox: Inbox,
    rpcs: Vec<(String, Vec<u8>)>,
}

impl Client {
    pub fn connect(server: &str, name: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(server)?;
        socket.set_nonblocking(true)?;
        socket.send(&Packet::Connect { name: name.to_string() }.encode())?;
        Ok(Self { socket, client_id: None, last_tick: 0, ack: 0, snapshots: 0, entities: HashMap::new(), received_entities: 0, partial: (0, Default::default()), seq: 0, out: Outbox::default(), inbox: Inbox::default(), rpcs: Vec::new() })
    }

    /// Re-send the handshake (call periodically until [`Client::client_id`] is set).
    pub fn retry_connect(&self, name: &str) {
        let _ = self.socket.send(&Packet::Connect { name: name.to_string() }.encode());
    }

    /// Send one input; returns its number.
    pub fn send_input(&mut self, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32, dt: f32) -> u32 {
        self.seq += 1;
        let _ = self.socket.send(&Packet::Input { seq: self.seq, movement, yaw, pitch, buttons, dt }.encode());
        self.seq
    }

    pub fn disconnect(&self) {
        let _ = self.socket.send(&Packet::Disconnect.encode());
    }

    /// Apply waiting packets. Snapshots older than the newest one seen are ignored (UDP may reorder).
    pub fn poll(&mut self) {
        let mut buf = [0u8; MAX_PACKET + 64];
        while let Ok(n) = self.socket.recv(&mut buf) {
            match Packet::decode(&buf[..n]) {
                Ok(Packet::Accepted { client_id }) => self.client_id = Some(client_id),
                Ok(Packet::Rpc { id, name, args }) => {
                    if self.inbox.acceptable(id) {
                        let _ = self.socket.send(&Packet::RpcAck { id }.encode());
                    }
                    self.rpcs.extend(self.inbox.accept(id, name, args));
                }
                Ok(Packet::RpcAck { id }) => self.out.ack(id),
                Ok(Packet::Snapshot { tick, ack, part, parts, entities }) if tick >= self.last_tick => {
                    self.last_tick = tick;
                    self.ack = self.ack.max(ack);
                    self.snapshots += 1;
                    self.received_entities += entities.len() as u64;
                    for e in entities {
                        self.entities.insert(e.id, e);
                    }
                    if self.partial.0 != tick {
                        self.partial = (tick, Default::default());
                    }
                    self.partial.1.insert(part);
                    // Acknowledge the tick once every part of it has arrived (repeats of the last part re-acknowledge).
                    if self.partial.1.len() >= parts as usize {
                        let _ = self.socket.send(&Packet::SnapshotAck { tick }.encode());
                    }
                }
                _ => {}
            }
        }
        for data in self.out.due() {
            let _ = self.socket.send(&data);
        }
    }

    /// Reliable call on the server. Fails when the call does not fit a packet.
    pub fn call(&mut self, name: &str, args: &[u8]) -> Result<()> {
        let data = self.out.push(name, args)?;
        let _ = self.socket.send(&data);
        Ok(())
    }

    /// Calls the server made on this client since the last time.
    pub fn take_rpcs(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.rpcs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(2) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn packets_round_trip() {
        let packets = [
            Packet::Connect { name: "Ünï".into() },
            Packet::Accepted { client_id: 7 },
            Packet::Input { seq: 9, movement: [1.0, -0.5], yaw: 90.0, pitch: -10.0, buttons: 3, dt: 0.016 },
            Packet::Snapshot { tick: 4, ack: 2, part: 0, parts: 1, entities: vec![EntityState { id: 1, position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0] }] },
            Packet::Disconnect,
        ];
        for p in packets {
            assert_eq!(Packet::decode(&p.encode()).unwrap(), p);
        }
    }

    #[test]
    fn prediction_replays_unacknowledged_inputs_on_the_server_state() {
        use super::replication::{Prediction, PLAYER_ID_BASE};
        use sbox_math::Vec3;
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        let own = PLAYER_ID_BASE + client.client_id.unwrap();
        let mut pred = Prediction::default();
        assert_eq!(pred.step(&mut client, [1.0, 0.0], 0.0, 0.0, 0, 0.1), None, "the server has not reported us yet");
        // The server has applied input 1 and put the player at x = 20 (not the 25 the client expected).
        client.entities.insert(own, EntityState { id: own, position: [20.0, 0.0, 5.0], rotation: [0.0, 0.0, 0.0, 1.0] });
        client.ack = 1;
        // Inputs 2 and 3 are still in flight: replayed on top of the corrected position.
        let p2 = pred.step(&mut client, [1.0, 0.0], 0.0, 0.0, 0, 0.1).unwrap();
        let p3 = pred.step(&mut client, [1.0, 0.0], 0.0, 0.0, 0, 0.1).unwrap();
        assert!((p2 - Vec3::new(45.0, 0.0, 5.0)).length() < 1e-3, "{p2:?}");
        assert!((p3 - Vec3::new(70.0, 0.0, 5.0)).length() < 1e-3, "{p3:?}");
        assert_eq!(pred.pending(), 2);
        // The server catches up to input 3: nothing left to replay.
        client.entities.insert(own, EntityState { id: own, position: [71.0, 0.0, 5.0], rotation: [0.0, 0.0, 0.0, 1.0] });
        client.ack = 3;
        let p = pred.position(&client).unwrap();
        assert!((p - Vec3::new(71.0, 0.0, 5.0)).length() < 1e-3);
        assert_eq!(pred.pending(), 0);
    }

    #[test]
    fn the_server_acknowledges_applied_inputs_in_snapshots() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        for _ in 0..3 {
            client.send_input([0.0, 0.0], 0.0, 0.0, 0, 0.016);
        }
        assert!(wait(|| {
            server.poll();
            server.acked.get(&1) == Some(&3)
        }));
        server.broadcast(&[]);
        assert!(wait(|| {
            client.poll();
            client.ack == 3
        }));
    }

    #[test]
    fn interpolation_blends_between_snapshots_a_little_in_the_past() {
        use super::replication::Interpolator;
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        let mut it = Interpolator::default();
        assert!(it.sample(7).is_none(), "nothing recorded");
        let at = |x: f32| EntityState { id: 7, position: [x, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] };
        // Snapshots at t = 0 (x = 0) and t = 0.1 (x = 100); `snapshots` marks each arrival.
        client.entities.insert(7, at(0.0));
        client.snapshots += 1;
        it.record(&client);
        it.advance(0.1);
        client.entities.insert(7, at(100.0));
        client.snapshots += 1;
        it.record(&client);
        // Now = 0.1, delay 0.1 -> shown at t = 0: the first snapshot.
        assert_eq!(it.sample(7).unwrap().0.x, 0.0);
        it.advance(0.05);
        let x = it.sample(7).unwrap().0.x;
        assert!((x - 50.0).abs() < 1e-3, "halfway between the snapshots, got {x}");
        it.advance(0.05);
        assert!((it.sample(7).unwrap().0.x - 100.0).abs() < 1e-3);
        // Past the newest snapshot it holds still instead of extrapolating.
        it.advance(1.0);
        assert_eq!(it.sample(7).unwrap().0.x, 100.0);
        // Recording without new snapshots changes nothing.
        it.record(&client);
        assert_eq!(it.sample(7).unwrap().0.x, 100.0);
    }

    #[test]
    fn calls_are_released_in_order() {
        let mut inbox = Inbox::default();
        let call = |n: &str| (n.to_string(), vec![]);
        assert!(inbox.accept(2, "b".into(), vec![]).is_empty(), "2 waits for 1");
        assert!(inbox.accept(3, "c".into(), vec![]).is_empty());
        assert_eq!(inbox.accept(1, "a".into(), vec![]), vec![call("a"), call("b"), call("c")]);
        assert!(inbox.accept(2, "b".into(), vec![]).is_empty(), "a retransmission is dropped");
        assert!(inbox.accept(1, "a".into(), vec![]).is_empty());
        assert_eq!(inbox.accept(4, "d".into(), vec![]), vec![call("d")]);
        // Far ahead of the window: neither held nor acknowledged.
        assert!(!inbox.acceptable(5 + RPC_WINDOW));
        assert!(inbox.accept(5 + RPC_WINDOW, "x".into(), vec![]).is_empty());
        assert!(inbox.held.is_empty());
        assert!(inbox.acceptable(4), "already delivered ids are still acknowledged");
    }

    #[test]
    fn snapshots_carry_only_what_changed_since_the_acknowledged_tick() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        let at = |id: u32, x: f32| EntityState { id, position: [x, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] };
        let mut world = vec![at(1, 0.0), at(2, 0.0), at(3, 0.0)];
        // Receive one broadcast and let the acknowledgement reach the server.
        let mut exchange = |server: &mut Server, client: &mut Client, world: &[EntityState]| -> u64 {
            let before = (client.received_entities, client.snapshots);
            server.broadcast(world);
            assert!(wait(|| {
                client.poll();
                client.snapshots > before.1
            }));
            // The client's SnapshotAck is read by the server's next poll.
            std::thread::sleep(Duration::from_millis(30));
            server.poll();
            client.received_entities - before.0
        };
        assert_eq!(exchange(&mut server, &mut client, &world), 3, "a new client gets everything");
        assert_eq!(exchange(&mut server, &mut client, &world), 0, "nothing changed");
        world[1] = at(2, 5.0);
        assert_eq!(exchange(&mut server, &mut client, &world), 1, "only the moved entity");
        assert_eq!(client.entities[&2].position[0], 5.0);
        assert_eq!(client.entities[&1].position[0], 0.0, "unchanged ones stay as they were");
        // A tick the client never acknowledged keeps its changes in the next delta: drop the acks by not polling the server.
        world[0] = at(1, 9.0);
        let before = client.received_entities;
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.received_entities > before
        }));
        // (the server has not read that ack yet) a second broadcast must still contain entity 1
        let before = client.received_entities;
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.received_entities > before
        }));
        assert_eq!(client.entities[&1].position[0], 9.0);
        assert!(client.received_entities - before >= 1, "the unacknowledged change was sent again");
    }

    #[test]
    fn a_delta_wider_than_one_packet_is_acknowledged_only_when_complete() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        let world: Vec<EntityState> = (0..100).map(|i| EntityState { id: i, position: [i as f32, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] }).collect();
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.entities.len() == 100
        }));
        assert!(client.snapshots >= 3, "100 entities need several packets, got {}", client.snapshots);
        std::thread::sleep(Duration::from_millis(30));
        server.poll();
        let before = client.received_entities;
        let snaps = client.snapshots;
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.snapshots > snaps
        }));
        assert_eq!(client.received_entities, before, "after the full tick was acknowledged nothing is resent");
    }

    #[test]
    fn rpc_packets_round_trip() {
        for p in [Packet::Rpc { id: 3, name: "chat".into(), args: vec![1, 2, 3] }, Packet::RpcAck { id: 3 }, Packet::Rpc { id: 1, name: String::new(), args: vec![] }] {
            assert_eq!(Packet::decode(&p.encode()).unwrap(), p);
        }
        assert!(Packet::decode(b"sb\x06\x01\x00\x00\x00\x02ab\xff\x03").is_err(), "arguments cut short");
    }

    #[test]
    fn rpcs_are_delivered_once_both_ways_and_acknowledged() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        client.call("chat", b"hello").unwrap();
        server.call(1, "kick", b"").unwrap();
        let mut got_server = Vec::new();
        let mut got_client = Vec::new();
        assert!(wait(|| {
            got_server.extend(server.poll());
            client.poll();
            got_client.extend(client.take_rpcs());
            !got_server.is_empty() && !got_client.is_empty()
        }));
        assert_eq!(got_server, vec![ServerEvent::Rpc { client_id: 1, name: "chat".into(), args: b"hello".to_vec() }]);
        assert_eq!(got_client, vec![("kick".to_string(), vec![])]);
        // Acknowledged: nothing left to resend.
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.out.pending.is_empty() && server.reliable[&1].0.pending.is_empty()
        }));
    }

    #[test]
    fn lost_calls_are_resent_and_duplicates_run_once() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        // Simulate a lost first transmission: register the call without sending it.
        client.out.push("save", b"x").unwrap();
        let mut events = Vec::new();
        assert!(wait(|| {
            events.extend(server.poll());
            client.poll();
            !events.is_empty()
        }), "the retransmission arrives");
        // Replaying the same call (a duplicate) must not run it again.
        let dup = Packet::Rpc { id: 1, name: "save".into(), args: b"x".to_vec() }.encode();
        client.socket.send(&dup).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        events.extend(server.poll());
        assert_eq!(events.len(), 1, "{events:?}");
    }

    #[test]
    fn oversized_rpcs_are_refused_and_strangers_cannot_call() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
        assert!(client.call("x", &[0; MAX_RPC_ARGS + 1]).is_err());
        assert!(client.call(&"n".repeat(MAX_RPC_NAME + 1), b"").is_err());
        assert!(server.call(99, "x", b"").is_err(), "unknown client");
        let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
        stranger.send_to(&Packet::Rpc { id: 1, name: "x".into(), args: vec![] }.encode(), server.local_addr().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(server.poll().iter().all(|e| !matches!(e, ServerEvent::Rpc { .. })));
    }

    #[test]
    fn malformed_packets_are_rejected() {
        assert!(Packet::decode(&[]).is_err());
        assert!(Packet::decode(b"xx\x05").is_err());
        assert!(Packet::decode(b"sb\x63").is_err());
        assert!(Packet::decode(b"sb\x03\x01").is_err(), "truncated input");
        // A snapshot claiming a billion entities must not allocate or panic.
        assert!(Packet::decode(b"sb\x04\x01\x00\x00\x00\xff\xff\xff\x3f").is_err());
    }

    #[test]
    fn client_connects_sends_input_and_receives_snapshots() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let mut client = Client::connect(&addr, "tester").unwrap();

        let mut events = Vec::new();
        assert!(wait(|| {
            events.extend(server.poll());
            client.poll();
            client.client_id.is_some()
        }));
        assert_eq!(events, vec![ServerEvent::Connected { client_id: 1, name: "tester".into() }]);

        client.send_input([1.0, 0.0], 45.0, 0.0, 1, 0.016);
        assert!(wait(|| {
            events.extend(server.poll());
            events.iter().any(|e| matches!(e, ServerEvent::Input { seq: 1, .. }))
        }));

        let world = vec![EntityState { id: 5, position: [10.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] }];
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.entities.contains_key(&5)
        }));
        assert_eq!(client.entities[&5].position, [10.0, 0.0, 0.0]);
        assert_eq!(client.last_tick, 1);

        client.disconnect();
        assert!(wait(|| {
            events.extend(server.poll());
            events.contains(&ServerEvent::Disconnected { client_id: 1 })
        }));
        assert_eq!(server.client_count(), 0);
    }

    #[test]
    fn unknown_senders_cannot_inject_input() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
        stranger.send_to(&Packet::Input { seq: 1, movement: [1.0, 1.0], yaw: 0.0, pitch: 0.0, buttons: 0, dt: 0.016 }.encode(), server.local_addr().unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(server.poll().is_empty());
    }

    #[test]
    fn large_worlds_are_split_across_packets() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap().to_string();
        let mut client = Client::connect(&addr, "c").unwrap();
        assert!(wait(|| {
            server.poll();
            client.poll();
            client.client_id.is_some()
        }));
        let world: Vec<_> = (0..200).map(|id| EntityState { id, position: [id as f32; 3], rotation: [0.0, 0.0, 0.0, 1.0] }).collect();
        server.broadcast(&world);
        assert!(wait(|| {
            client.poll();
            client.entities.len() == 200
        }));
    }
}

/// Scene replication: objects carrying a [`NetworkId`] have their world transform sent by the server and applied on the
/// client, which must have loaded the same scene (objects are matched by id, not spawned).
pub mod replication {
    use super::{Client, EntityState, ServerEvent};
    use sbox_math::{Angles, Quat, Transform, Vec3};
    use sbox_scene::{impl_component_any, Component, GoId, Scene};
    use std::collections::HashMap;

    /// Ids from here up belong to players (`PLAYER_ID_BASE + client id`); clients create a missing object for them.
    pub const PLAYER_ID_BASE: u32 = 1_000_000;
    /// Walking speed of a remote player in units per second.
    pub const PLAYER_SPEED: f32 = 250.0;

    /// One movement step of a player: `movement` is (forward, left) in -1..1, `yaw` in degrees. The server and the
    /// client's prediction both use this, so they agree exactly.
    pub fn move_step(position: Vec3, movement: [f32; 2], yaw: f32, dt: f32) -> Vec3 {
        let (sin, cos) = yaw.to_radians().sin_cos();
        let wish = Vec3::new(cos, sin, 0.0) * movement[0] + Vec3::new(-sin, cos, 0.0) * movement[1];
        position + wish.clamp_length_max(1.0) * PLAYER_SPEED * dt
    }

    /// Host-side objects of the connected clients. Every received input is applied once, kinematically: no collision,
    /// gravity or jumping, so players glide at a fixed height.
    #[derive(Default)]
    pub struct RemotePlayers {
        objects: HashMap<u32, GoId>,
    }

    impl RemotePlayers {
        /// Create / remove player objects and apply each client's inputs.
        pub fn handle(&mut self, scene: &mut Scene, event: &ServerEvent, spawn: Vec3) {
            match event {
                ServerEvent::Connected { client_id, name } => {
                    let go = scene.create_object(&format!("player {name}"));
                    scene.set_world_transform(go, Transform::from_position(spawn));
                    scene.add_component(go, NetworkId(PLAYER_ID_BASE + client_id));
                    self.objects.insert(*client_id, go);
                }
                ServerEvent::Input { client_id, movement, yaw, dt, .. } => {
                    if let Some(&go) = self.objects.get(client_id) {
                        let mut t = scene.world_transform(go);
                        // Clamp the frame time so a client cannot move faster by claiming long frames.
                        t.position = move_step(t.position, *movement, *yaw, dt.clamp(0.0, 0.25));
                        t.rotation = Angles::new(0.0, *yaw, 0.0).to_rotation();
                        scene.set_world_transform(go, t);
                    }
                }
                // Calls are for the game to interpret.
                ServerEvent::Rpc { .. } => {}
                ServerEvent::Disconnected { client_id } => {
                    if let Some(go) = self.objects.remove(client_id) {
                        scene.destroy_object(go);
                    }
                }
            }
        }

        pub fn object(&self, client_id: u32) -> Option<GoId> {
            self.objects.get(&client_id).copied()
        }
    }

    /// Client-side prediction of the local player: inputs are applied at once with [`move_step`] and, when the server
    /// reports a position, replayed on top of it (the server's answer wins for everything it has already applied).
    #[derive(Default)]
    pub struct Prediction {
        pending: std::collections::VecDeque<(u32, [f32; 2], f32, f32)>,
    }

    impl Prediction {
        /// Send an input and return where the local player is expected to be now. `None` until the server has
        /// reported the player once.
        pub fn step(&mut self, client: &mut Client, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32, dt: f32) -> Option<Vec3> {
            let seq = client.send_input(movement, yaw, pitch, buttons, dt);
            self.pending.push_back((seq, movement, yaw, dt.clamp(0.0, 0.25)));
            self.position(client)
        }

        /// Server state of the local player with the not yet acknowledged inputs replayed.
        pub fn position(&mut self, client: &Client) -> Option<Vec3> {
            while self.pending.front().is_some_and(|(seq, ..)| *seq <= client.ack) {
                self.pending.pop_front();
            }
            let own = client.entities.get(&(PLAYER_ID_BASE + client.client_id?))?;
            Some(self.pending.iter().fold(Vec3::from(own.position), |p, (_, m, yaw, dt)| move_step(p, *m, *yaw, *dt)))
        }

        pub fn pending(&self) -> usize {
            self.pending.len()
        }
    }

    /// Snapshot history for smooth remote motion: objects are shown `delay` seconds in the past, blended between the two
    /// snapshots around that moment. Time is the sum of the `dt`s passed to [`Interpolator::advance`].
    pub struct Interpolator {
        pub delay: f32,
        now: f32,
        seen: u32,
        samples: HashMap<u32, std::collections::VecDeque<(f32, Vec3, Quat)>>,
    }

    impl Default for Interpolator {
        fn default() -> Self {
            Self { delay: 0.1, now: 0.0, seen: 0, samples: HashMap::new() }
        }
    }

    impl Interpolator {
        pub fn advance(&mut self, dt: f32) {
            self.now += dt;
        }

        /// Remember the client's entity states when new snapshot packets have arrived since the last call.
        pub fn record(&mut self, client: &Client) {
            if client.snapshots == self.seen {
                return;
            }
            self.seen = client.snapshots;
            for e in client.entities.values() {
                let q = self.samples.entry(e.id).or_default();
                let (p, r) = (Vec3::from(e.position), Quat::from_array(e.rotation).normalize());
                // Several packets of one snapshot (or a repeat) may land in the same frame: keep the newest.
                if q.back().is_some_and(|(t, ..)| *t >= self.now) {
                    q.pop_back();
                }
                q.push_back((self.now, p, r));
            }
        }

        /// State of `id` at the delayed render time; before the first sample it is the first one, after the last one the
        /// last one (no extrapolation).
        pub fn sample(&mut self, id: u32) -> Option<(Vec3, Quat)> {
            let target = self.now - self.delay;
            let q = self.samples.get_mut(&id)?;
            // Drop samples that can no longer be needed, keeping the one at or before the target.
            while q.len() > 2 && q[1].0 <= target {
                q.pop_front();
            }
            let (t0, p0, r0) = *q.front()?;
            let Some(&(t1, p1, r1)) = q.get(1) else { return Some((p0, r0)) };
            if target <= t0 {
                return Some((p0, r0));
            }
            let k = ((target - t0) / (t1 - t0).max(1e-4)).clamp(0.0, 1.0);
            Some((p0.lerp(p1, k), r0.slerp(r1, k)))
        }
    }

    /// Move networked objects to their interpolated state (all but `skip`, the predicted local player).
    pub fn apply_interpolated(scene: &mut Scene, interp: &mut Interpolator, skip: Option<u32>) {
        // Objects for newly reported players are created by `apply_except`; this only moves existing ones.
        let ids: Vec<_> = scene.iter_components::<NetworkId>().filter(|(_, n)| Some(n.0) != skip).map(|(go, n)| (go, n.0)).collect();
        for (go, id) in ids {
            if let Some((p, r)) = interp.sample(id) {
                let mut t = scene.world_transform(go);
                t.position = p;
                t.rotation = r;
                scene.set_world_transform(go, t);
            }
        }
    }

    /// Marks an object as replicated; ids must be unique and identical on server and clients.
    pub struct NetworkId(pub u32);
    impl Component for NetworkId {
        impl_component_any!();
    }

    /// World transforms of every networked object.
    pub fn collect(scene: &Scene) -> Vec<EntityState> {
        scene
            .iter_components::<NetworkId>()
            .map(|(go, n)| {
                let t = scene.world_transform(go);
                EntityState { id: n.0, position: t.position.to_array(), rotation: t.rotation.to_array() }
            })
            .collect()
    }

    /// Move networked objects towards the client's latest state. `blend` is the fraction of the remaining distance covered
    /// (1.0 snaps; smaller values smooth between the snapshots). Ids the client has no state for are left alone.
    pub fn apply(scene: &mut Scene, client: &Client, blend: f32) {
        apply_except(scene, client, blend, None);
    }

    /// Like [`apply`], but leaves the object with id `skip` alone (the locally predicted player).
    pub fn apply_except(scene: &mut Scene, client: &Client, blend: f32, skip: Option<u32>) {
        // Players are not part of the loaded scene: create an object for each one the server reports.
        let known: Vec<u32> = scene.iter_components::<NetworkId>().map(|(_, n)| n.0).collect();
        let mut missing: Vec<&EntityState> = client.entities.values().filter(|e| e.id >= PLAYER_ID_BASE && !known.contains(&e.id)).collect();
        missing.sort_by_key(|e| e.id);
        for e in missing {
            let go = scene.create_object("remote player");
            scene.set_world_transform(go, Transform::from_position(Vec3::from(e.position)));
            scene.add_component(go, NetworkId(e.id));
        }
        let targets: Vec<_> = scene.iter_components::<NetworkId>().filter(|(_, n)| Some(n.0) != skip).filter_map(|(go, n)| client.entities.get(&n.0).map(|e| (go, e.clone()))).collect();
        for (go, e) in targets {
            let cur = scene.world_transform(go);
            let goal = Transform::new(Vec3::from(e.position), Quat::from_array(e.rotation).normalize(), cur.scale);
            scene.set_world_transform(go, Transform::lerp(&cur, &goal, blend.clamp(0.0, 1.0)));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::Server;

        #[test]
        fn players_follow_their_input_and_appear_on_other_clients() {
            let mut scene = Scene::new();
            let mut players = RemotePlayers::default();
            players.handle(&mut scene, &ServerEvent::Connected { client_id: 2, name: "bob".into() }, Vec3::new(0.0, 0.0, 10.0));
            let go = players.object(2).unwrap();
            // Walk forward along +Y (yaw 90 degrees) for one second, in ten inputs.
            players.handle(&mut scene, &ServerEvent::Input { client_id: 2, seq: 1, movement: [1.0, 0.0], yaw: 90.0, pitch: 0.0, buttons: 0, dt: 0.1 }, Vec3::ZERO);
            for _ in 0..9 {
                players.handle(&mut scene, &ServerEvent::Input { client_id: 2, seq: 1, movement: [1.0, 0.0], yaw: 90.0, pitch: 0.0, buttons: 0, dt: 0.1 }, Vec3::ZERO);
            }
            let p = scene.world_transform(go).position;
            assert!((p - Vec3::new(0.0, PLAYER_SPEED, 10.0)).length() < 0.01, "{p:?}");
            // Diagonal input is not faster than straight input.
            players.handle(&mut scene, &ServerEvent::Input { client_id: 2, seq: 2, movement: [1.0, 1.0], yaw: 0.0, pitch: 0.0, buttons: 0, dt: 0.25 }, Vec3::ZERO);
            let q = scene.world_transform(go).position;
            assert!(((q - p).length() - PLAYER_SPEED * 0.25).abs() < 0.01);
            // A claimed frame time above 0.25 s is clamped, so long frames do not speed a player up.
            players.handle(&mut scene, &ServerEvent::Input { client_id: 2, seq: 3, movement: [1.0, 0.0], yaw: 0.0, pitch: 0.0, buttons: 0, dt: 100.0 }, Vec3::ZERO);
            let q2 = scene.world_transform(go).position;
            assert!(((q2 - q).length() - PLAYER_SPEED * 0.25).abs() < 0.01);
            let q = q2;

            // A client that never loaded the player gets an object for it.
            let mut server = Server::bind("127.0.0.1:0").unwrap();
            let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "viewer").unwrap();
            for _ in 0..200 {
                server.poll();
                client.poll();
                if client.client_id.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            server.broadcast(&collect(&scene));
            for _ in 0..200 {
                client.poll();
                if !client.entities.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let mut client_scene = Scene::new();
            apply(&mut client_scene, &client, 1.0);
            let (go, _) = client_scene.iter_components::<NetworkId>().next().expect("player object created");
            assert!((client_scene.world_transform(go).position - q).length() < 1e-3);

            players.handle(&mut scene, &ServerEvent::Disconnected { client_id: 2 }, Vec3::ZERO);
            scene.tick(0.0); // destruction is queued until the next tick
            assert!(collect(&scene).is_empty(), "removed with the client");
        }

        #[test]
        fn server_scene_state_reaches_the_client_scene() {
            let mut server_scene = Scene::new();
            let a = server_scene.create_object("a");
            server_scene.add_component(a, NetworkId(1));
            server_scene.set_world_transform(a, Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
            let local_only = server_scene.create_object("local");
            server_scene.set_world_transform(local_only, Transform::from_position(Vec3::X));

            let mut client_scene = Scene::new();
            let ca = client_scene.create_object("a");
            client_scene.add_component(ca, NetworkId(1));

            let mut server = Server::bind("127.0.0.1:0").unwrap();
            let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
            for _ in 0..200 {
                server.poll();
                client.poll();
                if client.client_id.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let state = collect(&server_scene);
            assert_eq!(state.len(), 1, "only networked objects are replicated");
            server.broadcast(&state);
            for _ in 0..200 {
                client.poll();
                if !client.entities.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            apply(&mut client_scene, &client, 0.5);
            assert!((client_scene.world_transform(ca).position - Vec3::new(0.5, 1.0, 1.5)).length() < 1e-4, "half way");
            apply(&mut client_scene, &client, 1.0);
            assert!((client_scene.world_transform(ca).position - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-4);
        }
    }
}

/// Typed RPC arguments and a registry that dispatches calls to handlers by name, checking the argument types first.
/// The wire format is a tag byte per value; the whole list must fit [`MAX_RPC_ARGS`].
pub mod rpc {
    use super::{Client, Server, MAX_RPC_ARGS};
    use anyhow::{bail, Result};
    use std::collections::HashMap;

    #[derive(Clone, Debug, PartialEq)]
    pub enum Arg {
        Bool(bool),
        Int(i64),
        Float(f32),
        Str(String),
        Vec3([f32; 3]),
    }

    /// The type of an [`Arg`], for declaring handler signatures.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Bool,
        Int,
        Float,
        Str,
        Vec3,
    }

    impl Arg {
        pub fn kind(&self) -> Kind {
            match self {
                Arg::Bool(_) => Kind::Bool,
                Arg::Int(_) => Kind::Int,
                Arg::Float(_) => Kind::Float,
                Arg::Str(_) => Kind::Str,
                Arg::Vec3(_) => Kind::Vec3,
            }
        }
    }

    pub fn encode(args: &[Arg]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for a in args {
            match a {
                Arg::Bool(b) => out.extend([0, *b as u8]),
                Arg::Int(i) => {
                    out.push(1);
                    out.extend(i.to_le_bytes());
                }
                Arg::Float(f) => {
                    out.push(2);
                    out.extend(f.to_le_bytes());
                }
                Arg::Str(s) => {
                    if s.len() > u16::MAX as usize {
                        bail!("string argument too long");
                    }
                    out.push(3);
                    out.extend((s.len() as u16).to_le_bytes());
                    out.extend(s.as_bytes());
                }
                Arg::Vec3(v) => {
                    out.push(4);
                    for c in v {
                        out.extend(c.to_le_bytes());
                    }
                }
            }
        }
        if out.len() > MAX_RPC_ARGS {
            bail!("arguments take {} bytes, the limit is {MAX_RPC_ARGS}", out.len());
        }
        Ok(out)
    }

    pub fn decode(mut data: &[u8]) -> Result<Vec<Arg>> {
        fn take<'a>(d: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
            if d.len() < n {
                bail!("truncated arguments");
            }
            let (a, b) = d.split_at(n);
            *d = b;
            Ok(a)
        }
        let mut out = Vec::new();
        while !data.is_empty() {
            let tag = take(&mut data, 1)?[0];
            out.push(match tag {
                0 => Arg::Bool(take(&mut data, 1)?[0] != 0),
                1 => Arg::Int(i64::from_le_bytes(take(&mut data, 8)?.try_into()?)),
                2 => Arg::Float(f32::from_le_bytes(take(&mut data, 4)?.try_into()?)),
                3 => {
                    let n = u16::from_le_bytes(take(&mut data, 2)?.try_into()?) as usize;
                    Arg::Str(String::from_utf8_lossy(take(&mut data, n)?).into_owned())
                }
                4 => {
                    let mut v = [0.0; 3];
                    for c in &mut v {
                        *c = f32::from_le_bytes(take(&mut data, 4)?.try_into()?);
                    }
                    Arg::Vec3(v)
                }
                t => bail!("unknown argument tag {t}"),
            });
        }
        Ok(out)
    }

    impl Client {
        /// Typed reliable call on the server.
        pub fn call_typed(&mut self, name: &str, args: &[Arg]) -> Result<()> {
            self.call(name, &encode(args)?)
        }
    }

    impl Server {
        /// Typed reliable call on a client.
        pub fn call_typed(&mut self, client_id: u32, name: &str, args: &[Arg]) -> Result<()> {
            self.call(client_id, name, &encode(args)?)
        }
    }

    type Handler<C> = Box<dyn FnMut(&mut C, &[Arg])>;

    /// Handlers by RPC name; `C` is whatever context they act on (the scene, the game state, ...).
    pub struct Registry<C> {
        handlers: HashMap<String, (Vec<Kind>, Handler<C>)>,
    }

    impl<C> Default for Registry<C> {
        fn default() -> Self {
            Self { handlers: HashMap::new() }
        }
    }

    impl<C> Registry<C> {
        /// Declare `name` with its argument types; a later registration of the same name replaces the earlier one.
        pub fn register(&mut self, name: &str, signature: &[Kind], handler: impl FnMut(&mut C, &[Arg]) + 'static) {
            self.handlers.insert(name.to_string(), (signature.to_vec(), Box::new(handler)));
        }

        /// Run the handler for a received call. Unknown names, undecodable arguments and arguments that do not match
        /// the signature are rejected without running anything (remote input is untrusted).
        pub fn dispatch(&mut self, ctx: &mut C, name: &str, args: &[u8]) -> Result<()> {
            let Some((signature, handler)) = self.handlers.get_mut(name) else { bail!("no rpc named {name:?}") };
            let args = decode(args)?;
            let got: Vec<Kind> = args.iter().map(Arg::kind).collect();
            if got != *signature {
                bail!("rpc {name:?} takes {signature:?}, got {got:?}");
            }
            handler(ctx, &args);
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn arguments_round_trip() {
            let args = vec![Arg::Bool(true), Arg::Int(-5_000_000_000), Arg::Float(1.5), Arg::Str("héllo".into()), Arg::Vec3([1.0, -2.0, 3.5]), Arg::Str(String::new())];
            assert_eq!(decode(&encode(&args).unwrap()).unwrap(), args);
            assert_eq!(decode(&[]).unwrap(), vec![]);
        }

        #[test]
        fn bad_arguments_are_errors() {
            assert!(decode(&[9]).is_err(), "unknown tag");
            assert!(decode(&[1, 0, 0]).is_err(), "truncated int");
            assert!(decode(&[3, 5, 0, b'a']).is_err(), "string shorter than its length");
            assert!(encode(&[Arg::Str("x".repeat(MAX_RPC_ARGS))]).is_err(), "too big for one packet");
        }

        #[test]
        fn the_registry_checks_names_and_signatures_before_running() {
            let mut log: Vec<String> = Vec::new();
            let mut reg = Registry::<Vec<String>>::default();
            reg.register("say", &[Kind::Str, Kind::Int], |log, a| {
                if let [Arg::Str(s), Arg::Int(n)] = a {
                    log.push(format!("{s}x{n}"));
                }
            });
            reg.dispatch(&mut log, "say", &encode(&[Arg::Str("hi".into()), Arg::Int(3)]).unwrap()).unwrap();
            assert_eq!(log, vec!["hix3"]);
            assert!(reg.dispatch(&mut log, "nope", &[]).is_err());
            assert!(reg.dispatch(&mut log, "say", &encode(&[Arg::Int(3), Arg::Str("hi".into())]).unwrap()).is_err(), "wrong order");
            assert!(reg.dispatch(&mut log, "say", &encode(&[Arg::Str("hi".into())]).unwrap()).is_err(), "missing argument");
            assert!(reg.dispatch(&mut log, "say", &[200]).is_err(), "garbage");
            assert_eq!(log.len(), 1, "rejected calls ran nothing");
        }

        #[test]
        fn typed_calls_travel_both_ways_to_registered_handlers() {
            use crate::ServerEvent;
            let mut server = Server::bind("127.0.0.1:0").unwrap();
            let mut client = Client::connect(&server.local_addr().unwrap().to_string(), "c").unwrap();
            let t = std::time::Instant::now();
            let mut id = None;
            while id.is_none() && t.elapsed().as_secs() < 2 {
                for e in server.poll() {
                    if let ServerEvent::Connected { client_id, .. } = e {
                        id = Some(client_id);
                    }
                }
                client.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let id = id.expect("connected");
            client.call_typed("teleport", &[Arg::Vec3([1.0, 2.0, 3.0]), Arg::Bool(true)]).unwrap();
            server.call_typed(id, "notice", &[Arg::Str("hi".into())]).unwrap();
            let mut on_server = Registry::<Vec<String>>::default();
            on_server.register("teleport", &[Kind::Vec3, Kind::Bool], |log, a| log.push(format!("{a:?}")));
            let mut on_client = Registry::<Vec<String>>::default();
            on_client.register("notice", &[Kind::Str], |log, a| log.push(format!("{a:?}")));
            let (mut s_log, mut c_log) = (Vec::new(), Vec::new());
            let t = std::time::Instant::now();
            while (s_log.is_empty() || c_log.is_empty()) && t.elapsed().as_secs() < 2 {
                for e in server.poll() {
                    if let ServerEvent::Rpc { name, args, .. } = e {
                        on_server.dispatch(&mut s_log, &name, &args).unwrap();
                    }
                }
                client.poll();
                for (name, args) in client.take_rpcs() {
                    on_client.dispatch(&mut c_log, &name, &args).unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(s_log, vec!["[Vec3([1.0, 2.0, 3.0]), Bool(true)]"]);
            assert_eq!(c_log, vec!["[Str(\"hi\")]"]);
        }
    }
}
