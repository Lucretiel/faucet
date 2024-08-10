use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    mem::{transmute, transmute_copy, ManuallyDrop, MaybeUninit},
    pin::Pin,
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
    task::{Context, Poll},
};

use futures_util::task::AtomicWaker;
use pin_project::pin_project;
use pinned_aliasable::Aliasable;

pub struct Emit<'a, T> {
    ptr: &'a FaucetSharedData<T>,
}

impl<'a, T> Emit<'a, T> {
    pub fn emit(&mut self, item: T) -> EmitFuture<'_, T> {
        EmitFuture {
            ptr: self.ptr,
            item: Aliasable::new(UnsafeCell::new(Some(item))),
        }
    }
}

#[pin_project]
pub struct EmitFuture<'a, T> {
    ptr: &'a FaucetSharedData<T>,

    #[pin]
    item: Aliasable<UnsafeCell<Option<T>>>,
}

impl<'a, T> Future for EmitFuture<'a, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {}
}

pub trait FaucetStart {
    type Item;
    type Fut<'a>: Future<Output = ()>;

    fn start<'a>(self, emitter: Emit<'a, Self::Item>) -> Self::Fut<'a>;
}

#[pin_project(project = FaucetStatePinned)]
enum FaucetState<F: FaucetStart> {
    Init(Option<F>),
    Running(#[pin] MaybeUninit<F::Fut<'static>>),
}

struct FaucetSharedData<T> {
    stream_waker: AtomicWaker,
    emit_waker: AtomicWaker,
    stream_item_ptr: AtomicPtr<UnsafeCell<Option<T>>>,
}

#[pin_project]
pub struct Faucet<F: FaucetStart> {
    #[pin]
    state: FaucetState<F>,

    #[pin]
    output_slot: Aliasable<FaucetSharedData<F::Item>>,
}

impl<F> futures_util::Stream for Faucet<F>
where
    F: FaucetStart,
{
    type Item = F::Item;

    fn poll_next<'a>(self: Pin<&'a mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut output_item = None;

        let mut this = self.project();

        let output_slot = this.output_slot.as_ref().get();
        let output_slot_guard = OutputSlotGuard::initialize(output_slot, &mut output_item);

        loop {
            match this.state.as_mut().project() {
                FaucetStatePinned::Init(start) => match start.take() {
                    None => break Poll::Ready(None),
                    Some(func) => {
                        let emitter = Emit { ptr: output_slot };
                        let future = func.start(emitter);

                        // Here be dragons
                        let non_dropped_future = ManuallyDrop::new(future);
                        let evil_future = unsafe { transmute_copy(&*non_dropped_future) };
                        this.state.as_mut().set(FaucetState::Running(evil_future));

                        // Repeat the loop to perform the initial poll
                    }
                },
                FaucetStatePinned::Running(future) => {
                    let future: Pin<&mut F::Fut<'a>> = unsafe { transmute(future) };
                    let poll = future.poll(cx);

                    if let Poll::Ready(()) = poll {
                        this.state.as_mut().set(FaucetState::Init(None));
                    }

                    drop(output_slot_guard);
                    break match output_item {
                        Some(item) => Poll::Ready(Some(item)),
                        None => match poll {
                            Poll::Pending => Poll::Pending,
                            Poll::Ready(()) => Poll::Ready(None),
                        },
                    };
                }
            }
        }
    }
}

struct OutputSlotGuard<'a, T> {
    ptr: &'a AtomicPtr<T>,
}

impl<'a, T> OutputSlotGuard<'a, T> {
    pub fn initialize(ptr: &'a AtomicPtr<T>, item: &'a mut T) -> Self {
        let this = Self { ptr };

        if cfg!(debug_assertions) {
            let old_ptr = this.ptr.swap(item, Ordering::Release);
            debug_assert!(old_ptr.is_null());
        } else {
            this.ptr.store(item, Ordering::Release)
        }

        this
    }
}

impl<'a, T> Drop for OutputSlotGuard<'a, T> {
    fn drop(&mut self) {
        loop {
            let old_ptr = self.ptr.swap(ptr::null_mut(), Ordering::AcqRel);
            if let false = old_ptr.is_null() {
                return;
            }
        }
    }
}
