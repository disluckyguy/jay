use {
    crate::{
        async_engine::{AsyncEngine, SpawnedFuture},
        io_uring::{IoUring, IoUringError},
        pipewire::{
            pw_formatter::{PwFormatter, format},
            pw_ifs::{
                pw_client::{PwClient, PwClientMethods},
                pw_client_node::{
                    PW_CLIENT_NODE_FACTORY, PW_CLIENT_NODE_INTERFACE, PW_CLIENT_NODE_VERSION,
                    PwClientNode,
                },
                pw_core::{PW_CORE_VERSION, PwCore, PwCoreMethods},
                pw_registry::{PW_REGISTRY_VERSION, PwRegistry},
            },
            pw_mem::PwMemPool,
            pw_object::{PwObject, PwObjectData, PwObjectError, PwOpcode},
            pw_parser::{PwParser, PwParserError},
        },
        utils::{
            bitfield::Bitfield,
            bufio::{BufIo, BufIoError, BufIoIncoming, BufIoMessage},
            clonecell::CloneCell,
            copyhashmap::CopyHashMap,
            errorfmt::ErrorFmt,
            hash_map_ext::HashMapExt,
            numcell::NumCell,
            oserror::OsError,
            xrd::xrd,
        },
    },
    std::{
        cell::{Cell, RefCell},
        fmt::Display,
        io::Write,
        rc::{Rc, Weak},
    },
    thiserror::Error,
    uapi::{OwnedFd, c},
};

