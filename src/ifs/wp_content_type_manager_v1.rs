use {
    crate::{
        client::{Client, ClientError},
        globals::{Global, GlobalName},
        ifs::wp_content_type_v1::WpContentTypeV1,
        leaks::Tracker,
        object::{Object, Version},
        wire::{WpContentTypeManagerV1Id, wp_content_type_manager_v1::*},
    },
    std::rc::Rc,
    thiserror::Error,
};

pub struct WpContentTypeManagerV1Global {
    pub name: GlobalName,
}

impl WpContentTypeManagerV1Global {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: WpContentTypeManagerV1Id,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), WpContentTypeManagerV1Error> {
        let mgr = Rc::new(WpContentTypeManagerV1 {
            id,
            client: client.clone(),
            tracker: Default::default(),
            version,
        });
        track!(client, mgr);
        client.add_client_obj(&mgr)?;
        Ok(())
    }
}

global_base!(
    WpContentTypeManagerV1Global,
    WpContentTypeManagerV1,
    WpContentTypeManagerV1Error
);

simple_add_global!(WpContentTypeManagerV1Global);

impl Global for WpContentTypeManagerV1Global {
    fn singleton(&self) -> bool {
        true
    }

    fn version(&self) -> u32 {
        1
    }
}

pub struct WpContentTypeManagerV1 {
    pub id: WpContentTypeManagerV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl WpContentTypeManagerV1RequestHandler for WpContentTypeManagerV1 {
    type Error = WpContentTypeManagerV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_surface_content_type(
        &self,
        req: GetSurfaceContentType,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        let surface = self.client.lookup(req.surface)?;
        if surface.has_content_type_manager.replace(true) {
            return Err(WpContentTypeManagerV1Error::DuplicateContentType);
        }
        let device = Rc::new(WpContentTypeV1 {
            id: req.id,
            client: self.client.clone(),
            surface,
            tracker: Default::default(),
            version: self.version,
        });
        track!(self.client, device);
        self.client.add_client_obj(&device)?;
        Ok(())
    }
}

object_base! {
    self = WpContentTypeManagerV1;
    version = self.version;
}

impl Object for WpContentTypeManagerV1 {}

simple_add_obj!(WpContentTypeManagerV1);

#[derive(Debug, Error)]
pub enum WpContentTypeManagerV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error("Surface already has a content type object")]
    DuplicateContentType,
}
efrom!(WpContentTypeManagerV1Error, ClientError);
