use {
    crate::{
        client::{Client, ClientError},
        ifs::xdg_wm_base::XdgWmBase,
        leaks::Tracker,
        object::Object,
        rect::Rect,
        utils::buffd::{MsgParser, MsgParserError},
        wire::{xdg_positioner::*, XdgPositionerId},
    },
    std::{cell::RefCell, rc::Rc},
    thiserror::Error,
};

const INVALID_INPUT: u32 = 0;

const NONE: u32 = 0;
const TOP: u32 = 1;
const BOTTOM: u32 = 2;
const LEFT: u32 = 3;
const RIGHT: u32 = 4;
const TOP_LEFT: u32 = 5;
const BOTTOM_LEFT: u32 = 6;
const TOP_RIGHT: u32 = 7;
const BOTTOM_RIGHT: u32 = 8;

bitflags::bitflags! {
    #[derive(Default)]
    pub struct Edge: u32 {
        const TOP = 1 << 0;
        const BOTTOM = 1 << 1;
        const LEFT = 1 << 2;
        const RIGHT = 1 << 3;
    }
}

impl Edge {
    fn from_enum(e: u32) -> Option<Self> {
        let s = match e {
            NONE => Edge::empty(),
            TOP => Edge::TOP,
            BOTTOM => Edge::BOTTOM,
            LEFT => Edge::LEFT,
            RIGHT => Edge::RIGHT,
            TOP_LEFT => Edge::TOP | Edge::LEFT,
            BOTTOM_LEFT => Edge::BOTTOM | Edge::LEFT,
            TOP_RIGHT => Edge::TOP | Edge::RIGHT,
            BOTTOM_RIGHT => Edge::BOTTOM | Edge::RIGHT,
            _ => return None,
        };
        Some(s)
    }
}

bitflags::bitflags! {
    #[derive(Default)]
    pub struct CA: u32 {
        const NONE = 0;
        const SLIDE_X = 1;
        const SLIDE_Y = 2;
        const FLIP_X = 4;
        const FLIP_Y = 8;
        const RESIZE_X = 16;
        const RESIZE_Y = 32;
    }
}

pub struct XdgPositioner {
    id: XdgPositionerId,
    base: Rc<XdgWmBase>,
    client: Rc<Client>,
    position: RefCell<XdgPositioned>,
    pub tracker: Tracker<Self>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct XdgPositioned {
    pub size_width: i32,
    pub size_height: i32,
    pub ar: Rect,
    pub anchor: Edge,
    pub gravity: Edge,
    pub ca: CA,
    pub off_x: i32,
    pub off_y: i32,
    pub reactive: bool,
    pub parent_width: i32,
    pub parent_height: i32,
    pub parent_serial: u32,
}

impl XdgPositioned {
    pub fn is_complete(&self) -> bool {
        self.size_height != 0 && self.size_width != 0
    }

    pub fn get_position(&self, flip_x: bool, flip_y: bool) -> Rect {
        let mut anchor = self.anchor;
        let mut gravity = self.gravity;
        if flip_x {
            anchor ^= Edge::LEFT | Edge::RIGHT;
            gravity ^= Edge::LEFT | Edge::RIGHT;
        }
        if flip_y {
            anchor ^= Edge::TOP | Edge::BOTTOM;
            gravity ^= Edge::TOP | Edge::BOTTOM;
        }

        let mut x1 = self.off_x;
        let mut y1 = self.off_x;

        if anchor.contains(Edge::LEFT) {
            x1 += self.ar.x1();
        } else if anchor.contains(Edge::RIGHT) {
            x1 += self.ar.x2();
        } else {
            x1 += self.ar.x1() + self.ar.width() / 2;
        }

        if anchor.contains(Edge::TOP) {
            y1 += self.ar.y1();
        } else if anchor.contains(Edge::BOTTOM) {
            y1 += self.ar.y2();
        } else {
            y1 += self.ar.y1() + self.ar.height() / 2;
        }

        if gravity.contains(Edge::LEFT) {
            x1 -= self.size_width;
        } else if !gravity.contains(Edge::RIGHT) {
            x1 -= self.size_width / 2;
        }

        if gravity.contains(Edge::TOP) {
            y1 -= self.size_height;
        } else if !gravity.contains(Edge::BOTTOM) {
            y1 -= self.size_height / 2;
        }

        Rect::new_sized(x1, y1, self.size_width, self.size_height).unwrap()
    }
}

impl XdgPositioner {
    pub fn new(base: &Rc<XdgWmBase>, id: XdgPositionerId, client: &Rc<Client>) -> Self {
        Self {
            id,
            client: client.clone(),
            base: base.clone(),
            position: RefCell::new(Default::default()),
            tracker: Default::default(),
        }
    }

    pub fn value(&self) -> XdgPositioned {
        *self.position.borrow()
    }