#[derive(Debug, Error)]
pub enum PwConError {
    #[error("Could not create a unix socket")]
    CreateSocket(#[source] OsError),
    #[error("Could not connect to the pipewire daemon")]
    ConnectSocket(#[source] IoUringError),
    #[error(transparent)]
    BufIoError(#[from] BufIoError),
    #[error("Server did not sent a required fd")]
    MissingFd,
    #[error("XDG_RUNTIME_DIR is not set")]
    XrdNotSet,
    #[error(transparent)]
    PwObjectError(#[from] PwObjectError),
    #[error(transparent)]
    PwParserError(#[from] PwParserError),
}

pub struct PwConHolder {
    pub con: Rc<PwCon>,
    outgoing: Cell<Option<SpawnedFuture<()>>>,
    incoming: Cell<Option<SpawnedFuture<()>>>,
}

pub struct PwCon {
    send_seq: NumCell<u32>,
    pub io: Rc<BufIo>,
    holder: CloneCell<Weak<PwConHolder>>,
    dead: Cell<bool>,
    pub objects: CopyHashMap<u32, Rc<dyn PwObject>>,
    pub ids: RefCell<Bitfield>,
    pub mem: PwMemPool,
    pub ring: Rc<IoUring>,
    pub eng: Rc<AsyncEngine>,
    pub owner: CloneCell<Option<Rc<dyn PwConOwner>>>,

    registry_generation: Cell<u64>,
    ack_registry_generation: Cell<u64>,
}

pub trait PwConOwner {
    fn killed(&self) {}
}

impl PwCon {
    pub fn create_client_node(self: &Rc<Self>, props: &[(String, String)]) -> Rc<PwClientNode> {
        let node = Rc::new(PwClientNode {
            data: self.proxy_data(),
            con: self.clone(),
            ios: Default::default(),
            owner: CloneCell::new(None),
            ports: Default::default(),
            port_out_free: RefCell::new(Default::default()),
            port_in_free: RefCell::new(Default::default()),
            activation: Default::default(),
            transport_in: Cell::new(None),
            transport_out: Default::default(),
            activations: Default::default(),
        });
        if !self.dead.get() {
            self.objects.set(node.data.id, node.clone());
        }
        self.create_object(
            PW_CLIENT_NODE_FACTORY,
            PW_CLIENT_NODE_INTERFACE,
            PW_CLIENT_NODE_VERSION,
            props,
            node.data.id,
        );
        node.send_update();
        node
    }

    pub fn destroy_obj(&self, obj: &impl PwObject) {
        obj.break_loops();
        self.send2(0, "core", PwCoreMethods::Destroy, |f| {
            f.write_struct(|f| {
                f.write_uint(obj.data().id);
            });
        });
        self.objects.remove(&obj.data().id);
    }

    pub fn kill(&self) {
        for obj in self.objects.lock().drain_values() {
            obj.break_loops();
        }
        self.io.shutdown();
        self.dead.set(true);
        if let Some(con) = self.holder.get().upgrade() {
            con.outgoing.take();
            con.incoming.take();
        }
        if let Some(owner) = self.owner.take() {
            owner.killed();
        }
    }

    pub fn id(&self) -> u32 {
        self.ids.borrow_mut().acquire()
    }

    pub fn proxy_data(&self) -> PwObjectData {
        PwObjectData {
            id: self.id(),
            bound_id: Cell::new(None),
            sync_id: Default::default(),
        }
    }

    pub fn send<P, O, F>(&self, proxy: &P, opcode: O, f: F)
    where
        P: PwObject,
        O: PwOpcode,
        F: FnOnce(&mut PwFormatter),
    {
        self.send2(proxy.data().id, proxy.interface(), opcode, f);
    }

    pub fn send2<O, F>(&self, id: u32, interface: &str, opcode: O, f: F)
    where
        O: PwOpcode,
        F: FnOnce(&mut PwFormatter),
    {
        if self.dead.get() {
            return;
        }
        let mut buf = self.io.buf();
        let mut fds = vec![];
        format(
            &mut buf,
            &mut fds,
            id,
            opcode.id(),
            self.send_seq.fetch_add(1),
            |fmt| {
                f(fmt);
                if self.ack_registry_generation.get() != self.registry_generation.get() {
                    let generation = self.registry_generation.get();
                    fmt.write_struct(|f| {
                        f.write_id(FOOTER_REGISTRY_GENERATION);
                        f.write_struct(|f| {
                            f.write_ulong(generation);
                        });
                    });
                    self.ack_registry_generation.set(generation);
                }
            },
        );
        if log::log_enabled!(log::Level::Trace) {
            log::trace!("CALL {}@{}: `{:?}`:", interface, id, opcode);
            let mut parser = PwParser::new(&buf[16..buf.len()], &fds);
            while parser.len() > 0 {
                log::trace!("{:#?}", parser.read_pod().unwrap());
            }
        }
        self.io.send(BufIoMessage {
            fds,
            buf: buf.unwrap(),
        });
    }

    #[expect(dead_code)]
    pub fn sync<P: PwObject>(&self, p: &P) {
        let seq = p.data().sync_id.fetch_add(1) + 1;
        self.send2(0, "core", PwCoreMethods::Sync, |f| {
            f.write_struct(|f| {
                f.write_uint(p.data().id);
                f.write_uint(seq);
            });
        });
    }

    pub fn send_hello(&self) {
        self.send2(0, "core", PwCoreMethods::Hello, |f| {
            f.write_struct(|f| f.write_int(PW_CORE_VERSION));
        });
    }

    #[expect(dead_code)]
    pub fn get_registry(self: &Rc<Self>) -> Rc<PwRegistry> {
        let registry = Rc::new(PwRegistry {
            data: self.proxy_data(),
            _con: self.clone(),
        });
        if !self.dead.get() {
            self.objects.set(registry.data.id, registry.clone());
        }
        self.send2(0, "core", PwCoreMethods::GetRegistry, |f| {
            f.write_struct(|f| {
                f.write_int(PW_REGISTRY_VERSION);
                f.write_uint(registry.data.id);
            });
        });
        registry
    }

    pub fn create_object(
        &self,
        factory: &str,
        ty: &str,
        version: i32,
        props: &[(String, String)],
        new_id: u32,
    ) {
        self.send2(0, "core", PwCoreMethods::CreateObject, |f| {
            f.write_struct(|f| {
                f.write_string(factory);
                f.write_string(ty);
                f.write_int(version);
                f.write_struct(|f| {
                    f.write_int(props.len() as _);
                    for (key, val) in props {
                        f.write_string(key);
                        f.write_string(val);
                    }
                });
                f.write_uint(new_id);
            });
        });
    }

    pub fn send_properties(&self) {
        self.send2(1, "client", PwClientMethods::UpdateProperties, |f| {
            f.write_struct(|f| {
                f.write_struct(|f| {
                    f.write_int(1);
                    f.write_string("application.name");
                    f.write_string("jay-portal");
                });
            });
        });
    }

    async fn handle_outgoing(self: Rc<Self>) {
        if let Err(e) = self.io.clone().outgoing().await {
            log::error!("{}", ErrorFmt(e));
        }
        self.kill();
    }

    async fn handle_incoming(self: Rc<Self>) {
        let incoming = Incoming {
            incoming: self.io.clone().incoming(),
            con: self.clone(),
            buf: vec![],
            fds: vec![],
        };
        incoming.run().await;
    }
}

impl Drop for PwConHolder {
    fn drop(&mut self) {
        self.con.owner.take();
        self.con.kill();
    }
}

impl PwConHolder {
    pub async fn new(eng: &Rc<AsyncEngine>, ring: &Rc<IoUring>) -> Result<Rc<Self>, PwConError> {
        let fd = match uapi::socket(c::AF_UNIX, c::SOCK_STREAM | c::SOCK_CLOEXEC, 0) {
            Ok(fd) => Rc::new(fd),
            Err(e) => return Err(PwConError::CreateSocket(e.into())),
        };
        let mut addr = c::sockaddr_un {
            sun_family: c::AF_UNIX as _,
            ..uapi::pod_zeroed()
        };
        let xrd = match xrd() {
            Some(xrd) => xrd,
            _ => return Err(PwConError::XrdNotSet),
        };
        {
            let mut path = uapi::as_bytes_mut(&mut addr.sun_path[..]);
            let _ = write!(path, "{}/pipewire-0", xrd);
        }
        if let Err(e) = ring.connect(&fd, &addr).await {
            return Err(PwConError::ConnectSocket(e));
        }
        let io = Rc::new(BufIo::new(&fd, ring));
        let data = Rc::new(PwCon {
            send_seq: Default::default(),
            io,
            holder: Default::default(),
            dead: Cell::new(false),
            objects: Default::default(),
            ids: Default::default(),
            mem: Default::default(),
            ring: ring.clone(),
            eng: eng.clone(),
            owner: Default::default(),
            registry_generation: Cell::new(0),
            ack_registry_generation: Cell::new(0),
        });
        let core = Rc::new(PwCore {
            data: data.proxy_data(),
            con: data.clone(),
        });
        let client = Rc::new(PwClient {
            data: data.proxy_data(),
            _con: data.clone(),
        });
        data.objects.set(0, core.clone());
        data.objects.set(1, client.clone());
        data.send_hello();
        data.send_properties();
        let con = Rc::new(PwConHolder {
            outgoing: Cell::new(Some(
                eng.spawn("pw outgoing", data.clone().handle_outgoing()),
            )),
            incoming: Cell::new(Some(
                eng.spawn("pw incoming", data.clone().handle_incoming()),
            )),
            con: data,
        });
        con.con.holder.set(Rc::downgrade(&con));
        Ok(con)
    }
}

struct Incoming {
    con: Rc<PwCon>,
    incoming: BufIoIncoming,
    buf: Vec<u8>,
    fds: Vec<Rc<OwnedFd>>,
}

impl Incoming {
    async fn run(mut self) {
        loop {
            if let Err(e) = self.handle_msg().await {
                log::error!("Could not handle incoming message: {}", ErrorFmt(e));
                self.con.kill();
                return;
            }
        }
    }

    async fn handle_msg(&mut self) -> Result<(), PwConError> {
        self.buf.clear();
        self.incoming.fill_msg_buf(16, &mut self.buf).await?;
        let id: u32 = uapi::pod_read(&self.buf[0..4]).unwrap();
        let p2: u32 = uapi::pod_read(&self.buf[4..8]).unwrap();
        let opcode = (p2 >> 24) as u8;
        let size = (p2 & 0xff_ffff) as usize;
        let _seq: u32 = uapi::pod_read(&self.buf[8..12]).unwrap();
        let n_fds: u32 = uapi::pod_read(&self.buf[12..16]).unwrap();
        for _ in 0..n_fds {
            match self.incoming.fds.pop_front() {
                Some(fd) => self.fds.push(fd),
                _ => return Err(PwConError::MissingFd),
            }
        }
        self.buf.clear();
        self.incoming.fill_msg_buf(size, &mut self.buf).await?;
        if let Err(e) = self.handle_msg_data(id, opcode) {
            log::warn!("Could not handle incoming message: {}", ErrorFmt(e));
        }
        self.fds.clear();
        Ok(())
    }

    fn handle_msg_data(&self, id: u32, opcode: u8) -> Result<(), PwConError> {
        let parser = PwParser::new(&self.buf, &self.fds);
        {
            let mut parser = parser;
            parser.skip()?;
            if parser.len() > 0 {
                let s1 = parser.read_struct()?;
                let mut p2 = s1.fields;
                while p2.len() > 0 {
                    let opcode = p2.read_id()?;
                    let s2 = p2.read_struct()?;
                    if opcode == FOOTER_REGISTRY_GENERATION {
                        let mut p3 = s2.fields;
                        let generation = p3.read_ulong()?;
                        self.con.registry_generation.set(generation);
                        log::debug!("registry generation = {}", generation);
                    } else {
                        log::warn!("Unknown message footer: {}", opcode);
                    }
                }
            }
        }
        if let Some(obj) = self.con.objects.get(&id) {
            'log: {
                if log::log_enabled!(log::Level::Trace) {
                    let s;
                    let op: &dyn Display = match obj.event_name(opcode) {
                        Some(e) => {
                            s = e;
                            if e == "Done" && obj.interface() == "core" {
                                break 'log;
                            }
                            &s
                        }
                        _ => &opcode,
                    };
                    log::trace!("EVENT {}@{}: `{}`:", obj.interface(), obj.data().id, op);
                    let mut parser = parser;
                    while parser.len() > 0 {
                        log::trace!("{:#?}", parser.read_pod().unwrap());
                    }
                }
            }
            obj.handle_msg(opcode, parser)?;
        }
        Ok(())
    }
}

const FOOTER_REGISTRY_GENERATION: u32 = 0;
