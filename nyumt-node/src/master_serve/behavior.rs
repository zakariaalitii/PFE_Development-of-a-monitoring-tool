use std::{io, sync::Arc};
use libp2p::{
    swarm::NetworkBehaviourEventProcess,
    NetworkBehaviour,
    request_response::{ RequestResponse, RequestResponseCodec, ProtocolName, RequestResponseEvent, RequestResponseMessage },
    core::upgrade::{write_length_prefixed, read_length_prefixed},
    ping,
};
use async_trait::async_trait;
use futures::{prelude::*, AsyncWriteExt};

use super::SwarmSettings;
use super::req_resp;

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct NyumtServerProt {
    pub req_resp: RequestResponse<NyumtReqRespCodec>,
    pub ping: ping::Behaviour,

    #[behaviour(ignore)]
    #[allow(dead_code)]
    pub sett: Arc<SwarmSettings>
}

impl NetworkBehaviourEventProcess<ping::PingEvent> for NyumtServerProt {
    fn inject_event(&mut self, event: ping::PingEvent) {
    }
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<NyumtReq, NyumtResp>> for NyumtServerProt {
    fn inject_event(&mut self, message: RequestResponseEvent<NyumtReq, NyumtResp>) {
        match message {
            RequestResponseEvent::Message { peer, message: RequestResponseMessage::Request { request, channel, ..} } => {
                let sett = self.sett.clone();
                tokio::spawn(async move {
                    req_resp::handle(&request.0, peer, sett, channel).await;
                });
            },
            RequestResponseEvent::Message { peer, message: RequestResponseMessage::Response { response, ..} } => {
            },
            _ => ()
        }
    }
}

#[derive(Debug, Clone)]
pub struct NyumtReqRespProt();
#[derive(Clone)]
pub struct NyumtReqRespCodec();
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NyumtReq(pub Vec<u8>);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NyumtResp(pub Vec<u8>);

impl ProtocolName for NyumtReqRespProt {
    fn protocol_name(&self) -> &[u8] {
        "/nyumt".as_bytes()
    }
}

#[async_trait]
impl RequestResponseCodec for NyumtReqRespCodec {
    type Protocol = NyumtReqRespProt;
    type Request = NyumtReq;
    type Response = NyumtResp;

    async fn read_request<T>(&mut self, _: &NyumtReqRespProt, io: &mut T)
        -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 30000).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(NyumtReq(vec))
    }

    async fn read_response<T>(&mut self, _: &NyumtReqRespProt, io: &mut T)
        -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 30000).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(NyumtResp(vec))
    }

    async fn write_request<T>(&mut self, _: &NyumtReqRespProt, io: &mut T, NyumtReq(data): NyumtReq)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(&mut self, _: &NyumtReqRespProt, io: &mut T, NyumtResp(data): NyumtResp)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }
}
