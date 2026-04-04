use tokio::io::{AsyncReadExt};

use async_compression::tokio::bufread::GzipDecoder;

pub async fn decompress_bgzf_block(compressed_block: &[u8]) -> crate::error::Result<Vec<u8>>
{
	let mut bytes: Vec<u8> = Vec::new();
	let mut decoder = GzipDecoder::new(compressed_block); // Skip the header for decompression
	decoder
		.read_to_end(&mut bytes)
		.await
		.map_err(|_| crate::error::Error::Decompress)?; // Collect the decompressed bytes into a Vec<u8>
	Ok(bytes)
}
