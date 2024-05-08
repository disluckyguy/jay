use {
    crate::{
        client::{Client, ClientError},
        cursor::KnownCursor,
        fixed::Fixed,
        ifs::{
            wl_seat::{tablet::TabletTool, NodeSeatState, WlSeatGlobal},
            wl_surface::xdg_surface::{XdgSurface, XdgSurfaceError, XdgSurfaceExt},
            xdg_positioner::{
                XdgPositioned, XdgPositioner, CA_FLIP_X, CA_FLIP_Y, CA_RESIZE_X, CA_RESIZE_Y,
                CA_SLIDE_X, CA_SLIDE_Y,
            },
        },
        leaks::Tracker,
        object::Object,
        rect::Rect,
        renderer::Renderer,
        tree::{
            FindTreeResult, FindTreeUsecase, FoundNode, Node, NodeId, NodeVisitor, StackedNode,
            WorkspaceNode,
        },
        utils::{clonecell::CloneCell, linkedlist::LinkedNode},
        wire::{xdg_popup::*, XdgPopupId},
    },
    std::{
        cell::{Cell, RefCell},
        fmt::{Debug, Formatter},
        rc::Rc,
    },
    thiserror::Error,
};

#[allow(dead_code)]
const INVALID_GRAB: u32 = 1;

tree_id!(PopupId);

pub struct XdgPopup {
    id: XdgPopupId,
    node_id: PopupId,
    pub xdg: Rc<XdgSurface>,
    pub(super) parent: CloneCell<Option<Rc<XdgSurface>>>,
    relative_position: Cell<Rect>,
    display_link: RefCell<Option<LinkedNode<Rc<dyn StackedNode>>>>,
    workspace_link: RefCell<Option<LinkedNode<Rc<dyn StackedNode>>>>,
    pos: RefCell<XdgPositioned>,
    pub tracker: Tracker<Self>,
    seat_state: NodeSeatState,
    set_visible_prepared: Cell<bool>,
}

impl Debug for XdgPopup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XdgPopup").finish_non_exhaustive()
    }
}

impl XdgPopup {
    pub fn new(
        id: XdgPopupId,
        xdg: &Rc<XdgSurface>,
        parent: Option<&Rc<XdgSurface>>,
        pos: &Rc<XdgPositioner>,
    ) -> Result<Self, XdgPopupError> {
        let pos = pos.value();
        if !pos.is_complete() {
            return Err(XdgPopupError::Incomplete);
        }
        Ok(Self {
            id,
            node_id: xdg.surface.client.state.node_ids.next(),
            xdg: xdg.clone(),
            parent: CloneCell::new(parent.cloned()),
            relative_position: Cell::new(Default::default()),
            display_link: RefCell::new(None),
            workspace_link: RefCell::new(None),
            pos: RefCell::new(pos),
            tracker: Default::default(),
            seat_state: Default::default(),
            set_visible_prepared: Cell::new(false),
        })
    }

    fn send_configure(&self, x: i32, y: i32, width: i32, height: i32) {
        self.xdg.surface.client.event(Configure {
            self_id: self.id,
            x,
            y,
            width,
            height,
        })
    }

    fn send_repositioned(&self, token: u32) {
        self.xdg.surface.client.event(Repositioned {
            self_id: self.id,
            token,
        })
    }

    fn send_popup_done(&self) {
        self.xdg
            .surface
            .client
            .event(PopupDone { self_id: self.id })
    }

