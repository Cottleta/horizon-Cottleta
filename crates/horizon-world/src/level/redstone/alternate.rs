//! Alternate Current's wire handler, behind the `alternate_current` game rule (off by default).
//!
//! Ported from Alternate Current by Space Walker, MIT License, Copyright (c) 2022 Space Walker
//! (<https://github.com/SpaceWalkerRS/alternate-current>, `wire/WireHandler.java` and friends, at
//! d93fd52 for 26.3, with its default `HORIZONTAL_FIRST_OUTWARD` update order).
//!
//! Vanilla updates each wire on its own, so a line of wires updates its neighbors many times over.
//! This works out a whole network's power first, then changes each wire once, strongest first.
//! The end states match vanilla for ordinary circuits; the order wires change in does not, so a
//! machine that relies on vanilla's location-dependent order can act differently.
//!
//! Nodes live in an arena and refer to each other by index, where the original has references.
use super::*;
use crate::level::flags;
use crate::level::orientation::Orientation;
use crate::level::update::Step;
use std::collections::VecDeque;

const WEST: usize = 0;
const NORTH: usize = 1;
const EAST: usize = 2;
const SOUTH: usize = 3;
const DOWN: usize = 4;
const UP: usize = 5;
const ALL: [Direction; 6] = [
    Direction::West,
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::Down,
    Direction::Up,
];
const NONE: u32 = u32::MAX;
const POWER_MIN: i32 = 0;
const POWER_MAX: i32 = 15;
const POWER_STEP: i32 = 1;
/// The outgoing flow from the incoming flow's bits (west, north, east, south), -1 if ambiguous.
const FLOW_IN_TO_FLOW_OUT: [i8; 16] = [
    -1,
    WEST as i8,
    NORTH as i8,
    NORTH as i8,
    EAST as i8,
    -1,
    EAST as i8,
    NORTH as i8,
    SOUTH as i8,
    WEST as i8,
    -1,
    WEST as i8,
    SOUTH as i8,
    SOUTH as i8,
    EAST as i8,
    -1,
];
/// Vanilla's order of shape updates.
const SHAPE_UPDATE_ORDER: [usize; 6] = [WEST, EAST, NORTH, SOUTH, DOWN, UP];
/// `UpdateOrder.HORIZONTAL_FIRST_OUTWARD`'s cardinal neighbors, by direction of flow.
const CARDINAL: [[usize; 4]; 4] = [
    [WEST, EAST, NORTH, SOUTH],
    [NORTH, SOUTH, EAST, WEST],
    [EAST, WEST, SOUTH, NORTH],
    [SOUTH, NORTH, WEST, EAST],
];

fn opposite(dir: usize) -> usize {
    dir ^ (0b10 >> (dir >> 2))
}
fn index_of(dir: Direction) -> usize {
    ALL.iter().position(|d| *d == dir).unwrap_or(WEST)
}
fn key(pos: BlockPos) -> (i32, i32, i32) {
    (pos.x, pos.y, pos.z)
}

#[derive(Clone, Copy)]
struct Connection {
    wire: u32,
    dir: usize,
    offer: bool,
    accept: bool,
}

#[derive(Clone)]
struct Wire {
    /// A wire has at most two connections per side.
    connections: [Option<Connection>; 8],
    total: usize,
    flow_total: usize,
    /// The direction of flow from the connections to other wires; -1 if ambiguous.
    connection_flow: i32,
    current_power: i32,
    virtual_power: i32,
    external_power: i32,
    flow_in: usize,
    flow_dir: usize,
    added: bool,
    removed: bool,
    should_break: bool,
    root: bool,
    discovered: bool,
    searched: bool,
}

#[derive(Clone)]
struct Node {
    pos: BlockPos,
    state: BlockStateId,
    invalid: bool,
    conductor: bool,
    source: bool,
    neighbors: [u32; 6],
    /// What it was last queued with.
    priority: i32,
    /// Its place in the queue, 0 if not queued.
    ticket: u64,
    neighbor_wire: u32,
    wire: Option<Wire>,
}

/// Strongest first and in order of arrival within a strength. A node queued again with another
/// strength moves there: its old entry no longer matches its ticket.
#[derive(Default)]
struct Queue {
    buckets: [VecDeque<(u32, u64)>; 16],
    next_ticket: u64,
    len: usize,
}

