use {
    crate::{
        acceptor::Acceptor,
        async_engine::{AsyncEngine, SpawnedFuture},
        backend::{
            Backend, BackendEvent, Connector, ConnectorId, ConnectorIds, InputDevice,
            InputDeviceId, InputDeviceIds, MonitorInfo,
        },
        backends::dummy::DummyBackend,
        cli::RunArgs,
        client::{Client, Clients, SerialRange, NUM_CACHED_SERIAL_RANGES},
        config::ConfigProxy,
        cursor::ServerCursors,
        dbus::Dbus,
        event_loop::EventLoop,
        forker::ForkerProxy,
        globals::{Globals, GlobalsError, WaylandGlobal},
        ifs::{
            wl_seat::{SeatIds, WlSeatGlobal},
            wl_surface::{
                zwp_idle_inhibitor_v1::{IdleInhibitorId, IdleInhibitorIds, ZwpIdleInhibitorV1},
                NoneSurfaceExt,
            },
        },
        leaks::Tracker,
        logger::Logger,
        rect::Rect,
        render::RenderContext,
        theme::Theme,
        tree::{
            ContainerNode, ContainerSplit, DisplayNode, FloatNode, Node, NodeIds, NodeVisitorBase,
            OutputNode, PlaceholderNode, ToplevelNode, WorkspaceNode,
        },
        utils::{
            asyncevent::AsyncEvent, clonecell::CloneCell, copyhashmap::CopyHashMap,
            errorfmt::ErrorFmt, fdcloser::FdCloser, linkedlist::LinkedList, numcell::NumCell,
            queue::AsyncQueue, run_toplevel::RunToplevel,
        },
        wheel::Wheel,
        xkbcommon::{XkbContext, XkbKeymap},
        xwayland::{self, XWaylandEvent},
    },
    ahash::AHashMap,
    jay_config::Direction,
    std::{
        cell::{Cell, RefCell},
        fmt::{Debug, Formatter},
        mem,
        num::Wrapping,
        ops::DerefMut,
        rc::Rc,
        sync::Arc,
        time::Duration,
    },
};
use crate::ifs::wl_surface::WlSurface;

pub struct State {
    pub xkb_ctx: XkbContext,
    pub backend: CloneCell<Rc<dyn Backend>>,
    pub forker: CloneCell<Option<Rc<ForkerProxy>>>,
    pub default_keymap: Rc<XkbKeymap>,
    pub eng: Rc<AsyncEngine>,
    pub el: Rc<EventLoop>,
    pub render_ctx: CloneCell<Option<Rc<RenderContext>>>,
    pub render_ctx_version: NumCell<u32>,
    pub cursors: CloneCell<Option<Rc<ServerCursors>>>,
    pub wheel: Rc<Wheel>,
    pub clients: Clients,
    pub globals: Globals,
    pub connector_ids: ConnectorIds,
    pub seat_ids: SeatIds,
    pub idle_inhibitor_ids: IdleInhibitorIds,
    pub input_device_ids: InputDeviceIds,
    pub node_ids: NodeIds,
    pub root: Rc<DisplayNode>,
    pub workspaces: CopyHashMap<String, Rc<WorkspaceNode>>,
    pub dummy_output: CloneCell<Option<Rc<OutputNode>>>,
    pub backend_events: AsyncQueue<BackendEvent>,
    pub input_device_handlers: RefCell<AHashMap<InputDeviceId, InputDeviceData>>,
    pub seat_queue: LinkedList<Rc<WlSeatGlobal>>,
    pub slow_clients: AsyncQueue<Rc<Client>>,
    pub none_surface_ext: Rc<NoneSurfaceExt>,
    pub tree_changed_sent: Cell<bool>,
    pub config: CloneCell<Option<Rc<ConfigProxy>>>,
    pub theme: Theme,
    pub pending_container_layout: AsyncQueue<Rc<ContainerNode>>,
    pub pending_container_render_data: AsyncQueue<Rc<ContainerNode>>,
    pub pending_float_layout: AsyncQueue<Rc<FloatNode>>,
    pub pending_float_titles: AsyncQueue<Rc<FloatNode>>,
    pub dbus: Dbus,
    pub fdcloser: Arc<FdCloser>,
    pub logger: Option<Arc<Logger>>,
    pub connectors: CopyHashMap<ConnectorId, Rc<ConnectorData>>,
    pub outputs: CopyHashMap<ConnectorId, Rc<OutputData>>,
    pub status: CloneCell<Rc<String>>,
    pub idle: IdleState,
    pub run_args: RunArgs,
    pub xwayland: XWaylandState,
    pub acceptor: CloneCell<Option<Rc<Acceptor>>>,
    pub serial: NumCell<Wrapping<u32>>,
    pub run_toplevel: Rc<RunToplevel>,
    pub config_dir: Option<String>,
    pub config_file_id: NumCell<u64>,
    pub tracker: Tracker<Self>,
}

