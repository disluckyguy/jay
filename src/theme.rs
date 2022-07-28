use std::cell::{Cell, RefCell};

#[derive(Copy, Clone, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

fn to_f32(c: u8) -> f32 {
    c as f32 / 255f32
}

fn to_u8(c: f32) -> u8 {
    (c * 255f32) as u8
}

impl Color {
    pub fn from_gray(g: u8) -> Self {
        Self::from_rgb(g, g, g)
    }

    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: to_f32(r),
            g: to_f32(g),
            b: to_f32(b),
            a: 1.0,
        }
    }

    pub fn from_rgba_straight(r: u8, g: u8, b: u8, a: u8) -> Self {
        let alpha = to_f32(a);
        Self {
            r: to_f32(r) * alpha,
            g: to_f32(g) * alpha,
            b: to_f32(b) * alpha,
            a: alpha,
        }
    }

    #[cfg_attr(not(feature = "it"), allow(dead_code))]
    pub fn to_rgba_premultiplied(self) -> [u8; 4] {
        [to_u8(self.r), to_u8(self.g), to_u8(self.b), to_u8(self.a)]
    }
}

impl From<jay_config::theme::Color> for Color {
    fn from(f: jay_config::theme::Color) -> Self {
        let [r, g, b, a] = f.to_f32_premultiplied();
        Self { r, g, b, a }
    }
}

macro_rules! colors {
    ($($name:ident = ($r:expr, $g:expr, $b:expr),)*) => {
        pub struct ThemeColors {
            $(
                pub $name: Cell<Color>,
            )*
        }

        impl ThemeColors {
            pub fn reset(&self) {
                let default = Self::default();
                $(
                    self.$name.set(default.$name.get());
                )*
            }
        }

        impl Default for ThemeColors {
            fn default() -> Self {
                Self {
                    $(
                        $name: Cell::new(Color::from_rgb($r, $g, $b)),
                    )*
                }
            }
        }
    }
}

colors! {
    background = (0x00, 0x10, 0x19),
    unfocused_title_background = (0x22, 0x22, 0x22),
    focused_title_background = (0x28, 0x55, 0x77),
    focused_inactive_title_background = (0x5f, 0x67, 0x6a),
    unfocused_title_text = (0x88, 0x88, 0x88),
    focused_title_text = (0xff, 0xff, 0xff),
    focused_inactive_title_text = (0xff, 0xff, 0xff),
    separator = (0x33, 0x33, 0x33),
    border = (0x3f, 0x47, 0x4a),
    bar_background = (0x00, 0x00, 0x00),
    bar_text = (0xff, 0xff, 0xff),
}

macro_rules! sizes {
    ($($name:ident = ($min:expr, $max:expr, $def:expr),)*) => {
        pub struct ThemeSizes {
            $(
                pub $name: Cell<i32>,
            )*
        }

        #[derive(Copy, Clone, Debug)]
        #[allow(non_camel_case_types)]
        pub enum ThemeSized {
            $(
                $name,
            )*
        }

        impl ThemeSized {
            pub fn min(self) -> i32 {
                match self {
                    $(
                        Self::$name => $min,
                    )*
                }
            }

            pub fn max(self) -> i32 {
                match self {
                    $(
                        Self::$name => $max,
                    )*
                }
            }

            pub fn field(self, theme: &Theme) -> &Cell<i32> {
                let sizes = &theme.sizes;
                match self {
                    $(
                        Self::$name => &sizes.$name,
                    )*
                }
            }

            pub fn name(self) -> &'static str {
                match self {
                    $(
                        Self::$name => stringify!($name),
                    )*
                }
            }
        }

        impl ThemeSizes {
            pub fn reset(&self) {
                let default = Self::default();
                $(
                    self.$name.set(default.$name.get());
                )*
            }
        }

        impl Default for ThemeSizes {
            fn default() -> Self {
                Self {
                    $(
                        $name: Cell::new($def),
                    )*
                }
            }
        }
    }
}

sizes! {
    title_height = (1, 1000, 17),
    border_width = (1, 1000, 4),
}

pub const DEFAULT_FONT: &str = "monospace 8";

pub struct Theme {
    pub colors: ThemeColors,
    pub sizes: ThemeSizes,
    pub font: RefCell<String>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            colors: Default::default(),
            sizes: Default::default(),
            font: RefCell::new(DEFAULT_FONT.to_string()),
        }
    }
}
