#[cfg(feature = "it")]
use crate::it::test_backend::TestBackend;
use {
    crate::{
        acceptor::{Acceptor, AcceptorError},
        async_engine::{AsyncEngine, Phase, SpawnedFuture},
        backend::{self, Backend},
        backends::{
            dummy::{DummyBackend, DummyOutput},
            metal, x,
        },
        cli::{CliBackend, GlobalArgs, RunArgs},
        client::{ClientId, Clients},
        clientmem::{self, ClientMemError},
        config::ConfigProxy,
        dbus::Dbus,
        fixed::Fixed,
        forker,
        globals::Globals,
        ifs::{wl_output::WlOutputGlobal, wl_surface::NoneSurfaceExt},
        io_uring::{IoUring, IoUringError},
        leaks,
        logger::Logger,
        render::{self, RenderError},
        sighand::{self, SighandError},
        state::{ConnectorData, IdleState, ScreenlockState, State, XWaylandState},
        tasks::{self, idle},
        tree::{
            container_layout, container_render_data, float_layout, float_titles,
            output_render_data, DisplayNode, NodeIds, OutputNode, WorkspaceNode,
        },
        user_session::import_environment,
        utils::{
            clonecell::CloneCell, errorfmt::ErrorFmt, fdcloser::FdCloser, numcell::NumCell,
            oserror::OsError, queue::AsyncQueue, refcounted::RefCounted, run_toplevel::RunToplevel,
            tri::Try,
        },
        wheel::{Wheel, WheelError},
        xkbcommon::XkbContext,
    },
    ahash::AHashSet,
    forker::ForkerProxy,
    std::{cell::Cell, env, future::Future, ops::Deref, rc::Rc, sync::Arc, time::Duration},
    thiserror::Error,
    uapi::c,
};

pub const MAX_EXTENTS: i32 = (1 << 22) - 1;

pub fn start_compositor(global: GlobalArgs, args: RunArgs) {
    let forker = create_forker();
    let logger = Logger::install_compositor(global.log_level.into());
    let res = start_compositor2(Some(forker), Some(logger.clone()), args, None);
    leaks::log_leaked();
    if let Err(e) = res {
        let e = ErrorFmt(e);
        log::error!("A fatal error occurred: {}", e);
        eprintln!("A fatal error occurred: {}", e);
        eprintln!("See {} for more details.", logger.path());
        std::process::exit(1);
    }
    log::info!("Exit");
}

#[cfg(feature = "it")]
pub fn start_compositor_for_test(future: TestFuture) -> Result<(), CompositorError> {
    let res = start_compositor2(None, None, RunArgs::default(), Some(future));
    leaks::log_leaked();
    res
}

fn create_forker() -> Rc<ForkerProxy> {
    match ForkerProxy::create() {
        Ok(f) => Rc::new(f),
        Err(e) => fatal!("Could not create a forker process: {}", ErrorFmt(e)),
    }
}

