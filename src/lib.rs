use std::{
    fmt,
    future::{pending, Future},
    pin::{pin, Pin},
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

use slabmap::SlabMap;

#[cfg(doctest)]
pub mod tests {
    #[doc = include_str!("../README.md")]
    pub mod readme {}
}

struct RawTokenSource(Mutex<Option<Data>>);

impl RawTokenSource {
    fn new(parent: Option<Parent>) -> Self {
        Self(Mutex::new(Some(Data::new(parent))))
    }
    fn cancel(&self) {
        let Some(data) = self.0.lock().unwrap().take() else {
            return;
        };
        data.wakers.into_iter().for_each(|(_, waker)| waker.wake());
        data.childs.into_iter().for_each(|(_, child)| {
            if let Some(child) = child.upgrade() {
                child.cancel();
            }
        });
    }
    fn is_canceled(&self) -> bool {
        self.0.lock().unwrap().is_none()
    }
}

struct Parent {
    parent: Arc<RawTokenSource>,
    key: usize,
}
impl Drop for Parent {
    fn drop(&mut self) {
        if let Some(data) = &mut *self.parent.0.lock().unwrap() {
            data.childs.remove(self.key);
        }
    }
}

struct Data {
    wakers: SlabMap<Waker>,
    childs: SlabMap<Weak<RawTokenSource>>,
    _parent: Option<Parent>, // Prevent the parent from being released and breaking the link with its ancestors.
}
impl Data {
    fn new(parent: Option<Parent>) -> Self {
        Self {
            wakers: SlabMap::new(),
            childs: SlabMap::new(),
            _parent: parent,
        }
    }
}

/// An object for sending cancellation notifications.
///
/// Cancellation can be requested through the [`CancellationTokenSource::cancel`] method.
pub struct CancellationTokenSource(Option<Arc<RawTokenSource>>);

impl CancellationTokenSource {
    fn new_canceled() -> Self {
        Self(None)
    }

    /// Create a new CancellationTokenSource.
    pub fn new() -> Self {
        Self(Some(Arc::new(RawTokenSource::new(None))))
    }

    /// Create a new CancellationTokenSource with a parent token.
    ///
    /// When the parent token is canceled, the child token is also canceled.
    #[doc(alias = "CreateLinkedTokenSource")]
    pub fn with_parent(parent: &CancellationToken) -> Self {
        match &parent.0 {
            RawToken::IsCanceled(true) => Self::new_canceled(),
            RawToken::IsCanceled(false) => Self::new(),
            RawToken::Source(source) => {
                if let Some(data) = &mut *source.0.lock().unwrap() {
                    Self(Some(Arc::new_cyclic(|child| {
                        let parent = Parent {
                            parent: source.clone(),
                            key: data.childs.insert(child.clone()),
                        };
                        RawTokenSource::new(Some(parent))
                    })))
                } else {
                    Self::new_canceled()
                }
            }
        }
    }

    /// Send cancellation notification.
    pub fn cancel(&self) {
        if let Some(source) = &self.0 {
            source.cancel();
        }
    }

    /// Get [`CancellationToken`] to receive cancellation notification from this source.
    pub fn token(&self) -> CancellationToken {
        if let Some(source) = &self.0 {
            CancellationToken(RawToken::Source(source.clone()))
        } else {
            CancellationToken(RawToken::IsCanceled(true))
        }
    }

    /// Returns true if canceled.
    pub fn is_canceled(&self) -> bool {
        if let Some(source) = &self.0 {
            source.is_canceled()
        } else {
            true
        }
    }
}
impl Clone for CancellationTokenSource {
    /// Create a CancellationTokenSource that shares the destination for cancellation notifications.
    fn clone(&self) -> Self {
        Self(self.0.clone())
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
    Source(Arc<RawTokenSource>),
}

/// An object for receiving cancellation notifications.
#[derive(Clone)]
pub struct CancellationToken(RawToken);

impl CancellationToken {
    /// Create a new CancellationToken with cancellation state.
    pub const fn new(is_canceled: bool) -> Self {
        Self(RawToken::IsCanceled(is_canceled))
    }

    /// Return true if this token can be canceled.
    pub fn can_be_canceled(&self) -> bool {
        match &self.0 {
            RawToken::IsCanceled(is_canceled) => *is_canceled,
            RawToken::Source(_) => true,
        }
    }

    /// Returns true if canceled.
    pub fn is_canceled(&self) -> bool {
        match &self.0 {
            RawToken::IsCanceled(is_canceled) => *is_canceled,
            RawToken::Source(source) => source.is_canceled(),
        }
    }

    /// Returns `Err(Canceled)` if canceled, otherwise returns `Ok(())`.
    ///
    /// # Example
    ///
    /// ```
    /// use cancellation_token::{CancellationToken, MayBeCanceled};
    ///
    /// fn cancellable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    ///     for _ in 0..100 {
    ///         ct.canceled()?; // Return from this function if canceled
    ///         heavy_work();
    ///     }
    ///     Ok(100)
    /// }
    /// fn heavy_work() { }
    /// ```
    pub fn canceled(&self) -> MayBeCanceled {
        if self.is_canceled() {
            Err(Canceled)
        } else {
            Ok(())
        }
    }

    /// Wait until canceled.
    pub async fn wait_for_cancellation(&self) {
        match &self.0 {
            RawToken::IsCanceled(false) => pending().await,
            RawToken::IsCanceled(true) => {}
            RawToken::Source(source) => WaitForCancellation(WakerRegistration::new(source)).await,
        }
    }

    /// Call the specified Future and returns its result. However, if this token is canceled, the Future call is aborted and `Err(Canceled)` is returned.
    ///
    /// # Example
    ///
    /// ```
    /// use cancellation_token::{CancellationToken, MayBeCanceled};
    ///
    /// async fn cancellable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    ///     for _ in 0..100 {
    ///         ct.with(heavy_work()).await?;
    ///     }
    ///     Ok(100)
    /// }
    /// async fn heavy_work() { }
    /// ```
    pub async fn with<T>(&self, future: impl Future<Output = T>) -> MayBeCanceled<T> {
        match &self.0 {
            RawToken::IsCanceled(false) => Ok(future.await),
            RawToken::IsCanceled(true) => Err(Canceled),
            RawToken::Source(source) => {
                WithCanceled {
                    r: WakerRegistration::new(source),
                    future: pin!(future),
                }
                .await
            }
        }
    }
}
impl Default for CancellationToken {
    /// Create a CancellationToken that will never be canceled.
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
    source: &'a RawTokenSource,
    key: Option<usize>,
}
impl<'a> WakerRegistration<'a> {
    pub fn new(source: &'a RawTokenSource) -> Self {
        Self { source, key: None }
    }
    pub fn is_canceled(&self) -> bool {
        self.source.is_canceled()
    }
    pub fn set(&mut self, waker: &Waker) -> bool {
        if let Some(data) = &mut *self.source.0.lock().unwrap() {
            if let Some(key) = self.key {
                data.wakers[key] = waker.clone();
            } else {
                self.key = Some(data.wakers.insert(waker.clone()));
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
            if let Some(data) = &mut *self.source.0.lock().unwrap() {
                data.wakers.remove(key);
            }
        }
    }
}

struct WaitForCancellation<'a>(WakerRegistration<'a>);

impl Future for WaitForCancellation<'_> {
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
        if self.r.is_canceled() {
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

/// Return value of the method that may be canceled.
pub type MayBeCanceled<T = ()> = Result<T, Canceled>;

/// A value indicating that it has been canceled.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Canceled;
