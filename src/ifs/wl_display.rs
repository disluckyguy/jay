use {
    crate::{
        client::{Client, ClientError},
        ifs::{wl_callback::WlCallback, wl_registry::WlRegistry},
        leaks::Tracker,
        object::{Object, ObjectId, Version, WL_DISPLAY_ID},
        wire::{wl_display::*, WlDisplayId},
    },
    std::rc::Rc,
    thiserror::Error,
};

const INVALID_OBJECT: u32 = 0;
const INVALID_METHOD: u32 = 1;
#[expect(dead_code)]
const NO_MEMORY: u32 = 2;
const IMPLEMENTATION: u32 = 3;

pub struct WlDisplay {
    pub id: WlDisplayId,
    pub client: Rc<Client>,
    pub tracker: Tracker<WlDisplay>,
}

impl WlDisplay {
    pub fn new(client: &Rc<Client>) -> Self {
        Self {
            id: WL_DISPLAY_ID,
            client: client.clone(),
            tracker: Default::default(),
        }
    }
}

impl WlDisplayRequestHandler for WlDisplay {
    type Error = WlDisplayError;

    fn sync(&self, req: Sync, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        let cb = Rc::new(WlCallback::new(req.callback, &self.client));
        track!(self.client, cb);
        self.client.add_client_obj(&cb)?;
        cb.send_done(0);
        self.client.remove_obj(&*cb)?;
        Ok(())
    }

    fn get_registry(&self, req: GetRegistry, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        let registry = Rc::new(WlRegistry::new(req.registry, &self.client));
        track!(self.client, registry);
        self.client.add_client_obj(&registry)?;
        self.client.state.globals.notify_all(&registry);
        Ok(())
    }
}

impl WlDisplay {
    pub fn send_error<O: Into<ObjectId>>(&self, object_id: O, code: u32, message: &str) {
        self.client.event(Error {
            self_id: self.id,
            object_id: object_id.into(),
            code,
            message,
        })
    }

    pub fn send_invalid_request(self: &Rc<Self>, obj: &dyn Object, request: u32) {
        let id = obj.id();
        let msg = format!(
            "Object {} of type {} has no method {}",
            id,
            obj.interface().name(),
            request
        );
        self.send_error(id, INVALID_METHOD, &msg)
    }

    pub fn send_invalid_object(self: &Rc<Self>, id: ObjectId) {
        let msg = format!("Object {} does not exist", id,);
        self.send_error(id, INVALID_OBJECT, &msg)
    }

    pub fn send_implementation_error(self: &Rc<Self>, msg: String) {
        self.send_error(WL_DISPLAY_ID, IMPLEMENTATION, &msg)
    }

    pub fn send_delete_id(self: &Rc<Self>, id: ObjectId) {
        self.client.event(DeleteId {
            self_id: self.id,
            id: id.raw(),
        })
    }
}

object_base! {
    self = WlDisplay;
    version = Version(1);
}

impl Object for WlDisplay {}

#[derive(Debug, Error)]
pub enum WlDisplayError {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(WlDisplayError, ClientError);
