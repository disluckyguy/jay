use {
    crate::{
        client::{CAP_IDLE_NOTIFIER, Client, ClientCaps, ClientError},
        globals::{Global, GlobalName},
        ifs::ext_idle_notification_v1::ExtIdleNotificationV1,
        leaks::Tracker,
        object::{Object, Version},
        utils::errorfmt::ErrorFmt,
        wire::{ExtIdleNotificationV1Id, ExtIdleNotifierV1Id, WlSeatId, ext_idle_notifier_v1::*},
    },
    std::{cell::Cell, rc::Rc},
    thiserror::Error,
};

pub struct ExtIdleNotifierV1Global {
    pub name: GlobalName,
}

impl ExtIdleNotifierV1Global {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: ExtIdleNotifierV1Id,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), ExtIdleNotifierV1Error> {
        let obj = Rc::new(ExtIdleNotifierV1 {
            id,
            client: client.clone(),
            tracker: Default::default(),
            version,
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        Ok(())
    }
}

pub struct ExtIdleNotifierV1 {
    pub id: ExtIdleNotifierV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl ExtIdleNotifierV1 {
    fn create_notification(
        &self,
        id: ExtIdleNotificationV1Id,
        timeout: u32,
        seat: WlSeatId,
        skip_if_inhibited: bool,
    ) -> Result<(), ExtIdleNotifierV1Error> {
        let seat = self.client.lookup(seat)?;
        let notification = Rc::new(ExtIdleNotificationV1 {
            id,
            client: self.client.clone(),
            tracker: Default::default(),
            resume: Default::default(),
            task: Cell::new(None),
            seat: seat.global.clone(),
            duration_usec: (timeout as u64).max(1000).saturating_mul(1000),
            version: self.version,
        });
        track!(self.client, notification);
        self.client.add_client_obj(&notification)?;
        let future = self.client.state.eng.spawn(
            "idle notifier",
            run(notification.clone(), skip_if_inhibited),
        );
        notification.task.set(Some(future));
        Ok(())
    }
}

impl ExtIdleNotifierV1RequestHandler for ExtIdleNotifierV1 {
    type Error = ExtIdleNotifierV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_idle_notification(
        &self,
        req: GetIdleNotification,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        self.create_notification(req.id, req.timeout, req.seat, true)
    }

    fn get_input_idle_notification(
        &self,
        req: GetInputIdleNotification,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        self.create_notification(req.id, req.timeout, req.seat, false)
    }
}

async fn run(n: Rc<ExtIdleNotificationV1>, skip_if_inhibited: bool) {
    loop {
        let now = n.client.state.now_usec();
        let elapsed = now.saturating_sub(n.seat.last_input());
        if elapsed < n.duration_usec {
            let res = n
                .client
                .state
                .wheel
                .timeout((n.duration_usec - elapsed).div_ceil(1000))
                .await;
            if let Err(e) = res {
                log::error!("Could not wait for idle timeout to elapse: {}", ErrorFmt(e));
                return;
            }
        } else {
            let idle = &n.client.state.idle;
            let send_idle = !skip_if_inhibited || idle.inhibitors.is_empty();
            if send_idle {
                n.send_idled();
                n.seat.add_idle_notification(&n);
            } else {
                idle.add_inhibited_notification(&n);
            }
            n.resume.triggered().await;
            if send_idle {
                n.send_resumed();
            }
        }
    }
}

global_base!(
    ExtIdleNotifierV1Global,
    ExtIdleNotifierV1,
    ExtIdleNotifierV1Error
);

impl Global for ExtIdleNotifierV1Global {
    fn singleton(&self) -> bool {
        true
    }

    fn version(&self) -> u32 {
        2
    }

    fn required_caps(&self) -> ClientCaps {
        CAP_IDLE_NOTIFIER
    }
}

simple_add_global!(ExtIdleNotifierV1Global);

object_base! {
    self = ExtIdleNotifierV1;
    version = self.version;
}

impl Object for ExtIdleNotifierV1 {}

simple_add_obj!(ExtIdleNotifierV1);

#[derive(Debug, Error)]
pub enum ExtIdleNotifierV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtIdleNotifierV1Error, ClientError);
