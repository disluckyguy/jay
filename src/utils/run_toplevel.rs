use {
    crate::{
        async_engine::{AsyncEngine, SpawnedFuture},
        utils::queue::AsyncQueue,
    },
    std::rc::Rc,
};

pub struct RunToplevelFuture {
    _future: SpawnedFuture<()>,
}

pub struct RunToplevel {
    queue: AsyncQueue<Box<dyn FnOnce()>>,
}

impl RunToplevel {
    pub fn install(eng: &Rc<AsyncEngine>) -> (RunToplevelFuture, Rc<RunToplevel>) {
        let slf = Rc::new(RunToplevel {
            queue: Default::default(),
        });
        let future = eng.spawn("run toplevel", {
            let slf = slf.clone();
            async move {
                loop {
                    let f = slf.queue.pop().await;
                    f();
                }
            }
        });
        let future = RunToplevelFuture { _future: future };
        (future, slf)
    }

    pub fn schedule<F: FnOnce() + 'static>(&self, f: F) {
        self.schedule_dyn(Box::new(f));
    }

    pub fn clear(&self) {
        self.queue.clear();
    }

    fn schedule_dyn(&self, f: Box<dyn FnOnce()>) {
        self.queue.push(f);
    }
}
