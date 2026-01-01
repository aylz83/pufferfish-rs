use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum Error
{
	#[error("Unable to decompress BGZ block")]
	Decompress,
	#[error("Unable to read BGZ block")]
	Read,
	#[error("Invalid BGZ header: {0:?}")]
	InvalidHeader([u8; 18]),
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
}