// impl Drop for State {
//     fn drop(&mut self) {
//         log::info!("drop state");
//     }
// }

impl Debug for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").finish_non_exhaustive()
    }
}

pub struct XWaylandState {
    pub enabled: Cell<bool>,
    pub handler: RefCell<Option<SpawnedFuture<()>>>,
    pub queue: Rc<AsyncQueue<XWaylandEvent>>,
}

pub struct IdleState {
    pub input: Cell<bool>,
    pub change: AsyncEvent,
    pub timeout: Cell<Duration>,
    pub timeout_changed: Cell<bool>,
    pub inhibitors: CopyHashMap<IdleInhibitorId, Rc<ZwpIdleInhibitorV1>>,
    pub inhibitors_changed: Cell<bool>,
}

impl IdleState {
    pub fn set_timeout(&self, timeout: Duration) {
        self.timeout.set(timeout);
        self.timeout_changed.set(true);
        self.change.trigger();
    }

    pub fn add_inhibitor(&self, inhibitor: &Rc<ZwpIdleInhibitorV1>) {
        self.inhibitors.set(inhibitor.inhibit_id, inhibitor.clone());
        self.inhibitors_changed.set(true);
        self.change.trigger();
    }

    pub fn remove_inhibitor(&self, inhibitor: &ZwpIdleInhibitorV1) {
        self.inhibitors.remove(&inhibitor.inhibit_id);
        self.inhibitors_changed.set(true);
        self.change.trigger();
    }
}

pub struct InputDeviceData {
    pub handler: SpawnedFuture<()>,
    pub id: InputDeviceId,
    pub data: Rc<DeviceHandlerData>,
}

pub struct DeviceHandlerData {
    pub seat: CloneCell<Option<Rc<WlSeatGlobal>>>,
    pub device: Rc<dyn InputDevice>,
}

pub struct ConnectorData {
    pub connector: Rc<dyn Connector>,
    pub handler: Cell<Option<SpawnedFuture<()>>>,
    pub connected: Cell<bool>,
    pub name: String,
}

pub struct OutputData {
    pub connector: Rc<ConnectorData>,
    pub monitor_info: MonitorInfo,
    pub node: Rc<OutputNode>,
}

impl State {
    pub fn set_render_ctx(&self, ctx: Option<&Rc<RenderContext>>) {
        self.render_ctx.set(ctx.cloned());
        self.render_ctx_version.fetch_add(1);

        {
            struct Walker;
            impl NodeVisitorBase for Walker {
                fn visit_container(&mut self, node: &Rc<ContainerNode>) {
                    node.render_data.borrow_mut().titles.clear();
                    node.node_visit_children(self);
                }
                fn visit_output(&mut self, node: &Rc<OutputNode>) {
                    node.render_data.borrow_mut().titles.clear();
                    node.render_data.borrow_mut().status.take();
                    node.node_visit_children(self);
                }
                fn visit_float(&mut self, node: &Rc<FloatNode>) {
                    node.title_texture.set(None);
                    node.node_visit_children(self);
                }
                fn visit_placeholder(&mut self, node: &Rc<PlaceholderNode>) {
                    node.texture.set(None);
                    node.node_visit_children(self);
                }
                fn visit_surface(&mut self, node: &Rc<WlSurface>) {
                    if let Some(buffer) = node.buffer.get() {
                        buffer.handle_gfx_context_change();
                    }
                    node.node_visit_children(self);
                }
            }
            Walker.visit_display(&self.root);
            for client in self.clients.clients.borrow_mut().values() {
                for buffer in client.data.objects.buffers.lock().values() {
                    buffer.handle_gfx_context_change();
                }
            }
        }

        if let Some(ctx) = ctx {
            let cursors = match ServerCursors::load(ctx) {
                Ok(c) => Some(Rc::new(c)),
                Err(e) => {
                    log::error!("Could not load the cursors: {}", ErrorFmt(e));
                    None
                }
            };
            self.cursors.set(cursors);

            struct Walker;
            impl NodeVisitorBase for Walker {
                fn visit_container(&mut self, node: &Rc<ContainerNode>) {
                    node.schedule_compute_render_data();
                    node.node_visit_children(self);
                }
                fn visit_output(&mut self, node: &Rc<OutputNode>) {
                    node.update_render_data();
                    node.node_visit_children(self);
                }
                fn visit_float(&mut self, node: &Rc<FloatNode>) {
                    node.schedule_render_titles();
                    node.node_visit_children(self);
                }
                fn visit_placeholder(&mut self, node: &Rc<PlaceholderNode>) {
                    node.update_texture();
                    node.node_visit_children(self);
                }
            }
            Walker.visit_display(&self.root);
        }

        let seats = self.globals.seats.lock();
        for seat in seats.values() {
            seat.render_ctx_changed();
        }
    }

