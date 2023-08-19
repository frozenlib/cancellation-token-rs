use std::{
    future::pending,
    sync::Mutex,
    thread::{scope, sleep},
    time::Duration,
};

use cancellation_token::{Canceled, CancellationToken, CancellationTokenSource};
use rt_local::{
    runtime::core::{run, test as async_test},
    wait_for_idle,
};

#[test]
fn cancel_and_is_canceled() {
    let cts = CancellationTokenSource::new();
    let ct = cts.token();

    assert!(!cts.is_canceled());
    assert!(!ct.is_canceled());

    cts.cancel();
    assert!(cts.is_canceled());
    assert!(ct.is_canceled());
}

#[test]
fn token_new() {
    let ct = CancellationToken::new(false);
    assert!(!ct.can_be_canceled());
    assert!(!ct.is_canceled());

    let ct = CancellationToken::new(true);
    assert!(ct.can_be_canceled());
    assert!(ct.is_canceled());
}

#[test]
fn wait_for_canceled() {
    let logs = Logs::new();
    let cts = CancellationTokenSource::new();
    scope(|s| {
        run(async {
            let ct = cts.token();
            s.spawn(|| {
                sleep(Duration::from_millis(500));
                logs.push("cancel");
                cts.cancel();
            });
            logs.push("wait");
            ct.wait_for_canceled().await;
            logs.push("wake");
        })
    });
    logs.verify(&["wait", "cancel", "wake"]);
}

#[async_test]
async fn wait_for_canceled_already_canceled() {
    let cts = CancellationTokenSource::new();
    let ct = cts.token();
    cts.cancel();
    ct.wait_for_canceled().await;
}

#[test]
fn with() {
    let logs = Logs::new();
    let cts = CancellationTokenSource::new();
    scope(|s| {
        run(async {
            let ct = cts.token();
            s.spawn(|| {
                sleep(Duration::from_millis(500));
                logs.push("cancel");
                cts.cancel();
            });
            let r = ct
                .with(async {
                    logs.push("1");
                    wait_for_idle().await;
                    logs.push("2");
                    sleep(Duration::from_millis(1000));
                    wait_for_idle().await;
                    logs.push("3");
                })
                .await;
            assert_eq!(r, Err(Canceled));
            logs.push("finish");
        })
    });
    logs.verify(&["1", "2", "cancel", "finish"]);
}

#[async_test]
fn with_already_canceled() {
    let cts = CancellationTokenSource::new();
    let ct = cts.token();
    cts.cancel();
    let r = ct.with(pending::<()>()).await;
    assert_eq!(r, Err(Canceled));
}

#[test]
fn with_parent() {
    let parent = CancellationTokenSource::new();
    let child = CancellationTokenSource::with_parent(&parent.token());

    assert!(!parent.is_canceled());
    assert!(!child.is_canceled());

    parent.cancel();
    assert!(parent.is_canceled());
    assert!(child.is_canceled());
}

#[test]
fn with_parent_2() {
    let cts0 = CancellationTokenSource::new();
    let cts1 = CancellationTokenSource::with_parent(&cts0.token());
    let cts2 = CancellationTokenSource::with_parent(&cts1.token());

    cts0.cancel();
    assert!(cts0.is_canceled());
    assert!(cts1.is_canceled());
    assert!(cts2.is_canceled());
}

#[test]
fn with_parent_drop_middle() {
    let cts0 = CancellationTokenSource::new();
    let cts1 = CancellationTokenSource::with_parent(&cts0.token());
    let cts2 = CancellationTokenSource::with_parent(&cts1.token());
    drop(cts1);

    cts0.cancel();
    assert!(cts0.is_canceled());
    assert!(cts2.is_canceled());
}

#[test]
fn with_parent_many_child() {
    let parent = CancellationTokenSource::new();
    let child0 = CancellationTokenSource::with_parent(&parent.token());
    let child1 = CancellationTokenSource::with_parent(&parent.token());

    parent.cancel();
    assert!(parent.is_canceled());
    assert!(child0.is_canceled());
    assert!(child1.is_canceled());
}

#[test]
fn with_praent_already_canceled() {
    let parent = CancellationTokenSource::new();
    parent.cancel();

    let child = CancellationTokenSource::with_parent(&parent.token());

    assert!(parent.is_canceled());
    assert!(child.is_canceled());
}

#[test]
fn with_praent_is_canceled_true() {
    let cts = CancellationTokenSource::with_parent(&CancellationToken::new(true));
    assert!(cts.is_canceled());
}

#[test]
fn with_praent_is_canceled_false() {
    let cts = CancellationTokenSource::with_parent(&CancellationToken::new(false));
    assert!(!cts.is_canceled());
}

struct Logs(Mutex<Vec<&'static str>>);

impl Logs {
    fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
    fn push(&self, s: &'static str) {
        self.0.lock().unwrap().push(s);
    }
    fn verify(&self, expected: &[&'static str]) {
        assert_eq!(self.0.lock().unwrap().as_slice(), expected);
    }
}