    fn update_position(&self, parent: &XdgSurface) {
        let positioner = self.pos.borrow_mut();
        let parent_abs = parent.absolute_desired_extents.get();
        let mut rel_pos = positioner.get_position(false, false);
        let mut abs_pos = rel_pos.move_(parent_abs.x1(), parent_abs.y1());
        if let Some(ws) = parent.workspace.get() {
            let output_pos = ws.output.get().global.pos.get();
            let mut overflow = output_pos.get_overflow(&abs_pos);
            if !overflow.is_contained() {
                let mut flip_x = positioner.ca.contains(CA_FLIP_X) && overflow.x_overflow();
                let mut flip_y = positioner.ca.contains(CA_FLIP_Y) && overflow.y_overflow();
                if flip_x || flip_y {
                    let mut adj_rel = positioner.get_position(flip_x, flip_y);
                    let mut adj_abs = adj_rel.move_(parent_abs.x1(), parent_abs.y1());
                    let mut adj_overflow = output_pos.get_overflow(&adj_abs);
                    let mut recalculate = false;
                    if flip_x && adj_overflow.x_overflow() {
                        flip_x = false;
                        recalculate = true;
                    }
                    if flip_y && adj_overflow.y_overflow() {
                        flip_y = false;
                        recalculate = true;
                    }
                    if flip_x || flip_y {
                        if recalculate {
                            adj_rel = positioner.get_position(flip_x, flip_y);
                            adj_abs = adj_rel.move_(parent_abs.x1(), parent_abs.y1());
                            adj_overflow = output_pos.get_overflow(&adj_abs);
                        }
                        rel_pos = adj_rel;
                        abs_pos = adj_abs;
                        overflow = adj_overflow;
                    }
                }
                let (mut dx, mut dy) = (0, 0);
                if positioner.ca.contains(CA_SLIDE_X) && overflow.x_overflow() {
                    dx = if overflow.left > 0 || overflow.left + overflow.right > 0 {
                        parent_abs.x1() - abs_pos.x1()
                    } else {
                        parent_abs.x2() - abs_pos.x2()
                    };
                }
                if positioner.ca.contains(CA_SLIDE_Y) && overflow.y_overflow() {
                    dy = if overflow.top > 0 || overflow.top + overflow.bottom > 0 {
                        parent_abs.y1() - abs_pos.y1()
                    } else {
                        parent_abs.y2() - abs_pos.y2()
                    };
                }
                if dx != 0 || dy != 0 {
                    rel_pos = rel_pos.move_(dx, dy);
                    abs_pos = rel_pos.move_(parent_abs.x1(), parent_abs.y1());
                    overflow = output_pos.get_overflow(&abs_pos);
                }
                let (mut dx1, mut dx2, mut dy1, mut dy2) = (0, 0, 0, 0);
                if positioner.ca.contains(CA_RESIZE_X) {
                    dx1 = overflow.left.max(0);
                    dx2 = -overflow.right.max(0);
                }
                if positioner.ca.contains(CA_RESIZE_Y) {
                    dy1 = overflow.top.max(0);
                    dy2 = -overflow.bottom.max(0);
                }
                if dx1 > 0 || dx2 < 0 || dy1 > 0 || dy2 < 0 {
                    let maybe_abs_pos = Rect::new(
                        abs_pos.x1() + dx1,
                        abs_pos.y1() + dy1,
                        abs_pos.x2() + dx2,
                        abs_pos.y2() + dy2,
                    );
                    // If the popup is completely outside the output, this will fail. Just
                    // use its position as is.
                    if let Some(maybe_abs_pos) = maybe_abs_pos {
                        abs_pos = maybe_abs_pos;
                        rel_pos = Rect::new_sized(
                            abs_pos.x1() - parent_abs.x1(),
                            abs_pos.y1() - parent_abs.y1(),
                            abs_pos.width(),
                            abs_pos.height(),
                        )
                        .unwrap();
                    }
                }
            }
        }
        self.relative_position.set(rel_pos);
        self.xdg.set_absolute_desired_extents(&abs_pos);
    }

    pub fn update_absolute_position(&self) {
        if let Some(parent) = self.parent.get() {
            let rel = self.relative_position.get();
            let parent = parent.absolute_desired_extents.get();
            self.xdg
                .set_absolute_desired_extents(&rel.move_(parent.x1(), parent.y1()));
        }
    }
}

impl XdgPopupRequestHandler for XdgPopup {
    type Error = XdgPopupError;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.destroy_node();
        {
            if let Some(parent) = self.parent.take() {
                parent.popups.remove(&self.id);
            }
        }
        self.xdg.ext.set(None);
        self.xdg.surface.client.remove_obj(self)?;
        *self.display_link.borrow_mut() = None;
        *self.workspace_link.borrow_mut() = None;
        Ok(())
    }

    fn grab(&self, _req: Grab, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn reposition(&self, req: Reposition, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        *self.pos.borrow_mut() = self.xdg.surface.client.lookup(req.positioner)?.value();
        if let Some(parent) = self.parent.get() {
            self.update_position(&parent);
            let rel = self.relative_position.get();
            self.send_repositioned(req.token);
            self.send_configure(rel.x1(), rel.y1(), rel.width(), rel.height());
            self.xdg.do_send_configure();
        }
        Ok(())
    }
}

impl XdgPopup {
    fn get_parent_workspace(&self) -> Option<Rc<WorkspaceNode>> {
        self.parent.get()?.workspace.get()
    }

    pub fn set_visible(&self, visible: bool) {
        // log::info!("set visible = {}", visible);
        self.set_visible_prepared.set(false);
        self.xdg.set_visible(visible);
        self.seat_state.set_visible(self, visible);
    }

    pub fn destroy_node(&self) {
        let _v = self.display_link.borrow_mut().take();
        let _v = self.workspace_link.borrow_mut().take();
        self.xdg.destroy_node();
        self.seat_state.destroy_node(self);
    }

    pub fn detach_node(&self) {
        let _v = self.workspace_link.borrow_mut().take();
        self.xdg.detach_node();
        self.seat_state.destroy_node(self);
    }

