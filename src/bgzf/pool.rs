use crossbeam::queue::SegQueue;
use std::sync::Arc;

#[derive(Clone)]
pub struct BgzfBlockPool
{
	pool: Arc<SegQueue<Vec<u8>>>,
	block_size: usize,
}

impl BgzfBlockPool
{
	pub fn new(pool_size: usize, block_size: usize) -> Self
	{
		let pool = SegQueue::new();
		for _ in 0..pool_size
		{
			pool.push(vec![0; block_size]);
		}

		Self {
			pool: Arc::new(pool),
			block_size,
		}
	}

	pub fn acquire(&self) -> Vec<u8>
	{
		self.pool.pop().unwrap_or_else(|| vec![0; self.block_size])
	}

	pub fn release(&self, mut buffer: Vec<u8>)
	{
		buffer.clear(); // prepare for reuse
		self.pool.push(buffer);
	}
}
