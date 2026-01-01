use async_trait::async_trait;
use async_compression::tokio::bufread::ZlibDecoder;
use tokio::io::{SeekFrom, AsyncRead, AsyncReadExt, AsyncSeekExt, BufReader};

use crate::error;

pub const BIGBED_MAGIC: u32 = 0x8789F2EB;

// need to move these functions, cleanup, add additional functions (maybe switch to byteorder?)
// #[derive(Debug, Clone, Copy)]
// pub enum Endian
// {
// 	Little,
// 	Big,
// }

// pub async fn read_u16<R: AsyncRead + Unpin>(reader: &mut R, endian: Endian) -> error::Result<u16>
// {
// 	let v = reader.read_u16_le().await?;
// 	Ok(match endian
// 	{
// 		Endian::Little => v,
// 		Endian::Big => v.swap_bytes(),
// 	})
// }

// pub async fn read_u32<R: AsyncRead + Unpin>(reader: &mut R, endian: Endian) -> error::Result<u32>
// {
// 	let v = reader.read_u32_le().await?;
// 	Ok(match endian
// 	{
// 		Endian::Little => v,
// 		Endian::Big => v.swap_bytes(),
// 	})
// }

// pub async fn read_u64<R: AsyncRead + Unpin>(reader: &mut R, endian: Endian) -> error::Result<u64>
// {
// 	let v = reader.read_u64_le().await?;
// 	Ok(match endian
// 	{
// 		Endian::Little => v,
// 		Endian::Big => v.swap_bytes(),
// 	})
// }

#[async_trait]
pub trait ZlibBlock
{
	/// `block_size` is the number of compressed bytes to read (from BigBed metadata)
	async fn read_zlib_block(&mut self, block_size: usize) -> crate::error::Result<Vec<u8>>;
	async fn is_bigbed(&mut self) -> bool;
}

#[async_trait]
impl<R> ZlibBlock for BufReader<R>
where
	R: AsyncRead + AsyncSeekExt + Unpin + Send,
{
	async fn read_zlib_block(
		&mut self,
		max_uncompressed_size: usize,
	) -> crate::error::Result<Vec<u8>>
	{
		let mut decoder = ZlibDecoder::new(self);

		let mut decompressed = Vec::with_capacity(max_uncompressed_size);

		decoder.read_to_end(&mut decompressed).await?;

		Ok(decompressed)
	}

	async fn is_bigbed(&mut self) -> bool
	{
		let current_pos = match self.seek(SeekFrom::Current(0)).await
		{
			Ok(value) => value,
			Err(_) => return false,
		};

		let mut magic = [0u8; 4];
		let read_result = self.read_exact(&mut magic).await;

		// ALWAYS seek back
		if self.seek(SeekFrom::Start(current_pos)).await.is_err()
		{
			return false;
		}

		match read_result
		{
			Ok(_) =>
			{
				let le = u32::from_le_bytes(magic);
				let be = u32::from_be_bytes(magic);
				le == BIGBED_MAGIC || be == BIGBED_MAGIC
			}
			Err(_) => false,
		}
	}
}
