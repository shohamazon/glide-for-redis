use std::{io, time::Duration};

use futures_util::Future;

#[cfg(feature = "tokio-comp")]
use super::tokio;
use super::RedisRuntime;
use crate::types::RedisError;

#[derive(Clone, Debug)]
pub(crate) enum Runtime {
    #[cfg(feature = "tokio-comp")]
    Tokio,
}

impl Runtime {
    pub(crate) fn locate() -> Self {
        #[cfg(not(feature = "tokio-comp"))]
        {
            compile_error!("tokio-comp feature is required for aio feature")
        }
        #[cfg(feature = "tokio-comp")]
        {
            Runtime::Tokio
        }
    }

    #[allow(dead_code)]
    pub(super) fn spawn(&self, f: impl Future<Output = ()> + Send + 'static) {
        match self {
            #[cfg(feature = "tokio-comp")]
            Runtime::Tokio => tokio::Tokio::spawn(f),
        }
    }

    pub(crate) async fn timeout<F: Future>(
        &self,
        duration: Duration,
        future: F,
    ) -> Result<F::Output, Elapsed>
    where
        F::Output: std::fmt::Debug,
    {
        //println!("Starting timeout with duration: {:?}", duration);

        let result = match self {
            #[cfg(feature = "tokio-comp")]
            Runtime::Tokio => ::tokio::time::timeout(duration, future)
                .await
                .map_err(|_| Elapsed(())),
        };

        if result.is_err() {
            println!(
                "Timeout completed with result err: {:?}",
                result.as_ref().unwrap_err()
            );
        }
        result
    }
}

#[derive(Debug)]
pub(crate) struct Elapsed(());

impl From<Elapsed> for RedisError {
    fn from(_: Elapsed) -> Self {
        io::Error::from(io::ErrorKind::TimedOut).into()
    }
}
