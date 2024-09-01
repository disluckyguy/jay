use {
    crate::{
        allocator::{AllocatorError, BufferObject, BO_USE_LINEAR, BO_USE_RENDERING},
        format::XRGB8888,
        gfx_api::GfxError,
        scale::Scale,
        state::State,
        video::{drm::DrmError, INVALID_MODIFIER, LINEAR_MODIFIER},
    },
    jay_config::video::Transform,
    std::{ops::Deref, rc::Rc},
    thiserror::Error,
    uapi::OwnedFd,
};

#[derive(Debug, Error)]
pub enum ScreenshooterError {
    #[error("There is no render context")]
    NoRenderContext,
    #[error("Display is empty")]
    EmptyDisplay,
    #[error(transparent)]
    AllocatorError(#[from] AllocatorError),
    #[error(transparent)]
    RenderError(#[from] GfxError),
    #[error(transparent)]
    DrmError(#[from] DrmError),
    #[error("Render context does not support XRGB8888")]
    XRGB8888,
    #[error("Render context supports neither linear nor invalid modifier for XRGB8888 rendering")]
    Linear,
}

pub struct Screenshot {
    pub drm: Option<Rc<OwnedFd>>,
    pub bo: Rc<dyn BufferObject>,
}

pub fn take_screenshot(
    state: &State,
    include_cursor: bool,
) -> Result<Screenshot, ScreenshooterError> {
    let ctx = match state.render_ctx.get() {
        Some(ctx) => ctx,
        _ => return Err(ScreenshooterError::NoRenderContext),
    };
    let extents = state.root.extents.get();
    if extents.is_empty() {
        return Err(ScreenshooterError::EmptyDisplay);
    }
    let formats = ctx.formats();
    let mut usage = BO_USE_RENDERING;
    let modifiers = match formats.get(&XRGB8888.drm) {
        None => return Err(ScreenshooterError::XRGB8888),
        Some(f) if f.write_modifiers.contains(&LINEAR_MODIFIER) => &[LINEAR_MODIFIER],
        Some(f) if f.write_modifiers.contains(&INVALID_MODIFIER) => {
            usage |= BO_USE_LINEAR;
            &[INVALID_MODIFIER]
        }
        Some(_) => return Err(ScreenshooterError::Linear),
    };
    let allocator = ctx.allocator();
    let bo = allocator.create_bo(
        &state.dma_buf_ids,
        extents.width(),
        extents.height(),
        XRGB8888,
        modifiers,
        usage,
    )?;
    let fb = ctx.clone().dmabuf_fb(bo.dmabuf())?;
    fb.render_node(
        state.root.deref(),
        state,
        Some(state.root.extents.get()),
        None,
        Scale::from_int(1),
        include_cursor,
        true,
        false,
        Transform::None,
    )?;
    let drm = match allocator.drm() {
        Some(drm) => Some(drm.dup_render()?.fd().clone()),
        _ => None,
    };
    Ok(Screenshot { drm, bo })
}
