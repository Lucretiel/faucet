#![feature(impl_trait_in_assoc_type)]

use std::{future::Future, time::Duration};

use faucet::{Faucet, FaucetStart};
use futures_util::StreamExt;

struct Thing;

impl FaucetStart for Thing {
    type Item = i32;
    type Fut<'a> = impl Future<Output = ()>;

    fn start(self, mut emitter: faucet::Emit<'_, Self::Item>) -> Self::Fut<'_> {
        async move {
            emitter.emit(1).await;
            tokio::time::sleep(Duration::from_millis(100)).await;

            emitter.emit(2).await;
            tokio::time::sleep(Duration::from_millis(100)).await;

            emitter.emit(3).await;
            tokio::time::sleep(Duration::from_millis(100)).await;

            emitter.emit(4).await;
        }
    }
}

#[tokio::test]
async fn basic_test() {
    let faucet = Faucet::new(Thing);

    let list: Vec<i32> = faucet.collect().await;

    assert_eq!(list, [1, 2, 3, 4]);
}
