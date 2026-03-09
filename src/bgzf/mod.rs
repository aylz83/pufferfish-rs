pub mod pool;

use crate::bgzf::pool::BgzfBlockPool;

use tokio::io::{AsyncSeekExt, SeekFrom, AsyncRead, AsyncReadExt, BufReader};

use std::sync::Arc;

use crate::error;

pub type BgzfCompressedBlock = Vec<u8>;

#[cfg(feature = "bgzf-async")]
pub mod async_decompression;
#[cfg(feature = "bgzf-stream")]
pub mod stream;

const BGZIP_EOF_BLOCK: [u8; 28] = [
	0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00,
	0x1b, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const BAM_EOF_BLOCK: [u8; 12] = [
	0x1b, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub(crate) fn is_valid_bgzf_header(header: &[u8]) -> bool
{
	if header.len() != 18
	{
		return false; // Invalid header size.
	}

	// Check the fixed values in the header (no need for endianness consideration here).
	if header[0] != 0x1f || header[1] != 0x8b || header[2] != 0x08
	{
		return false; // Not a valid GZIP header.
	}

	// Check the subfield identifiers and length.
	if header[10] != 0x06 || header[11] != 0x00 || // XLEN = 6
		  header[12] != 0x42 || header[13] != 0x43 || // SI1 = 'B', SI2 = 'C'
	      header[14] != 0x02 || header[15] != 0x00
	{
		// SLEN = 2
		return false; // Not a valid BGZF header.
	}

	// Interpret BSIZE as a little-endian 16-bit integer.
	let bsize = u16::from_le_bytes([header[16], header[17]]);
	if bsize < 18
	{
		return false; // BSIZE should be at least 18 for a valid BGZF block.
	}

	true
}

pub fn is_bgzf_eof(bsize: usize, block: &[u8]) -> bool
{
	bsize == 28 && block == BGZIP_EOF_BLOCK
}

pub fn is_bam_eof(bsize: usize, bytes: &[u8]) -> bool
{
	bsize == 28 && bytes[16..=27] == BAM_EOF_BLOCK
}

pub type EofCheck = fn(usize, &[u8]) -> bool;

#[async_trait::async_trait]
pub trait BGZ
{
	async fn is_bgz(&mut self) -> bool;

	async fn read_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<BgzfCompressedBlock>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send;
	async fn read_and_decompress_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<Vec<u8>>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send;
	async fn read_bgzf_block_pool<F>(
		&mut self,
		pool: Arc<BgzfBlockPool>,
		eof_check: Option<F>,
	) -> error::Result<Option<BgzfCompressedBlock>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send;
}

#[async_trait::async_trait]
impl<R> BGZ for BufReader<R>
where
	R: AsyncRead + std::marker::Send + std::marker::Unpin + AsyncSeekExt,
{
	async fn is_bgz(&mut self) -> bool
	{
		let mut header = [0u8; 18];

		let current_position = match self.seek(SeekFrom::Current(0)).await
		{
			Ok(value) => value,
			Err(_) => return false,
		};

		let read_result = self.read_exact(&mut header).await;

		// ALWAYS seek back
		if self
			.get_mut()
			.seek(SeekFrom::Start(current_position))
			.await
			.is_err()
		{
			return false;
		}

		match read_result
		{
			Ok(_) => is_valid_bgzf_header(&header),
			Err(_) => false,
		}
	}

	async fn read_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<BgzfCompressedBlock>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send,
	{
		let mut header = [0; 18];

		match self.read_exact(&mut header).await
		{
			Ok(_) =>
			{
				if !is_valid_bgzf_header(&header)
				{
					return Err(error::Error::InvalidHeader(header)); // Skip to the next block if header is invalid.
				}

				// Calculate the size of the compressed block using BSIZE.
				let bsize = u16::from_le_bytes([header[16], header[17]]) as usize + 1;

				//println!("Header block size {} with contents: {:?}", bsize, header);

				// Read the rest of the BGZF block (bsize - 18 bytes).
				let mut compressed_block = vec![0; bsize];
				compressed_block[..18].copy_from_slice(&header);
				self.read_exact(&mut compressed_block[18..])
					.await
					.map_err(|_| crate::error::Error::Read)?;

				// If a custom EOF check is provided, check if this is true, otherwise
				// assume EOF if bsize == 28
				if matches!(eof_check, Some(ref check_fn) if check_fn(bsize, &compressed_block))
					|| bsize == 28
				{
					// debug!("EOF header = {:?}", compressed_block);
					return Ok(None);
				}

				Ok(Some(compressed_block))
			}
			Err(_) =>
			{
				return Err(error::Error::Read);
			}
		}
	}

	#[cfg(all(feature = "bgzf-async", not(feature = "bgzf-stream")))]
	async fn read_and_decompress_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<Vec<u8>>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send,
	{
		let block = self.read_bgzf_block(eof_check).await?;
		match block
		{
			Some(block) => Ok(Some(
				crate::bgzf::async_decompression::decompress_bgzf_block(&block).await?,
			)),
			None => Ok(None),
		}
	}

	#[cfg(feature = "bgzf-stream")]
	async fn read_and_decompress_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<Vec<u8>>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send,
	{
		let block = self.read_bgzf_block(eof_check).await?;
		match block
		{
			Some(block) => Ok(Some(crate::bgzf::stream::decompress_bgzf_block(&block)?)),
			None => Ok(None),
		}
	}

	async fn read_bgzf_block_pool<F>(
		&mut self,
		pool: Arc<BgzfBlockPool>,
		eof_check: Option<F>,
	) -> error::Result<Option<BgzfCompressedBlock>>
	where
		F: Fn(usize, &[u8]) -> bool + std::marker::Send,
	{
		let mut header = [0u8; 18];

		match self.read_exact(&mut header).await
		{
			Ok(_) =>
			{
				if !crate::bgzf::is_valid_bgzf_header(&header)
				{
					return Err(error::Error::InvalidHeader(header));
				}

				// Determine the full block size
				let bsize = u16::from_le_bytes([header[16], header[17]]) as usize + 1;

				// Acquire a buffer from the pool
				let mut compressed_block = pool.acquire();
				if compressed_block.len() < bsize
				{
					compressed_block.resize(bsize, 0); // grow if needed
				}

				// Copy header and read rest
				compressed_block[..18].copy_from_slice(&header);
				self.read_exact(&mut compressed_block[18..bsize])
					.await
					.map_err(|_| error::Error::Read)?;

				// Check for EOF
				if matches!(eof_check, Some(ref check_fn) if check_fn(bsize, &compressed_block[..bsize]))
					|| bsize == 28
				{
					// Return buffer to pool
					pool.release(compressed_block);
					return Ok(None);
				}

				Ok(Some(compressed_block[..bsize].to_vec()))
			}
			Err(_) => Err(error::Error::Read),
		}
	}
}
