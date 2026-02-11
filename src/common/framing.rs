use derive_more::{Constructor, From};
use smol::io::{self, AsyncRead, AsyncReadExt, AsyncWrite};

type PrefixType = u16;
const PREFIX_LEN: usize = (PrefixType::BITS / 8) as usize;
pub const MAX_FRAME_SIZE: usize = PrefixType::MAX as usize;

#[derive(Constructor, From)]
pub struct FramedStream<S: Unpin>(S);

impl<S: AsyncWrite + Unpin> FramedStream<S> {
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        let len = buf.len();
        assert!(len <= MAX_FRAME_SIZE);
        let prefix = (len as PrefixType).to_be_bytes();
        let chain = prefix.chain(buf);
        io::copy(chain, &mut self.0).await?;
        Ok(())
    }
}

impl<S: AsyncRead + Unpin> FramedStream<S> {
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        let mut prefix_buf = [0; PREFIX_LEN];
        self.0.read_exact(&mut prefix_buf).await?;
        let len = PrefixType::from_be_bytes(prefix_buf);
        let mut buf = vec![0; len as usize];
        self.0.read_exact(&mut buf).await?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use smol::block_on;

    use super::*;

    #[test]
    fn framed_stream_writes() {
        let mut buf = Vec::<u8>::new();
        {
            let mut writer = FramedStream(&mut buf);
            assert!(block_on(writer.write(&(0..10).collect::<Vec<u8>>())).is_ok());
        }
        let mut buf2 = vec![0, 10];
        buf2.extend(0..10);
        assert_eq!(buf, buf2);
    }

    #[test]
    fn framed_stream_reads() {
        let mut buf: Vec<u8> = vec![0, 10];
        buf.extend(0..10);
        let read_buf = {
            let mut reader = FramedStream(buf.as_slice());
            block_on(reader.read()).unwrap()
        };
        assert_eq!(read_buf, (0..10).collect::<Vec<u8>>());
    }
}
