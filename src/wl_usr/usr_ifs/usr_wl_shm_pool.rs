use {
    crate::{
        object::Version,
        wire::{WlShmPoolId, wl_shm_pool::*},
        wl_usr::{UsrCon, usr_object::UsrObject},
    },
    std::{convert::Infallible, rc::Rc},
};

pub struct UsrWlShmPool {
    pub id: WlShmPoolId,
    pub con: Rc<UsrCon>,
    pub version: Version,
}

impl UsrWlShmPool {
    #[expect(dead_code)]
    pub fn resize(&self, size: i32) {
        self.con.request(Resize {
            self_id: self.id,
            size,
        });
    }
}

impl WlShmPoolEventHandler for UsrWlShmPool {
    type Error = Infallible;
}

usr_object_base! {
    self = UsrWlShmPool = WlShmPool;
    version = self.version;
}

impl UsrObject for UsrWlShmPool {
    fn destroy(&self) {
        self.con.request(Destroy { self_id: self.id });
    }
}
