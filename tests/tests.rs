use cancellation_token::{CancellationToken, CancellationTokenSource};

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
