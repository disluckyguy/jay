use {
    crate::{
        gfx_apis::gl::sys::{GLint, GL_BGRA_EXT, GL_RGBA, GL_UNSIGNED_BYTE},
        pipewire::pw_pod::{
            SPA_VIDEO_FORMAT_BGRx, SPA_VIDEO_FORMAT_RGBx, SpaVideoFormat, SPA_VIDEO_FORMAT_BGRA,
            SPA_VIDEO_FORMAT_RGBA,
        },
        utils::debug_fn::debug_fn,
    },
    ahash::AHashMap,
    once_cell::sync::Lazy,
    std::fmt::{Debug, Write},
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Format {
    pub name: &'static str,
    pub bpp: u32,
    pub gl_format: GLint,
    pub gl_type: GLint,
    pub drm: u32,
    pub wl_id: Option<u32>,
    pub external_only_guess: bool,
    pub has_alpha: bool,
    pub shm_supported: bool,
    pub pipewire: SpaVideoFormat,
}

static FORMATS_MAP: Lazy<AHashMap<u32, &'static Format>> = Lazy::new(|| {
    let mut map = AHashMap::new();
    for format in FORMATS {
        assert!(map.insert(format.drm, format).is_none());
    }
    map
});

static PW_FORMATS_MAP: Lazy<AHashMap<SpaVideoFormat, &'static Format>> = Lazy::new(|| {
    let mut map = AHashMap::new();
    for format in FORMATS {
        assert!(map.insert(format.pipewire, format).is_none());
    }
    map
});

pub fn formats() -> &'static AHashMap<u32, &'static Format> {
    &FORMATS_MAP
}

pub fn pw_formats() -> &'static AHashMap<SpaVideoFormat, &'static Format> {
    &PW_FORMATS_MAP
}

const fn fourcc_code(a: char, b: char, c: char, d: char) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

#[allow(dead_code)]
pub fn debug(fourcc: u32) -> impl Debug {
    debug_fn(move |fmt| {
        fmt.write_char(fourcc as u8 as char)?;
        fmt.write_char((fourcc >> 8) as u8 as char)?;
        fmt.write_char((fourcc >> 16) as u8 as char)?;
        fmt.write_char((fourcc >> 24) as u8 as char)?;
        Ok(())
    })
}

const ARGB8888_ID: u32 = 0;
const ARGB8888_DRM: u32 = fourcc_code('A', 'R', '2', '4');

const XRGB8888_ID: u32 = 1;
const XRGB8888_DRM: u32 = fourcc_code('X', 'R', '2', '4');

pub fn map_wayland_format_id(id: u32) -> u32 {
    match id {
        ARGB8888_ID => ARGB8888_DRM,
        XRGB8888_ID => XRGB8888_DRM,
        _ => id,
    }
}

#[allow(dead_code)]
pub static ARGB8888: &Format = &FORMATS[0];
pub static XRGB8888: &Format = &FORMATS[1];

