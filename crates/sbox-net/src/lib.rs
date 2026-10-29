//! Authoritative client-server networking over UDP. The server owns the world and sends transform snapshots; clients
//! send their name and input. This is a small protocol of our own, not s&box's (whose networking is closed C# / Steam
//! Networking). Packets are tiny and unreliable except for the connect handshake, which the client retries.

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};

const MAGIC: [u8; 2] = *b"sb";
const MAX_PACKET: usize = 1400;
/// Bytes one entity takes in a snapshot: id + 3 position + 4 rotation floats.
const ENTITY_BYTES: usize = 32;

/// State of one networked object in a snapshot.
#[derive(Clone, Debug, PartialEq)]
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
    Input { seq: u32, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32 },
    Snapshot { tick: u32, entities: Vec<EntityState> },
    Disconnect,
}

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
            Packet::Input { seq, movement, yaw, pitch, buttons } => {
                w.0.push(3);
                w.u32(*seq);
                for v in [movement[0], movement[1], *yaw, *pitch] {
                    w.f32(v);
                }
                w.u32(*buttons);
            }
            Packet::Snapshot { tick, entities } => {
                w.0.push(4);
                w.u32(*tick);
                w.u32(entities.len() as u32);
                for e in entities {
                    w.u32(e.id);
                    for v in e.position.iter().chain(&e.rotation) {
                        w.f32(*v);
                    }
                }
            }
            Packet::Disconnect => w.0.push(5),
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
            3 => Packet::Input { seq: r.u32()?, movement: [r.f32()?, r.f32()?], yaw: r.f32()?, pitch: r.f32()?, buttons: r.u32()? },
            4 => {
                let tick = r.u32()?;
                let n = r.u32()? as usize;
                // Reject counts the packet cannot hold before allocating.
                if n > MAX_PACKET / ENTITY_BYTES {
                    bail!("too many entities")
                }
                let mut entities = Vec::with_capacity(n);
                for _ in 0..n {
                    entities.push(EntityState { id: r.u32()?, position: [r.f32()?, r.f32()?, r.f32()?], rotation: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] });
                }
                Packet::Snapshot { tick, entities }
            }
            5 => Packet::Disconnect,
            t => bail!("unknown packet type {t}"),
        })
    }
}

/// Events the server reports from [`Server::poll`].
#[derive(Clone, Debug, PartialEq)]
pub enum ServerEvent {
    Connected { client_id: u32, name: String },
    Input { client_id: u32, seq: u32, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32 },
    Disconnected { client_id: u32 },
}

pub struct Server {
    socket: UdpSocket,
    clients: HashMap<SocketAddr, u32>,
    next_id: u32,
    pub tick: u32,
}

impl Server {
    pub fn bind(addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, clients: HashMap::new(), next_id: 1, tick: 0 })
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
                            events.push(ServerEvent::Connected { client_id: id, name });
                            id
                        }
                    };
                    let _ = self.socket.send_to(&Packet::Accepted { client_id: id }.encode(), from);
                }
                Packet::Input { seq, movement, yaw, pitch, buttons } => {
                    if let Some(&client_id) = self.clients.get(&from) {
                        events.push(ServerEvent::Input { client_id, seq, movement, yaw, pitch, buttons });
                    }
                }
                Packet::Disconnect => {
                    if let Some(client_id) = self.clients.remove(&from) {
                        events.push(ServerEvent::Disconnected { client_id });
                    }
                }
                _ => {}
            }
        }
        events
    }

    /// Send the world state to every client, split into packets that fit the MTU. Advances the tick.
    pub fn broadcast(&mut self, entities: &[EntityState]) {
        self.tick += 1;
        let per_packet = MAX_PACKET / ENTITY_BYTES - 1;
        let chunks: Vec<&[EntityState]> = if entities.is_empty() { vec![&[]] } else { entities.chunks(per_packet).collect() };
        for chunk in chunks {
            let data = Packet::Snapshot { tick: self.tick, entities: chunk.to_vec() }.encode();
            for addr in self.clients.keys() {
                let _ = self.socket.send_to(&data, addr);
            }
        }
    }
}

pub struct Client {
    socket: UdpSocket,
    pub client_id: Option<u32>,
    pub last_tick: u32,
    /// Latest known state of every entity, by id.
    pub entities: HashMap<u32, EntityState>,
    seq: u32,
}

impl Client {
    pub fn connect(server: &str, name: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(server)?;
        socket.set_nonblocking(true)?;
        socket.send(&Packet::Connect { name: name.to_string() }.encode())?;
        Ok(Self { socket, client_id: None, last_tick: 0, entities: HashMap::new(), seq: 0 })
    }

    /// Re-send the handshake (call periodically until [`Client::client_id`] is set).
    pub fn retry_connect(&self, name: &str) {
        let _ = self.socket.send(&Packet::Connect { name: name.to_string() }.encode());
    }

    pub fn send_input(&mut self, movement: [f32; 2], yaw: f32, pitch: f32, buttons: u32) {
        self.seq += 1;
        let _ = self.socket.send(&Packet::Input { seq: self.seq, movement, yaw, pitch, buttons }.encode());
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
                Ok(Packet::Snapshot { tick, entities }) if tick >= self.last_tick => {
                    self.last_tick = tick;
                    for e in entities {
                        self.entities.insert(e.id, e);
                    }
                }
                _ => {}
            }
        }
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
            Packet::Input { seq: 9, movement: [1.0, -0.5], yaw: 90.0, pitch: -10.0, buttons: 3 },
            Packet::Snapshot { tick: 4, entities: vec![EntityState { id: 1, position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0] }] },
            Packet::Disconnect,
        ];
        for p in packets {
            assert_eq!(Packet::decode(&p.encode()).unwrap(), p);
        }
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

        client.send_input([1.0, 0.0], 45.0, 0.0, 1);
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
        stranger.send_to(&Packet::Input { seq: 1, movement: [1.0, 1.0], yaw: 0.0, pitch: 0.0, buttons: 0 }.encode(), server.local_addr().unwrap()).unwrap();
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
