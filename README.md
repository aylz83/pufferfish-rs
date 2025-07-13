# pufferfish-rs

A small Rust crate to read and uncompress blocks from BGZip data (BAM for example) in memory or from a file.

## Example usage -

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
```
