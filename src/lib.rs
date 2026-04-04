pub mod error;

#[cfg(any(feature = "bgzf-async", feature = "bgzf-stream"))]
pub mod bgzf;

#[cfg(feature = "zlib")]
pub mod zlib;

pub mod prelude
{
	#[cfg(all(feature = "bgzf-async", not(feature = "bgzf-stream")))]
	pub use crate::bgzf::async_decompression::*;

	#[cfg(feature = "bgzf-stream")]
	pub use crate::bgzf::stream::*;

	#[cfg(any(feature = "bgzf-async", feature = "bgzf-stream"))]
	pub use crate::bgzf::*;

	#[cfg(feature = "zlib")]
	pub use crate::zlib::*;
}
