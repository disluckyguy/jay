use {
    crate::{
        cli::CliLogLevel,
        client::{Client, ClientError},
        globals::{Global, GlobalName},
        ifs::{
            jay_idle::JayIdle, jay_log_file::JayLogFile, jay_output::JayOutput,
            jay_render_ctx::JayRenderCtx, jay_screencast::JayScreencast,
            jay_screenshot::JayScreenshot, jay_seat_events::JaySeatEvents,
            jay_workspace_watcher::JayWorkspaceWatcher,
        },
        leaks::Tracker,
        object::Object,
        screenshoter::take_screenshot,
        utils::{
            buffd::{MsgParser, MsgParserError},
            clonecell::CloneCell,
            errorfmt::ErrorFmt,
        },
        wire::{jay_compositor::*, JayCompositorId},
    },
    bstr::ByteSlice,
    log::Level,
    std::{ops::Deref, rc::Rc},
    thiserror::Error,
};

pub struct JayCompositorGlobal {
    name: GlobalName,
}

impl JayCompositorGlobal {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: JayCompositorId,
        client: &Rc<Client>,
        _version: u32,
    ) -> Result<(), JayCompositorError> {
        let obj = Rc::new(JayCompositor {
            id,
            client: client.clone(),
            tracker: Default::default(),
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        Ok(())
    }
}

global_base!(JayCompositorGlobal, JayCompositor, JayCompositorError);

impl Global for JayCompositorGlobal {
    fn singleton(&self) -> bool {
        true
    }

    fn version(&self) -> u32 {
        1
    }

    fn secure(&self) -> bool {
        true
    }
}

simple_add_global!(JayCompositorGlobal);

pub struct JayCompositor {
    id: JayCompositorId,
    client: Rc<Client>,
    tracker: Tracker<Self>,
}

impl JayCompositor {
    fn destroy(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: Destroy = self.client.parse(self, parser)?;
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_log_file(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: GetLogFile = self.client.parse(self, parser)?;
        let log_file = Rc::new(JayLogFile::new(req.id, &self.client));
        track!(self.client, log_file);
        self.client.add_client_obj(&log_file)?;
        let path = match &self.client.state.logger {
            Some(logger) => logger.path(),
            _ => "".as_bytes().as_bstr(),
        };
        log_file.send_path(path);
        Ok(())
    }

    fn quit(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: Quit = self.client.parse(self, parser)?;
        log::info!("Quitting");
        self.client.state.ring.stop();
        Ok(())
    }

    fn set_log_level(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: SetLogLevel = self.client.parse(self, parser)?;
        const ERROR: u32 = CliLogLevel::Error as u32;
        const WARN: u32 = CliLogLevel::Warn as u32;
        const INFO: u32 = CliLogLevel::Info as u32;
        const DEBUG: u32 = CliLogLevel::Debug as u32;
        const TRACE: u32 = CliLogLevel::Trace as u32;
        let level = match req.level {
            ERROR => Level::Error,
            WARN => Level::Warn,
            INFO => Level::Info,
            DEBUG => Level::Debug,
            TRACE => Level::Trace,
            _ => return Err(JayCompositorError::UnknownLogLevel(req.level)),
        };
        if let Some(logger) = &self.client.state.logger {
            logger.set_level(level);
        }
        Ok(())
    }

    fn take_screenshot(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: TakeScreenshot = self.client.parse(self, parser)?;
        let ss = Rc::new(JayScreenshot {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
        });
        track!(self.client, ss);
        self.client.add_client_obj(&ss)?;
        match take_screenshot(&self.client.state) {
            Ok(s) => {
                let dmabuf = s.bo.dmabuf();
                let plane = &dmabuf.planes[0];
                ss.send_dmabuf(
                    &s.drm,
                    &plane.fd,
                    dmabuf.width,
                    dmabuf.height,
                    plane.offset,
                    plane.stride,
                );
            }
            Err(e) => {
                let msg = ErrorFmt(e).to_string();
                ss.send_error(&msg);
            }
        }
        self.client.remove_obj(ss.deref())?;
        Ok(())
    }

    fn get_idle(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: GetIdle = self.client.parse(self, parser)?;
        let idle = Rc::new(JayIdle {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
        });
        track!(self.client, idle);
        self.client.add_client_obj(&idle)?;
        Ok(())
    }

    fn get_client_id(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: GetClientId = self.client.parse(self, parser)?;
        self.client.event(ClientId {
            self_id: self.id,
            client_id: self.client.id.raw(),
        });
        Ok(())
    }

    fn enable_symmetric_delete(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: EnableSymmetricDelete = self.client.parse(self, parser)?;
        self.client.symmetric_delete.set(true);
        Ok(())
    }

    fn unlock(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: Unlock = self.client.parse(self, parser)?;
        let state = &self.client.state;
        if state.lock.locked.replace(false) {
            if let Some(lock) = state.lock.lock.take() {
                lock.finish();
            }
            for output in state.outputs.lock().values() {
                if let Some(surface) = output.node.lock_surface.take() {
                    surface.destroy_node();
                }
            }
            state.tree_changed();
            state.damage();
        }
        self.client.symmetric_delete.set(true);
        Ok(())
    }

    fn get_seats(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let _req: GetSeats = self.client.parse(self, parser)?;
        for seat in self.client.state.globals.seats.lock().values() {
            self.client.event(Seat {
                self_id: self.id,
                id: seat.id().raw(),
                name: seat.seat_name(),
            })
        }
        Ok(())
    }

    fn seat_events(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: SeatEvents = self.client.parse(self, parser)?;
        let se = Rc::new(JaySeatEvents {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
        });
        track!(self.client, se);
        self.client.add_client_obj(&se)?;
        self.client
            .state
            .testers
            .borrow_mut()
            .insert((self.client.id, req.id), se);
        Ok(())
    }

    fn create_screencast(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: CreateScreencast = self.client.parse(self, parser)?;
        let sc = Rc::new(JayScreencast::new(req.id, &self.client));
        track!(self.client, sc);
        self.client.add_client_obj(&sc)?;
        Ok(())
    }

    fn get_output(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: GetOutput = self.client.parse(self, parser)?;
        let output = self.client.lookup(req.output)?;
        let jo = Rc::new(JayOutput {
            id: req.id,
            client: self.client.clone(),
            output: CloneCell::new(output.global.node.get()),
            tracker: Default::default(),
        });
        track!(self.client, jo);
        self.client.add_client_obj(&jo)?;
        if let Some(node) = jo.output.get() {
            node.jay_outputs.set((self.client.id, req.id), jo.clone());
            jo.send_linear_id();
        } else {
            jo.send_destroyed();
        }
        Ok(())
    }

    fn watch_workspaces(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: WatchWorkspaces = self.client.parse(self, parser)?;
        let watcher = Rc::new(JayWorkspaceWatcher {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
        });
        track!(self.client, watcher);
        self.client.add_client_obj(&watcher)?;
        self.client
            .state
            .workspace_watchers
            .set((self.client.id, req.id), watcher.clone());
        for ws in self.client.state.workspaces.lock().values() {
            watcher.send_workspace(ws)?;
        }
        Ok(())
    }

    fn get_render_ctx(&self, parser: MsgParser<'_, '_>) -> Result<(), JayCompositorError> {
        let req: GetRenderCtx = self.client.parse(self, parser)?;
        let ctx = Rc::new(JayRenderCtx {
            id: req.id,
            client: self.client.clone(),
            tracker: Default::default(),
        });
        track!(self.client, ctx);
        self.client.add_client_obj(&ctx)?;
        self.client
            .state
            .render_ctx_watchers
            .set((self.client.id, req.id), ctx.clone());
        let rctx = self.client.state.render_ctx.get();
        ctx.send_render_ctx(rctx.as_ref());
        Ok(())
    }
}

object_base! {
    JayCompositor;

    DESTROY => destroy,
    GET_LOG_FILE => get_log_file,
    QUIT => quit,
    SET_LOG_LEVEL => set_log_level,
    TAKE_SCREENSHOT => take_screenshot,
    GET_IDLE => get_idle,
    GET_CLIENT_ID => get_client_id,
    ENABLE_SYMMETRIC_DELETE => enable_symmetric_delete,
    UNLOCK => unlock,
    GET_SEATS => get_seats,
    SEAT_EVENTS => seat_events,
    CREATE_SCREENCAST => create_screencast,
    GET_OUTPUT => get_output,
    WATCH_WORKSPACES => watch_workspaces,
    GET_RENDER_CTX => get_render_ctx,
}

impl Object for JayCompositor {
    fn num_requests(&self) -> u32 {
        GET_RENDER_CTX + 1
    }
}

simple_add_obj!(JayCompositor);

#[derive(Debug, Error)]
pub enum JayCompositorError {
    #[error("Parsing failed")]
    MsgParserError(#[source] Box<MsgParserError>),
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error("Unknown log level {0}")]
    UnknownLogLevel(u32),
}
efrom!(JayCompositorError, ClientError);
efrom!(JayCompositorError, MsgParserError);
