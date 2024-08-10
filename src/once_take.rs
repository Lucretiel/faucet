use std::{
    cell::UnsafeCell,
    mem::{self, MaybeUninit},
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};

pub struct OnceTake<T> {
    item: UnsafeCell<MaybeUninit<T>>,
    taken: AtomicBool,
}

impl<T> OnceTake<T> {
    pub fn new(item: T) -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::new(item)),
            taken: AtomicBool::new(false),
        }
    }

    pub fn take(&self) -> Option<T> {
        match self.taken.swap(true, Relaxed) {
            false => Some(unsafe {
                self.item
                    .get()
                    .as_mut()
                    .expect("Unsafe Cell never returns a null pointer")
                    .assume_init_read()
            }),
            true => None,
        }
    }

    pub fn take_mut(&mut self) -> Option<T> {
        match mem::replace(self.taken.get_mut(), true) {
            false => Some(unsafe { self.item.get_mut().assume_init_read() }),
            true => None,
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        match *self.taken.get_mut() {
            false => Some(unsafe { self.item.get_mut().assume_init_mut() }),
            true => None,
        }
    }
}

impl<T> Drop for OnceTake<T> {
    fn drop(&mut self) {
        match *self.taken.get_mut() {
            false => unsafe { self.item.get_mut().assume_init_drop() },
            true => {}
        }
    }
}

unsafe impl<T: Send> Send for OnceTake<T> {}
unsafe impl<T: Send> Sync for OnceTake<T> {}