    fn destroy(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let _req: Destroy = self.client.parse(self, parser)?;
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn set_size(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetSize = self.client.parse(self, parser)?;
        if req.width <= 0 || req.height <= 0 {
            self.client.protocol_error(
                self,
                INVALID_INPUT,
                &format!("Cannot set a non-positive size"),
            );
            return Err(XdgPositionerError::NonPositiveSize);
        }
        let mut position = self.position.borrow_mut();
        position.size_width = req.width;
        position.size_height = req.height;
        Ok(())
    }

    fn set_anchor_rect(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetAnchorRect = self.client.parse(self, parser)?;
        if req.width < 0 || req.height < 0 {
            self.client.protocol_error(
                self,
                INVALID_INPUT,
                &format!("Cannot set an anchor rect with negative size"),
            );
            return Err(XdgPositionerError::NegativeAnchorRect);
        }
        let mut position = self.position.borrow_mut();
        position.ar = Rect::new_sized(req.x, req.y, req.width, req.height).unwrap();
        Ok(())
    }

    fn set_anchor(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetAnchor = self.client.parse(self, parser)?;
        let anchor = match Edge::from_enum(req.anchor) {
            Some(a) => a,
            _ => return Err(XdgPositionerError::UnknownAnchor(req.anchor)),
        };
        self.position.borrow_mut().anchor = anchor;
        Ok(())
    }

    fn set_gravity(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetGravity = self.client.parse(self, parser)?;
        let gravity = match Edge::from_enum(req.gravity) {
            Some(a) => a,
            _ => return Err(XdgPositionerError::UnknownGravity(req.gravity)),
        };
        self.position.borrow_mut().gravity = gravity;
        Ok(())
    }

    fn set_constraint_adjustment(
        &self,
        parser: MsgParser<'_, '_>,
    ) -> Result<(), XdgPositionerError> {
        let req: SetConstraintAdjustment = self.client.parse(self, parser)?;
        let ca = match CA::from_bits(req.constraint_adjustment) {
            Some(c) => c,
            _ => return Err(XdgPositionerError::UnknownCa(req.constraint_adjustment)),
        };
        self.position.borrow_mut().ca = ca;
        Ok(())
    }

    fn set_offset(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetOffset = self.client.parse(self, parser)?;
        let mut position = self.position.borrow_mut();
        position.off_x = req.x;
        position.off_y = req.y;
        Ok(())
    }

    fn set_reactive(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let _req: SetReactive = self.client.parse(self, parser)?;
        self.position.borrow_mut().reactive = true;
        Ok(())
    }

    fn set_parent_size(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetParentSize = self.client.parse(self, parser)?;
        if req.parent_width < 0 || req.parent_height < 0 {
            self.client.protocol_error(
                self,
                INVALID_INPUT,
                &format!("Cannot set a negative parent size"),
            );
            return Err(XdgPositionerError::NegativeParentSize);
        }
        let mut position = self.position.borrow_mut();
        position.parent_width = req.parent_width;
        position.parent_height = req.parent_height;
        Ok(())
    }

    fn set_parent_configure(&self, parser: MsgParser<'_, '_>) -> Result<(), XdgPositionerError> {
        let req: SetParentConfigure = self.client.parse(self, parser)?;
        self.position.borrow_mut().parent_serial = req.serial;
        Ok(())
    }
}

object_base! {
    XdgPositioner;

    DESTROY => destroy,
    SET_SIZE => set_size,
    SET_ANCHOR_RECT => set_anchor_rect,
    SET_ANCHOR => set_anchor,
    SET_GRAVITY => set_gravity,
    SET_CONSTRAINT_ADJUSTMENT => set_constraint_adjustment,
    SET_OFFSET => set_offset,
    SET_REACTIVE => set_reactive,
    SET_PARENT_SIZE => set_parent_size,
    SET_PARENT_CONFIGURE => set_parent_configure,
}

impl Object for XdgPositioner {
    fn num_requests(&self) -> u32 {
        if self.base.version < 3 {
            SET_OFFSET + 1
        } else {
            SET_PARENT_CONFIGURE + 1
        }
    }
}

dedicated_add_obj!(XdgPositioner, XdgPositionerId, xdg_positioners);

#[derive(Debug, Error)]
pub enum XdgPositionerError {
    #[error("Cannot set a non-positive size")]
    NonPositiveSize,
    #[error("Cannot set an anchor rect with a negative size")]
    NegativeAnchorRect,
    #[error("Unknown anchor {0}")]
    UnknownAnchor(u32),
    #[error("Unknown gravity {0}")]
    UnknownGravity(u32),
    #[error("Unknown constraint adjustment {0}")]
    UnknownCa(u32),
    #[error("Parsing failed")]
    MsgParserError(#[source] Box<MsgParserError>),
    #[error("Cannot set a negative parent size")]
    NegativeParentSize,
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(XdgPositionerError, MsgParserError);
efrom!(XdgPositionerError, ClientError);
