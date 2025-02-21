use {
    crate::{
        client::Client,
        leaks::Tracker,
        object::{Object, Version},
        wire::{WlCallbackId, wl_callback::*},
    },
    std::{convert::Infallible, rc::Rc},
    thiserror::Error,
};

pub struct WlCallback {
    pub client: Rc<Client>,
    pub id: WlCallbackId,
    pub tracker: Tracker<Self>,
}

impl WlCallback {
    pub fn new(id: WlCallbackId, client: &Rc<Client>) -> Self {
        Self {
            client: client.clone(),
            id,
            tracker: Default::default(),
        }
    }

    pub fn send_done(&self, data: u32) {
        self.client.event(Done {
            self_id: self.id,
            callback_data: data,
        });
    }
}

impl WlCallbackRequestHandler for WlCallback {
    type Error = Infallible;
}

object_base! {
    self = WlCallback;
    version = Version(1);
}

impl Object for WlCallback {}

simple_add_obj!(WlCallback);

#[derive(Debug, Error)]
pub enum WlCallbackError {}
