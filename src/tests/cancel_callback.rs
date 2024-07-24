use std::sync::{Arc, Mutex, Weak};

use crate::CancelCallback;

#[test]
fn from_arc_fn() {
    struct X(Mutex<bool>);

    let x = Arc::new(X(Mutex::new(false)));
    let cb = CancelCallback::from_arc_fn(Arc::clone(&x), |x| {
        *x.0.lock().unwrap() = true;
    });
    cb.call();
    assert!(*x.0.lock().unwrap());
}

#[test]
fn from_weak_fn_live() {
    struct X(Mutex<bool>);

    let x = Arc::new(X(Mutex::new(false)));
    let cb = CancelCallback::from_weak_fn(Arc::downgrade(&x), |x| {
        *x.0.lock().unwrap() = true;
    });
    cb.call();
    assert!(*x.0.lock().unwrap());
}

#[test]
fn from_weak_fn_dead() {
    struct X(Mutex<bool>);

    let x_arc = Arc::new(X(Mutex::new(false)));
    let x = Arc::downgrade(&x_arc);
    let cb = CancelCallback::from_weak_fn(Weak::clone(&x), |x| {
        *x.0.lock().unwrap() = true;
    });
    drop(x_arc);
    cb.call();
    assert!(x.upgrade().is_none());
}

#[test]
fn from_box_fn() {
    let f = Box::new(|| {});
    let cb = CancelCallback::from(f);
    cb.call();
}

#[test]
fn from_box_dyn_fn_once() {
    let f: Box<dyn FnOnce() + Send + Sync> = Box::new(|| {});
    let cb = CancelCallback::from(f);
    cb.call();
}
