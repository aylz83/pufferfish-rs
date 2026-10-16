use tokio::io::{AsyncSeekExt, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Mutex;
use std::sync::Arc;

use libdeflater::Decompressor;

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use futures::future::BoxFuture;

use crate::bgzf::BgzfBlockPool;
use crate::bgzf::{BgzfCompressedBlock, BGZ, EofCheck};
use crate::error;

#[allow(dead_code)]
#[derive(Clone)]
pub struct BgzfBlockStream<R>
{
	reader: Arc<Mutex<BufReader<R>>>,
	pool: Arc<BgzfBlockPool>,
	// index: u64,
	eof_check: Option<EofCheck>,
}

impl<R> BgzfBlockStream<R>
where
	R: AsyncRead + AsyncSeekExt + std::marker::Send + std::marker::Unpin,
{
	pub fn new(reader: R, pool: Arc<BgzfBlockPool>, eof_check: Option<EofCheck>) -> Self
	{
		Self {
			reader: Arc::new(Mutex::new(BufReader::new(reader))),
			pool,
			// index: 0,
			eof_check,
		}
	}
}

impl<R> BgzfBlockStream<R>
where
	R: AsyncRead + AsyncSeekExt + Send + Unpin,
{
	pub async fn reset(&self) -> error::Result<()>
	{
		let mut reader = self.reader.lock().await;

		reader.seek(std::io::SeekFrom::Start(0)).await?;

		let buffered_len = reader.buffer().len();
		reader.consume(buffered_len);

		Ok(())
	}

	pub async fn seek_to(&self, seek_position: std::io::SeekFrom) -> error::Result<u64>
	{
		let mut reader = self.reader.lock().await;

		let final_position = reader.seek(seek_position).await?;

		let buffered_len = reader.buffer().len();
		reader.consume(buffered_len);

		Ok(final_position)
	}
}

impl<R> Stream for BgzfBlockStream<R>
where
	R: AsyncRead + AsyncSeekExt + std::marker::Send + std::marker::Unpin + 'static,
{
	type Item = BoxFuture<'static, error::Result<Option<BgzfCompressedBlock>>>;

	fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>>
	{
		let reader = self.reader.clone();
		let pool = self.pool.clone();
		let eof_check = self.eof_check;
		// let idx = self.index;

		// self.index += 1;

		let fut = async move {
			let mut locked = reader.lock().await;
			locked.read_bgzf_block_pool(pool, eof_check).await
		};

		Poll::Ready(Some(Box::pin(async move {
			match fut.await?
			{
				Some(block) => Ok(Some(block)),
				None => Ok(None),
			}
		})))
	}
}

pub fn decompress_bgzf_block(compressed_block: &[u8]) -> crate::error::Result<Vec<u8>>
{
	if compressed_block.len() < 18
	{
		return Err(crate::error::Error::Decompress);
	}

	let isize_bytes: [u8; 4] = compressed_block[compressed_block.len() - 4..]
		.try_into()
		.map_err(|_| crate::error::Error::Decompress)?;
	let uncompressed_size = u32::from_le_bytes(isize_bytes) as usize;

	let mut bytes = vec![0u8; uncompressed_size];
	let mut decompressor = Decompressor::new();
	decompressor
		.gzip_decompress(compressed_block, &mut bytes)
		.map_err(|_| crate::error::Error::Decompress)?;

	Ok(bytes)
}