    pub fn restack(&self) {
        let state = &self.xdg.surface.client.state;
        let dl = self.display_link.borrow();
        if let Some(dl) = &*dl {
            state.root.stacked.add_last_existing(dl);
        }
        self.xdg.restack_popups();
        state.tree_changed();
    }
}

object_base! {
    self = XdgPopup;
    version = self.xdg.base.version;
}

impl Object for XdgPopup {
    fn break_loops(&self) {
        self.destroy_node();
        self.parent.set(None);
        *self.display_link.borrow_mut() = None;
        *self.workspace_link.borrow_mut() = None;
    }
}

simple_add_obj!(XdgPopup);

impl Node for XdgPopup {
    fn node_id(&self) -> NodeId {
        self.node_id.into()
    }

    fn node_seat_state(&self) -> &NodeSeatState {
        &self.seat_state
    }

    fn node_visit(self: Rc<Self>, visitor: &mut dyn NodeVisitor) {
        visitor.visit_popup(&self);
    }

    fn node_visit_children(&self, visitor: &mut dyn NodeVisitor) {
        visitor.visit_surface(&self.xdg.surface);
    }

    fn node_visible(&self) -> bool {
        self.xdg.surface.visible.get()
    }

    fn node_absolute_position(&self) -> Rect {
        self.xdg.absolute_desired_extents.get()
    }

    fn node_find_tree_at(
        &self,
        x: i32,
        y: i32,
        tree: &mut Vec<FoundNode>,
        usecase: FindTreeUsecase,
    ) -> FindTreeResult {
        if usecase == FindTreeUsecase::SelectToplevel {
            return FindTreeResult::Other;
        }
        self.xdg.find_tree_at(x, y, tree)
    }

    fn node_render(&self, renderer: &mut Renderer, x: i32, y: i32, bounds: Option<&Rect>) {
        renderer.render_xdg_surface(&self.xdg, x, y, bounds)
    }

    fn node_client(&self) -> Option<Rc<Client>> {
        Some(self.xdg.surface.client.clone())
    }

    fn node_on_pointer_enter(self: Rc<Self>, seat: &Rc<WlSeatGlobal>, _x: Fixed, _y: Fixed) {
        seat.enter_popup(&self);
    }

    fn node_on_pointer_focus(&self, seat: &Rc<WlSeatGlobal>) {
        // log::info!("xdg-popup focus");
        seat.pointer_cursor().set_known(KnownCursor::Default);
    }

    fn node_on_tablet_tool_enter(
        self: Rc<Self>,
        tool: &Rc<TabletTool>,
        _time_usec: u64,
        _x: Fixed,
        _y: Fixed,
    ) {
        tool.cursor().set_known(KnownCursor::Default)
    }
}

impl StackedNode for XdgPopup {
    stacked_node_impl!();

    fn stacked_prepare_set_visible(&self) {
        self.set_visible_prepared.set(true);
    }

    fn stacked_needs_set_visible(&self) -> bool {
        self.set_visible_prepared.get()
    }

    fn stacked_set_visible(&self, visible: bool) {
        self.set_visible(visible);
    }

    fn stacked_has_workspace_link(&self) -> bool {
        self.workspace_link.borrow().is_some()
    }

    fn stacked_absolute_position_constrains_input(&self) -> bool {
        false
    }
}

impl XdgSurfaceExt for XdgPopup {
    fn initial_configure(self: Rc<Self>) -> Result<(), XdgSurfaceError> {
        if let Some(parent) = self.parent.get() {
            self.update_position(&parent);
            let rel = self.relative_position.get();
            self.send_configure(rel.x1(), rel.y1(), rel.width(), rel.height());
        }
        Ok(())
    }

    fn post_commit(self: Rc<Self>) {
        let mut wl = self.workspace_link.borrow_mut();
        let mut dl = self.display_link.borrow_mut();
        let ws = match self.get_parent_workspace() {
            Some(ws) => ws,
            _ => {
                log::info!("no ws");
                return;
            }
        };
        let surface = &self.xdg.surface;
        let state = &surface.client.state;
        if surface.buffer.is_some() {
            if wl.is_none() {
                self.xdg.set_workspace(&ws);
                *wl = Some(ws.stacked.add_last(self.clone()));
                *dl = Some(state.root.stacked.add_last(self.clone()));
                state.tree_changed();
                self.set_visible(
                    self.parent
                        .get()
                        .map(|p| p.surface.visible.get())
                        .unwrap_or(false),
                );
            }
        } else {
            if wl.take().is_some() {
                drop(wl);
                drop(dl);
                self.set_visible(false);
                self.destroy_node();
                self.send_popup_done();
            }
        }
    }

    fn extents_changed(&self) {
        self.xdg.surface.client.state.tree_changed();
    }
}

#[derive(Debug, Error)]
pub enum XdgPopupError {
    #[error("The `xdg_positioner` is incomplete")]
    Incomplete,
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(XdgPopupError, ClientError);