/// `WireHandler`: the snapshot of a network while its power changes.
#[derive(Default)]
pub(crate) struct WireHandler {
    nodes: Vec<Node>,
    index: rustc_hash::FxHashMap<(i32, i32, i32), u32>,
    /// The breadth-first search through the network, with `searched` up to here.
    search: Vec<u32>,
    updates: Queue,
    updating: bool,
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub(crate) fn alternate_current(&self) -> bool {
        self.level.game_rules().alternate_current
    }
    fn wires(&mut self) -> &mut WireHandler {
        self.alternate.get_or_insert_default()
    }
    fn node(&mut self, n: u32) -> &mut Node {
        &mut self.wires().nodes[n as usize]
    }
    fn wire(&mut self, n: u32) -> &mut Wire {
        self.node(n).wire.as_mut().expect("a wire node")
    }
    fn is_wire_node(&mut self, n: u32) -> bool {
        self.node(n).wire.is_some()
    }
    fn is_wire_state(&self, state: BlockStateId) -> bool {
        self.b.is_block(state, self.b.named.redstone_wire)
    }

    fn new_wire_node(&mut self, pos: BlockPos, state: BlockStateId) -> u32 {
        let power = self.b.table.int(state, self.b.props.power);
        self.push_node(Node {
            pos,
            state,
            invalid: false,
            conductor: false,
            source: false,
            neighbors: [NONE; 6],
            priority: power.clamp(POWER_MIN, POWER_MAX),
            ticket: 0,
            neighbor_wire: NONE,
            wire: Some(Wire {
                connections: [None; 8],
                total: 0,
                flow_total: 0,
                connection_flow: -1,
                current_power: power,
                virtual_power: power,
                external_power: 0,
                flow_in: 0,
                flow_dir: 0,
                added: false,
                removed: false,
                should_break: false,
                root: false,
                discovered: false,
                searched: false,
            }),
        })
    }
    fn new_plain_node(&mut self, pos: BlockPos, state: BlockStateId) -> u32 {
        let (conductor, source) = (
            self.is_redstone_conductor(state, pos),
            self.is_signal_source(state),
        );
        self.push_node(Node {
            pos,
            state,
            invalid: false,
            conductor,
            source,
            neighbors: [NONE; 6],
            priority: 0,
            ticket: 0,
            neighbor_wire: NONE,
            wire: None,
        })
    }
    fn push_node(&mut self, node: Node) -> u32 {
        let handler = self.wires();
        handler.nodes.push(node);
        handler.nodes.len() as u32 - 1
    }
    fn next_node(&mut self, pos: BlockPos, state: BlockStateId) -> u32 {
        if self.is_wire_state(state) {
            self.new_wire_node(pos, state)
        } else {
            self.new_plain_node(pos, state)
        }
    }
    fn get_or_add_node(&mut self, pos: BlockPos, state: Option<BlockStateId>) -> u32 {
        let existing = self.wires().index.get(&key(pos)).copied();
        let n = match existing {
            None => {
                let state = state.unwrap_or_else(|| self.state(pos));
                self.next_node(pos, state)
            }
            Some(n) if self.node(n).invalid => self.revalidate(n),
            Some(n) => n,
        };
        self.wires().index.insert(key(pos), n);
        n
    }
    fn revalidate(&mut self, n: u32) -> u32 {
        if !self.node(n).invalid {
            return n;
        }
        let pos = self.node(n).pos;
        let state = self.state(pos);
        let was_wire = self.is_wire_node(n);
        let is_wire = self.is_wire_state(state);
        if was_wire != is_wire {
            return self.next_node(pos, state);
        }
        self.node(n).invalid = false;
        if is_wire {
            // Its state is read again just before its power is set.
            let wire = self.wire(n);
            wire.root = false;
            wire.discovered = false;
            wire.searched = false;
        } else {
            let (conductor, source) = (
                self.is_redstone_conductor(state, pos),
                self.is_signal_source(state),
            );
            let node = self.node(n);
            node.state = state;
            node.conductor = conductor;
            node.source = source;
        }
        n
    }
    fn neighbor_of(&mut self, n: u32, dir: usize) -> u32 {
        let neighbor = self.node(n).neighbors[dir];
        if neighbor != NONE && !self.node(neighbor).invalid {
            return neighbor;
        }
        let pos = rel(self.node(n).pos, ALL[dir]);
        let found = self.get_or_add_node(pos, None);
        if found != neighbor {
            self.node(n).neighbors[dir] = found;
            self.node(found).neighbors[opposite(dir)] = n;
        }
        found
    }

