use {
    crate::{
        client::{Client, ClientError},
        format::XRGB8888,
        ifs::{
            wl_buffer::{WlBuffer, WlBufferError, WlBufferStorage},
            wl_output::OutputGlobalOpt,
        },
        leaks::Tracker,
        object::{Object, Version},
        rect::Rect,
        wire::{zwlr_screencopy_frame_v1::*, WlBufferId, ZwlrScreencopyFrameV1Id},
    },
    std::{cell::Cell, ops::Deref, rc::Rc},
    thiserror::Error,
};

#[expect(dead_code)]
pub const FLAGS_Y_INVERT: u32 = 1;

pub struct ZwlrScreencopyFrameV1 {
    pub id: ZwlrScreencopyFrameV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub output: Rc<OutputGlobalOpt>,
    pub rect: Rect,
    pub _overlay_cursor: bool,
    pub used: Cell<bool>,
    pub with_damage: Cell<bool>,
    pub buffer: Cell<Option<Rc<WlBuffer>>>,
    pub version: Version,
}

impl ZwlrScreencopyFrameV1 {
    pub fn send_ready(&self, tv_sec: u64, tv_nsec: u32) {
        self.client.event(Ready {
            self_id: self.id,
            tv_sec_hi: (tv_sec >> 32) as u32,
            tv_sec_lo: tv_sec as u32,
            tv_nsec,
        });
    }

    pub fn send_failed(&self) {
        self.client.event(Failed { self_id: self.id });
    }

    pub fn send_damage(&self) {
        if let Some(output) = self.output.get() {
            let pos = output.pos.get();
            self.client.event(Damage {
                self_id: self.id,
                x: 0,
                y: 0,
                width: pos.width() as _,
                height: pos.height() as _,
            });
        }
    }

    pub fn send_buffer(&self) {
        self.client.event(Buffer {
            self_id: self.id,
            format: XRGB8888.wl_id.unwrap(),
            width: self.rect.width() as _,
            height: self.rect.height() as _,
            stride: self.rect.width() as u32 * 4, // TODO
        });
    }

    pub fn send_linux_dmabuf(&self) {
        self.client.event(LinuxDmabuf {
            self_id: self.id,
            format: XRGB8888.drm,
            width: self.rect.width() as _,
            height: self.rect.height() as _,
        });
    }

    pub fn send_buffer_done(&self) {
        self.client.event(BufferDone { self_id: self.id })
    }

    #[expect(dead_code)]
    pub fn send_flags(&self, flags: u32) {
        self.client.event(Flags {
            self_id: self.id,
            flags,
        })
    }

    fn do_copy(
        self: &Rc<Self>,
        buffer_id: WlBufferId,
        with_damage: bool,
    ) -> Result<(), ZwlrScreencopyFrameV1Error> {
        if self.used.replace(true) {
            return Err(ZwlrScreencopyFrameV1Error::AlreadyUsed);
        }
        let Some(node) = self.output.node() else {
            self.send_failed();
            return Ok(());
        };
        let buffer = self.client.lookup(buffer_id)?;
        if (buffer.rect.width(), buffer.rect.height()) != (self.rect.width(), self.rect.height()) {
            return Err(ZwlrScreencopyFrameV1Error::InvalidBufferSize);
        }
        if buffer.format != XRGB8888 {
            return Err(ZwlrScreencopyFrameV1Error::InvalidBufferFormat);
        }
        buffer.update_framebuffer()?;
        if let Some(WlBufferStorage::Shm { stride, .. }) = buffer.storage.borrow_mut().deref() {
            if *stride != self.rect.width() * 4 {
                return Err(ZwlrScreencopyFrameV1Error::InvalidBufferStride);
            }
        }
        self.buffer.set(Some(buffer));
        if !with_damage {
            if let Some(global) = self.output.get() {
                global.connector.damage();
            }
        }
        self.with_damage.set(with_damage);
        node.screencopies
            .set((self.client.id, self.id), self.clone());
        node.screencast_changed();
        Ok(())
    }

    fn detach(&self) {
        if let Some(node) = self.output.node() {
            node.screencopies.remove(&(self.client.id, self.id));
            node.screencast_changed();
        }
    }
}

impl ZwlrScreencopyFrameV1RequestHandler for ZwlrScreencopyFrameV1 {
    type Error = ZwlrScreencopyFrameV1Error;

    fn copy(&self, req: Copy, slf: &Rc<Self>) -> Result<(), Self::Error> {
        slf.do_copy(req.buffer, false)
    }

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.detach();
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn copy_with_damage(&self, req: CopyWithDamage, slf: &Rc<Self>) -> Result<(), Self::Error> {
        slf.do_copy(req.buffer, true)
    }
}

object_base! {
    self = ZwlrScreencopyFrameV1;
    version = self.version;
}

simple_add_obj!(ZwlrScreencopyFrameV1);

impl Object for ZwlrScreencopyFrameV1 {
    fn break_loops(&self) {
        self.detach();
    }
}

#[derive(Debug, Error)]
pub enum ZwlrScreencopyFrameV1Error {
    #[error("This frame has already been used")]
    AlreadyUsed,
    #[error("The buffer has an invalid size for the frame")]
    InvalidBufferSize,
    #[error("The buffer has an invalid stride for the frame")]
    InvalidBufferStride,
    #[error("The buffer has an invalid format")]
    InvalidBufferFormat,
    #[error(transparent)]
    WlBufferError(Box<WlBufferError>),
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ZwlrScreencopyFrameV1Error, WlBufferError);
efrom!(ZwlrScreencopyFrameV1Error, ClientError);
