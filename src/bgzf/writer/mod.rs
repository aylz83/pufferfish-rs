use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use std::sync::Arc;

use libdeflater::{CompressionLvl, Compressor};

use rayon::prelude::*;

use crate::error;
use crate::bgzf::BgzfCompressedBlock;
use crate::bgzf::BgzfBlockPool;

use crate::bgzf::BGZIP_EOF_BLOCK;

const BGZF_MAX_BLOCK_SIZE: usize = 65_536;
const BGZF_HEADER_SIZE: usize = 18;
const BGZF_TRAILER_SIZE: usize = 8;
const BGZF_OVERHEAD: usize = BGZF_HEADER_SIZE + BGZF_TRAILER_SIZE;

pub trait EncodeBgzfPayload
{
	fn encode_into(self, bytes: &mut Vec<u8>) -> error::Result<()>;
}

#[derive(Clone)]
pub struct BgzfBlockWriter<W>
{
	writer: Arc<Mutex<BufWriter<W>>>,
	pool: Arc<rayon::ThreadPool>,
	block_pool: Arc<BgzfBlockPool>,
	remainder: Arc<Mutex<Vec<u8>>>,
}

impl<W> BgzfBlockWriter<W>
where
	W: AsyncWrite + Send + Unpin,
{
	pub fn new(writer: W, pool: Arc<rayon::ThreadPool>, block_pool: Arc<BgzfBlockPool>) -> Self
	{
		Self {
			writer: Arc::new(Mutex::new(BufWriter::new(writer))),
			pool,
			block_pool,
			remainder: Arc::new(Mutex::new(Vec::new())),
		}
	}

	pub async fn new_with_header<H>(
		writer: W,
		header: H,
		pool: Arc<rayon::ThreadPool>,
		block_pool: Arc<BgzfBlockPool>,
	) -> error::Result<Self>
	where
		H: EncodeBgzfPayload,
	{
		let writer = Self::new(writer, pool, block_pool);

		writer.write(std::iter::once(header)).await?;

		Ok(writer)
	}

	pub async fn write<I, T>(&self, values: I) -> error::Result<()>
	where
		I: IntoIterator<Item = T>,
		T: EncodeBgzfPayload,
	{
		let mut remainder = self.remainder.lock().await;

		let blocks =
			encode_and_compress_bgzf_blocks(values, &mut remainder, &self.pool, &self.block_pool)?;

		let mut writer = self.writer.lock().await;

		for block in blocks
		{
			let result = writer.write_all(&block).await;

			self.block_pool.release(block);

			result?;
		}

		Ok(())
	}

	pub async fn finish(&self) -> error::Result<()>
	{
		self.flush().await?;

		let mut writer = self.writer.lock().await;

		writer
			.write_all(&BGZIP_EOF_BLOCK)
			.await
			.map_err(|_| error::Error::Write)?;

		writer.flush().await.map_err(|_| error::Error::Write)?;

		Ok(())
	}

	pub async fn flush(&self) -> error::Result<()>
	{
		let mut remainder = self.remainder.lock().await;

		if !remainder.is_empty()
		{
			let block = compress_bgzf_block(&remainder, &self.block_pool)?;

			let mut writer = self.writer.lock().await;

			let result = writer.write_all(&block).await;

			self.block_pool.release(block);

			result?;

			remainder.clear();
		}

		let mut writer = self.writer.lock().await;
		writer.flush().await.map_err(|_| error::Error::Write)?;

		Ok(())
	}
}

pub fn compress_bgzf_block(
	bytes: &[u8],
	block_pool: &BgzfBlockPool,
) -> crate::error::Result<BgzfCompressedBlock>
{
	let mut compressor = Compressor::new(CompressionLvl::default());

	let max_size = compressor.deflate_compress_bound(bytes.len());

	if max_size > BGZF_MAX_BLOCK_SIZE - BGZF_OVERHEAD
	{
		return Err(error::Error::Compress);
	}

	let mut block = block_pool.acquire();

	// Reserve space for the header and maximum possible DEFLATE payload.
	block.resize(BGZF_HEADER_SIZE + max_size, 0);

	let compressed_size = match compressor.deflate_compress(bytes, &mut block[BGZF_HEADER_SIZE..])
	{
		Ok(size) => size,
		Err(_) =>
		{
			block_pool.release(block);
			return Err(error::Error::Compress);
		}
	};

	let total_size = BGZF_OVERHEAD + compressed_size;

	let bsize = match u16::try_from(total_size - 1)
	{
		Ok(value) if total_size <= BGZF_MAX_BLOCK_SIZE => value,
		_ =>
		{
			block_pool.release(block);
			return Err(error::Error::Compress);
		}
	};

	// Remove unused DEFLATE buffer capacity from the logical length.
	block.truncate(BGZF_HEADER_SIZE + compressed_size);

	// BGZF header.
	block[..BGZF_HEADER_SIZE].copy_from_slice(&build_bgzf_header(bsize));

	// CRC32.
	block.extend_from_slice(&libdeflater::crc32(bytes).to_le_bytes());

	// ISIZE.
	block.extend_from_slice(&(bytes.len() as u32).to_le_bytes());

	Ok(block)
}

