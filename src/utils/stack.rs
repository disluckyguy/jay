use {
    crate::utils::{
        clonecell::UnsafeCellCloneSafe,
        ptr_ext::{MutPtrExt, PtrExt},
    },
    std::{cell::UnsafeCell, mem},
};

pub struct Stack<T> {
    vec: UnsafeCell<Vec<T>>,
}

impl<T> Default for Stack<T> {
    fn default() -> Self {
        Self {
            vec: Default::default(),
        }
    }
}

impl<T> Stack<T> {
    pub fn push(&self, v: T) {
        unsafe {
            self.vec.get().deref_mut().push(v);
        }
    }

    pub fn pop(&self) -> Option<T> {
        unsafe { self.vec.get().deref_mut().pop() }
    }

    pub fn to_vec(&self) -> Vec<T>
    where
        T: UnsafeCellCloneSafe,
    {
        unsafe {
            let v = self.vec.get().deref();
            (*v).clone()
        }
    }

    pub fn take(&self) -> Vec<T> {
        unsafe { mem::take(self.vec.get().deref_mut()) }
    }
}
