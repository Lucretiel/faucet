use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{
        AtomicU8,
        Ordering::{Acquire, Relaxed, Release},
    },
};

const EMPTY: u8 = 0;
const MOVING: u8 = 1;
const FULL: u8 = 2;

pub struct AtomicSwap<T> {
    item: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}

impl<T> AtomicSwap<T> {
    pub fn new() -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(EMPTY),
        }
    }

    // Try to insert an item. Returns the item without changing the underlying
    // value if this swap is full.
    pub fn put(&self, item: T) -> Option<T> {
        if self
            .state
            .compare_exchange(EMPTY, MOVING, Acquire, Relaxed)
            .is_ok()
        {
            let slot = unsafe {
                self.item
                    .get()
                    .as_mut()
                    .expect("UnsafeCell never returns a null pointer")
            };
            slot.write(item);
            self.state.store(FULL, Release);
            None
        } else {
            Some(item)
        }
    }

    pub fn take(&self) -> Option<T> {
        if self
            .state
            .compare_exchange(FULL, MOVING, Acquire, Relaxed)
            .is_ok()
        {
            let slot = unsafe {
                self.item
                    .get()
                    .as_mut()
                    .expect("UnsafeCell never returns a null pointer")
            };
            let item = unsafe { slot.assume_init_read() };
            self.state.store(EMPTY, Release);
            Some(item)
        } else {
            None
        }
    }
}

impl<T> Drop for AtomicSwap<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == FULL {
            let slot = self.item.get_mut();
            unsafe { slot.assume_init_drop() };
        }
    }
}