pub fn compress_bgzf_blocks(
	bytes: &[u8],
	remainder: &mut Vec<u8>,
	pool: &rayon::ThreadPool,
	block_pool: &BgzfBlockPool,
) -> error::Result<Vec<BgzfCompressedBlock>>
{
	let chunk_size = bgzf_chunk_size();

	if chunk_size == 0
	{
		return Err(error::Error::Compress);
	}

	// Combine the previous remainder with the new bytes.
	let mut buffer = Vec::with_capacity(remainder.len() + bytes.len());

	buffer.extend_from_slice(remainder);
	buffer.extend_from_slice(bytes);

	// Determine how many complete chunks we have.
	let complete_len = (buffer.len() / chunk_size) * chunk_size;

	let complete = &buffer[..complete_len];

	// Compress independent chunks in parallel.
	// Collect every result so successful buffers can be recycled on failure.
	let results: Vec<error::Result<BgzfCompressedBlock>> = pool.install(|| {
		complete
			.par_chunks(chunk_size)
			.map(|chunk| compress_bgzf_block(chunk, block_pool))
			.collect()
	});

	let blocks = collect_compression_results(results, block_pool)?;

	*remainder = buffer[complete_len..].to_vec();

	Ok(blocks)
}

pub fn encode_and_compress_bgzf_blocks<I, T>(
	values: I,
	remainder: &mut Vec<u8>,
	thread_pool: &rayon::ThreadPool,
	block_pool: &BgzfBlockPool,
) -> error::Result<Vec<BgzfCompressedBlock>>
where
	I: IntoIterator<Item = T>,
	T: EncodeBgzfPayload,
{
	let mut bytes = Vec::new();

	for value in values
	{
		value.encode_into(&mut bytes)?;
	}

	compress_bgzf_blocks(&bytes, remainder, thread_pool, block_pool)
}

fn build_bgzf_header(bsize: u16) -> [u8; BGZF_HEADER_SIZE]
{
	let mut header = [
		0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02,
		0x00, 0x00, 0x00,
	];

	header[16..18].copy_from_slice(&bsize.to_le_bytes());

	header
}

fn bgzf_chunk_size() -> usize
{
	let mut compressor = Compressor::new(CompressionLvl::default());

	let mut low = 0;
	let mut high = BGZF_MAX_BLOCK_SIZE;

	while low < high
	{
		let mid = low + (high - low + 1) / 2;

		let bound = compressor.deflate_compress_bound(mid);

		if bound <= BGZF_MAX_BLOCK_SIZE - BGZF_OVERHEAD
		{
			low = mid;
		}
		else
		{
			high = mid - 1;
		}
	}

	low
}

fn collect_compression_results(
	results: Vec<error::Result<BgzfCompressedBlock>>,
	block_pool: &BgzfBlockPool,
) -> error::Result<Vec<BgzfCompressedBlock>>
{
	let mut blocks = Vec::with_capacity(results.len());
	let mut compression_error = None;

	for result in results
	{
		match result
		{
			Ok(block) => blocks.push(block),
			Err(err) =>
			{
				if compression_error.is_none()
				{
					compression_error = Some(err);
				}
			}
		}
	}

	if let Some(err) = compression_error
	{
		for block in blocks
		{
			block_pool.release(block);
		}

		return Err(err);
	}

	Ok(blocks)
}

#[cfg(test)]
mod tests
{
	use super::*;

	struct TestPayload(Vec<u8>);

	impl EncodeBgzfPayload for TestPayload
	{
		fn encode_into(self, bytes: &mut Vec<u8>) -> error::Result<()>
		{
			bytes.extend_from_slice(&self.0);
			Ok(())
		}
	}

	fn test_thread_pool() -> Arc<rayon::ThreadPool>
	{
		Arc::new(
			rayon::ThreadPoolBuilder::new()
				.num_threads(2)
				.build()
				.unwrap(),
		)
	}

	#[test]
	fn compress_bgzf_block_produces_valid_block_structure()
	{
		let block_pool = BgzfBlockPool::new(2, BGZF_MAX_BLOCK_SIZE);
		let bytes = vec![42u8; 1024];

		let block = compress_bgzf_block(&bytes, &block_pool).unwrap();

		assert!(block.len() <= BGZF_MAX_BLOCK_SIZE);
		assert!(crate::bgzf::is_valid_bgzf_header(
			&block[..BGZF_HEADER_SIZE]
		));

		let bsize = u16::from_le_bytes([block[16], block[17]]) as usize + 1;

		assert_eq!(bsize, block.len());

		let isize = u32::from_le_bytes(block[block.len() - 4..].try_into().unwrap()) as usize;

		assert_eq!(isize, bytes.len());
	}

