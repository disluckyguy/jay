mod event_handling;
mod kb_owner;
mod pointer_owner;
pub mod wl_keyboard;
pub mod wl_pointer;
pub mod wl_touch;
pub mod zwp_pointer_constraints_v1;
pub mod zwp_relative_pointer_manager_v1;
pub mod zwp_relative_pointer_v1;

pub use event_handling::NodeSeatState;
use {
    crate::{
        async_engine::SpawnedFuture,
        client::{Client, ClientError, ClientId},
        cursor::{Cursor, KnownCursor},
        fixed::Fixed,
        globals::{Global, GlobalName},
        ifs::{
            ipc,
            ipc::{
                wl_data_device::{ClipboardIpc, WlDataDevice},
                wl_data_source::WlDataSource,
                zwp_primary_selection_device_v1::{
                    PrimarySelectionIpc, ZwpPrimarySelectionDeviceV1,
                },
                zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
                IpcError,
            },
            wl_seat::{
                kb_owner::KbOwnerHolder,
                pointer_owner::PointerOwnerHolder,
                wl_keyboard::{WlKeyboard, WlKeyboardError, REPEAT_INFO_SINCE},
                wl_pointer::WlPointer,
                wl_touch::WlTouch,
                zwp_pointer_constraints_v1::{SeatConstraint, SeatConstraintStatus},
                zwp_relative_pointer_v1::ZwpRelativePointerV1,
            },
            wl_surface::WlSurface,
        },
        leaks::Tracker,
        object::Object,
        rect::Rect,
        state::State,
        tree::{
            generic_node_visitor, ContainerNode, ContainerSplit, Direction, FloatNode, FoundNode,
            Node, OutputNode, WorkspaceNode,
        },
        utils::{
            asyncevent::AsyncEvent,
            buffd::{MsgParser, MsgParserError},
            clonecell::CloneCell,
            copyhashmap::CopyHashMap,
            errorfmt::ErrorFmt,
            linkedlist::LinkedNode,
            numcell::NumCell,
            rc_eq::rc_eq,
        },
        wire::{
            wl_seat::*, WlDataDeviceId, WlKeyboardId, WlPointerId, WlSeatId,
            ZwpPrimarySelectionDeviceV1Id, ZwpRelativePointerV1Id,
        },
        xkbcommon::{XkbKeymap, XkbState},
    },
    ahash::{AHashMap, AHashSet},
    jay_config::keyboard::mods::Modifiers,
    smallvec::SmallVec,
    std::{
        cell::{Cell, RefCell},
        collections::hash_map::Entry,
        mem,
        ops::{Deref, DerefMut},
        rc::Rc,
    },
    thiserror::Error,
    uapi::{c, Errno, OwnedFd},
};

pub const POINTER: u32 = 1;
pub const KEYBOARD: u32 = 2;
#[allow(dead_code)]
const TOUCH: u32 = 4;

#[allow(dead_code)]
const MISSING_CAPABILITY: u32 = 0;

pub const BTN_LEFT: u32 = 0x110;

pub const SEAT_NAME_SINCE: u32 = 2;

pub const PX_PER_SCROLL: f64 = 15.0;

#[derive(Clone)]
pub struct Dnd {
    pub seat: Rc<WlSeatGlobal>,
    client: Rc<Client>,
    src: Option<Rc<WlDataSource>>,
}

pub struct DroppedDnd {
    dnd: Dnd,
}

impl Drop for DroppedDnd {
    fn drop(&mut self) {
        if let Some(src) = self.dnd.src.take() {
            ipc::detach_seat::<ClipboardIpc>(&src);
        }
    }
}

linear_ids!(SeatIds, SeatId);

