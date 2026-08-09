//! Internal runtime facade supporting Tokio and deterministic madsim tests.

#[cfg(feature = "runtime-madsim-tokio")]
pub use runtime_madsim_tokio::*;

#[cfg(feature = "runtime-tokio")]
pub use runtime_tokio::*;

#[cfg(all(feature = "runtime-tokio", feature = "runtime-madsim-tokio"))]
compile_error!("runtime-tokio and runtime-madsim-tokio cannot both be enabled");

#[cfg(not(any(feature = "runtime-tokio", feature = "runtime-madsim-tokio")))]
compile_error!("either runtime-tokio or runtime-madsim-tokio must be enabled");
