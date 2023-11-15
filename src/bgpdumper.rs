// Based on the "bgpdumper" example of zettabgp by Vladimir
// Melnikov, which is licensed under the MIT license.

use bytes::{Buf, BytesMut};
use futures_util::Stream;
use futures_util::StreamExt;
use log::*;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tokio_util::codec::FramedRead;
use tokio_util::codec::LengthDelimitedCodec;
use zettabgp::prelude::*;

pub struct BgpDumper {
    pub params: BgpSessionParams,
    pub read: FramedRead<OwnedReadHalf, LengthDelimitedCodec>,
    pub write: Arc<Mutex<OwnedWriteHalf>>,
    pub stop_keepalives: Option<oneshot::Sender<()>>,
}

impl BgpDumper {
    pub fn new(bgp_params: BgpSessionParams, tcpstream: TcpStream) -> BgpDumper {
        let (read, write) = tcpstream.into_split();
        BgpDumper {
            params: bgp_params,
            write: Arc::new(Mutex::new(write)),
            read: LengthDelimitedCodec::builder()
                .length_field_offset(16)
                .length_field_type::<u16>()
                .length_adjustment(0)
                .num_skip(0)
                .max_frame_length(4096)
                .new_read(read),
            stop_keepalives: None,
        }
    }
    pub async fn start_active(&mut self) -> Result<BgpOpenMessage, BgpError> {
        let mut bom = self.params.open_message();
        let mut buf = [255 as u8; 4096];
        let messagelen = match bom.encode_to(&self.params, &mut buf[19..]) {
            Err(e) => {
                return Err(e);
            }
            Ok(sz) => sz,
        };
        let blen = self
            .params
            .prepare_message_buf(&mut buf, BgpMessageType::Open, messagelen)?;
        self.write.lock().await.write_all(&buf[0..blen]).await?;
        let (msgtype, buf) = self.next_message().await?;
        if msgtype != BgpMessageType::Open {
            return Err(BgpError::static_str("Invalid state to start_active"));
        }
        bom.decode_from(&self.params, &buf[..])?;
        debug!("{:?}", bom);
        self.params.hold_time = bom.hold_time;
        self.params.caps = bom.caps.clone();
        self.params.check_caps();
        Ok(bom)
    }
    fn start_keepalives(&self) -> oneshot::Sender<()> {
        let (tx, mut rx) = oneshot::channel();
        let slp = std::time::Duration::new((self.params.hold_time / 3) as u64, 0);
        let write = self.write.clone();
        tokio::task::spawn(async move {
            let mut buf = [255 as u8; 19];
            buf[0..16].clone_from_slice(&[255 as u8; 16]);
            buf[16] = 0;
            buf[17] = 19;
            buf[18] = 4; //keepalive
            loop {
                if write.lock().await.write_all(&buf).await.is_err() {
                    warn!("error sending keepalive");
                }
                tokio::select! {
                    _ = tokio::time::sleep(slp) => {},
                    _ = &mut rx => break,
                }
            }
        });
        tx
    }
    async fn next_message(&mut self) -> Result<(BgpMessageType, BytesMut), BgpError> {
        let mut buf = self
            .read
            .next()
            .await
            .ok_or(BgpError::static_str("unexpected end of stream"))??;
        let msg = self.params.decode_message_head(&buf)?;
        buf.advance(19);
        buf.truncate(msg.1);
        Ok((msg.0, buf))
    }
    pub fn lifecycle(
        mut self,
    ) -> impl Stream<Item = Result<BgpUpdateMessage, Result<BgpNotificationMessage, BgpError>>> + Send
    {
        self.stop_keepalives = Some(self.start_keepalives());

        async_stream::try_stream! {
            loop {
                let (msgtype, buf) = self.next_message().await.map_err(Err)?;
                if msgtype == BgpMessageType::Keepalive {
                    continue;
                }
                match msgtype {
                    BgpMessageType::Open => {
                        Err(Err(BgpError::static_str("Incorrect open message")))?;
                    }
                    BgpMessageType::Keepalive => {}
                    BgpMessageType::Notification => {
                        let mut msgnotification = BgpNotificationMessage::new();
                        msgnotification.decode_from(&self.params, &buf[..]).map_err(Err)?;
                        Err(Ok(msgnotification))?;
                    }
                    BgpMessageType::Update => {
                        let mut msgupdate = BgpUpdateMessage::new();
                        if let Err(e) = msgupdate.decode_from(&self.params, &buf[..]) {
                            warn!("BGP update decode error: {:?}", e);
                            warn!("{:x?}", &buf[..]);
                            continue;
                        }
                        yield msgupdate;
                    }
                }
            }
        }
    }
}
impl Drop for BgpDumper {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_keepalives.take() {
            let _ = tx.send(());
        }
    }
}