pub struct WlSeatGlobal {
    id: SeatId,
    name: GlobalName,
    state: Rc<State>,
    seat_name: String,
    move_: Cell<bool>,
    move_start_pos: Cell<(Fixed, Fixed)>,
    extents_start_pos: Cell<(i32, i32)>,
    pos_time_usec: Cell<u64>,
    pos: Cell<(Fixed, Fixed)>,
    pointer_stack: RefCell<Vec<Rc<dyn Node>>>,
    pointer_stack_modified: Cell<bool>,
    found_tree: RefCell<Vec<FoundNode>>,
    keyboard_node: CloneCell<Rc<dyn Node>>,
    pressed_keys: RefCell<AHashSet<u32>>,
    bindings: RefCell<AHashMap<ClientId, AHashMap<WlSeatId, Rc<WlSeat>>>>,
    data_devices: RefCell<AHashMap<ClientId, AHashMap<WlDataDeviceId, Rc<WlDataDevice>>>>,
    primary_selection_devices: RefCell<
        AHashMap<
            ClientId,
            AHashMap<ZwpPrimarySelectionDeviceV1Id, Rc<ZwpPrimarySelectionDeviceV1>>,
        >,
    >,
    repeat_rate: Cell<(i32, i32)>,
    kb_map: CloneCell<Rc<XkbKeymap>>,
    kb_state: RefCell<XkbState>,
    cursor: CloneCell<Option<Rc<dyn Cursor>>>,
    tree_changed: Rc<AsyncEvent>,
    selection: CloneCell<Option<Rc<WlDataSource>>>,
    selection_serial: Cell<u32>,
    primary_selection: CloneCell<Option<Rc<ZwpPrimarySelectionSourceV1>>>,
    primary_selection_serial: Cell<u32>,
    pointer_owner: PointerOwnerHolder,
    kb_owner: KbOwnerHolder,
    dropped_dnd: RefCell<Option<DroppedDnd>>,
    shortcuts: CopyHashMap<(u32, u32), Modifiers>,
    queue_link: Cell<Option<LinkedNode<Rc<Self>>>>,
    tree_changed_handler: Cell<Option<SpawnedFuture<()>>>,
    output: CloneCell<Rc<OutputNode>>,
    desired_known_cursor: Cell<Option<KnownCursor>>,
    changes: NumCell<u32>,
    cursor_size: Cell<u32>,
    hardware_cursor: Cell<bool>,
    constraint: CloneCell<Option<Rc<SeatConstraint>>>,
}

const CHANGE_CURSOR_MOVED: u32 = 1 << 0;
const CHANGE_TREE: u32 = 1 << 1;

const DEFAULT_CURSOR_SIZE: u32 = 16;

impl Drop for WlSeatGlobal {
    fn drop(&mut self) {
        self.state.remove_cursor_size(self.cursor_size.get());
    }
}

impl WlSeatGlobal {
    pub fn new(name: GlobalName, seat_name: &str, state: &Rc<State>) -> Rc<Self> {
        let slf = Rc::new(Self {
            id: state.seat_ids.next(),
            name,
            state: state.clone(),
            seat_name: seat_name.to_string(),
            move_: Cell::new(false),
            move_start_pos: Cell::new((Fixed(0), Fixed(0))),
            extents_start_pos: Cell::new((0, 0)),
            pos_time_usec: Cell::new(0),
            pos: Cell::new((Fixed(0), Fixed(0))),
            pointer_stack: RefCell::new(vec![]),
            pointer_stack_modified: Cell::new(false),
            found_tree: RefCell::new(vec![]),
            keyboard_node: CloneCell::new(state.root.clone()),
            pressed_keys: RefCell::new(Default::default()),
            bindings: Default::default(),
            data_devices: RefCell::new(Default::default()),
            primary_selection_devices: RefCell::new(Default::default()),
            repeat_rate: Cell::new((25, 250)),
            kb_map: CloneCell::new(state.default_keymap.clone()),
            kb_state: RefCell::new(state.default_keymap.state().unwrap()),
            cursor: Default::default(),
            tree_changed: Default::default(),
            selection: Default::default(),
            selection_serial: Cell::new(0),
            primary_selection: Default::default(),
            primary_selection_serial: Cell::new(0),
            pointer_owner: Default::default(),
            kb_owner: Default::default(),
            dropped_dnd: RefCell::new(None),
            shortcuts: Default::default(),
            queue_link: Cell::new(None),
            tree_changed_handler: Cell::new(None),
            output: CloneCell::new(state.dummy_output.get().unwrap()),
            desired_known_cursor: Cell::new(None),
            changes: NumCell::new(CHANGE_CURSOR_MOVED | CHANGE_TREE),
            cursor_size: Cell::new(DEFAULT_CURSOR_SIZE),
            hardware_cursor: Cell::new(state.globals.seats.len() == 0),
            constraint: Default::default(),
        });
        state.add_cursor_size(DEFAULT_CURSOR_SIZE);
        let seat = slf.clone();
        let future = state.eng.spawn(async move {
            loop {
                seat.tree_changed.triggered().await;
                seat.state.tree_changed_sent.set(false);
                seat.changes.or_assign(CHANGE_TREE);
                // log::info!("tree_changed");
                seat.apply_changes();
            }
        });
        slf.tree_changed_handler.set(Some(future));
        slf
    }

    pub fn set_hardware_cursor(&self, hardware_cursor: bool) {
        self.hardware_cursor.set(hardware_cursor);
    }

