use testcontainers::{
    ContainerAsync, ContainerRequest, Image, TestcontainersError, runners::AsyncRunner,
};
use tokio::sync::Mutex;

static FIRST_START: Mutex<bool> = Mutex::const_new(false);

/// Starts containers like [`AsyncRunner::start`], serializing only the first
/// start in the process.
///
/// testcontainers' `watchdog` feature registers every container in a
/// `conquer_once::Lazy`. In conquer-once 0.4.0, two threads reaching its first
/// access together can hit `unreachable!()`: `try_get` reports the cell
/// uninitialized, the other thread finishes initializing it, and the
/// compare-and-swap that follows reports `AlreadyInit`. Once one container has
/// registered, the cell is ready and later starts run concurrently.
pub trait StartContainer<I: Image> {
    /// Starts the container.
    ///
    /// # Errors
    ///
    /// Returns the error from [`AsyncRunner::start`].
    fn start_container(
        self,
    ) -> impl std::future::Future<Output = Result<ContainerAsync<I>, TestcontainersError>> + Send;
}

impl<I: Image> StartContainer<I> for ContainerRequest<I> {
    async fn start_container(self) -> Result<ContainerAsync<I>, TestcontainersError> {
        let request = self;
        let mut started = FIRST_START.lock().await;
        if *started {
            drop(started);
            return request.start().await;
        }
        let container = request.start().await?;
        *started = true;
        Ok(container)
    }
}
