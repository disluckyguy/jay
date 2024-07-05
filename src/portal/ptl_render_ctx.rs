use {crate::gfx_api::GfxContext, std::rc::Rc, uapi::c};

pub struct PortalRenderCtx {
    pub _dev_id: c::dev_t,
    pub ctx: Rc<dyn GfxContext>,
}
