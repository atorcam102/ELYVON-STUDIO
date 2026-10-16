//! Native MIDI 1.0 ports. Only this non-RT owner calls OS send/connect APIs.
use super::{Packet, ScheduledPacket};
use elyvon_project::{TrackId, TrackIo};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use rtrb::{Consumer, RingBuffer};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Instant;
use ts_rs::TS;
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiPort {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiPortList {
    pub inputs: Vec<MidiPort>,
    pub outputs: Vec<MidiPort>,
    pub error: Option<String>,
    pub virtual_ports: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub index: usize,
    pub track: TrackId,
    pub io: TrackIo,
}
#[derive(Debug, Clone, Copy)]
pub struct Incoming {
    pub port: usize,
    pub when: Instant,
    pub native_us: u64,
    pub packet: Packet,
}
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiObservation {
    pub direction: String,
    pub port: String,
    pub track: Option<TrackId>,
    pub bytes: Vec<u8>,
    #[ts(type = "number")]
    pub timestamp_us: u64,
    #[ts(type = "number | null")]
    pub native_timestamp_us: Option<u64>,
}
struct Input {
    _connection: MidiInputConnection<()>,
    rx: Consumer<Incoming>,
}
struct Output {
    connection: MidiOutputConnection,
    notes: Box<[[u8; 128]; 16]>,
}
#[derive(Debug, Default)]
pub struct PortHealth {
    pub overflow: AtomicBool,
    pub invalid: AtomicU64,
}
/// Deliberately not Debug: native connection types contain platform handles.
pub struct Ports {
    pub list: MidiPortList,
    pub routes: Vec<Route>,
    inputs: Vec<Input>,
    outputs: HashMap<String, Output>,
    pub monitor: VecDeque<MidiObservation>,
    pub health: Arc<PortHealth>,
    epoch: Instant,
    pending: Vec<(u64, ScheduledPacket)>,
    serial: u64,
    pub max_jitter_us: u64,
    pub errors: VecDeque<String>,
    last_inventory: Instant,
}
impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports")
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}
impl Ports {
    pub fn new() -> Self {
        Self {
            list: enumerate(),
            routes: vec![],
            inputs: vec![],
            outputs: HashMap::new(),
            monitor: VecDeque::new(),
            health: Arc::new(PortHealth::default()),
            epoch: Instant::now(),
            pending: vec![],
            serial: 0,
            max_jitter_us: 0,
            errors: VecDeque::new(),
            last_inventory: Instant::now(),
        }
    }
    pub fn error(&mut self, message: String) {
        if self.errors.back() != Some(&message) {
            if self.errors.len() == 16 {
                self.errors.pop_front();
            }
            self.errors.push_back(message);
        }
    }
    pub fn observe(
        &mut self,
        direction: &str,
        port: &str,
        track: Option<TrackId>,
        packet: Packet,
        when: Instant,
    ) {
        if self.monitor.len() == 128 {
            self.monitor.pop_front();
        }
        self.monitor.push_back(MidiObservation {
            direction: direction.into(),
            port: port.into(),
            track,
            bytes: packet.bytes().to_vec(),
            timestamp_us: when.saturating_duration_since(self.epoch).as_micros() as u64,
            native_timestamp_us: None,
        });
    }
    pub fn configure(&mut self, routes: Vec<Route>, force: bool) {
        if !force && self.routes == routes {
            return;
        }
        self.panic();
        self.inputs.clear();
        self.outputs.clear();
        self.routes = routes;
        self.list = enumerate();
        if let Some(e) = &self.list.error {
            self.error(e.clone());
        }
        let selected: Vec<String> = self
            .list
            .inputs
            .iter()
            .filter(|p| {
                self.routes.iter().any(|r| {
                    r.io.midi_input
                        .as_ref()
                        .map_or(!p.id.starts_with("virtual:"), |wanted| wanted == &p.id)
                        && (r.io.armed || r.io.midi_monitor)
                })
            })
            .map(|p| p.id.clone())
            .collect();
        if selected.len() > 16 {
            self.error("More than 16 MIDI inputs selected; narrow the input routing.".into());
            return;
        }
        for (index, port) in self
            .list
            .inputs
            .clone()
            .iter()
            .enumerate()
            .filter(|(_, p)| selected.contains(&p.id))
        {
            let (tx, rx) = RingBuffer::new(1024);
            let health = Arc::clone(&self.health);
            match open_input(&port.id, index, tx, health) {
                Ok(c) => self.inputs.push(Input { _connection: c, rx }),
                Err(e) => self.error(e),
            }
        }
        let outputs: Vec<String> = self
            .routes
            .iter()
            .filter_map(|r| r.io.midi_output.clone())
            .collect();
        for wanted in outputs {
            if self.outputs.contains_key(&wanted) {
                continue;
            }
            match open_output(&wanted) {
                Ok(connection) => {
                    self.outputs.insert(
                        wanted,
                        Output {
                            connection,
                            notes: Box::new([[0; 128]; 16]),
                        },
                    );
                }
                Err(e) => self.error(e),
            }
        }
        self.send_panic(); // clear device state on a newly connected/recovered port
        for route in self.routes.clone() {
            if let Some(w) = &route.io.midi_input
                && !self.list.inputs.iter().any(|p| &p.id == w)
            {
                self.error(format!(
                    "MIDI input {w} is absent; routing was preserved without fallback."
                ));
            }
        }
    }
    pub fn incoming(&mut self) -> Vec<Incoming> {
        let mut events = Vec::new();
        for input in &mut self.inputs {
            while let Ok(event) = input.rx.pop() {
                events.push(event);
            }
        }
        events.sort_by_key(|e| e.when);
        if self.health.overflow.swap(false, Ordering::AcqRel) {
            self.error("MIDI input queue overflow: panic sent, capture may be incomplete.".into());
            self.panic();
        }
        events
    }
    pub fn matching(&self, event: &Incoming) -> Vec<Route> {
        let port = self.list.inputs.get(event.port).map(|p| p.id.as_str());
        self.routes
            .iter()
            .filter(|r| {
                r.io.midi_input.as_deref().is_none_or(|p| Some(p) == port)
                    && r.io
                        .midi_channel
                        .is_none_or(|ch| event.packet.channel() == Some(ch))
            })
            .cloned()
            .collect()
    }
    pub fn send(
        &mut self,
        track: TrackId,
        mut packet: Packet,
        direction: &str,
        when: Instant,
    ) -> Result<(), String> {
        let route = self
            .routes
            .iter()
            .find(|r| r.track == track)
            .cloned()
            .ok_or("MIDI track is unavailable")?;
        if let Some(channel) = route.io.output_channel
            && packet.channel().is_some()
        {
            packet.data[0] = (packet.data[0] & 0xf0) | channel;
        }
        let Some(port) = route.io.midi_output else {
            self.observe("monitor", direction, Some(track), packet, when);
            return Ok(());
        };
        let Some(output) = self.outputs.get_mut(&port) else {
            return Err(format!(
                "MIDI output {port} is disconnected; choose Refresh ports to reconnect."
            ));
        };
        if let Err(e) = output.connection.send(packet.bytes()) {
            self.outputs.remove(&port);
            return Err(format!(
                "MIDI output {port} failed: {e}; note-off delivery cannot be guaranteed to an unplugged device."
            ));
        }
        if let Some(ch) = packet.channel() {
            let ch = usize::from(ch);
            let key = usize::from(packet.data[1]);
            match packet.data[0] & 0xf0 {
                0x90 if packet.data[2] > 0 => {
                    output.notes[ch][key] = output.notes[ch][key].saturating_add(1)
                }
                0x80 | 0x90 => output.notes[ch][key] = output.notes[ch][key].saturating_sub(1),
                0xb0 if matches!(packet.data[1], 120 | 123) => output.notes[ch].fill(0),
                _ => {}
            }
        }
        self.observe("output", &port, Some(track), packet, when);
        Ok(())
    }
    pub fn schedule(&mut self, packets: Vec<ScheduledPacket>, overflow: bool) {
        if overflow || self.pending.len() + packets.len() > 8192 {
            self.error(
                "MIDI output scheduler overflow: pending events discarded and panic sent.".into(),
            );
            self.panic();
            return;
        }
        for p in packets {
            self.serial = self.serial.wrapping_add(1);
            self.pending.push((self.serial, p));
        }
        self.pending
            .sort_by_key(|(serial, p)| (p.deadline, *serial));
    }
    pub fn tick(&mut self) {
        let now = Instant::now();
        if self.list.error.is_none()
            && self.last_inventory.elapsed() >= std::time::Duration::from_secs(2)
        {
            self.last_inventory = now;
            let inventory = enumerate();
            let old: Vec<_> = self
                .list
                .inputs
                .iter()
                .chain(&self.list.outputs)
                .map(|p| &p.id)
                .collect();
            let new: Vec<_> = inventory
                .inputs
                .iter()
                .chain(&inventory.outputs)
                .map(|p| &p.id)
                .collect();
            if old != new || inventory.error.is_some() {
                self.panic();
                self.inputs.clear();
                self.outputs.clear();
                self.list = inventory;
                self.error("MIDI port availability changed. Notes were released; Refresh ports to reconnect saved routing.".into());
            }
        }
        let ready = self.pending.partition_point(|(_, p)| p.deadline <= now);
        let events: Vec<_> = self.pending.drain(..ready).map(|(_, p)| p).collect();
        for event in events {
            self.max_jitter_us = self
                .max_jitter_us
                .max(now.saturating_duration_since(event.deadline).as_micros() as u64);
            if event.packet.len == 0 {
                self.send_panic();
                continue;
            }
            if let Some(track) = self
                .routes
                .iter()
                .find(|r| r.index == event.track)
                .map(|r| r.track)
                && let Err(e) = self.send(track, event.packet, "timeline", now)
            {
                self.error(e);
            }
        }
    }
    fn send_panic(&mut self) {
        let mut errors = Vec::new();
        for (name, output) in &mut self.outputs {
            for channel in 0u8..16 {
                for pitch in 0u8..128 {
                    let count = output.notes[channel as usize][pitch as usize];
                    for _ in 0..count {
                        if let Err(e) = output.connection.send(&[0x80 | channel, pitch, 0]) {
                            errors.push(format!("panic on {name}: {e}"));
                            break;
                        }
                    }
                }
                for controller in [64, 120, 123] {
                    if let Err(e) = output.connection.send(&[0xb0 | channel, controller, 0]) {
                        errors.push(format!("panic on {name}: {e}"));
                    }
                }
            }
            output.notes.fill([0; 128]);
        }
        for e in errors {
            self.error(e);
        }
    }
    pub fn panic(&mut self) {
        self.pending.clear();
        self.send_panic();
    }
}
impl Default for Ports {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Ports {
    fn drop(&mut self) {
        self.panic();
    }
}
fn names(names: Vec<String>, direction: &str) -> Vec<MidiPort> {
    let mut count = HashMap::<String, usize>::new();
    names
        .into_iter()
        .map(|name| {
            let n = count.entry(name.clone()).or_default();
            let id = format!("{direction}:{name}#{}", *n);
            *n += 1;
            MidiPort { id, name }
        })
        .collect()
}
fn input_ports(input: &MidiInput) -> Vec<midir::MidiInputPort> {
    input
        .ports()
        .into_iter()
        .filter(|p| {
            !input
                .port_name(p)
                .is_ok_and(|n| n.contains("Elyvon Studio virtual"))
        })
        .collect()
}
fn output_ports(output: &MidiOutput) -> Vec<midir::MidiOutputPort> {
    output
        .ports()
        .into_iter()
        .filter(|p| {
            !output
                .port_name(p)
                .is_ok_and(|n| n.contains("Elyvon Studio virtual"))
        })
        .collect()
}
pub fn enumerate() -> MidiPortList {
    let result = (|| -> Result<MidiPortList, String> {
        let input = MidiInput::new("Elyvon Studio input inventory").map_err(|e| e.to_string())?;
        let output =
            MidiOutput::new("Elyvon Studio output inventory").map_err(|e| e.to_string())?;
        let mut inputs = names(
            input_ports(&input)
                .iter()
                .map(|p| {
                    input
                        .port_name(p)
                        .unwrap_or_else(|_| "Unavailable MIDI input".into())
                })
                .collect(),
            "input",
        );
        let mut outputs = names(
            output_ports(&output)
                .iter()
                .map(|p| {
                    output
                        .port_name(p)
                        .unwrap_or_else(|_| "Unavailable MIDI output".into())
                })
                .collect(),
            "output",
        );
        let virtual_ports = cfg!(any(target_os = "linux", target_os = "macos"));
        if virtual_ports {
            inputs.push(MidiPort {
                id: "virtual:elyvon-in".into(),
                name: "Elyvon Studio virtual input".into(),
            });
            outputs.push(MidiPort {
                id: "virtual:elyvon-out".into(),
                name: "Elyvon Studio virtual output".into(),
            });
        }
        Ok(MidiPortList {
            inputs,
            outputs,
            error: None,
            virtual_ports,
        })
    })();
    result.unwrap_or_else(|e| MidiPortList {
        error: Some(format!("Native MIDI ports are unavailable: {e}")),
        ..MidiPortList::default()
    })
}
fn open_output(wanted: &str) -> Result<MidiOutputConnection, String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if wanted == "virtual:elyvon-out" {
        use midir::os::unix::VirtualOutput;
        return MidiOutput::new("Elyvon Studio virtual output")
            .map_err(|e| e.to_string())?
            .create_virtual("Elyvon Studio virtual output")
            .map_err(|e| e.to_string());
    }
    let output = MidiOutput::new("Elyvon Studio output").map_err(|e| e.to_string())?;
    let ports = output_ports(&output);
    let list = names(
        ports
            .iter()
            .map(|p| output.port_name(p).unwrap_or_default())
            .collect(),
        "output",
    );
    let index = list
        .iter()
        .position(|p| p.id == wanted)
        .ok_or_else(|| format!("MIDI output {wanted} is absent"))?;
    output
        .connect(&ports[index], "Elyvon Studio playback")
        .map_err(|e| e.to_string())
}
fn open_input(
    wanted: &str,
    index: usize,
    mut tx: rtrb::Producer<Incoming>,
    health: Arc<PortHealth>,
) -> Result<MidiInputConnection<()>, String> {
    let mut origin: Option<(u64, Instant)> = None;
    let callback = move |timestamp: u64, bytes: &[u8], _: &mut ()| {
        if let Some(packet) = Packet::parse(bytes) {
            let now = Instant::now();
            let (base, clock) = *origin.get_or_insert((timestamp, now));
            let when = timestamp
                .checked_sub(base)
                .and_then(|us| clock.checked_add(std::time::Duration::from_micros(us)))
                .filter(|t| *t <= now + std::time::Duration::from_millis(2))
                .unwrap_or_else(|| {
                    origin = Some((timestamp, now));
                    now
                });
            if tx
                .push(Incoming {
                    port: index,
                    when,
                    native_us: timestamp,
                    packet,
                })
                .is_err()
            {
                health.overflow.store(true, Ordering::Release);
            }
        } else {
            health.invalid.fetch_add(1, Ordering::Relaxed);
        }
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if wanted == "virtual:elyvon-in" {
        use midir::os::unix::VirtualInput;
        let mut input = MidiInput::new("Elyvon Studio virtual input").map_err(|e| e.to_string())?;
        input.ignore(Ignore::None);
        return input
            .create_virtual("Elyvon Studio virtual input", callback, ())
            .map_err(|e| e.to_string());
    }
    let mut input = MidiInput::new("Elyvon Studio input").map_err(|e| e.to_string())?;
    input.ignore(Ignore::None);
    let ports = input_ports(&input);
    let list = names(
        ports
            .iter()
            .map(|p| input.port_name(p).unwrap_or_default())
            .collect(),
        "input",
    );
    let at = list
        .iter()
        .position(|p| p.id == wanted)
        .ok_or_else(|| format!("MIDI input {wanted} is absent"))?;
    input
        .connect(&ports[at], "Elyvon Studio capture", callback, ())
        .map_err(|e| e.to_string())
}
