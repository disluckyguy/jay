mod sys;

use {
    crate::{
        utils::oserror::OsError,
        video::drm::sys::{
            create_lease, drm_event, drm_event_vblank, gem_close, get_cap,
            get_device_name_from_fd2, get_minor_name_from_fd, get_node_type_from_fd, get_nodes,
            mode_addfb2, mode_atomic, mode_create_blob, mode_destroy_blob, mode_get_resources,
            mode_getconnector, mode_getencoder, mode_getplane, mode_getplaneresources,
            mode_getprobblob, mode_getproperty, mode_obj_getproperties, mode_rmfb,
            prime_fd_to_handle, set_client_cap, DRM_DISPLAY_MODE_LEN, DRM_MODE_ATOMIC_TEST_ONLY,
            DRM_MODE_FB_MODIFIERS, DRM_MODE_OBJECT_BLOB, DRM_MODE_OBJECT_CONNECTOR,
            DRM_MODE_OBJECT_CRTC, DRM_MODE_OBJECT_ENCODER, DRM_MODE_OBJECT_FB,
            DRM_MODE_OBJECT_MODE, DRM_MODE_OBJECT_PLANE, DRM_MODE_OBJECT_PROPERTY,
        },
    },
    ahash::AHashMap,
    bstr::{BString, ByteSlice},
    std::{
        cell::RefCell,
        ffi::CString,
        fmt::{Debug, Display, Formatter},
        mem::{self, MaybeUninit},
        ops::Deref,
        rc::{Rc, Weak},
    },
    thiserror::Error,
    uapi::{c, Errno, OwnedFd, Pod, Ustring},
};

use crate::{
    backend,
    utils::{errorfmt::ErrorFmt, stack::Stack, syncqueue::SyncQueue, vec_ext::VecExt},
    video::{
        dmabuf::DmaBuf,
        drm::sys::{get_version, DRM_CAP_CURSOR_HEIGHT, DRM_CAP_CURSOR_WIDTH},
        INVALID_MODIFIER,
    },
};
pub use sys::{
    drm_mode_modeinfo, DRM_CLIENT_CAP_ATOMIC, DRM_MODE_ATOMIC_ALLOW_MODESET,
    DRM_MODE_ATOMIC_NONBLOCK, DRM_MODE_PAGE_FLIP_EVENT,
};

