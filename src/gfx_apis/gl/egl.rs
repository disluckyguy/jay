use {
    crate::gfx_apis::gl::{
        RenderError,
        egl::sys::{
            EGL_DEBUG_MSG_CRITICAL_KHR, EGL_DEBUG_MSG_ERROR_KHR, EGL_DEBUG_MSG_INFO_KHR,
            EGL_DEBUG_MSG_WARN_KHR, EGL_NONE, EGL_OPENGL_ES_API, EGL_TRUE, EGLAttrib, EGLLabelKHR,
            EGLenum, EGLint,
        },
        ext::{ClientExt, EXT_PLATFORM_BASE, KHR_DEBUG, KHR_PLATFORM_GBM, get_client_ext},
        proc::ExtProc,
        sys::EGL,
    },
    bstr::ByteSlice,
    log::Level,
    once_cell::sync::Lazy,
    std::ffi::CStr,
    sys::{
        EGL_BAD_ACCESS, EGL_BAD_ALLOC, EGL_BAD_ATTRIBUTE, EGL_BAD_CONFIG, EGL_BAD_CONTEXT,
        EGL_BAD_CURRENT_SURFACE, EGL_BAD_DEVICE_EXT, EGL_BAD_DISPLAY, EGL_BAD_MATCH,
        EGL_BAD_NATIVE_PIXMAP, EGL_BAD_NATIVE_WINDOW, EGL_BAD_PARAMETER, EGL_BAD_SURFACE,
        EGL_CONTEXT_LOST, EGL_NOT_INITIALIZED, EGL_SUCCESS,
    },
    uapi::c,
};

pub mod context;
pub mod display;
pub mod image;
pub mod sys;

pub(crate) static PROCS: Lazy<Option<ExtProc>> = Lazy::new(ExtProc::load);

pub(crate) static EXTS: Lazy<ClientExt> = Lazy::new(get_client_ext);

pub(in crate::gfx_apis::gl) fn init() -> Result<(), RenderError> {
    let Some(egl) = EGL.as_ref() else {
        return Err(RenderError::LoadEgl);
    };
    let Some(procs) = PROCS.as_ref() else {
        return Err(RenderError::LoadEglProcs);
    };
    if !EXTS.contains(EXT_PLATFORM_BASE) {
        return Err(RenderError::ExtPlatformBase);
    }
    if !EXTS.contains(KHR_PLATFORM_GBM) {
        return Err(RenderError::GbmExt);
    }
    if EXTS.contains(KHR_DEBUG) {
        let attrib: &[EGLAttrib] = &[
            EGL_DEBUG_MSG_CRITICAL_KHR as _,
            EGL_TRUE as _,
            EGL_DEBUG_MSG_ERROR_KHR as _,
            EGL_TRUE as _,
            EGL_DEBUG_MSG_WARN_KHR as _,
            EGL_TRUE as _,
            EGL_DEBUG_MSG_INFO_KHR as _,
            EGL_TRUE as _,
            EGL_NONE as _,
        ];
        unsafe {
            procs.eglDebugMessageControlKHR(egl_log, attrib.as_ptr());
        }
    }
    if unsafe { (egl.eglBindAPI)(EGL_OPENGL_ES_API) } != EGL_TRUE {
        return Err(RenderError::BindFailed);
    }
    Ok(())
}

unsafe extern "C" fn egl_log(
    error: EGLenum,
    command: *const c::c_char,
    message_type: EGLint,
    _thread_label: EGLLabelKHR,
    _object_label: EGLLabelKHR,
    message: *const c::c_char,
) {
    let level = match message_type {
        EGL_DEBUG_MSG_CRITICAL_KHR => Level::Error,
        EGL_DEBUG_MSG_ERROR_KHR => Level::Error,
        EGL_DEBUG_MSG_WARN_KHR => Level::Warn,
        EGL_DEBUG_MSG_INFO_KHR => Level::Info,
        _ => Level::Warn,
    };
    let command = if !command.is_null() {
        unsafe { CStr::from_ptr(command).to_bytes() }
    } else {
        b"none"
    };
    let message = if !message.is_null() {
        unsafe { CStr::from_ptr(message).to_bytes() }
    } else {
        b"none"
    };
    let err_name = error_name(error);
    log::log!(
        level,
        "EGL: command: {}, error: {} (0x{:x}), message: {}",
        command.as_bstr(),
        err_name,
        error,
        message.as_bstr()
    );
}

fn error_name(error: EGLenum) -> &'static str {
    macro_rules! en {
        ($($name:ident,)*) => {
            match error as _ {
                $($name => stringify!($name),)*
                _ => "unknown",
            }
        }
    }
    en! {
        EGL_SUCCESS,
        EGL_NOT_INITIALIZED,
        EGL_BAD_ACCESS,
        EGL_BAD_ALLOC,
        EGL_BAD_ATTRIBUTE,
        EGL_BAD_CONTEXT,
        EGL_BAD_CONFIG,
        EGL_BAD_CURRENT_SURFACE,
        EGL_BAD_DISPLAY,
        EGL_BAD_DEVICE_EXT,
        EGL_BAD_SURFACE,
        EGL_BAD_MATCH,
        EGL_BAD_PARAMETER,
        EGL_BAD_NATIVE_PIXMAP,
        EGL_BAD_NATIVE_WINDOW,
        EGL_CONTEXT_LOST,
    }
}
