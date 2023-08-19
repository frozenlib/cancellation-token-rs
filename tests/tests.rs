use std::{
    sync::Mutex,
    thread::{scope, sleep},
    time::Duration,
};

use cancellation_token::{CancellationToken, CancellationTokenSource};
use rt_local::runtime::core::run;

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