#[derive(Debug, Error)]
pub enum DrmError {
    #[error("Could not reopen a node")]
    ReopenNode(#[source] crate::utils::oserror::OsError),
    #[error("Could not retrieve the render node name")]
    RenderNodeName(#[source] OsError),
    #[error("Could not retrieve the device node name")]
    DeviceNodeName(#[source] OsError),
    #[error("Could not retrieve device nodes")]
    GetNodes(#[source] OsError),
    #[error("Could not retrieve device type")]
    GetDeviceType(#[source] OsError),
    #[error("Could not perform drm property ioctl")]
    GetProperty(#[source] OsError),
    #[error("Could not perform drm getencoder ioctl")]
    GetEncoder(#[source] OsError),
    #[error("Could not perform drm getresources ioctl")]
    GetResources(#[source] OsError),
    #[error("Could not perform drm getplaneresources ioctl")]
    GetPlaneResources(#[source] OsError),
    #[error("Could not perform drm getplane ioctl")]
    GetPlane(#[source] OsError),
    #[error("Could not create a blob")]
    CreateBlob(#[source] OsError),
    #[error("Could not perform drm getconnector ioctl")]
    GetConnector(#[source] OsError),
    #[error("Could not perform drm getprobblob ioctl")]
    GetPropBlob(#[source] OsError),
    #[error("Property has an invalid size")]
    InvalidProbSize,
    #[error("Property has a size that is not a multiple of the vector type")]
    UnalignedPropSize,
    #[error("Could not perform drm properties ioctl")]
    GetProperties(#[source] OsError),
    #[error("Could not perform drm atomic ioctl")]
    Atomic(#[source] OsError),
    #[error("Could not inspect a connector")]
    CreateConnector(#[source] Box<DrmError>),
    #[error("Drm property has an unknown type {0}")]
    UnknownPropertyType(u32),
    #[error("Range property does not have exactly two values")]
    RangeValues,
    #[error("Object property does not have exactly one value")]
    ObjectValues,
    #[error("Object does not have the required property {0}")]
    MissingProperty(Box<str>),
    #[error("Plane has an unknown type {0}")]
    UnknownPlaneType(BString),
    #[error("Plane has an invalid type {0}")]
    InvalidPlaneType(u64),
    #[error("Plane type property has an invalid property type")]
    InvalidPlaneTypeProperty,
    #[error("Could not create a framebuffer")]
    AddFb(#[source] OsError),
    #[error("Could not convert prime fd to gem handle")]
    GemHandle(#[source] OsError),
    #[error("Could not read events from the drm fd")]
    ReadEvents(#[source] OsError),
    #[error("Read invalid data from drm device")]
    InvalidRead,
    #[error("Could not determine the drm version")]
    Version(#[source] OsError),
}

fn render_node_name(fd: c::c_int) -> Result<Ustring, DrmError> {
    get_minor_name_from_fd(fd, NodeType::Render).map_err(DrmError::RenderNodeName)
}

fn device_node_name(fd: c::c_int) -> Result<Ustring, DrmError> {
    get_device_name_from_fd2(fd).map_err(DrmError::DeviceNodeName)
}

fn reopen(fd: c::c_int, need_primary: bool) -> Result<Rc<OwnedFd>, DrmError> {
    if let Ok((fd, _)) = create_lease(fd, &[], c::O_CLOEXEC as _) {
        return Ok(Rc::new(fd));
    }
    let path = 'path: {
        if get_node_type_from_fd(fd).map_err(DrmError::GetDeviceType)? == NodeType::Render {
            break 'path uapi::format_ustr!("/proc/self/fd/{}", fd);
        }
        if !need_primary {
            if let Ok(path) = render_node_name(fd) {
                break 'path path;
            }
        }
        device_node_name(fd)?
    };
    match uapi::open(&path, c::O_RDWR | c::O_CLOEXEC, 0) {
        Ok(f) => Ok(Rc::new(f)),
        Err(e) => Err(DrmError::ReopenNode(e.into())),
    }
}

pub struct Drm {
    fd: Rc<OwnedFd>,
}

impl Drm {
    #[cfg_attr(not(feature = "it"), allow(dead_code))]
    pub fn open_existing(fd: Rc<OwnedFd>) -> Self {
        Self { fd }
    }

    pub fn reopen(fd: c::c_int, need_primary: bool) -> Result<Self, DrmError> {
        Ok(Self {
            fd: reopen(fd, need_primary)?,
        })
    }

    pub fn fd(&self) -> &Rc<OwnedFd> {
        &self.fd
    }

    pub fn raw(&self) -> c::c_int {
        self.fd.raw()
    }

    pub fn dup_render(&self) -> Result<Self, DrmError> {
        Self::reopen(self.fd.raw(), false)
    }

    pub fn get_nodes(&self) -> Result<AHashMap<NodeType, CString>, DrmError> {
        get_nodes(self.fd.raw()).map_err(DrmError::GetNodes)
    }

    pub fn version(&self) -> Result<DrmVersion, DrmError> {
        get_version(self.fd.raw()).map_err(DrmError::Version)
    }
}

pub struct DrmMaster {
    drm: Drm,
    u32_bufs: Stack<Vec<u32>>,
    u64_bufs: Stack<Vec<u64>>,
    gem_handles: RefCell<AHashMap<u32, Weak<GemHandle>>>,
    events: SyncQueue<DrmEvent>,
    buf: RefCell<Box<[MaybeUninit<u8>; 1024]>>,
}

impl Debug for DrmMaster {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.drm.raw())
    }
}

impl Deref for DrmMaster {
    type Target = Drm;

    fn deref(&self) -> &Self::Target {
        &self.drm
    }
}

impl DrmMaster {
    pub fn new(fd: Rc<OwnedFd>) -> Self {
        Self {
            drm: Drm { fd },
            u32_bufs: Default::default(),
            u64_bufs: Default::default(),
            gem_handles: Default::default(),
            events: Default::default(),
            buf: RefCell::new(Box::new([MaybeUninit::uninit(); 1024])),
        }
    }

    pub fn raw(&self) -> c::c_int {
        self.drm.raw()
    }

    pub fn get_property(&self, prop: DrmProperty) -> Result<DrmPropertyDefinition, DrmError> {
        mode_getproperty(self.raw(), prop)
    }

    pub fn get_properties<T: DrmObject>(&self, t: T) -> Result<Vec<DrmPropertyValue>, DrmError> {
        mode_obj_getproperties(self.raw(), t.id(), T::TYPE)
    }

    pub fn get_resources(&self) -> Result<DrmCardResources, DrmError> {
        mode_get_resources(self.raw())
    }

    #[allow(dead_code)]
    pub fn get_cap(&self, cap: u64) -> Result<u64, OsError> {
        get_cap(self.raw(), cap)
    }

    pub fn set_client_cap(&self, cap: u64, value: u64) -> Result<(), OsError> {
        set_client_cap(self.raw(), cap, value)
    }

    pub fn get_planes(&self) -> Result<Vec<DrmPlane>, DrmError> {
        mode_getplaneresources(self.raw())
    }

    pub fn get_plane_info(&self, plane: DrmPlane) -> Result<DrmPlaneInfo, DrmError> {
        mode_getplane(self.raw(), plane.0)
    }

    pub fn get_encoder_info(&self, encoder: DrmEncoder) -> Result<DrmEncoderInfo, DrmError> {
        mode_getencoder(self.raw(), encoder.0)
    }

    pub fn get_cursor_size(&self) -> Result<(u64, u64), OsError> {
        let width = self.get_cap(DRM_CAP_CURSOR_WIDTH)?;
        let height = self.get_cap(DRM_CAP_CURSOR_HEIGHT)?;
        Ok((width, height))
    }

    pub fn get_connector_info(
        &self,
        connector: DrmConnector,
        force: bool,
    ) -> Result<DrmConnectorInfo, DrmError> {
        mode_getconnector(self.raw(), connector.0, force)
    }

    pub fn change(self: &Rc<Self>) -> Change {
        let mut res = Change {
            master: self.clone(),
            objects: self.u32_bufs.pop().unwrap_or_default(),
            object_lengths: self.u32_bufs.pop().unwrap_or_default(),
            props: self.u32_bufs.pop().unwrap_or_default(),
            values: self.u64_bufs.pop().unwrap_or_default(),
        };
        res.objects.clear();
        res.object_lengths.clear();
        res.props.clear();
        res.values.clear();
        res
    }

    pub fn create_blob<T>(self: &Rc<Self>, t: &T) -> Result<PropBlob, DrmError> {
        match mode_create_blob(self.raw(), t) {
            Ok(b) => Ok(PropBlob {
                master: self.clone(),
                id: b,
            }),
            Err(e) => Err(DrmError::CreateBlob(e)),
        }
    }

    pub fn add_fb(self: &Rc<Self>, dma: &DmaBuf) -> Result<DrmFramebuffer, DrmError> {
        let mut modifier = 0;
        let mut flags = 0;
        if dma.modifier != INVALID_MODIFIER {
            modifier = dma.modifier;
            flags |= DRM_MODE_FB_MODIFIERS;
        }
        let mut strides = [0; 4];
        let mut offsets = [0; 4];
        let mut modifiers = [0; 4];
        let mut handles = [0; 4];
        let mut handles_ = vec![];
        for (idx, plane) in dma.planes.iter().enumerate() {
            strides[idx] = plane.stride;
            offsets[idx] = plane.offset;
            modifiers[idx] = modifier;
            let handle = self.gem_handle(plane.fd.raw())?;
            handles[idx] = handle.handle();
            handles_.push(handle);
        }
        match mode_addfb2(
            self.raw(),
            dma.width as _,
            dma.height as _,
            dma.format.drm,
            flags,
            handles,
            strides,
            offsets,
            modifiers,
        ) {
            Ok(fb) => Ok(DrmFramebuffer {
                master: self.clone(),
                fb,
            }),
            Err(e) => Err(DrmError::AddFb(e)),
        }
    }

    pub fn gem_handle(self: &Rc<Self>, fd: c::c_int) -> Result<Rc<GemHandle>, DrmError> {
        let handle = match prime_fd_to_handle(self.raw(), fd) {
            Ok(h) => h,
            Err(e) => return Err(DrmError::GemHandle(e)),
        };
        let mut handles = self.gem_handles.borrow_mut();
        if let Some(h) = handles.get(&handle) {
            if let Some(h) = h.upgrade() {
                return Ok(h);
            }
        }
        let h = Rc::new(GemHandle {
            master: self.clone(),
            handle,
        });
        handles.insert(handle, Rc::downgrade(&h));
        Ok(h)
    }

    pub fn getblob<T: Pod>(&self, blob: DrmBlob) -> Result<T, DrmError> {
        let mut t = MaybeUninit::<T>::uninit();
        match mode_getprobblob(self.raw(), blob.0, &mut t) {
            Err(e) => Err(DrmError::GetPropBlob(e)),
            Ok(n) if n != mem::size_of::<T>() => Err(DrmError::InvalidProbSize),
            _ => unsafe { Ok(t.assume_init()) },
        }
    }

    pub fn getblob_vec<T: Pod>(&self, blob: DrmBlob) -> Result<Vec<T>, DrmError> {
        assert_ne!(mem::size_of::<T>(), 0);
        let mut vec = vec![];
        loop {
            let (_, bytes) = vec.split_at_spare_mut_bytes_ext();
            match mode_getprobblob(self.raw(), blob.0, bytes) {
                Err(e) => return Err(DrmError::GetPropBlob(e)),
                Ok(n) if n % mem::size_of::<T>() != 0 => return Err(DrmError::UnalignedPropSize),
                Ok(n) if n <= bytes.len() => {
                    unsafe {
                        vec.set_len(n / mem::size_of::<T>());
                    }
                    return Ok(vec);
                }
                Ok(n) => vec.reserve_exact(n / mem::size_of::<T>()),
            }
        }
    }

    pub fn event(&self) -> Result<Option<DrmEvent>, DrmError> {
        if self.events.is_empty() {
            let mut buf = self.buf.borrow_mut();
            let mut buf = match uapi::read(self.raw(), buf.as_mut_slice()) {
                Ok(b) => b,
                Err(Errno(c::EAGAIN)) => return Ok(None),
                Err(e) => return Err(DrmError::ReadEvents(e.into())),
            };
            while buf.len() > 0 {
                let header: drm_event = match uapi::pod_read_init(buf) {
                    Ok(e) => e,
                    _ => return Err(DrmError::InvalidRead),
                };
                let len = header.length as usize;
                if len > buf.len() {
                    return Err(DrmError::InvalidRead);
                }
                match header.ty {
                    sys::DRM_EVENT_FLIP_COMPLETE => {
                        let event: drm_event_vblank = match uapi::pod_read_init(buf) {
                            Ok(e) => e,
                            _ => return Err(DrmError::InvalidRead),
                        };
                        self.events.push(DrmEvent::FlipComplete {
                            tv_sec: event.tv_sec,
                            tv_usec: event.tv_usec,
                            sequence: event.sequence,
                            crtc_id: DrmCrtc(event.crtc_id),
                        });
                    }
                    _ => {}
                }
                buf = &mut buf[len as usize..];
            }
        }
        Ok(self.events.pop())
    }
}

pub enum DrmEvent {
    FlipComplete {
        tv_sec: u32,
        tv_usec: u32,
        sequence: u32,
        crtc_id: DrmCrtc,
    },
}

pub struct DrmFramebuffer {
    master: Rc<DrmMaster>,
    fb: DrmFb,
}

impl Debug for DrmFramebuffer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrmFramebuffer")
            .field("fb", &self.fb)
            .finish_non_exhaustive()
    }
}

impl DrmFramebuffer {
    pub fn id(&self) -> DrmFb {
        self.fb
    }
}

impl Drop for DrmFramebuffer {
    fn drop(&mut self) {
        if let Err(e) = mode_rmfb(self.master.raw(), self.fb) {
            log::error!("Could not delete framebuffer: {}", ErrorFmt(e));
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum NodeType {
    Primary,
    Control,
    Render,
}

impl NodeType {
    fn name(self) -> &'static str {
        match self {
            NodeType::Primary => "card",
            NodeType::Control => "controlD",
            NodeType::Render => "renderD",
        }
    }
}

#[derive(Debug)]
pub struct DrmPropertyDefinition {
    pub id: DrmProperty,
    pub name: BString,
    pub immutable: bool,
    pub atomic: bool,
    pub ty: DrmPropertyType,
}

#[derive(Debug)]
pub enum DrmPropertyType {
    Range {
        min: u64,
        max: u64,
    },
    SignedRange {
        min: i64,
        max: i64,
    },
    Object {
        ty: u32,
    },
    Blob,
    Enum {
        values: Vec<DrmPropertyEnumValue>,
        bitmask: bool,
    },
}

#[derive(Debug)]
pub struct DrmPropertyEnumValue {
    pub value: u64,
    pub name: BString,
}

#[derive(Debug)]
pub struct DrmPropertyValue {
    pub id: DrmProperty,
    pub value: u64,
}

pub trait DrmObject {
    const TYPE: u32;
    const NONE: Self;
    fn id(&self) -> u32;
    fn is_some(&self) -> bool;
    fn is_none(&self) -> bool;
}

macro_rules! drm_obj {
    ($name:ident, $ty:expr) => {
        #[repr(transparent)]
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default)]
        pub struct $name(pub u32);

        impl DrmObject for $name {
            const TYPE: u32 = $ty;
            const NONE: Self = Self(0);

            fn id(&self) -> u32 {
                self.0
            }

            fn is_some(&self) -> bool {
                self.0 != 0
            }

            fn is_none(&self) -> bool {
                self.0 == 0
            }
        }
    };
}
drm_obj!(DrmCrtc, DRM_MODE_OBJECT_CRTC);
drm_obj!(DrmConnector, DRM_MODE_OBJECT_CONNECTOR);
drm_obj!(DrmEncoder, DRM_MODE_OBJECT_ENCODER);
drm_obj!(DrmMode, DRM_MODE_OBJECT_MODE);
drm_obj!(DrmProperty, DRM_MODE_OBJECT_PROPERTY);
drm_obj!(DrmFb, DRM_MODE_OBJECT_FB);
drm_obj!(DrmBlob, DRM_MODE_OBJECT_BLOB);
drm_obj!(DrmPlane, DRM_MODE_OBJECT_PLANE);

#[derive(Debug)]
pub struct DrmCardResources {
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
    pub fbs: Vec<DrmFb>,
    pub crtcs: Vec<DrmCrtc>,
    pub connectors: Vec<DrmConnector>,
    pub encoders: Vec<DrmEncoder>,
}

#[derive(Debug)]
pub struct DrmPlaneInfo {
    pub plane_id: DrmPlane,
    pub crtc_id: DrmCrtc,
    pub fb_id: DrmFb,
    pub possible_crtcs: u32,
    pub gamma_size: u32,
    pub format_types: Vec<u32>,
}

#[derive(Debug)]
pub struct DrmEncoderInfo {
    pub encoder_id: DrmEncoder,
    pub encoder_type: u32,
    pub crtc_id: DrmCrtc,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DrmModeInfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,

    pub vrefresh: u32,

    pub flags: u32,
    pub ty: u32,
    pub name: BString,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name: BString,
    pub date: BString,
    pub desc: BString,
}

impl DrmModeInfo {
    pub fn create_blob(&self, master: &Rc<DrmMaster>) -> Result<PropBlob, DrmError> {
        let raw = self.to_raw();
        master.create_blob(&raw)
    }

    pub fn to_raw(&self) -> drm_mode_modeinfo {
        let mut name = [0u8; DRM_DISPLAY_MODE_LEN];
        let len = name.len().min(self.name.len());
        name[..len].copy_from_slice(&self.name.as_bytes()[..len]);
        drm_mode_modeinfo {
            clock: self.clock,
            hdisplay: self.hdisplay,
            hsync_start: self.hsync_start,
            hsync_end: self.hsync_end,
            htotal: self.htotal,
            hskew: self.hskew,
            vdisplay: self.vdisplay,
            vsync_start: self.vsync_start,
            vsync_end: self.vsync_end,
            vtotal: self.vtotal,
            vscan: self.vscan,
            vrefresh: self.vrefresh,
            flags: self.flags,
            ty: self.ty,
            name,
        }
    }

    pub fn to_backend(&self) -> backend::Mode {
        backend::Mode {
            width: self.hdisplay as _,
            height: self.vdisplay as _,
            refresh_rate_millihz: self.refresh_rate_millihz(),
        }
    }

    pub fn refresh_rate_millihz(&self) -> u32 {
        let clock_millihz = self.clock as u64 * 1_000_000;
        let htotal = self.htotal as u64;
        let vtotal = self.vtotal as u64;
        (((clock_millihz / htotal) + (vtotal / 2)) / vtotal) as u32
        // simplifies to
        //     clock_millihz / (htotal * vtotal) + 1/2
        // why round up (+1/2) instead of down?
    }
}

#[derive(Debug)]
pub struct DrmConnectorInfo {
    pub encoders: Vec<DrmEncoder>,
    pub modes: Vec<DrmModeInfo>,
    pub props: Vec<DrmPropertyValue>,

    pub encoder_id: DrmEncoder,
    pub connector_id: DrmConnector,
    pub connector_type: u32,
    pub connector_type_id: u32,

    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
}

pub struct Change {
    master: Rc<DrmMaster>,
    objects: Vec<u32>,
    object_lengths: Vec<u32>,
    props: Vec<u32>,
    values: Vec<u64>,
}

pub struct ObjectChange<'a> {
    change: &'a mut Change,
}

impl Change {
    #[allow(dead_code)]
    pub fn test(&self, flags: u32) -> Result<(), DrmError> {
        mode_atomic(
            self.master.raw(),
            flags | DRM_MODE_ATOMIC_TEST_ONLY,
            &self.objects,
            &self.object_lengths,
            &self.props,
            &self.values,
            0,
        )
    }

    pub fn commit(&self, flags: u32, user_data: u64) -> Result<(), DrmError> {
        mode_atomic(
            self.master.raw(),
            flags,
            &self.objects,
            &self.object_lengths,
            &self.props,
            &self.values,
            user_data,
        )
    }

    pub fn change_object<T, F>(&mut self, obj: T, f: F)
    where
        T: DrmObject,
        F: FnOnce(&mut ObjectChange),
    {
        let old_len = self.props.len();
        let mut oc = ObjectChange { change: self };
        f(&mut oc);
        if self.props.len() > old_len {
            let new = (self.props.len() - old_len) as u32;
            if self.objects.last() == Some(&obj.id()) {
                *self.object_lengths.last_mut().unwrap() += new;
            } else {
                self.objects.push(obj.id());
                self.object_lengths.push(new);
            }
        }
    }
}

impl<'a> ObjectChange<'a> {
    pub fn change(&mut self, property_id: DrmProperty, value: u64) {
        self.change.props.push(property_id.0);
        self.change.values.push(value);
    }
}

impl Drop for Change {
    fn drop(&mut self) {
        self.master.u32_bufs.push(mem::take(&mut self.objects));
        self.master
            .u32_bufs
            .push(mem::take(&mut self.object_lengths));
        self.master.u32_bufs.push(mem::take(&mut self.props));
        self.master.u64_bufs.push(mem::take(&mut self.values));
    }
}

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
pub enum ConnectorType {
    Unknown(u32),
    VGA,
    DVII,
    DVID,
    DVIA,
    Composite,
    SVIDEO,
    LVDS,
    Component,
    _9PinDIN,
    DisplayPort,
    HDMIA,
    HDMIB,
    TV,
    eDP,
    VIRTUAL,
    DSI,
    DPI,
    WRITEBACK,
    SPI,
    USB,
    EmbeddedWindow,
}

impl ConnectorType {
    pub fn from_drm(v: u32) -> Self {
        match v {
            sys::DRM_MODE_CONNECTOR_VGA => Self::VGA,
            sys::DRM_MODE_CONNECTOR_DVII => Self::DVII,
            sys::DRM_MODE_CONNECTOR_DVID => Self::DVID,
            sys::DRM_MODE_CONNECTOR_DVIA => Self::DVIA,
            sys::DRM_MODE_CONNECTOR_Composite => Self::Composite,
            sys::DRM_MODE_CONNECTOR_SVIDEO => Self::SVIDEO,
            sys::DRM_MODE_CONNECTOR_LVDS => Self::LVDS,
            sys::DRM_MODE_CONNECTOR_Component => Self::Component,
            sys::DRM_MODE_CONNECTOR_9PinDIN => Self::_9PinDIN,
            sys::DRM_MODE_CONNECTOR_DisplayPort => Self::DisplayPort,
            sys::DRM_MODE_CONNECTOR_HDMIA => Self::HDMIA,
            sys::DRM_MODE_CONNECTOR_HDMIB => Self::HDMIB,
            sys::DRM_MODE_CONNECTOR_TV => Self::TV,
            sys::DRM_MODE_CONNECTOR_eDP => Self::eDP,
            sys::DRM_MODE_CONNECTOR_VIRTUAL => Self::VIRTUAL,
            sys::DRM_MODE_CONNECTOR_DSI => Self::DSI,
            sys::DRM_MODE_CONNECTOR_DPI => Self::DPI,
            sys::DRM_MODE_CONNECTOR_WRITEBACK => Self::WRITEBACK,
            sys::DRM_MODE_CONNECTOR_SPI => Self::SPI,
            sys::DRM_MODE_CONNECTOR_USB => Self::USB,
            _ => Self::Unknown(v),
        }
    }

    #[allow(dead_code)]
    pub fn to_drm(self) -> u32 {
        match self {
            Self::Unknown(n) => n,
            Self::VGA => sys::DRM_MODE_CONNECTOR_VGA,
            Self::DVII => sys::DRM_MODE_CONNECTOR_DVII,
            Self::DVID => sys::DRM_MODE_CONNECTOR_DVID,
            Self::DVIA => sys::DRM_MODE_CONNECTOR_DVIA,
            Self::Composite => sys::DRM_MODE_CONNECTOR_Composite,
            Self::SVIDEO => sys::DRM_MODE_CONNECTOR_SVIDEO,
            Self::LVDS => sys::DRM_MODE_CONNECTOR_LVDS,
            Self::Component => sys::DRM_MODE_CONNECTOR_Component,
            Self::_9PinDIN => sys::DRM_MODE_CONNECTOR_9PinDIN,
            Self::DisplayPort => sys::DRM_MODE_CONNECTOR_DisplayPort,
            Self::HDMIA => sys::DRM_MODE_CONNECTOR_HDMIA,
            Self::HDMIB => sys::DRM_MODE_CONNECTOR_HDMIB,
            Self::TV => sys::DRM_MODE_CONNECTOR_TV,
            Self::eDP => sys::DRM_MODE_CONNECTOR_eDP,
            Self::VIRTUAL => sys::DRM_MODE_CONNECTOR_VIRTUAL,
            Self::DSI => sys::DRM_MODE_CONNECTOR_DSI,
            Self::DPI => sys::DRM_MODE_CONNECTOR_DPI,
            Self::WRITEBACK => sys::DRM_MODE_CONNECTOR_WRITEBACK,
            Self::SPI => sys::DRM_MODE_CONNECTOR_SPI,
            Self::USB => sys::DRM_MODE_CONNECTOR_USB,
            Self::EmbeddedWindow => sys::DRM_MODE_CONNECTOR_Unknown,
        }
    }

    pub fn to_config(self) -> jay_config::video::connector_type::ConnectorType {
        use jay_config::video::connector_type::*;
        match self {
            Self::Unknown(_) => CON_UNKNOWN,
            Self::VGA => CON_VGA,
            Self::DVII => CON_DVII,
            Self::DVID => CON_DVID,
            Self::DVIA => CON_DVIA,
            Self::Composite => CON_COMPOSITE,
            Self::SVIDEO => CON_SVIDEO,
            Self::LVDS => CON_LVDS,
            Self::Component => CON_COMPONENT,
            Self::_9PinDIN => CON_9PIN_DIN,
            Self::DisplayPort => CON_DISPLAY_PORT,
            Self::HDMIA => CON_HDMIA,
            Self::HDMIB => CON_HDMIB,
            Self::TV => CON_TV,
            Self::eDP => CON_EDP,
            Self::VIRTUAL => CON_VIRTUAL,
            Self::DSI => CON_DSI,
            Self::DPI => CON_DPI,
            Self::WRITEBACK => CON_WRITEBACK,
            Self::SPI => CON_SPI,
            Self::USB => CON_USB,
            Self::EmbeddedWindow => CON_EMBEDDED_WINDOW,
        }
    }
}

impl Display for ConnectorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown(n) => return write!(f, "Unknown({})", n),
            Self::VGA => "VGA",
            Self::DVII => "DVI-I",
            Self::DVID => "DVI-D",
            Self::DVIA => "DVI-A",
            Self::Composite => "Composite",
            Self::SVIDEO => "SVIDEO",
            Self::LVDS => "LVDS",
            Self::Component => "Component",
            Self::_9PinDIN => "DIN",
            Self::DisplayPort => "DP",
            Self::HDMIA => "HDMI-A",
            Self::HDMIB => "HDMI-B",
            Self::TV => "TV",
            Self::eDP => "eDP",
            Self::VIRTUAL => "Virtual",
            Self::DSI => "DSI",
            Self::DPI => "DPI",
            Self::WRITEBACK => "Writeback",
            Self::SPI => "SPI",
            Self::USB => "USB",
            Self::EmbeddedWindow => "EmbeddedWindow",
        };
        f.write_str(s)
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ConnectorStatus {
    Connected,
    Disconnected,
    Unknown,
    Other(u32),
}

impl ConnectorStatus {
    pub fn from_drm(v: u32) -> Self {
        match v {
            sys::CONNECTOR_STATUS_CONNECTED => Self::Connected,
            sys::CONNECTOR_STATUS_DISCONNECTED => Self::Disconnected,
            sys::CONNECTOR_STATUS_UNKNOWN => Self::Unknown,
            _ => Self::Other(v),
        }
    }
}

#[derive(Debug)]
pub struct PropBlob {
    master: Rc<DrmMaster>,
    id: DrmBlob,
}

impl PropBlob {
    pub fn id(&self) -> DrmBlob {
        self.id
    }
}

impl Drop for PropBlob {
    fn drop(&mut self) {
        if let Err(e) = mode_destroy_blob(self.master.raw(), self.id) {
            log::error!("Could not destroy blob: {}", ErrorFmt(e));
        }
    }
}

pub struct GemHandle {
    master: Rc<DrmMaster>,
    handle: u32,
}

impl GemHandle {
    pub fn handle(&self) -> u32 {
        self.handle
    }
}

impl Drop for GemHandle {
    fn drop(&mut self) {
        self.master.gem_handles.borrow_mut().remove(&self.handle);
        if let Err(e) = gem_close(self.master.raw(), self.handle) {
            log::error!("Could not close gem handle: {}", ErrorFmt(e));
        }
    }
}