    /// `onWireUpdated`: a wire received a block update.
    pub(crate) fn alternate_wire_updated(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        orientation: Option<Orientation>,
    ) {
        let wire = self.get_or_add_node(pos, Some(state));
        if !self.is_wire_node(wire) {
            return;
        }
        self.invalidate();
        self.revalidate(wire);
        self.find_roots(wire, orientation);
        self.try_update();
    }
    /// `onWireAdded`.
    pub(crate) fn alternate_wire_added(&mut self, pos: BlockPos, state: BlockStateId) {
        let wire = self.get_or_add_node(pos, Some(state));
        if !self.is_wire_node(wire) {
            return;
        }
        self.wire(wire).added = true;
        self.invalidate();
        self.revalidate(wire);
        self.find_root(wire, -1);
        self.try_update();
    }
    /// `onWireRemoved`.
    pub(crate) fn alternate_wire_removed(&mut self, pos: BlockPos, state: BlockStateId) {
        let wire = match self.wires().index.remove(&key(pos)) {
            Some(n) if self.is_wire_node(n) => n,
            _ => self.new_wire_node(pos, state),
        };
        self.node(wire).invalid = true;
        self.wire(wire).removed = true;
        // Part of power changes already going on.
        if self.wires().updating && self.wire(wire).should_break {
            return;
        }
        self.invalidate();
        self.revalidate(wire);
        self.find_root(wire, -1);
        self.try_update();
    }
    /// Blocks can have changed while power changes are carried out: every node is checked
    /// again before it is used.
    fn invalidate(&mut self) {
        let handler = self.wires();
        if handler.updating {
            let nodes = &mut handler.nodes;
            for &n in handler.index.values() {
                nodes[n as usize].invalid = true;
            }
        }
    }
    fn find_roots(&mut self, wire: u32, orientation: Option<Orientation>) {
        // The horizontal direction, for the order of updates.
        let bias = orientation.map_or(-1, |o| {
            let front = o.front();
            let dir = if matches!(front, Direction::Up | Direction::Down) {
                o.up()
            } else {
                front
            };
            index_of(dir) as i32
        });
        self.find_root(wire, bias);
        if !self.wire(wire).searched {
            return;
        }
        match orientation {
            None => {
                let flow = self.wire(wire).flow_dir;
                let cardinal = CARDINAL[flow];
                for dir in cardinal.into_iter().chain([DOWN, UP]) {
                    self.find_roots_around(wire, dir);
                }
            }
            // The update came from the front: only behind is left to look at.
            Some(o) => self.find_roots_around(wire, index_of(o.front().opposite())),
        }
    }
    fn find_roots_around(&mut self, wire: u32, dir: usize) {
        let node = self.neighbor_of(wire, dir);
        if self.node(node).conductor || self.node(node).source {
            let flow = self.wire(wire).flow_dir;
            for side in CARDINAL[flow] {
                let neighbor = self.neighbor_of(node, side);
                if self.is_wire_node(neighbor) {
                    self.find_root(neighbor, side as i32);
                }
            }
        }
    }
    fn find_root(&mut self, wire: u32, discovery_dir: i32) {
        if self.wire(wire).discovered {
            return;
        }
        self.discover(wire);
        self.find_external_power(wire);
        self.find_power(wire, false);
        if self.needs_update(wire) {
            // The flow of its connections comes first.
            let connection_flow = self.wire(wire).connection_flow;
            let backup = if connection_flow >= 0 {
                connection_flow as usize
            } else if discovery_dir < 0 {
                0
            } else {
                discovery_dir as usize
            };
            self.search_wire(wire, true, backup);
        }
    }
    fn discover(&mut self, wire: u32) {
        if self.wire(wire).discovered {
            return;
        }
        let (pos, state) = (self.node(wire).pos, self.node(wire).state);
        let w = self.wire(wire);
        w.discovered = true;
        w.searched = false;
        let check = !w.removed && !w.should_break;
        if check && !self.can_survive(state, pos) {
            self.wire(wire).should_break = true;
        }
        let w = self.wire(wire);
        w.virtual_power = w.current_power;
        w.external_power = POWER_MIN - 1;
        self.set_connections(wire);
    }
    fn set_connections(&mut self, owner: u32) {
        let w = self.wire(owner);
        w.connections = [None; 8];
        w.total = 0;
        w.flow_total = 0;
        w.connection_flow = -1;
        let below = self.neighbor_of(owner, DOWN);
        let below_is_conductor = self.node(below).conductor;
        let up = self.neighbor_of(owner, UP);
        let above_is_conductor = self.node(up).conductor;
        for dir in 0..4 {
            let neighbor = self.neighbor_of(owner, dir);
            if self.is_wire_node(neighbor) {
                self.add_connection(owner, neighbor, dir, true, true);
            } else {
                let side_is_conductor = self.node(neighbor).conductor;
                if !side_is_conductor {
                    let node = self.neighbor_of(neighbor, DOWN);
                    if self.is_wire_node(node) {
                        self.add_connection(owner, node, dir, below_is_conductor, true);
                    }
                }
                if !above_is_conductor {
                    let node = self.neighbor_of(neighbor, UP);
                    if self.is_wire_node(node) {
                        self.add_connection(owner, node, dir, true, side_is_conductor);
                    }
                }
            }
        }
        let w = self.wire(owner);
        if w.total > 0 {
            w.connection_flow = i32::from(FLOW_IN_TO_FLOW_OUT[w.flow_total]);
        }
    }
    fn add_connection(&mut self, owner: u32, wire: u32, dir: usize, offer: bool, accept: bool) {
        let w = self.wire(owner);
        w.connections[w.total] = Some(Connection {
            wire,
            dir,
            offer,
            accept,
        });
        w.total += 1;
        w.flow_total |= 1 << dir;
    }
    /// Connections in the order of the flow; the order they were found in, without one.
    fn connections_in_order(&mut self, wire: u32, flow: usize) -> ([Option<Connection>; 8], usize) {
        let w = self.wire(wire);
        let mut out = [None; 8];
        let mut len = 0;
        for dir in CARDINAL[flow] {
            for c in w.connections[..w.total].iter().flatten() {
                if c.dir == dir {
                    out[len] = Some(*c);
                    len += 1;
                }
            }
        }
        (out, len)
    }
    fn find_power(&mut self, wire: u32, ignore_searched: bool) {
        let w = self.wire(wire);
        w.virtual_power = w.external_power;
        w.flow_in = 0;
        if w.removed || w.should_break {
            return;
        }
        // Wires give at most POWER_MAX - POWER_STEP.
        if w.external_power < POWER_MAX - POWER_STEP {
            let (connections, total) = (w.connections, w.total);
            for c in connections[..total].iter().flatten() {
                if !c.accept {
                    continue;
                }
                let neighbor = self.wire(c.wire);
                if !ignore_searched || !neighbor.searched {
                    let power = (neighbor.virtual_power - POWER_STEP).max(POWER_MIN);
                    self.offer_power(wire, power, opposite(c.dir));
                }
            }
        }
    }
    fn offer_power(&mut self, wire: u32, power: i32, dir: usize) -> bool {
        let w = self.wire(wire);
        if w.removed || w.should_break {
            return false;
        }
        if power == w.virtual_power {
            w.flow_in |= 1 << dir;
            return false;
        }
        if power > w.virtual_power {
            w.virtual_power = power;
            w.flow_in = 1 << dir;
            return true;
        }
        false
    }
    fn find_external_power(&mut self, wire: u32) {
        let w = self.wire(wire);
        if w.removed || w.should_break || w.external_power >= POWER_MIN {
            return;
        }
        let power = self.external_power_of(wire);
        let w = self.wire(wire);
        w.external_power = power;
        if power > w.virtual_power {
            w.virtual_power = power;
        }
    }
    fn external_power_of(&mut self, wire: u32) -> i32 {
        let mut power = POWER_MIN;
        for dir in 0..6 {
            let neighbor = self.neighbor_of(wire, dir);
            if self.is_wire_node(neighbor) {
                continue;
            }
            // A target block is both.
            if self.node(neighbor).conductor {
                power = power.max(self.direct_signal_through(neighbor));
            }
            if self.node(neighbor).source {
                let (state, pos) = (self.node(neighbor).state, self.node(neighbor).pos);
                power = power.max(self.state_signal(state, pos, ALL[dir]));
            }
            if power >= POWER_MAX {
                return POWER_MAX;
            }
        }
        power
    }
    fn direct_signal_through(&mut self, conductor: u32) -> i32 {
        let mut power = POWER_MIN;
        for dir in 0..6 {
            let neighbor = self.neighbor_of(conductor, dir);
            if self.node(neighbor).source {
                let (state, pos) = (self.node(neighbor).state, self.node(neighbor).pos);
                power = power.max(self.state_direct_signal(state, pos, ALL[dir]));
                if power >= POWER_MAX {
                    return POWER_MAX;
                }
            }
        }
        power
    }
    fn needs_update(&mut self, wire: u32) -> bool {
        let w = self.wire(wire);
        w.removed || w.should_break || w.virtual_power != w.current_power
    }
    fn search_wire(&mut self, wire: u32, root: bool, backup_flow: usize) {
        self.wires().search.push(wire);
        let w = self.wire(wire);
        w.root = root;
        w.searched = true;
        w.flow_dir = backup_flow;
    }
    fn try_update(&mut self) {
        if !self.wires().search.is_empty() {
            self.search_network();
            self.depower_network();
            self.power_network();
        }
        let handler = self.wires();
        if !handler.updating {
            handler.nodes.clear();
            handler.index.clear();
        }
    }
    fn search_network(&mut self) {
        let mut at = 0;
        while at < self.wires().search.len() {
            let wire = self.wires().search[at];
            at += 1;
            let flow = self.wire(wire).flow_dir;
            let (connections, len) = self.connections_in_order(wire, flow);
            for c in connections[..len].iter().flatten() {
                if !c.offer || self.wire(c.wire).searched {
                    continue;
                }
                self.discover(c.wire);
                self.find_power(c.wire, false);
                // Wires gave less: how far it falls depends on what else powers it.
                let w = self.wire(c.wire);
                if w.virtual_power < w.current_power {
                    self.find_external_power(c.wire);
                }
                if self.needs_update(c.wire) {
                    self.search_wire(c.wire, false, c.dir);
                }
            }
        }
    }
    fn depower_network(&mut self) {
        let search = std::mem::take(&mut self.wires().search);
        for &wire in &search {
            self.find_power(wire, true);
            let w = self.wire(wire);
            if w.root || w.removed || w.should_break || w.virtual_power > POWER_MIN {
                self.queue_wire(wire);
            } else {
                // So that any power offered queues its change.
                w.virtual_power -= 1;
            }
        }
        let mut search = search;
        search.clear();
        self.wires().search = search;
    }
    fn power_network(&mut self) {
        // A chain reached this while it was running: its changes joined the queue.
        if self.wires().updating {
            return;
        }
        self.wires().updating = true;
        while let Some(n) = self.poll_update() {
            if self.is_wire_node(n) {
                if !self.needs_update(n) {
                    continue;
                }
                self.find_power_flow(n);
                self.transmit_power(n);
                if self.set_power(n) {
                    self.queue_neighbors(n);
                    // Observers hear about the change as they did before 1.19.
                    self.update_neighbor_shapes(n);
                }
            } else {
                let wire = self.node(n).neighbor_wire;
                if wire != NONE {
                    let (pos, block) = (self.node(n).pos, self.node(wire).state);
                    let block = self.b.block(block);
                    self.execute(Step::Neighbor {
                        state: None,
                        pos,
                        block,
                        orientation: None,
                        moved: false,
                    });
                }
            }
        }
        self.wires().updating = false;
    }
    fn set_power(&mut self, wire: u32) -> bool {
        if self.wire(wire).removed {
            return true;
        }
        let pos = self.node(wire).pos;
        let state = self.state(pos);
        self.node(wire).state = state;
        if !self.is_wire_state(state) {
            return false;
        }
        if self.wire(wire).should_break {
            self.drop_resources(state, pos);
            self.set_block(pos, self.b.air(), flags::UPDATE_CLIENTS);
            return true;
        }
        let w = self.wire(wire);
        let power = w.virtual_power.clamp(POWER_MIN, POWER_MAX);
        w.current_power = power;
        let added = w.added;
        let next = self.b.table.set_int(state, self.b.props.power, power);
        self.node(wire).state = next;
        if next == state {
            return false;
        }
        // `LevelHelper.setWireState`: neighbor shapes only for a new wire.
        let update = if added {
            flags::UPDATE_CLIENTS
        } else {
            flags::UPDATE_CLIENTS | flags::UPDATE_KNOWN_SHAPE
        };
        self.set_block(pos, next, update);
        true
    }
    fn find_power_flow(&mut self, wire: u32) {
        let w = self.wire(wire);
        let flow = FLOW_IN_TO_FLOW_OUT[w.flow_in];
        if flow >= 0 {
            w.flow_dir = flow as usize;
        } else if w.connection_flow >= 0 {
            w.flow_dir = w.connection_flow as usize;
        }
    }
    fn transmit_power(&mut self, wire: u32) {
        let w = self.wire(wire);
        let (flow, power) = (w.flow_dir, (w.virtual_power - POWER_STEP).max(POWER_MIN));
        let (connections, len) = self.connections_in_order(wire, flow);
        for c in connections[..len].iter().flatten() {
            if c.offer && self.offer_power(c.wire, power, c.dir) {
                self.queue_wire(c.wire);
            }
        }
    }
    fn update_neighbor_shapes(&mut self, wire: u32) {
        let (pos, state) = (self.node(wire).pos, self.node(wire).state);
        for dir in SHAPE_UPDATE_ORDER {
            let neighbor = self.neighbor_of(wire, dir);
            let target = self.node(neighbor).state;
            if !self.is_wire_node(neighbor) && !self.b.is_air(target) {
                let at = self.node(neighbor).pos;
                self.execute(Step::Shape {
                    direction: ALL[opposite(dir)],
                    neighbor_state: state,
                    pos: at,
                    neighbor_pos: pos,
                    flags: flags::UPDATE_CLIENTS,
                    limit: flags::UPDATE_LIMIT,
                });
            }
        }
    }
    /// `HORIZONTAL_FIRST_OUTWARD.forEachNeighbor`: direct, then diagonal, then far neighbors.
    fn queue_neighbors(&mut self, wire: u32) {
        let forward = self.wire(wire).flow_dir;
        let (rightward, backward, leftward) = (
            (forward + 1) & 0b11,
            (forward + 2) & 0b11,
            (forward + 3) & 0b11,
        );
        let front = self.neighbor_of(wire, forward);
        let right = self.neighbor_of(wire, rightward);
        let back = self.neighbor_of(wire, backward);
        let left = self.neighbor_of(wire, leftward);
        let below = self.neighbor_of(wire, DOWN);
        let up = self.neighbor_of(wire, UP);
        for node in [front, back, right, left, below, up] {
            self.queue_neighbor(node, wire);
        }
        for (from, dir) in [
            (front, rightward),
            (back, leftward),
            (front, leftward),
            (back, rightward),
            (front, DOWN),
            (back, UP),
            (front, UP),
            (back, DOWN),
            (right, DOWN),
            (left, UP),
            (right, UP),
            (left, DOWN),
            (front, forward),
            (back, backward),
            (right, rightward),
            (left, leftward),
            (below, DOWN),
            (up, UP),
        ] {
            let node = self.neighbor_of(from, dir);
            self.queue_neighbor(node, wire);
        }
    }
    fn queue_neighbor(&mut self, node: u32, wire: u32) {
        // Wires get theirs when power is transmitted, and an update to air does nothing.
        let state = self.node(node).state;
        if !self.is_wire_node(node) && !self.b.is_air(state) {
            self.node(node).neighbor_wire = wire;
            self.offer_update(node);
        }
    }
    fn queue_wire(&mut self, wire: u32) {
        if self.needs_update(wire) {
            self.offer_update(wire);
        } else {
            self.find_power_flow(wire);
            self.transmit_power(wire);
        }
    }
    fn offer_update(&mut self, n: u32) {
        let priority = match self.node(n).wire.as_ref() {
            Some(w) => w.virtual_power.clamp(POWER_MIN, POWER_MAX),
            None => {
                let wire = self.node(n).neighbor_wire;
                self.node(wire).priority
            }
        };
        let node = self.node(n);
        if node.ticket != 0 && node.priority == priority {
            return;
        }
        let handler = self.wires();
        handler.updates.next_ticket += 1;
        let ticket = handler.updates.next_ticket;
        if handler.nodes[n as usize].ticket == 0 {
            handler.updates.len += 1;
        }
        handler.updates.buckets[priority as usize].push_back((n, ticket));
        let node = &mut handler.nodes[n as usize];
        node.priority = priority;
        node.ticket = ticket;
    }
    fn poll_update(&mut self) -> Option<u32> {
        let handler = self.wires();
        if handler.updates.len == 0 {
            return None;
        }
        for bucket in handler.updates.buckets.iter_mut().rev() {
            while let Some((n, ticket)) = bucket.pop_front() {
                // Moved to another strength since.
                if handler.nodes[n as usize].ticket == ticket {
                    handler.nodes[n as usize].ticket = 0;
                    handler.updates.len -= 1;
                    return Some(n);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_directions() {
        assert_eq!(opposite(WEST), EAST);
        assert_eq!(opposite(NORTH), SOUTH);
        assert_eq!(opposite(EAST), WEST);
        assert_eq!(opposite(SOUTH), NORTH);
        assert_eq!(opposite(DOWN), UP);
        assert_eq!(opposite(UP), DOWN);
    }

}