#[derive(Debug, Error)]
pub enum CompositorError {
    #[error("The client acceptor caused an error")]
    AcceptorError(#[from] AcceptorError),
    #[error("The signal handler caused an error")]
    SighandError(#[from] SighandError),
    #[error("The clientmem subsystem caused an error")]
    ClientmemError(#[from] ClientMemError),
    #[error("The timer subsystem caused an error")]
    WheelError(#[from] WheelError),
    #[error("The render backend caused an error")]
    RenderError(#[from] RenderError),
    #[error("Could not create an io-uring")]
    IoUringError(#[from] IoUringError),
}

pub const WAYLAND_DISPLAY: &str = "WAYLAND_DISPLAY";
pub const DISPLAY: &str = "DISPLAY";

const STATIC_VARS: &[(&str, &str)] = &[
    ("XDG_CURRENT_DESKTOP", "jay"),
    ("XDG_SESSION_TYPE", "wayland"),
    ("_JAVA_AWT_WM_NONREPARENTING", "1"),
];

pub type TestFuture = Box<dyn Fn(&Rc<State>) -> Box<dyn Future<Output = ()>>>;

fn start_compositor2(
    forker: Option<Rc<ForkerProxy>>,
    logger: Option<Arc<Logger>>,
    run_args: RunArgs,
    test_future: Option<TestFuture>,
) -> Result<(), CompositorError> {
    log::info!("pid = {}", uapi::getpid());
    init_fd_limit();
    leaks::init();
    render::init()?;
    clientmem::init()?;
    let xkb_ctx = XkbContext::new().unwrap();
    let xkb_keymap = xkb_ctx.keymap_from_str(include_str!("keymap.xkb")).unwrap();
    let engine = AsyncEngine::new();
    let ring = IoUring::new(&engine, 32)?;
    let _signal_future = sighand::install(&engine, &ring)?;
    let wheel = Wheel::new(&engine, &ring)?;
    let (_run_toplevel_future, run_toplevel) = RunToplevel::install(&engine);
    let node_ids = NodeIds::default();
    let scales = RefCounted::default();
    scales.add(Fixed::from_int(1));
    let state = Rc::new(State {
        xkb_ctx,
        backend: CloneCell::new(Rc::new(DummyBackend)),
        forker: Default::default(),
        default_keymap: xkb_keymap,
        eng: engine.clone(),
        render_ctx: Default::default(),
        render_ctx_version: NumCell::new(1),
        render_ctx_ever_initialized: Cell::new(false),
        cursors: Default::default(),
        wheel,
        clients: Clients::new(),
        globals: Globals::new(),
        connector_ids: Default::default(),
        root: Rc::new(DisplayNode::new(node_ids.next())),
        workspaces: Default::default(),
        dummy_output: Default::default(),
        node_ids,
        backend_events: AsyncQueue::new(),
        seat_ids: Default::default(),
        seat_queue: Default::default(),
        slow_clients: AsyncQueue::new(),
        none_surface_ext: Rc::new(NoneSurfaceExt),
        tree_changed_sent: Cell::new(false),
        config: Default::default(),
        input_device_ids: Default::default(),
        input_device_handlers: Default::default(),
        theme: Default::default(),
        pending_container_layout: Default::default(),
        pending_container_render_data: Default::default(),
        pending_output_render_data: Default::default(),
        pending_float_layout: Default::default(),
        pending_float_titles: Default::default(),
        dbus: Dbus::new(&engine, &ring, &run_toplevel),
        fdcloser: FdCloser::new(),
        logger,
        connectors: Default::default(),
        outputs: Default::default(),
        drm_devs: Default::default(),
        status: Default::default(),
        idle: IdleState {
            input: Default::default(),
            change: Default::default(),
            timeout: Cell::new(Duration::from_secs(10 * 60)),
            timeout_changed: Default::default(),
            inhibitors: Default::default(),
            inhibitors_changed: Default::default(),
        },
        run_args,
        xwayland: XWaylandState {
            enabled: Cell::new(true),
            handler: Default::default(),
            queue: Default::default(),
        },
        acceptor: Default::default(),
        serial: Default::default(),
        idle_inhibitor_ids: Default::default(),
        run_toplevel,
        config_dir: config_dir(),
        config_file_id: NumCell::new(1),
        tracker: Default::default(),
        data_offer_ids: Default::default(),
        drm_dev_ids: Default::default(),
        ring: ring.clone(),
        lock: ScreenlockState {
            locked: Cell::new(false),
            lock: Default::default(),
        },
        scales,
        cursor_sizes: Default::default(),
        hardware_tick_cursor: Default::default(),
        testers: Default::default(),
        workspace_watchers: Default::default(),
        render_ctx_watchers: Default::default(),
    });
    state.tracker.register(ClientId::from_raw(0));
    create_dummy_output(&state);
    let (acceptor, _acceptor_future) = Acceptor::install(&state)?;
    if let Some(forker) = forker {
        forker.install(&state);
        forker.setenv(
            WAYLAND_DISPLAY.as_bytes(),
            acceptor.socket_name().as_bytes(),
        );
        for (key, val) in STATIC_VARS {
            forker.setenv(key.as_bytes(), val.as_bytes());
        }
    }
    let _compositor = engine.spawn(start_compositor3(state.clone(), test_future));
    ring.run()?;
    state.clear();
    Ok(())
}

async fn start_compositor3(state: Rc<State>, test_future: Option<TestFuture>) {
    let is_test = test_future.is_some();

    let backend = match create_backend(&state, test_future).await {
        Some(b) => b,
        _ => {
            log::error!("Could not create a backend");
            state.ring.stop();
            return;
        }
    };
    state.backend.set(backend.clone());
    state.globals.add_backend_singletons(&backend);

    if backend.import_environment() {
        if let Some(acc) = state.acceptor.get() {
            import_environment(&state, WAYLAND_DISPLAY, acc.socket_name());
        }
        for (key, val) in STATIC_VARS {
            import_environment(&state, key, val);
        }
    }

    let config = load_config(&state, is_test);
    config.configure(false);
    state.config.set(Some(Rc::new(config)));

    let _geh = start_global_event_handlers(&state, &backend);
    state.start_xwayland();

    match backend.run().await {
        Err(e) => log::error!("Backend failed: {}", ErrorFmt(e.deref())),
        _ => log::error!("Backend stopped without an error"),
    }
    state.ring.stop();
}

fn load_config(state: &Rc<State>, #[allow(unused_variables)] for_test: bool) -> ConfigProxy {
    #[cfg(feature = "it")]
    if for_test {
        return ConfigProxy::for_test(state);
    }
    match ConfigProxy::from_config_dir(state) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Could not load config.so: {}", ErrorFmt(e));
            log::warn!("Using default config");
            ConfigProxy::default(state)
        }
    }
}

fn start_global_event_handlers(
    state: &Rc<State>,
    backend: &Rc<dyn Backend>,
) -> Vec<SpawnedFuture<()>> {
    let eng = &state.eng;
    let mut res = vec![];

    res.push(eng.spawn(tasks::handle_backend_events(state.clone())));
    res.push(eng.spawn(tasks::handle_slow_clients(state.clone())));
    res.push(eng.spawn(tasks::handle_hardware_cursor_tick(state.clone())));
    res.push(eng.spawn2(Phase::Layout, container_layout(state.clone())));
    res.push(eng.spawn2(Phase::PostLayout, container_render_data(state.clone())));
    res.push(eng.spawn2(Phase::PostLayout, output_render_data(state.clone())));
    res.push(eng.spawn2(Phase::Layout, float_layout(state.clone())));
    res.push(eng.spawn2(Phase::PostLayout, float_titles(state.clone())));
    res.push(eng.spawn2(Phase::PostLayout, idle(state.clone(), backend.clone())));

    res
}

async fn create_backend(
    state: &Rc<State>,
    #[allow(unused_variables)] test_future: Option<TestFuture>,
) -> Option<Rc<dyn Backend>> {
    #[cfg(feature = "it")]
    if let Some(tf) = test_future {
        return Some(Rc::new(TestBackend::new(state, tf)));
    }
    let mut backends = &state.run_args.backends[..];
    if backends.is_empty() {
        backends = &[CliBackend::X11, CliBackend::Metal];
    }
    let mut tried_backends = AHashSet::new();
    for &backend in backends {
        if !tried_backends.insert(backend) {
            continue;
        }
        match backend {
            CliBackend::X11 => {
                log::info!("Trying to create X backend");
                match x::create(state).await {
                    Ok(b) => return Some(b),
                    Err(e) => {
                        log::error!("Could not create X backend: {}", ErrorFmt(e));
                    }
                }
            }
            CliBackend::Metal => {
                log::info!("Trying to create metal backend");
                match metal::create(state).await {
                    Ok(b) => return Some(b),
                    Err(e) => {
                        log::error!("Could not create metal backend: {}", ErrorFmt(e));
                    }
                }
            }
        }
    }
    None
}

fn init_fd_limit() {
    let res = OsError::tri(|| {
        let mut cur = uapi::getrlimit(c::RLIMIT_NOFILE as _)?;
        if cur.rlim_cur < cur.rlim_max {
            log::info!(
                "Increasing file descriptor limit from {} to {}",
                cur.rlim_cur,
                cur.rlim_max
            );
            cur.rlim_cur = cur.rlim_max;
            uapi::setrlimit(c::RLIMIT_NOFILE as _, &cur)?;
        }
        Ok(())
    });
    if let Err(e) = res {
        log::warn!("Could not increase file descriptor limit: {}", ErrorFmt(e));
    }
}

fn create_dummy_output(state: &Rc<State>) {
    let dummy_output = Rc::new(OutputNode {
        id: state.node_ids.next(),
        global: Rc::new(WlOutputGlobal::new(
            state.globals.name(),
            state,
            &Rc::new(ConnectorData {
                connector: Rc::new(DummyOutput {
                    id: state.connector_ids.next(),
                }),
                handler: Cell::new(None),
                connected: Cell::new(true),
                name: "Dummy".to_string(),
                drm_dev: None,
                async_event: Default::default(),
            }),
            0,
            &backend::Mode {
                width: 0,
                height: 0,
                refresh_rate_millihz: 0,
            },
            "jay",
            "dummy-output",
            "0",
            0,
            0,
        )),
        jay_outputs: Default::default(),
        workspaces: Default::default(),
        workspace: Default::default(),
        seat_state: Default::default(),
        layers: Default::default(),
        render_data: Default::default(),
        state: state.clone(),
        is_dummy: true,
        status: Default::default(),
        scroll: Default::default(),
        pointer_positions: Default::default(),
        lock_surface: Default::default(),
        preferred_scale: Cell::new(Fixed::from_int(1)),
        hardware_cursor: Default::default(),
        update_render_data_scheduled: Default::default(),
    });
    let dummy_workspace = Rc::new(WorkspaceNode {
        id: state.node_ids.next(),
        is_dummy: true,
        output: CloneCell::new(dummy_output.clone()),
        position: Default::default(),
        container: Default::default(),
        stacked: Default::default(),
        seat_state: Default::default(),
        name: "dummy".to_string(),
        output_link: Default::default(),
        visible: Default::default(),
        fullscreen: Default::default(),
        visible_on_desired_output: Default::default(),
        desired_output: CloneCell::new(dummy_output.global.output_id.clone()),
        jay_workspaces: Default::default(),
    });
    dummy_workspace.output_link.set(Some(
        dummy_output.workspaces.add_last(dummy_workspace.clone()),
    ));
    dummy_output.show_workspace(&dummy_workspace);
    state.dummy_output.set(Some(dummy_output));
}

fn config_dir() -> Option<String> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        Some(format!("{}/jay", xdg))
    } else if let Ok(home) = env::var("HOME") {
        Some(format!("{}/.config/jay", home))
    } else {
        log::warn!("Neither XDG_CONFIG_HOME nor HOME are set. Using default config.");
        None
    }
}