    pub fn hardware_cursor(&self) -> bool {
        self.hardware_cursor.get()
    }

    fn update_hardware_cursor_position(&self) {
        self.update_hardware_cursor_(false);
    }

    pub fn update_hardware_cursor(&self) {
        self.update_hardware_cursor_(true);
    }

    fn update_hardware_cursor_(&self, render: bool) {
        if !self.hardware_cursor.get() {
            return;
        }
        let cursor = match self.get_cursor() {
            Some(c) => c,
            _ => {
                self.state.disable_hardware_cursors();
                return;
            }
        };
        if render {
            cursor.tick();
        }
        let (x, y) = self.get_position();
        for output in self.state.root.outputs.lock().values() {
            if let Some(hc) = output.hardware_cursor.get() {
                let scale = output.preferred_scale.get();
                let extents = cursor.extents_at_scale(scale);
                if render {
                    let (max_width, max_height) = hc.max_size();
                    if extents.width() > max_width || extents.height() > max_height {
                        hc.set_enabled(false);
                        hc.commit();
                        continue;
                    }
                }
                let opos = output.global.pos.get();
                let (x_rel, y_rel);
                if scale == 1 {
                    x_rel = x.round_down() - opos.x1();
                    y_rel = y.round_down() - opos.y1();
                } else {
                    let scalef = scale.to_f64();
                    x_rel = ((x - Fixed::from_int(opos.x1())).to_f64() * scalef).round() as i32;
                    y_rel = ((y - Fixed::from_int(opos.y1())).to_f64() * scalef).round() as i32;
                }
                let mode = output.global.mode.get();
                if extents
                    .intersects(&Rect::new_sized(-x_rel, -y_rel, mode.width, mode.height).unwrap())
                {
                    if render {
                        let buffer = hc.get_buffer();
                        buffer.render_hardware_cursor(cursor.deref(), &self.state, scale);
                        hc.swap_buffer();
                    }
                    hc.set_enabled(true);
                    hc.set_position(x_rel + extents.x1(), y_rel + extents.y1());
                } else {
                    hc.set_enabled(false);
                }
                hc.commit();
            }
        }
    }

    pub fn set_cursor_size(&self, size: u32) {
        let old = self.cursor_size.replace(size);
        if size != old {
            self.state.remove_cursor_size(old);
            self.state.add_cursor_size(size);
            self.reload_known_cursor();
        }
    }

    pub fn add_data_device(&self, device: &Rc<WlDataDevice>) {
        let mut dd = self.data_devices.borrow_mut();
        dd.entry(device.client.id)
            .or_default()
            .insert(device.id, device.clone());
    }

    pub fn remove_data_device(&self, device: &WlDataDevice) {
        let mut dd = self.data_devices.borrow_mut();
        if let Entry::Occupied(mut e) = dd.entry(device.client.id) {
            e.get_mut().remove(&device.id);
            if e.get().is_empty() {
                e.remove();
            }
        }
    }

    pub fn add_primary_selection_device(&self, device: &Rc<ZwpPrimarySelectionDeviceV1>) {
        let mut dd = self.primary_selection_devices.borrow_mut();
        dd.entry(device.client.id)
            .or_default()
            .insert(device.id, device.clone());
    }

    pub fn remove_primary_selection_device(&self, device: &ZwpPrimarySelectionDeviceV1) {
        let mut dd = self.primary_selection_devices.borrow_mut();
        if let Entry::Occupied(mut e) = dd.entry(device.client.id) {
            e.get_mut().remove(&device.id);
            if e.get().is_empty() {
                e.remove();
            }
        }
    }

    pub fn get_output(&self) -> Rc<OutputNode> {
        self.output.get()
    }

    pub fn set_workspace(&self, ws: &Rc<WorkspaceNode>) {
        let tl = match self.keyboard_node.get().node_toplevel() {
            Some(tl) => tl,
            _ => return,
        };
        if tl.tl_data().is_fullscreen.get() {
            return;
        }
        let old_ws = match tl.tl_data().workspace.get() {
            Some(ws) => ws,
            _ => return,
        };
        if old_ws.id == ws.id {
            return;
        }
        let cn = match tl
            .tl_data()
            .parent
            .get()
            .and_then(|p| p.node_into_containing_node())
        {
            Some(cn) => cn,
            _ => return,
        };
        let kb_foci = collect_kb_foci(tl.clone().tl_into_node());
        cn.cnode_remove_child2(tl.tl_as_node(), true);
        if !ws.visible.get() {
            for focus in kb_foci {
                old_ws.clone().node_do_focus(&focus, Direction::Unspecified);
            }
        }
        if tl.tl_data().is_floating.get() {
            self.state.map_floating(
                tl.clone(),
                tl.tl_data().float_width.get(),
                tl.tl_data().float_height.get(),
                ws,
            );
        } else {
            self.state.map_tiled_on(tl, ws);
        }
    }