pub static FORMATS: &[Format] = &[
    Format {
        name: "argb8888",
        bpp: 4,
        gl_format: GL_BGRA_EXT,
        gl_type: GL_UNSIGNED_BYTE,
        drm: ARGB8888_DRM,
        wl_id: Some(ARGB8888_ID),
        external_only_guess: false,
        has_alpha: true,
        shm_supported: true,
        pipewire: SPA_VIDEO_FORMAT_BGRA,
    },
    Format {
        name: "xrgb8888",
        bpp: 4,
        gl_format: GL_BGRA_EXT,
        gl_type: GL_UNSIGNED_BYTE,
        drm: XRGB8888_DRM,
        wl_id: Some(XRGB8888_ID),
        external_only_guess: false,
        has_alpha: false,
        shm_supported: true,
        pipewire: SPA_VIDEO_FORMAT_BGRx,
    },
    Format {
        name: "abgr8888",
        bpp: 4,
        gl_format: GL_RGBA,
        gl_type: GL_UNSIGNED_BYTE,
        drm: fourcc_code('A', 'B', '2', '4'),
        wl_id: None,
        external_only_guess: false,
        has_alpha: true,
        shm_supported: true,
        pipewire: SPA_VIDEO_FORMAT_RGBA,
    },
    Format {
        name: "xbgr8888",
        bpp: 4,
        gl_format: GL_RGBA,
        gl_type: GL_UNSIGNED_BYTE,
        drm: fourcc_code('X', 'B', '2', '4'),
        wl_id: None,
        external_only_guess: false,
        has_alpha: false,
        shm_supported: true,
        pipewire: SPA_VIDEO_FORMAT_RGBx,
    },
    // Format {
    //     name: "nv12",
    //     bpp: 1,                    // wrong but only used for shm
    //     gl_format: 0,              // wrong but only used for shm
    //     gl_type: GL_UNSIGNED_BYTE, // wrong but only used for shm
    //     drm: fourcc_code('N', 'V', '1', '2'),
    //     wl_id: None,
    //     external_only_guess: true,
    //     has_alpha: false,
    //     shm_supported: false,
    //     pipewire: SPA_VIDEO_FORMAT_NV12,
    // },
    // Format {
    //     id: fourcc_code('C', '8', ' ', ' '),
    //     name: "c8",
    // },
    // Format {
    //     id: fourcc_code('R', '8', ' ', ' '),
    //     name: "r8",
    // },
    // Format {
    //     id: fourcc_code('R', '1', '6', ' '),
    //     name: "r16",
    // },
    // Format {
    //     id: fourcc_code('R', 'G', '8', '8'),
    //     name: "rg88",
    // },
    // Format {
    //     id: fourcc_code('G', 'R', '8', '8'),
    //     name: "gr88",
    // },
    // Format {
    //     id: fourcc_code('R', 'G', '3', '2'),
    //     name: "rg1616",
    // },
    // Format {
    //     id: fourcc_code('G', 'R', '3', '2'),
    //     name: "gr1616",
    // },
    // Format {
    //     id: fourcc_code('R', 'G', 'B', '8'),
    //     name: "rgb332",
    // },
    // Format {
    //     id: fourcc_code('B', 'G', 'R', '8'),
    //     name: "bgr233",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '1', '2'),
    //     name: "xrgb4444",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '1', '2'),
    //     name: "xbgr4444",
    // },
    // Format {
    //     id: fourcc_code('R', 'X', '1', '2'),
    //     name: "rgbx4444",
    // },
    // Format {
    //     id: fourcc_code('B', 'X', '1', '2'),
    //     name: "bgrx4444",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '1', '2'),
    //     name: "argb4444",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '1', '2'),
    //     name: "abgr4444",
    // },
    // Format {
    //     id: fourcc_code('R', 'A', '1', '2'),
    //     name: "rgba4444",
    // },
    // Format {
    //     id: fourcc_code('B', 'A', '1', '2'),
    //     name: "bgra4444",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '1', '5'),
    //     name: "xrgb1555",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '1', '5'),
    //     name: "xbgr1555",
    // },
    // Format {
    //     id: fourcc_code('R', 'X', '1', '5'),
    //     name: "rgbx5551",
    // },
    // Format {
    //     id: fourcc_code('B', 'X', '1', '5'),
    //     name: "bgrx5551",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '1', '5'),
    //     name: "argb1555",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '1', '5'),
    //     name: "abgr1555",
    // },
    // Format {
    //     id: fourcc_code('R', 'A', '1', '5'),
    //     name: "rgba5551",
    // },
    // Format {
    //     id: fourcc_code('B', 'A', '1', '5'),
    //     name: "bgra5551",
    // },
    // Format {
    //     id: fourcc_code('R', 'G', '1', '6'),
    //     name: "rgb565",
    // },
    // Format {
    //     id: fourcc_code('B', 'G', '1', '6'),
    //     name: "bgr565",
    // },
    // Format {
    //     id: fourcc_code('R', 'G', '2', '4'),
    //     name: "rgb888",
    // },
    // Format {
    //     id: fourcc_code('B', 'G', '2', '4'),
    //     name: "bgr888",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '2', '4'),
    //     name: "xrgb8888",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '2', '4'),
    //     name: "xbgr8888",
    // },
    // Format {
    //     id: fourcc_code('R', 'X', '2', '4'),
    //     name: "rgbx8888",
    // },
    // Format {
    //     id: fourcc_code('B', 'X', '2', '4'),
    //     name: "bgrx8888",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '2', '4'),
    //     name: "argb8888",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '2', '4'),
    //     name: "abgr8888",
    // },
    // Format {
    //     id: fourcc_code('R', 'A', '2', '4'),
    //     name: "rgba8888",
    // },
    // Format {
    //     id: fourcc_code('B', 'A', '2', '4'),
    //     name: "bgra8888",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '3', '0'),
    //     name: "xrgb2101010",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '3', '0'),
    //     name: "xbgr2101010",
    // },
    // Format {
    //     id: fourcc_code('R', 'X', '3', '0'),
    //     name: "rgbx1010102",
    // },
    // Format {
    //     id: fourcc_code('B', 'X', '3', '0'),
    //     name: "bgrx1010102",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '3', '0'),
    //     name: "argb2101010",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '3', '0'),
    //     name: "abgr2101010",
    // },
    // Format {
    //     id: fourcc_code('R', 'A', '3', '0'),
    //     name: "rgba1010102",
    // },
    // Format {
    //     id: fourcc_code('B', 'A', '3', '0'),
    //     name: "bgra1010102",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '4', '8'),
    //     name: "xrgb16161616",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '4', '8'),
    //     name: "xbgr16161616",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '4', '8'),
    //     name: "argb16161616",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '4', '8'),
    //     name: "abgr16161616",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', '4', 'H'),
    //     name: "xrgb16161616f",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', '4', 'H'),
    //     name: "xbgr16161616f",
    // },
    // Format {
    //     id: fourcc_code('A', 'R', '4', 'H'),
    //     name: "argb16161616f",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '4', 'H'),
    //     name: "abgr16161616f",
    // },
    // Format {
    //     id: fourcc_code('A', 'B', '1', '0'),
    //     name: "axbxgxrx106106106106",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', 'Y', 'V'),
    //     name: "yuyv",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', 'Y', 'U'),
    //     name: "yvyu",
    // },
    // Format {
    //     id: fourcc_code('U', 'Y', 'V', 'Y'),
    //     name: "uyvy",
    // },
    // Format {
    //     id: fourcc_code('V', 'Y', 'U', 'Y'),
    //     name: "vyuy",
    // },
    // Format {
    //     id: fourcc_code('A', 'Y', 'U', 'V'),
    //     name: "ayuv",
    // },
    // Format {
    //     id: fourcc_code('X', 'Y', 'U', 'V'),
    //     name: "xyuv8888",
    // },
    // Format {
    //     id: fourcc_code('V', 'U', '2', '4'),
    //     name: "vuy888",
    // },
    // Format {
    //     id: fourcc_code('V', 'U', '3', '0'),
    //     name: "vuy101010",
    // },
    // Format {
    //     id: fourcc_code('Y', '2', '1', '0'),
    //     name: "y210",
    // },
    // Format {
    //     id: fourcc_code('Y', '2', '1', '2'),
    //     name: "y212",
    // },
    // Format {
    //     id: fourcc_code('Y', '2', '1', '6'),
    //     name: "y216",
    // },
    // Format {
    //     id: fourcc_code('Y', '4', '1', '0'),
    //     name: "y410",
    // },
    // Format {
    //     id: fourcc_code('Y', '4', '1', '2'),
    //     name: "y412",
    // },
    // Format {
    //     id: fourcc_code('Y', '4', '1', '6'),
    //     name: "y416",
    // },
    // Format {
    //     id: fourcc_code('X', 'V', '3', '0'),
    //     name: "xvyu2101010",
    // },
    // Format {
    //     id: fourcc_code('X', 'V', '3', '6'),
    //     name: "xvyu12_16161616",
    // },
    // Format {
    //     id: fourcc_code('X', 'V', '4', '8'),
    //     name: "xvyu16161616",
    // },
    // Format {
    //     id: fourcc_code('Y', '0', 'L', '0'),
    //     name: "y0l0",
    // },
    // Format {
    //     id: fourcc_code('X', '0', 'L', '0'),
    //     name: "x0l0",
    // },
    // Format {
    //     id: fourcc_code('Y', '0', 'L', '2'),
    //     name: "y0l2",
    // },
    // Format {
    //     id: fourcc_code('X', '0', 'L', '2'),
    //     name: "x0l2",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '0', '8'),
    //     name: "yuv420_8bit",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '1', '0'),
    //     name: "yuv420_10bit",
    // },
    // Format {
    //     id: fourcc_code('X', 'R', 'A', '8'),
    //     name: "xrgb8888_a8",
    // },
    // Format {
    //     id: fourcc_code('X', 'B', 'A', '8'),
    //     name: "xbgr8888_a8",
    // },
    // Format {
    //     id: fourcc_code('R', 'X', 'A', '8'),
    //     name: "rgbx8888_a8",
    // },
    // Format {
    //     id: fourcc_code('B', 'X', 'A', '8'),
    //     name: "bgrx8888_a8",
    // },
    // Format {
    //     id: fourcc_code('R', '8', 'A', '8'),
    //     name: "rgb888_a8",
    // },
    // Format {
    //     id: fourcc_code('B', '8', 'A', '8'),
    //     name: "bgr888_a8",
    // },
    // Format {
    //     id: fourcc_code('R', '5', 'A', '8'),
    //     name: "rgb565_a8",
    // },
    // Format {
    //     id: fourcc_code('B', '5', 'A', '8'),
    //     name: "bgr565_a8",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '1', '2'),
    //     name: "nv12",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '2', '1'),
    //     name: "nv21",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '1', '6'),
    //     name: "nv16",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '6', '1'),
    //     name: "nv61",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '2', '4'),
    //     name: "nv24",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '4', '2'),
    //     name: "nv42",
    // },
    // Format {
    //     id: fourcc_code('N', 'V', '1', '5'),
    //     name: "nv15",
    // },
    // Format {
    //     id: fourcc_code('P', '2', '1', '0'),
    //     name: "p210",
    // },
    // Format {
    //     id: fourcc_code('P', '0', '1', '0'),
    //     name: "p010",
    // },
    // Format {
    //     id: fourcc_code('P', '0', '1', '2'),
    //     name: "p012",
    // },
    // Format {
    //     id: fourcc_code('P', '0', '1', '6'),
    //     name: "p016",
    // },
    // Format {
    //     id: fourcc_code('Q', '4', '1', '0'),
    //     name: "q410",
    // },
    // Format {
    //     id: fourcc_code('Q', '4', '0', '1'),
    //     name: "q401",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', 'V', '9'),
    //     name: "yuv410",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', 'U', '9'),
    //     name: "yvu410",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '1', '1'),
    //     name: "yuv411",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', '1', '1'),
    //     name: "yvu411",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '1', '2'),
    //     name: "yuv420",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', '1', '2'),
    //     name: "yvu420",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '1', '6'),
    //     name: "yuv422",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', '1', '6'),
    //     name: "yvu422",
    // },
    // Format {
    //     id: fourcc_code('Y', 'U', '2', '4'),
    //     name: "yuv444",
    // },
    // Format {
    //     id: fourcc_code('Y', 'V', '2', '4'),
    //     name: "yvu444",
    // },
];
