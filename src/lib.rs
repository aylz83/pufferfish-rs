pub mod error;

#[cfg(feature = "bgzf")]
pub mod bgzf;

#[cfg(feature = "zlib")]
pub mod zlib;

pub mod prelude
{
	#[cfg(feature = "bgzf")]
	pub use crate::bgzf::*;
	#[cfg(feature = "zlib")]
	pub use crate::zlib::*;
}
