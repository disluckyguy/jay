use {
    crate::open,
    repc::layout::{Type, TypeVariant},
    std::{env, io::Write},
};

#[expect(unused_macros)]
#[macro_use]
#[path = "../src/macros.rs"]
mod macros;

#[path = "../src/libinput/consts.rs"]
mod libinput;

#[path = "../src/pango/consts.rs"]
mod pango;

fn get_target() -> repc::Target {
    let rustc_target = env::var("TARGET").unwrap();
    repc::TARGET_MAP
        .iter()
        .cloned()
        .find(|t| t.0 == rustc_target)
        .unwrap()
        .1
}

fn get_enum_ty(variants: Vec<i128>) -> anyhow::Result<u64> {
    let target = get_target();
    let ty = Type {
        layout: (),
        annotations: vec![],
        variant: TypeVariant::Enum(variants),
    };
    let ty = repc::compute_layout(target, &ty)?;
    assert!(ty.layout.pointer_alignment_bits <= ty.layout.size_bits);
    Ok(ty.layout.size_bits)
}

fn write_ty<W: Write>(f: &mut W, vals: &[i32], ty: &str) -> anyhow::Result<()> {
    let variants: Vec<_> = vals.iter().cloned().map(|v| v as i128).collect();
    let size = get_enum_ty(variants)?;
    writeln!(f, "#[allow(clippy::allow_attributes, dead_code)]")?;
    writeln!(f, "pub type {} = i{};", ty, size)?;
    Ok(())
}

pub fn main() -> anyhow::Result<()> {
    let mut f = open("libinput_tys.rs")?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_LOG_PRIORITY,
        "libinput_log_priority",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_DEVICE_CAPABILITY,
        "libinput_device_capability",
    )?;
    write_ty(&mut f, libinput::LIBINPUT_KEY_STATE, "libinput_key_state")?;
    write_ty(&mut f, libinput::LIBINPUT_LED, "libinput_led")?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_BUTTON_STATE,
        "libinput_button_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_POINTER_AXIS,
        "libinput_pointer_axis",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_POINTER_AXIS_SOURCE,
        "libinput_pointer_axis_source",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_TABLET_PAD_RING_AXIS_SOURCE,
        "libinput_tablet_pad_ring_axis_source",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_TABLET_PAD_STRIP_AXIS_SOURCE,
        "libinput_tablet_pad_strip_axis_source",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_TABLET_TOOL_TYPE,
        "libinput_tablet_tool_type",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_TABLET_TOOL_PROXIMITY_STATE,
        "libinput_tablet_tool_proximity_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_TABLET_TOOL_TIP_STATE,
        "libinput_tablet_tool_tip_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_SWITCH_STATE,
        "libinput_switch_state",
    )?;
    write_ty(&mut f, libinput::LIBINPUT_SWITCH, "libinput_switch")?;
    write_ty(&mut f, libinput::LIBINPUT_EVENT_TYPE, "libinput_event_type")?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_STATUS,
        "libinput_config_status",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_ACCEL_PROFILE,
        "libinput_config_accel_profile",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_TAP_STATE,
        "libinput_config_tap_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_DRAG_STATE,
        "libinput_config_drag_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_DRAG_LOCK_STATE,
        "libinput_config_drag_lock_state",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_CLICK_METHOD,
        "libinput_config_click_method",
    )?;
    write_ty(
        &mut f,
        libinput::LIBINPUT_CONFIG_MIDDLE_EMULATION_STATE,
        "libinput_config_middle_emulation_state",
    )?;

    let mut f = open("pango_tys.rs")?;
    write_ty(&mut f, pango::CAIRO_FORMATS, "cairo_format_t")?;
    write_ty(&mut f, pango::CAIRO_STATUSES, "cairo_status_t")?;
    write_ty(&mut f, pango::CAIRO_OPERATORS, "cairo_operator_t")?;
    write_ty(&mut f, pango::PANGO_ELLIPSIZE_MODES, "PangoEllipsizeMode_")?;

    Ok(())
}
