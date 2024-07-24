use std::{
    any::Any,
    error, fmt,
    future::{pending, Future},
    pin::{pin, Pin},
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

use slabmap::SlabMap;

#[cfg(doctest)]
mod tests_readme {
    #[doc = include_str!("../README.md")]
    pub mod readme {}
}

#[cfg(test)]
mod tests;

struct RawTokenSource(Mutex<Option<Data>>);

impl RawTokenSource {
    fn new(parent: CancellationTokenRegistration) -> Self {
        Self(Mutex::new(Some(Data::new(parent))))
    }
    fn is_canceled(&self) -> bool {
        self.0.lock().unwrap().is_none()
    }
    fn cancel(&self) {
        let Some(data) = self.0.lock().unwrap().take() else {
            return;
        };
        data.cbs.into_iter().for_each(|(_, cb)| cb.call());
    }
}

struct Data {
    cbs: SlabMap<CancelCallback>,
    _parent: CancellationTokenRegistration, // Prevent the parent from being released and breaking the link with its ancestors.
}
impl Data {
    fn new(parent: CancellationTokenRegistration) -> Self {
        Self {
            cbs: SlabMap::new(),
            _parent: parent,
        }
    }
}

/// An object for sending cancellation notifications.
///
/// Use [`cancel()`](CancellationTokenSource::cancel) to notify cancellation.
pub struct CancellationTokenSource(Option<Arc<RawTokenSource>>);

impl CancellationTokenSource {
    fn new_canceled() -> Self {
        Self(None)
    }

    /// Create a new CancellationTokenSource.
    pub fn new() -> Self {
        Self(Some(Arc::new(RawTokenSource::new(
            CancellationTokenRegistration(None),
        ))))
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
                        RawTokenSource::new(CancellationTokenRegistration(Some(RawRegistration {
                            source: source.clone(),
                            key: data
                                .cbs
                                .insert(CancelCallback::from_raw_token_source(child)),
                        })))
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

    /// Create an object for which a cancellation notification is sent when dropped.
    pub fn cancel_defer(&self) -> CancelOnDrop {
        CancelOnDrop(Some(self.clone()))
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
///
/// Obtained by [`CancellationTokenSource::token()`] or [`new()`](CancellationToken::new).
///
/// - Use [`is_canceled()`](CancellationToken::is_canceled) to see if the cancellation has been notified.
/// - Use [`canceled()`](CancellationToken::canceled) to implement cancellation using the `?` operator.
/// - Use [`run()`](CancellationToken::run) to apply cancellation to async functions.
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
    /// This method corresponds to `ThrowIfCancellationRequested` in C#.
    ///
    /// # Example
    ///
    /// ```
    /// use cancellation_token::{CancellationToken, MayBeCanceled};
    ///
    /// fn cancelable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    ///     for _ in 0..100 {
    ///         ct.canceled()?; // Return from this function if canceled
    ///         heavy_work();
    ///     }
    ///     Ok(100)
    /// }
    /// fn heavy_work() { }
    /// ```
    #[doc(alias = "ThrowIfCancellationRequested")]
    pub fn canceled(&self) -> MayBeCanceled {
        if self.is_canceled() {
            Err(Canceled)
        } else {
            Ok(())
        }
    }

    /// Register a callback to be called when this token is canceled.
    ///
    /// If this token has already been canceled, the callback is called before the function returns.
    ///
    /// Callbacks are called synchronously when [`CancellationTokenSource::cancel()`] is called, so care must be taken to avoid deadlocks.
    pub fn register(&self, cb: CancelCallback) -> CancellationTokenRegistration {
        // Compared to other methods, this method is more prone to deadlocks, so it has been intentionally designed to be verbose.
        let is_canceled = match &self.0 {
            RawToken::IsCanceled(is_canceled) => *is_canceled,
            RawToken::Source(source) => {
                if let Some(data) = &mut *source.0.lock().unwrap() {
                    return CancellationTokenRegistration(Some(RawRegistration {
                        source: source.clone(),
                        key: data.cbs.insert(cb),
                    }));
                } else {
                    true
                }
            }
        };
        if is_canceled {
            cb.call();
        }
        CancellationTokenRegistration::empty()
    }

    /// Wait until canceled.
    pub async fn wait(&self) {
        match &self.0 {
            RawToken::IsCanceled(false) => pending().await,
            RawToken::IsCanceled(true) => {}
            RawToken::Source(source) => WaitForCancellation(WakerRegistration::new(source)).await,
        }
    }

    /// Runs the specified future. However, if this token is canceled, it will stop the running of that future and return `Err(Canceled)`.
    ///
    /// # Example
    ///
    /// ```
    /// use cancellation_token::{CancellationToken, MayBeCanceled};
    ///
    /// async fn cancelable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    ///     for _ in 0..100 {
    ///         ct.run(heavy_work()).await?;
    ///     }
    ///     Ok(100)
    /// }
    /// async fn heavy_work() { }
    /// ```
    pub async fn run<T>(&self, future: impl Future<Output = T>) -> MayBeCanceled<T> {
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

/// Callback function passed to [`CancellationToken::register()`].
///
/// You can create `CancelCallback` using the following methods.
///
/// - [`new`](Self::new)
/// - [`from_arc_fn`](Self::from_arc_fn)
/// - [`from_weak_fn`](Self::from_weak_fn)
/// - [`From::from`] or [`Into::into`] (convert from [`Waker`], `&Waker`, or [`Box<impl FnOnce()>`](FnOnce))
///
/// The callback function are called synchronously when [`CancellationTokenSource::cancel()`] is called,
/// so care must be taken to avoid deadlocks.
pub struct CancelCallback(RawCancelCallback);

impl CancelCallback {
    /// Create a new callback from a function.
    pub fn new(f: impl FnOnce() + Sync + Send + 'static) -> Self {
        Self(RawCancelCallback::Fn(Box::new(f)))
    }

    /// Create a new callback from an `Arc` and a function.
    ///
    /// If `f` is a ZST (Zero-Sized Type), this method does not allocate memory.
    pub fn from_arc_fn<T: Send + Sync + 'static>(
        this: Arc<T>,
        f: impl FnOnce(Arc<T>) + Sync + Send + Copy + 'static,
    ) -> Self {
        Self(RawCancelCallback::Arc {
            this,
            f: Box::new(move |this| f(this.downcast().unwrap())),
        })
    }
    /// Create a new callback from an `Weak` and a function.
    ///
    /// Calls the function only if the specified weak reference is alive when cancelled.
    ///
    /// If `f` is a ZST (Zero-Sized Type), this method does not allocate memory.
    pub fn from_weak_fn<T: Send + Sync + 'static>(
        this: Weak<T>,
        f: impl FnOnce(Arc<T>) + Sync + Send + Copy + 'static,
    ) -> Self {
        Self(RawCancelCallback::Weak {
            this,
            f: Box::new(move |this| f(this.downcast().unwrap())),
        })
    }
    fn from_raw_token_source(source: &Weak<RawTokenSource>) -> Self {
        Self::from_weak_fn(source.clone(), |source| source.cancel())
    }
    fn call(self) {
        self.0.call();
    }
}
impl From<Box<dyn FnOnce() + Sync + Send>> for CancelCallback {
    fn from(value: Box<dyn FnOnce() + Sync + Send>) -> Self {
        Self(RawCancelCallback::Fn(value))
    }
}
impl<F: FnOnce() + Sync + Send + 'static> From<Box<F>> for CancelCallback {
    fn from(value: Box<F>) -> Self {
        Self(RawCancelCallback::Fn(value))
    }
}
impl From<Waker> for CancelCallback {
    fn from(value: Waker) -> Self {
        Self(RawCancelCallback::Waker(value))
    }
}
impl From<&Waker> for CancelCallback {
    fn from(value: &Waker) -> Self {
        Self(RawCancelCallback::Waker(value.clone()))
    }
}

enum RawCancelCallback {
    Fn(Box<dyn FnOnce() + Sync + Send>),
    Waker(Waker),
    Arc {
        this: Arc<dyn Any + Sync + Send>,
        f: Box<dyn FnOnce(Arc<dyn Any + Send + Sync>) + Sync + Send>,
    },
    Weak {
        this: Weak<dyn Any + Sync + Send>,
        f: Box<dyn FnOnce(Arc<dyn Any + Send + Sync>) + Sync + Send>,
    },
}
impl RawCancelCallback {
    fn call(self) {
        match self {
            Self::Fn(f) => f(),
            Self::Waker(waker) => waker.wake(),
            Self::Arc { this, f } => f(this),
            Self::Weak { this, f } => {
                if let Some(this) = this.upgrade() {
                    f(this);
                }
            }
        }
    }
}

struct RawRegistration {
    source: Arc<RawTokenSource>,
    key: usize,
}

/// An object to unregister callback.
///
/// Callbacks are automatically unregistered when dropped.
#[derive(Default)]
pub struct CancellationTokenRegistration(Option<RawRegistration>);

impl CancellationTokenRegistration {
    fn empty() -> Self {
        Self(None)
    }

    /// Ensure the callback is never unregistered.
    pub fn detach(mut self) {
        self.0.take();
    }
}
impl fmt::Debug for CancellationTokenRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationTokenRegistration")
            .field("is_empty", &self.0.is_none())
            .finish()
    }
}
impl Drop for CancellationTokenRegistration {
    fn drop(&mut self) {
        if let Some(raw) = self.0.take() {
            if let Some(data) = &mut *raw.source.0.lock().unwrap() {
                data.cbs.remove(raw.key);
            }
        }
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
            let cb = CancelCallback::from(waker);
            if let Some(key) = self.key {
                data.cbs[key] = cb;
            } else {
                self.key = Some(data.cbs.insert(cb));
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
                data.cbs.remove(key);
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

impl fmt::Display for Canceled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "operation has been cancelled".fmt(f)
    }
}

impl error::Error for Canceled {}

/// An object for which a cancellation notification is sent when dropped.
///
/// Returned by [`CancellationTokenSource::cancel_defer()`].
pub struct CancelOnDrop(Option<CancellationTokenSource>);

impl CancelOnDrop {
    /// Drop the object without sending a cancellation notification.
    pub fn detach(mut self) {
        self.0.take();
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            source.cancel();
        }
    }
}
