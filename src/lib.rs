use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use async_trait::async_trait;

use async_compression::tokio::bufread::GzipDecoder;

use log::debug;

pub mod error;

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

pub(crate) async fn decompress_bgz_block(compressed_block: &[u8]) -> crate::error::Result<Vec<u8>>
{
	let mut bytes: Vec<u8> = Vec::new();
	let mut decoder = GzipDecoder::new(compressed_block); // Skip the header for decompression
	decoder
		.read_to_end(&mut bytes)
		.await
		.map_err(|_| crate::error::Error::BGZDecompress)?; // Collect the decompressed bytes into a Vec<u8>
	Ok(bytes)
}

pub fn is_bam_eof(bytes: &[u8]) -> bool
{
	bytes[16..=27]
		== [
			0x1b, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
		]
}

#[async_trait]
pub trait BGZ
{
	async fn read_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<Vec<u8>>>
	where
		F: Fn(&[u8]) -> bool + std::marker::Send;
}

#[async_trait]
impl BGZ for BufReader<File>
{
	async fn read_bgzf_block<F>(
		&mut self,
		eof_check: Option<F>,
	) -> crate::error::Result<Option<Vec<u8>>>
	where
		F: Fn(&[u8]) -> bool + std::marker::Send,
	{
		let mut header = [0; 18];

		match self.read_exact(&mut header).await
		{
			Ok(_) =>
			{
				if !is_valid_bgzf_header(&header)
				{
					return Err(error::Error::BGZInvalidHeader(header)); // Skip to the next block if header is invalid.
				}

				// Calculate the size of the compressed block using BSIZE.
				let bsize = u16::from_le_bytes([header[16], header[17]]) as usize + 1;

				//println!("Header block size {} with contents: {:?}", bsize, header);

				// Read the rest of the BGZF block (bsize - 18 bytes).
				let mut compressed_block = vec![0; bsize];
				compressed_block[..18].copy_from_slice(&header);
				self.read_exact(&mut compressed_block[18..])
					.await
					.map_err(|_| crate::error::Error::BGZRead)?;

				match eof_check
				{
					Some(check_fn) =>
					{
						if bsize == 28 && check_fn(&compressed_block)
						{
							debug!("EOF header = {:?}", compressed_block);
							return Ok(None);
						}
					}
					None =>
					{
						debug!("EOF header = {:?}", compressed_block);
						return Ok(None);
					}
				}

				let decompressed_block = decompress_bgz_block(&compressed_block).await?;

				Ok(Some(decompressed_block))
			}
			Err(_) =>
			{
				return Err(crate::error::Error::BGZRead);
			}
		}
	}
}
