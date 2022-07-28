use {
    crate::{
        utils::{
            buffd::{MsgParser, MsgParserError},
            clonecell::CloneCell,
        },
        wire::{jay_workspace::*, JayWorkspaceId},
        wl_usr::{usr_object::UsrObject, UsrCon},
    },
    std::rc::Rc,
};

pub struct UsrJayWorkspace {
    pub id: JayWorkspaceId,
    pub con: Rc<UsrCon>,
    pub owner: CloneCell<Option<Rc<dyn UsrJayWorkspaceOwner>>>,
}

pub trait UsrJayWorkspaceOwner {
    fn linear_id(self: Rc<Self>, ev: &LinearId) {
        let _ = ev;
    }

    fn name(&self, ev: &Name) {
        let _ = ev;
    }

    fn destroyed(&self) {}

    fn done(&self) {}

    fn output(self: Rc<Self>, ev: &Output) {
        let _ = ev;
    }

    fn visible(&self, visible: bool) {
        let _ = visible;
    }
}

impl UsrJayWorkspace {
    fn linear_id(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let ev: LinearId = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.linear_id(&ev);
        }
        Ok(())
    }

    fn name(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let ev: Name = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.name(&ev);
        }
        Ok(())
    }

    fn destroyed(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let _ev: Destroyed = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.destroyed();
        }
        Ok(())
    }

    fn done(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let _ev: Done = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.done();
        }
        Ok(())
    }

    fn output(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let ev: Output = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.output(&ev);
        }
        Ok(())
    }

    fn visible(&self, parser: MsgParser<'_, '_>) -> Result<(), MsgParserError> {
        let ev: Visible = self.con.parse(self, parser)?;
        if let Some(owner) = self.owner.get() {
            owner.visible(ev.visible != 0);
        }
        Ok(())
    }
}

usr_object_base! {
    UsrJayWorkspace, JayWorkspace;

    LINEAR_ID => linear_id,
    NAME => name,
    DESTROYED => destroyed,
    DONE => done,
    OUTPUT => output,
    VISIBLE => visible,
}

impl UsrObject for UsrJayWorkspace {
    fn destroy(&self) {
        self.con.request(Destroy { self_id: self.id });
    }

    fn break_loops(&self) {
        self.owner.set(None);
    }
}
