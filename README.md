# pufferfish-rs

A small Rust crate to read and uncompress blocks from BGZip data (BAM for example) in memory or from a file.

```{rust}
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use pufferfish::BGZ;

let gzip_file = File::open("my_file.bed.gz").await?;
let mut reader = BufReader::new(gzip_file);

loop
{
  match reader
  // read bgzip blocks and decompresses, while looking for a bgzip EOF marker
	.read_bgzf_block(Some(pufferfish::is_bgzip_eof))
	.await
	{
		Ok(Some(bytes)) =>
    {
      // do something with the bytes, such as parsing the bed bytes with the nom crate
    },
    // EOF received, break out of the loop
		Ok(None) => break,
    // Some form of error reading the BGZIP block, such as corruption
		Err(err) => return Err(err.into()),
	}
}

match reader.read_bgzf_block(Some(pufferfish::is_bgz_eof_block)).await
		{
			Ok(_) =>
			{
				if !Reader::is_valid_bgzf_header(&header)
				{
					debug!("Invalid BGZF header");
					bail!("Invalid BGZF header"); // Skip to the next block if header is invalid.
				}

				// Calculate the size of the compressed block using BSIZE.
				let bsize = u16::from_le_bytes([header[16], header[17]]) as usize + 1;

				debug!("Header block size {} with contents: {:?}", bsize, header);

				// Read the rest of the BGZF block (bsize - 18 bytes).
				let mut compressed_block = vec![0; bsize];
				compressed_block[..18].copy_from_slice(&header);
				reader.read_exact(&mut compressed_block[18..]).await?;

				if bsize == 28
				{
					debug!("EOF header = {:?}", compressed_block);
					return Ok(None);
				}

				let decompressed_block = Reader::decompress_block(&compressed_block).await?;
				Ok(Some(decompressed_block))
			}
			Err(e) =>
			{
				bail!("Failed to read BGZF header: {:?}", e);
			}
		}
}

```
