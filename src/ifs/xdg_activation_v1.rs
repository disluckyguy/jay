use {
    crate::{
        client::{Client, ClientError},
        globals::{Global, GlobalName},
        ifs::xdg_activation_token_v1::XdgActivationTokenV1,
        leaks::Tracker,
        object::{Object, Version},
        utils::{activation_token::ActivationToken, errorfmt::ErrorFmt, opaque::OpaqueError},
        wire::{XdgActivationV1Id, xdg_activation_v1::*},
    },
    std::rc::Rc,
    thiserror::Error,
};

pub struct XdgActivationV1Global {
    pub name: GlobalName,
}

impl XdgActivationV1Global {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: XdgActivationV1Id,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), XdgActivationV1Error> {
        let mgr = Rc::new(XdgActivationV1 {
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

global_base!(XdgActivationV1Global, XdgActivationV1, XdgActivationV1Error);

simple_add_global!(XdgActivationV1Global);

impl Global for XdgActivationV1Global {
    fn singleton(&self) -> bool {
        true
    }

    fn version(&self) -> u32 {
        1
    }
}

pub struct XdgActivationV1 {
    pub id: XdgActivationV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl XdgActivationV1RequestHandler for XdgActivationV1 {
    type Error = XdgActivationV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_activation_token(
        &self,
        req: GetActivationToken,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        let token = Rc::new(XdgActivationTokenV1::new(
            req.id,
            &self.client,
            self.version,
        ));
        track!(self.client, token);
        self.client.add_client_obj(&token)?;
        Ok(())
    }

    fn activate(&self, req: Activate, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        let token: ActivationToken = match req.token.parse() {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Could not parse client activation token: {}", ErrorFmt(e));
                return Ok(());
            }
        };
        let surface = self.client.lookup(req.surface)?;
        if self.client.state.activation_tokens.remove(&token).is_none() {
            log::warn!(
                "Client requested activation with unknown token {}",
                req.token
            );
            return Ok(());
        }
        surface.request_activation();
        Ok(())
    }
}

object_base! {
    self = XdgActivationV1;
    version = self.version;
}

impl Object for XdgActivationV1 {}

simple_add_obj!(XdgActivationV1);

#[derive(Debug, Error)]
pub enum XdgActivationV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
    #[error("Could not parse the activation token")]
    ParseActivationToken(#[from] OpaqueError),
}
efrom!(XdgActivationV1Error, ClientError);
