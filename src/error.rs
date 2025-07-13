use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum Error
{
	#[error("Unable to decompress BGZ block")]
	BGZDecompress,
	#[error("Unable to read BGZ block")]
	BGZRead,
	#[error("Invalid BGZ header: {0:?}")]
	BGZInvalidHeader([u8; 18]),
}