	#[test]
	fn compress_bgzf_blocks_retains_incomplete_remainder()
	{
		let thread_pool = test_thread_pool();
		let block_pool = BgzfBlockPool::new(2, BGZF_MAX_BLOCK_SIZE);
		let chunk_size = bgzf_chunk_size();

		let bytes = vec![42u8; chunk_size + 17];
		let mut remainder = Vec::new();

		let blocks =
			compress_bgzf_blocks(&bytes, &mut remainder, &thread_pool, &block_pool).unwrap();

		assert_eq!(blocks.len(), 1);
		assert_eq!(remainder.len(), 17);
		assert_eq!(remainder, vec![42u8; 17]);
	}

	#[test]
	fn compress_bgzf_blocks_completes_previous_remainder()
	{
		let thread_pool = test_thread_pool();
		let block_pool = BgzfBlockPool::new(2, BGZF_MAX_BLOCK_SIZE);
		let chunk_size = bgzf_chunk_size();

		let mut remainder = Vec::new();

		let first = vec![42u8; chunk_size - 10];

		let blocks =
			compress_bgzf_blocks(&first, &mut remainder, &thread_pool, &block_pool).unwrap();

		assert!(blocks.is_empty());
		assert_eq!(remainder.len(), chunk_size - 10);

		let second = vec![24u8; 20];

		let blocks =
			compress_bgzf_blocks(&second, &mut remainder, &thread_pool, &block_pool).unwrap();

		assert_eq!(blocks.len(), 1);
		assert_eq!(remainder.len(), 10);
		assert_eq!(remainder, vec![24u8; 10]);
	}

	#[test]
	fn compression_error_returns_successful_buffers_to_pool()
	{
		let block_pool = BgzfBlockPool::new(0, BGZF_MAX_BLOCK_SIZE);

		let mut block = block_pool.acquire();
		block.reserve(1024);
		block.extend_from_slice(b"compressed block");

		let original_ptr = block.as_ptr();

		let results = vec![Ok(block), Err(error::Error::Compress)];

		let result = collect_compression_results(results, &block_pool);

		assert!(result.is_err());

		// The successfully compressed buffer should be reusable.
		let recycled = block_pool.acquire();

		assert_eq!(recycled.as_ptr(), original_ptr);
		assert!(recycled.is_empty());
	}

	#[tokio::test]
	async fn writer_flushes_final_remainder()
	{
		let thread_pool = test_thread_pool();
		let block_pool = Arc::new(BgzfBlockPool::new(2, BGZF_MAX_BLOCK_SIZE));

		let writer = BgzfBlockWriter::new(Vec::<u8>::new(), thread_pool, block_pool);

		let payload = TestPayload(vec![42u8; 1024]);

		writer.write([payload]).await.unwrap();

		// The payload is smaller than a complete chunk.
		{
			let output = writer.writer.lock().await;
			assert!(output.get_ref().is_empty());
		}

		writer.flush().await.unwrap();

		let output = writer.writer.lock().await;
		let bytes = output.get_ref();

		assert!(!bytes.is_empty());
		assert!(crate::bgzf::is_valid_bgzf_header(
			&bytes[..BGZF_HEADER_SIZE]
		));

		let bsize = u16::from_le_bytes([bytes[16], bytes[17]]) as usize + 1;

		assert_eq!(bsize, bytes.len());
	}

	#[tokio::test]
	async fn write_bgzf_file_and_read_with_gzip()
	{
		use flate2::read::MultiGzDecoder;
		use std::io::Read;

		let thread_pool = Arc::new(
			rayon::ThreadPoolBuilder::new()
				.num_threads(4)
				.build()
				.unwrap(),
		);

		let block_pool = Arc::new(BgzfBlockPool::new(4, BGZF_MAX_BLOCK_SIZE));

		let path = std::env::temp_dir().join("test_output.txt.bgz");

		let file = tokio::fs::File::create(&path).await.unwrap();

		let writer = BgzfBlockWriter::new(file, thread_pool, block_pool);

		let sections = [
			"This is the first section.\n",
			"This is the second section.\n",
			"This is the third section.\n",
			"Testing BGZF compression with multiple writes.\n",
		];

		let expected = sections.concat();

		for section in sections
		{
			writer
				.write([TestPayload(section.as_bytes().to_vec())])
				.await
				.unwrap();
		}

		writer.flush().await.unwrap();

		drop(writer);

		// Read the BGZF file using a standard gzip-compatible decoder.
		let compressed = std::fs::read(&path).unwrap();

		let mut decoder = MultiGzDecoder::new(compressed.as_slice());

		let mut decompressed = String::new();

		decoder.read_to_string(&mut decompressed).unwrap();

		assert_eq!(decompressed, expected);

		std::fs::remove_file(path).unwrap();
	}
}