    pub fn add_global<T: WaylandGlobal>(&self, global: &Rc<T>) {
        self.globals.add_global(self, global)
    }

    pub fn remove_global<T: WaylandGlobal>(&self, global: &T) -> Result<(), GlobalsError> {
        self.globals.remove(self, global)
    }

    pub fn tree_changed(&self) {
        // log::info!("state.tree_changed\n{:?}", Backtrace::new());
        if self.tree_changed_sent.replace(true) {
            return;
        }
        let seats = self.globals.seats.lock();
        for seat in seats.values() {
            seat.trigger_tree_changed();
        }
    }

    pub fn map_tiled(self: &Rc<Self>, node: Rc<dyn ToplevelNode>) {
        let seat = self.seat_queue.last();
        self.do_map_tiled(seat.as_deref(), node.clone());
        if node.node_visible() {
            if let Some(seat) = seat {
                node.node_do_focus(&seat, Direction::Unspecified);
            }
        }
    }

    fn do_map_tiled(self: &Rc<Self>, seat: Option<&Rc<WlSeatGlobal>>, node: Rc<dyn ToplevelNode>) {
        let output = seat
            .map(|s| s.get_output())
            .or_else(|| self.root.outputs.lock().values().next().cloned())
            .or_else(|| self.dummy_output.get())
            .unwrap();
        let ws = output.ensure_workspace();
        self.map_tiled_on(node, &ws);
    }

    pub fn map_tiled_on(self: &Rc<Self>, node: Rc<dyn ToplevelNode>, ws: &Rc<WorkspaceNode>) {
        if let Some(c) = ws.container.get() {
            let la = c.clone().tl_last_active_child();
            let lap = la
                .tl_data()
                .parent
                .get()
                .and_then(|n| n.node_into_container());
            if let Some(lap) = lap {
                lap.add_child_after(la.tl_as_node(), node);
            } else {
                c.append_child(node);
            }
        } else {
            let container =
                ContainerNode::new(self, &ws, ws.clone(), node, ContainerSplit::Horizontal);
            ws.set_container(&container);
        }
    }

    pub fn map_floating(
        self: &Rc<Self>,
        node: Rc<dyn ToplevelNode>,
        mut width: i32,
        mut height: i32,
        workspace: &Rc<WorkspaceNode>,
    ) {
        node.clone().tl_set_workspace(workspace);
        width += 2 * self.theme.border_width.get();
        height += 2 * self.theme.border_width.get() + self.theme.title_height.get();
        let output = workspace.output.get();
        let output_rect = output.global.pos.get();
        let position = {
            let mut x1 = output_rect.x1();
            let mut y1 = output_rect.y1();
            if width < output_rect.width() {
                x1 += (output_rect.width() - width) as i32 / 2;
            } else {
                width = output_rect.width();
            }
            if height < output_rect.height() {
                y1 += (output_rect.height() - height) as i32 / 2;
            } else {
                height = output_rect.height();
            }
            Rect::new_sized(x1, y1, width, height).unwrap()
        };
        FloatNode::new(self, workspace, position, node);
    }