    pub fn mark_last_active(self: &Rc<Self>) {
        self.queue_link
            .set(Some(self.state.seat_queue.add_last(self.clone())));
    }

    pub fn disable_pointer_constraint(&self) {
        if let Some(constraint) = self.constraint.get() {
            constraint.deactivate();
            if constraint.status.get() == SeatConstraintStatus::Inactive {
                constraint
                    .status
                    .set(SeatConstraintStatus::ActivatableOnFocus);
            }
        }
    }

    fn maybe_constrain_pointer_node(&self) {
        if let Some(pn) = self.pointer_node() {
            if let Some(surface) = pn.node_into_surface() {
                let (mut x, mut y) = self.pos.get();
                let (sx, sy) = surface.buffer_abs_pos.get().position();
                x -= Fixed::from_int(sx);
                y -= Fixed::from_int(sy);
                self.maybe_constrain(&surface, x, y);
            }
        }
    }

    fn maybe_constrain(&self, surface: &WlSurface, x: Fixed, y: Fixed) {
        if self.constraint.get().is_some() {
            return;
        }
        let candidate = match surface.constraints.get(&self.id) {
            Some(c) if c.status.get() == SeatConstraintStatus::Inactive => c,
            _ => return,
        };
        if !candidate.contains(x.round_down(), y.round_down()) {
            return;
        }
        candidate.status.set(SeatConstraintStatus::Active);
        if let Some(owner) = candidate.owner.get() {
            owner.send_enabled();
        }
        self.constraint.set(Some(candidate));
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        if let Some(tl) = self.keyboard_node.get().node_toplevel() {
            tl.tl_set_fullscreen(fullscreen);
        }
    }

    pub fn get_fullscreen(&self) -> bool {
        if let Some(tl) = self.keyboard_node.get().node_toplevel() {
            return tl.tl_data().is_fullscreen.get();
        }
        false
    }

