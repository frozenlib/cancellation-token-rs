use std::{
    fmt,
    future::{pending, Future},
    pin::{pin, Pin},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use slabmap::SlabMap;

#[derive(Clone)]
struct Wakers(Arc<Mutex<Option<SlabMap<Waker>>>>);

impl Wakers {
    fn is_canceled(&self) -> bool {
        self.0.lock().unwrap().is_none()
    }
}

#[derive(Clone)]
pub struct CancellationTokenSource(Wakers);

impl CancellationTokenSource {
    pub fn new() -> Self {
        Self(Wakers(Arc::new(Mutex::new(Some(SlabMap::new())))))
    }
    pub fn cancel(&self) {
        if let Some(wakers) = self.0 .0.lock().unwrap().take() {
            wakers.into_iter().for_each(|(_, waker)| waker.wake());
        }
    }
    pub fn token(&self) -> CancellationToken {
        CancellationToken(RawToken::Wakers(self.0.clone()))
    }
    pub fn is_canceled(&self) -> bool {
        self.0.is_canceled()
    }
}
impl Default for CancellationTokenSource {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for CancellationTokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationTokenSource")
            .field("is_canceled", &self.is_canceled())
            .finish()
    }
}

#[derive(Clone)]
enum RawToken {
    IsCanceled(bool),
    Wakers(Wakers),
}

#[derive(Clone)]
pub struct CancellationToken(RawToken);

impl CancellationToken {
    pub const fn new(is_canceled: bool) -> Self {
        Self(RawToken::IsCanceled(is_canceled))
    }

    pub fn can_be_canceled(&self) -> bool {
        match &self.0 {
            RawToken::IsCanceled(is_canceled) => *is_canceled,
            RawToken::Wakers(_) => true,
        }
    }
    pub fn is_canceled(&self) -> bool {
        match &self.0 {
            RawToken::IsCanceled(is_canceled) => *is_canceled,
            RawToken::Wakers(wakers) => wakers.is_canceled(),
        }
    }
    pub fn err_if_canceled(&self) -> Result<(), Canceled> {
        if self.is_canceled() {
            Err(Canceled)
        } else {
            Ok(())
        }
    }

    pub async fn wait_for_canceled(&self) {
        match &self.0 {
            RawToken::IsCanceled(false) => pending().await,
            RawToken::IsCanceled(true) => {}
            RawToken::Wakers(wakers) => WaitForCanceled(WakerRegistration::new(wakers)).await,
        }
    }
    pub async fn with<T>(&self, future: impl Future<Output = T>) -> Result<T, Canceled> {
        match &self.0 {
            RawToken::IsCanceled(false) => Ok(future.await),
            RawToken::IsCanceled(true) => Err(Canceled),
            RawToken::Wakers(wakers) => {
                WithCanceled {
                    r: WakerRegistration::new(wakers),
                    future: pin!(future),
                }
                .await
            }
        }
    }
}
impl Default for CancellationToken {
    fn default() -> Self {
        Self::new(false)
    }
}
impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("can_be_canceled", &self.can_be_canceled())
            .field("is_canceled", &self.is_canceled())
            .finish()
    }
}

struct WakerRegistration<'a> {
    wakers: &'a Wakers,
    key: Option<usize>,
}
impl<'a> WakerRegistration<'a> {
    pub fn new(wakers: &'a Wakers) -> Self {
        Self { wakers, key: None }
    }
    pub fn is_cancelled(&self) -> bool {
        self.wakers.is_canceled()
    }
    pub fn set(&mut self, waker: &Waker) -> bool {
        if let Some(wakers) = &mut *self.wakers.0.lock().unwrap() {
            if let Some(key) = self.key {
                wakers[key] = waker.clone();
            } else {
                self.key = Some(wakers.insert(waker.clone()));
            }
            true
        } else {
            false
        }
    }
}

impl Drop for WakerRegistration<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            if let Some(wakers) = &mut *self.wakers.0.lock().unwrap() {
                wakers.remove(key);
            }
        }
    }
}

struct WaitForCanceled<'a>(WakerRegistration<'a>);

impl Future for WaitForCanceled<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.0.set(cx.waker()) {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

struct WithCanceled<'a, Fut> {
    r: WakerRegistration<'a>,
    future: Pin<&'a mut Fut>,
}
impl<Fut: Future> Future for WithCanceled<'_, Fut> {
    type Output = Result<Fut::Output, Canceled>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.r.is_cancelled() {
            return Poll::Ready(Err(Canceled));
        }
        match Pin::new(&mut self.future).poll(cx) {
            Poll::Pending => {
                if self.r.set(cx.waker()) {
                    Poll::Pending
                } else {
                    Poll::Ready(Err(Canceled))
                }
            }
            Poll::Ready(v) => Poll::Ready(Ok(v)),
        }
    }
}

#[derive(Debug)]
pub struct Canceled;
