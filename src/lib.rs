#![no_std]

mod atomic_swap;

use core::{
    future::Future,
    mem::{transmute, transmute_copy, ManuallyDrop, MaybeUninit},
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::task::AtomicWaker;
use pin_project::pin_project;
use pinned_aliasable::Aliasable;

use crate::atomic_swap::AtomicSwap;

pub struct Emit<'a, T> {
    shared_state: &'a FaucetSharedData<T>,
}

impl<'a, T> Emit<'a, T> {
    pub fn emit(&mut self, item: T) -> EmitFuture<'_, T> {
        EmitFuture {
            shared_state: self.shared_state,
            item: Some(item),
        }
    }
}

#[must_use]
#[pin_project]
pub struct EmitFuture<'a, T> {
    shared_state: &'a FaucetSharedData<T>,
    item: Option<T>,
}

impl<'a, T> Future for EmitFuture<'a, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let Some(item) = this.item.take() else {
            return Poll::Ready(());
        };

        // Two possibilities: either we can emit our item, or we can't.
        // In both cases we need to register a waker.
        this.shared_state.emit_waker.register(cx.waker());

        match this.shared_state.stream_item.put(item) {
            Some(rejected_item) => {
                *this.item = Some(rejected_item);
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            None => {
                this.shared_state.stream_waker.wake();
                Poll::Pending
            }
        }
    }
}

pub trait FaucetStart {
    type Item;
    type Fut<'a>: Future<Output = ()>;

    fn start(self, emitter: Emit<'_, Self::Item>) -> Self::Fut<'_>;
}

#[pin_project(project = FaucetStatePinned)]
enum FaucetState<F: FaucetStart> {
    Init(Option<F>),
    Running(#[pin] MaybeUninit<F::Fut<'static>>),
}

struct FaucetSharedData<T> {
    stream_waker: AtomicWaker,
    emit_waker: AtomicWaker,
    stream_item: AtomicSwap<T>,
}

#[pin_project]
pub struct Faucet<F: FaucetStart> {
    #[pin]
    future_state: FaucetState<F>,

    #[pin]
    shared_state: Aliasable<FaucetSharedData<F::Item>>,
}

impl<F: FaucetStart> Faucet<F> {
    pub fn new(func: F) -> Self {
        Self {
            future_state: FaucetState::Init(Some(func)),
            shared_state: Aliasable::new(FaucetSharedData {
                stream_waker: AtomicWaker::new(),
                emit_waker: AtomicWaker::new(),
                stream_item: AtomicSwap::new(),
            }),
        }
    }
}

impl<F> futures_util::Stream for Faucet<F>
where
    F: FaucetStart,
{
    type Item = F::Item;

    fn poll_next<'a>(self: Pin<&'a mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let shared_state = this.shared_state.as_ref().get();

        loop {
            match this.future_state.as_mut().project() {
                FaucetStatePinned::Init(start) => match start.take() {
                    None => break Poll::Ready(None),
                    Some(func) => {
                        let emitter = Emit { shared_state };

                        let future = func.start(emitter);

                        // Here be dragons
                        let non_dropped_future = ManuallyDrop::new(future);
                        let evil_future = unsafe { transmute_copy(&*non_dropped_future) };
                        this.future_state
                            .as_mut()
                            .set(FaucetState::Running(evil_future));

                        // Repeat the loop to perform the initial poll
                    }
                },
                FaucetStatePinned::Running(future) => {
                    shared_state.stream_waker.register(cx.waker());
                    let future: Pin<&mut F::Fut<'a>> = unsafe { transmute(future) };
                    let poll = future.poll(cx);

                    if let Poll::Ready(()) = poll {
                        this.future_state.as_mut().set(FaucetState::Init(None));
                    }

                    let emitted_item = shared_state.stream_item.take();

                    if emitted_item.is_some() {
                        shared_state.emit_waker.wake();
                    }

                    break match emitted_item {
                        Some(item) => Poll::Ready(Some(item)),
                        None => match poll {
                            Poll::Ready(()) => Poll::Ready(None),
                            Poll::Pending => Poll::Pending,
                        },
                    };
                }
            }
        }
    }
}