    pub fn set_keymap(&self, keymap: &Rc<XkbKeymap>) {
        let state = match keymap.state() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Could not create keymap state: {}", ErrorFmt(e));
                return;
            }
        };
        self.kb_map.set(keymap.clone());
        *self.kb_state.borrow_mut() = state;
        let bindings = self.bindings.borrow_mut();
        for (id, client) in bindings.iter() {
            for seat in client.values() {
                let kbs = seat.keyboards.lock();
                for kb in kbs.values() {
                    let fd = match seat.keymap_fd(keymap) {
                        Ok(fd) => fd,
                        Err(e) => {
                            log::error!("Could not creat a file descriptor to transfer the keymap to client {}: {}", id, ErrorFmt(e));
                            continue;
                        }
                    };
                    kb.send_keymap(wl_keyboard::XKB_V1, fd, keymap.map_len as _);
                }
            }
        }
    }

    pub fn prepare_for_lock(self: &Rc<Self>) {
        self.pointer_owner.revert_to_default(self);
        self.kb_owner.ungrab(self);
    }

    pub fn set_position(&self, x: i32, y: i32) {
        self.pos.set((Fixed::from_int(x), Fixed::from_int(y)));
        self.update_hardware_cursor_position();
        self.trigger_tree_changed();
        let output = 'set_output: {
            let outputs = self.state.outputs.lock();
            for output in outputs.values() {
                if output.node.global.pos.get().contains(x, y) {
                    break 'set_output output.node.clone();
                }
            }
            self.state.dummy_output.get().unwrap()
        };
        self.set_output(&output);
    }

    fn set_output(&self, output: &Rc<OutputNode>) {
        self.output.set(output.clone());
        if let Some(cursor) = self.cursor.get() {
            cursor.set_output(output);
        }
        if let Some(dnd) = self.pointer_owner.dnd_icon() {
            dnd.set_output(output);
        }
    }

    pub fn position(&self) -> (Fixed, Fixed) {
        self.pos.get()
    }

    pub fn render_ctx_changed(&self) {
        if let Some(cursor) = self.desired_known_cursor.get() {
            self.set_known_cursor(cursor);
        }
    }

    pub fn kb_parent_container(&self) -> Option<Rc<ContainerNode>> {
        if let Some(tl) = self.keyboard_node.get().node_toplevel() {
            if let Some(parent) = tl.tl_data().parent.get() {
                if let Some(container) = parent.node_into_container() {
                    return Some(container);
                }
            }
        }
        None
    }

    pub fn get_mono(&self) -> Option<bool> {
        self.kb_parent_container()
            .map(|c| c.mono_child.get().is_some())
    }

    pub fn get_split(&self) -> Option<ContainerSplit> {
        self.kb_parent_container().map(|c| c.split.get())
    }

    pub fn set_mono(&self, mono: bool) {
        if let Some(tl) = self.keyboard_node.get().node_toplevel() {
            if let Some(parent) = tl.tl_data().parent.get() {
                if let Some(container) = parent.node_into_container() {
                    let node = if mono { Some(tl.deref()) } else { None };
                    container.set_mono(node);
                }
            }
        }
    }

    pub fn set_split(&self, axis: ContainerSplit) {
        if let Some(c) = self.kb_parent_container() {
            c.set_split(axis);
        }
    }

    pub fn create_split(&self, axis: ContainerSplit) {
        let tl = match self.keyboard_node.get().node_toplevel() {
            Some(tl) => tl,
            _ => return,
        };
        if tl.tl_data().is_fullscreen.get() {
            return;
        }
        let ws = match tl.tl_data().workspace.get() {
            Some(ws) => ws,
            _ => return,
        };
        let pn = match tl.tl_data().parent.get() {
            Some(pn) => pn,
            _ => return,
        };
        if let Some(pn) = pn.node_into_containing_node() {
            let cn = ContainerNode::new(&self.state, &ws, pn.clone(), tl.clone(), axis);
            pn.cnode_replace_child(tl.tl_as_node(), cn);
        }
    }

    pub fn focus_parent(self: &Rc<Self>) {
        if let Some(tl) = self.keyboard_node.get().node_toplevel() {
            if let Some(parent) = tl.tl_data().parent.get() {
                self.focus_node(parent.cnode_into_node());
            }
        }
    }

    pub fn get_floating(self: &Rc<Self>) -> Option<bool> {
        match self.keyboard_node.get().node_toplevel() {
            Some(tl) => Some(tl.tl_data().is_floating.get()),
            _ => None,
        }
    }

    pub fn set_floating(self: &Rc<Self>, floating: bool) {
        let tl = match self.keyboard_node.get().node_toplevel() {
            Some(tl) => tl,
            _ => return,
        };
        let data = tl.tl_data();
        if data.is_fullscreen.get() {
            return;
        }
        if data.is_floating.get() == floating {
            return;
        }
        let parent = match data.parent.get() {
            Some(p) => p,
            _ => return,
        };
        if let Some(cn) = parent.node_into_containing_node() {
            if !floating {
                cn.cnode_remove_child2(tl.tl_as_node(), true);
                self.state.map_tiled(tl);
            } else if let Some(ws) = data.workspace.get() {
                cn.cnode_remove_child2(tl.tl_as_node(), true);
                let (width, height) = data.float_size(&ws);
                self.state.map_floating(tl, width, height, &ws);
            }
        }
    }

    pub fn get_rate(&self) -> (i32, i32) {
        self.repeat_rate.get()
    }

    pub fn set_rate(&self, rate: i32, delay: i32) {
        self.repeat_rate.set((rate, delay));
        let bindings = self.bindings.borrow_mut();
        for client in bindings.values() {
            for seat in client.values() {
                if seat.version >= REPEAT_INFO_SINCE {
                    let kbs = seat.keyboards.lock();
                    for kb in kbs.values() {
                        kb.send_repeat_info(rate, delay);
                    }
                }
            }
        }
    }

    pub fn close(self: &Rc<Self>) {
        let kb_node = self.keyboard_node.get();
        if let Some(tl) = kb_node.node_toplevel() {
            tl.tl_close();
        }
    }

    pub fn move_focus(self: &Rc<Self>, direction: Direction) {
        let tl = match self.keyboard_node.get().node_toplevel() {
            Some(tl) => tl,
            _ => return,
        };
        if direction == Direction::Down && tl.node_is_container() {
            tl.node_do_focus(self, direction);
        } else if let Some(p) = tl.tl_data().parent.get() {
            if let Some(c) = p.node_into_container() {
                c.move_focus_from_child(self, tl.deref(), direction);
            }
        }
    }

    pub fn move_focused(self: &Rc<Self>, direction: Direction) {
        let kb_node = self.keyboard_node.get();
        if let Some(tl) = kb_node.node_toplevel() {
            if let Some(parent) = tl.tl_data().parent.get() {
                if let Some(c) = parent.node_into_container() {
                    c.move_child(tl, direction);
                }
            }
        }
    }

    fn set_selection_<T: ipc::IpcVtable>(
        self: &Rc<Self>,
        field: &CloneCell<Option<Rc<T::Source>>>,
        src: Option<Rc<T::Source>>,
    ) -> Result<(), WlSeatError> {
        if let Some(new) = &src {
            ipc::attach_seat::<T>(new, self, ipc::Role::Selection)?;
        }
        if let Some(old) = field.set(src.clone()) {
            ipc::detach_seat::<T>(&old);
        }
        if let Some(client) = self.keyboard_node.get().node_client() {
            match src {
                Some(src) => ipc::offer_source_to::<T>(&src, &client),
                _ => T::for_each_device(self, client.id, |device| {
                    T::send_selection(device, None);
                }),
            }
            // client.flush();
        }
        Ok(())
    }

    pub fn start_drag(
        self: &Rc<Self>,
        origin: &Rc<WlSurface>,
        source: Option<Rc<WlDataSource>>,
        icon: Option<Rc<WlSurface>>,
        serial: u32,
    ) -> Result<(), WlSeatError> {
        if let Some(icon) = &icon {
            icon.set_output(&self.output.get());
        }
        self.pointer_owner
            .start_drag(self, origin, source, icon, serial)
    }

    pub fn cancel_dnd(self: &Rc<Self>) {
        self.pointer_owner.cancel_dnd(self);
    }

    pub fn unset_selection(self: &Rc<Self>) {
        let _ = self.set_selection(None, None);
    }

    pub fn set_selection(
        self: &Rc<Self>,
        selection: Option<Rc<WlDataSource>>,
        serial: Option<u32>,
    ) -> Result<(), WlSeatError> {
        if let Some(serial) = serial {
            self.selection_serial.set(serial);
        }
        self.set_selection_::<ClipboardIpc>(&self.selection, selection)
    }

    pub fn may_modify_selection(&self, client: &Rc<Client>, serial: u32) -> bool {
        let dist = serial.wrapping_sub(self.selection_serial.get()) as i32;
        if dist < 0 {
            return false;
        }
        self.keyboard_node.get().node_client_id() == Some(client.id)
    }

    pub fn may_modify_primary_selection(&self, client: &Rc<Client>, serial: Option<u32>) -> bool {
        if let Some(serial) = serial {
            let dist = serial.wrapping_sub(self.primary_selection_serial.get()) as i32;
            if dist < 0 {
                return false;
            }
        }
        self.keyboard_node.get().node_client_id() == Some(client.id)
            || self.pointer_node().and_then(|n| n.node_client_id()) == Some(client.id)
    }

    pub fn unset_primary_selection(self: &Rc<Self>) {
        let _ = self.set_primary_selection(None, None);
    }

    pub fn set_primary_selection(
        self: &Rc<Self>,
        selection: Option<Rc<ZwpPrimarySelectionSourceV1>>,
        serial: Option<u32>,
    ) -> Result<(), WlSeatError> {
        if let Some(serial) = serial {
            self.primary_selection_serial.set(serial);
        }
        self.set_selection_::<PrimarySelectionIpc>(&self.primary_selection, selection)
    }

    pub fn reload_known_cursor(&self) {
        if let Some(kc) = self.desired_known_cursor.get() {
            self.set_known_cursor(kc);
        }
    }

    pub fn set_known_cursor(&self, cursor: KnownCursor) {
        self.desired_known_cursor.set(Some(cursor));
        let cursors = match self.state.cursors.get() {
            Some(c) => c,
            None => {
                self.set_cursor2(None);
                return;
            }
        };
        let tpl = match cursor {
            KnownCursor::Default => &cursors.default,
            KnownCursor::Pointer => &cursors.pointer,
            KnownCursor::ResizeLeftRight => &cursors.resize_left_right,
            KnownCursor::ResizeTopBottom => &cursors.resize_top_bottom,
            KnownCursor::ResizeTopLeft => &cursors.resize_top_left,
            KnownCursor::ResizeTopRight => &cursors.resize_top_right,
            KnownCursor::ResizeBottomLeft => &cursors.resize_bottom_left,
            KnownCursor::ResizeBottomRight => &cursors.resize_bottom_right,
        };
        self.set_cursor2(Some(tpl.instantiate(self.cursor_size.get())));
    }

    pub fn set_app_cursor(&self, cursor: Option<Rc<dyn Cursor>>) {
        self.set_cursor2(cursor);
        self.desired_known_cursor.set(None);
    }

    fn set_cursor2(&self, cursor: Option<Rc<dyn Cursor>>) {
        if let Some(old) = self.cursor.get() {
            if let Some(new) = cursor.as_ref() {
                if rc_eq(&old, new) {
                    return;
                }
            }
            old.handle_unset();
        }
        if let Some(cursor) = cursor.as_ref() {
            cursor.set_output(&self.output.get());
        }
        self.cursor.set(cursor.clone());
        self.state.hardware_tick_cursor.push(cursor);
        self.update_hardware_cursor();
    }

    pub fn dnd_icon(&self) -> Option<Rc<WlSurface>> {
        self.pointer_owner.dnd_icon()
    }

    pub fn remove_dnd_icon(&self) {
        self.pointer_owner.remove_dnd_icon();
    }

    pub fn get_position(&self) -> (Fixed, Fixed) {
        self.pos.get()
    }

    pub fn get_cursor(&self) -> Option<Rc<dyn Cursor>> {
        self.cursor.get()
    }

    pub fn clear(self: &Rc<Self>) {
        mem::take(self.pointer_stack.borrow_mut().deref_mut());
        mem::take(self.found_tree.borrow_mut().deref_mut());
        self.keyboard_node.set(self.state.root.clone());
        self.state
            .root
            .clone()
            .node_visit(&mut generic_node_visitor(|node| {
                node.node_seat_state().on_seat_remove(self);
            }));
        self.bindings.borrow_mut().clear();
        self.data_devices.borrow_mut().clear();
        self.primary_selection_devices.borrow_mut().clear();
        self.cursor.set(None);
        self.selection.set(None);
        self.primary_selection.set(None);
        self.pointer_owner.clear();
        self.kb_owner.clear();
        *self.dropped_dnd.borrow_mut() = None;
        self.queue_link.set(None);
        self.tree_changed_handler.set(None);
        self.output.set(self.state.dummy_output.get().unwrap());
        self.constraint.take();
    }

    pub fn id(&self) -> SeatId {
        self.id
    }

    pub fn seat_name(&self) -> &str {
        &self.seat_name
    }

    fn bind_(
        self: Rc<Self>,
        id: WlSeatId,
        client: &Rc<Client>,
        version: u32,
    ) -> Result<(), WlSeatError> {
        let obj = Rc::new(WlSeat {
            global: self.clone(),
            id,
            client: client.clone(),
            pointers: Default::default(),
            relative_pointers: Default::default(),
            keyboards: Default::default(),
            version,
            tracker: Default::default(),
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        obj.send_capabilities();
        if version >= SEAT_NAME_SINCE {
            obj.send_name(&self.seat_name);
        }
        {
            let mut bindings = self.bindings.borrow_mut();
            let bindings = bindings.entry(client.id).or_insert_with(Default::default);
            bindings.insert(id, obj.clone());
        }
        Ok(())
    }
}

global_base!(WlSeatGlobal, WlSeat, WlSeatError);

impl Global for WlSeatGlobal {
    fn singleton(&self) -> bool {
        false
    }

    fn version(&self) -> u32 {
        8
    }

    fn break_loops(&self) {
        self.bindings.borrow_mut().clear();
        self.queue_link.take();
        self.tree_changed_handler.take();
    }
}

dedicated_add_global!(WlSeatGlobal, seats);

pub struct WlSeat {
    pub global: Rc<WlSeatGlobal>,
    pub id: WlSeatId,
    pub client: Rc<Client>,
    pointers: CopyHashMap<WlPointerId, Rc<WlPointer>>,
    relative_pointers: CopyHashMap<ZwpRelativePointerV1Id, Rc<ZwpRelativePointerV1>>,
    keyboards: CopyHashMap<WlKeyboardId, Rc<WlKeyboard>>,
    version: u32,
    tracker: Tracker<Self>,
}

const READ_ONLY_KEYMAP_SINCE: u32 = 7;

impl WlSeat {
    fn send_capabilities(self: &Rc<Self>) {
        self.client.event(Capabilities {
            self_id: self.id,
            capabilities: POINTER | KEYBOARD,
        })
    }

    fn send_name(self: &Rc<Self>, name: &str) {
        self.client.event(Name {
            self_id: self.id,
            name,
        })
    }

    pub fn move_(&self, node: &Rc<FloatNode>) {
        self.global.move_(node);
    }

    fn get_pointer(self: &Rc<Self>, parser: MsgParser<'_, '_>) -> Result<(), WlSeatError> {
        let req: GetPointer = self.client.parse(&**self, parser)?;
        let p = Rc::new(WlPointer::new(req.id, self));
        track!(self.client, p);
        self.client.add_client_obj(&p)?;
        self.pointers.set(req.id, p);
        Ok(())
    }

    fn get_keyboard(self: &Rc<Self>, parser: MsgParser<'_, '_>) -> Result<(), WlSeatError> {
        let req: GetKeyboard = self.client.parse(&**self, parser)?;
        let p = Rc::new(WlKeyboard::new(req.id, self));
        track!(self.client, p);
        self.client.add_client_obj(&p)?;
        self.keyboards.set(req.id, p.clone());
        let keymap = self.global.kb_map.get();
        p.send_keymap(
            wl_keyboard::XKB_V1,
            self.keymap_fd(&keymap)?,
            keymap.map_len as _,
        );
        if self.version >= REPEAT_INFO_SINCE {
            let (rate, delay) = self.global.repeat_rate.get();
            p.send_repeat_info(rate, delay);
        }
        Ok(())
    }

    pub fn keymap_fd(&self, keymap: &XkbKeymap) -> Result<Rc<OwnedFd>, WlKeyboardError> {
        if self.version >= READ_ONLY_KEYMAP_SINCE {
            return Ok(keymap.map.clone());
        }
        let fd = match uapi::memfd_create("shared-keymap", c::MFD_CLOEXEC) {
            Ok(fd) => fd,
            Err(e) => return Err(WlKeyboardError::KeymapMemfd(e.into())),
        };
        let target = keymap.map_len as c::off_t;
        let mut pos = 0;
        while pos < target {
            let rem = target - pos;
            let res = uapi::sendfile(fd.raw(), keymap.map.raw(), Some(&mut pos), rem as usize);
            match res {
                Ok(_) | Err(Errno(c::EINTR)) => {}
                Err(e) => return Err(WlKeyboardError::KeymapCopy(e.into())),
            }
        }
        Ok(Rc::new(fd))
    }

    fn get_touch(self: &Rc<Self>, parser: MsgParser<'_, '_>) -> Result<(), WlSeatError> {
        let req: GetTouch = self.client.parse(&**self, parser)?;
        let p = Rc::new(WlTouch::new(req.id, self));
        track!(self.client, p);
        self.client.add_client_obj(&p)?;
        Ok(())
    }

    fn release(&self, parser: MsgParser<'_, '_>) -> Result<(), WlSeatError> {
        let _req: Release = self.client.parse(self, parser)?;
        {
            let mut bindings = self.global.bindings.borrow_mut();
            if let Entry::Occupied(mut hm) = bindings.entry(self.client.id) {
                hm.get_mut().remove(&self.id);
                if hm.get().is_empty() {
                    hm.remove();
                }
            }
        }
        self.client.remove_obj(self)?;
        Ok(())
    }
}

object_base! {
    WlSeat;

    GET_POINTER => get_pointer,
    GET_KEYBOARD => get_keyboard,
    GET_TOUCH => get_touch,
    RELEASE => release,
}

impl Object for WlSeat {
    fn num_requests(&self) -> u32 {
        if self.version < 5 {
            GET_TOUCH + 1
        } else {
            RELEASE + 1
        }
    }

    fn break_loops(&self) {
        {
            let mut bindings = self.global.bindings.borrow_mut();
            if let Entry::Occupied(mut hm) = bindings.entry(self.client.id) {
                hm.get_mut().remove(&self.id);
                if hm.get().is_empty() {
                    hm.remove();
                }
            }
        }
        self.pointers.clear();
        self.relative_pointers.clear();
        self.keyboards.clear();
    }
}

dedicated_add_obj!(WlSeat, WlSeatId, seats);

#[derive(Debug, Error)]
pub enum WlSeatError {
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error(transparent)]
    IpcError(#[from] IpcError),
    #[error("Parsing failed")]
    MsgParserError(#[source] Box<MsgParserError>),
    #[error(transparent)]
    WlKeyboardError(Box<WlKeyboardError>),
}
efrom!(WlSeatError, ClientError);
efrom!(WlSeatError, MsgParserError);
efrom!(WlSeatError, WlKeyboardError);

pub fn collect_kb_foci2(node: Rc<dyn Node>, seats: &mut SmallVec<[Rc<WlSeatGlobal>; 3]>) {
    node.node_visit(&mut generic_node_visitor(|node| {
        node.node_seat_state().for_each_kb_focus(|s| seats.push(s));
    }));
}

pub fn collect_kb_foci(node: Rc<dyn Node>) -> SmallVec<[Rc<WlSeatGlobal>; 3]> {
    let mut res = SmallVec::new();
    collect_kb_foci2(node, &mut res);
    res
}