    pub fn show_workspace(&self, seat: &Rc<WlSeatGlobal>, name: &str) {
        let output = match self.workspaces.get(name) {
            Some(ws) => {
                let output = ws.output.get();
                let did_change = output.show_workspace(&ws);
                ws.node_do_focus(seat, Direction::Unspecified);
                if !did_change {
                    return;
                }
                output
            }
            _ => {
                let output = seat.get_output();
                if output.is_dummy {
                    log::warn!("Not showing workspace because seat is on dummy output");
                    return;
                }
                let workspace = Rc::new(WorkspaceNode {
                    id: self.node_ids.next(),
                    output: CloneCell::new(output.clone()),
                    position: Cell::new(Default::default()),
                    container: Default::default(),
                    stacked: Default::default(),
                    seat_state: Default::default(),
                    name: name.to_string(),
                    output_link: Cell::new(None),
                    visible: Cell::new(false),
                    fullscreen: Default::default(),
                });
                workspace
                    .output_link
                    .set(Some(output.workspaces.add_last(workspace.clone())));
                output.show_workspace(&workspace);
                self.workspaces.set(name.to_string(), workspace);
                output
            }
        };
        output.update_render_data();
        self.tree_changed();
        // let seats = self.globals.seats.lock();
        // for seat in seats.values() {
        //     seat.workspace_changed(&output);
        // }
    }

    pub fn float_map_ws(&self) -> Rc<WorkspaceNode> {
        if let Some(seat) = self.seat_queue.last() {
            let output = seat.get_output();
            if !output.is_dummy {
                return output.ensure_workspace();
            }
        }
        if let Some(output) = self.root.outputs.lock().values().cloned().next() {
            return output.ensure_workspace();
        }
        self.dummy_output.get().unwrap().ensure_workspace()
    }

    pub fn set_status(&self, status: &str) {
        let status = Rc::new(status.to_owned());
        self.status.set(status.clone());
        let outputs = self.root.outputs.lock();
        for output in outputs.values() {
            output.set_status(&status);
        }
    }

    pub fn input_occurred(&self) {
        if !self.idle.input.replace(true) {
            self.idle.change.trigger();
        }
    }

    pub fn start_xwayland(self: &Rc<Self>) {
        if !self.xwayland.enabled.get() {
            return;
        }
        let mut handler = self.xwayland.handler.borrow_mut();
        if handler.is_none() {
            *handler = Some(self.eng.spawn(xwayland::manage(self.clone())));
        }
    }

    pub fn next_serial(&self, client: Option<&Client>) -> u32 {
        let serial = self.serial.fetch_add(Wrapping(1)).0;
        if let Some(client) = client {
            'update_range: {
                let mut serials = client.serials.borrow_mut();
                if let Some(last) = serials.back_mut() {
                    if last.hi.wrapping_add(1) == serial {
                        last.hi = serial;
                        break 'update_range;
                    }
                }
                if serials.len() >= NUM_CACHED_SERIAL_RANGES {
                    serials.pop_front();
                }
                serials.push_back(SerialRange {
                    lo: serial,
                    hi: serial,
                });
            }
        }
        serial as _
    }

    pub fn damage(&self) {
        for connector in self.connectors.lock().values() {
            if connector.connected.get() {
                connector.connector.damage();
            }
        }
    }

    pub fn clear(&self) {
        self.xwayland.handler.borrow_mut().take();
        self.clients.clear();
        if let Some(config) = self.config.set(None) {
            config.clear();
        }
        if let Some(forker) = self.forker.set(None) {
            forker.clear();
        }
        if let Some(output) = self.dummy_output.set(None) {
            output.clear();
        }
        self.acceptor.set(None);
        self.backend.set(Rc::new(DummyBackend));
        self.run_toplevel.clear();
        self.xwayland.handler.borrow_mut().take();
        self.xwayland.queue.clear();
        self.idle.inhibitors.clear();
        for (_, connector) in self.connectors.lock().drain() {
            connector.handler.take();
        }
        for (_, output) in self.outputs.lock().drain() {
            output.node.clear();
        }
        self.dbus.clear();
        self.pending_container_layout.clear();
        self.pending_container_render_data.clear();
        self.pending_float_layout.clear();
        self.pending_float_titles.clear();
        self.slow_clients.clear();
        self.input_device_handlers.borrow_mut().clear();
        self.backend_events.clear();
        self.workspaces.clear();
        self.globals.clear();
        self.render_ctx.set(None);
        self.root.clear();
        {
            let seats = mem::take(self.globals.seats.lock().deref_mut());
            for seat in seats.values() {
                seat.clear();
            }
        }
    }
}
