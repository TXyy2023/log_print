//! Versioned messages and real client/server transports. Core data is memory-only.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{mpsc, oneshot},
    task::AbortHandle,
};

pub const PROTOCOL: &str = "log-print/2";
pub const MAX_WIRE: usize = 1024 * 1024;
pub const MAX_DATAGRAM: usize = 60 * 1024;
pub const MAX_PAYLOAD: usize = 64 * 1024;
pub const EVENT_QUEUE: usize = 64;
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    #[default]
    Tcp,
    Udp,
}
impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        })
    }
}
impl std::str::FromStr for TransportKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            _ => bail!("transport must be tcp or udp"),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Input,
    Output,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: String,
    pub plugin: String,
    pub token: String,
    #[serde(default)]
    pub events: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub op: String,
    #[serde(default)]
    pub args: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response {
        id: u64,
        result: Value,
        error: Option<Fault>,
    },
    Record {
        record: Record,
    },
    Gap {
        stream: String,
        epoch: String,
        from: u64,
        to: u64,
        reason: String,
    },
    Control {
        call_id: u64,
        method: String,
        args: Value,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub stream: String,
    pub epoch: String,
    pub seq: u64,
    pub key: String,
    pub payload: Vec<u8>,
    pub source_ts_ns: Option<u64>,
    pub observed_ts_ns: u64,
    #[serde(default)]
    pub upstream: BTreeMap<String, u64>,
    #[serde(default)]
    pub upstream_epochs: BTreeMap<String, String>,
    #[serde(default)]
    pub source_seq: Option<u64>,
    #[serde(default)]
    pub channel: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamSpec {
    /// Configuration alias, never the runtime stream identity.
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parents: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSpec {
    pub id: String,
    pub role: Role,
    pub bin: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "yes")]
    pub autostart: bool,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub streams: Vec<StreamSpec>,
    #[serde(default)]
    pub config: Value,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreOptions {
    pub transport: TransportKind,
    pub buffer_bytes: usize,
    pub buffer_records: usize,
    pub max_payload_bytes: usize,
    pub read_batch_records: usize,
    pub queue_records: usize,
}
impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            transport: TransportKind::Tcp,
            buffer_bytes: 4 * 1024 * 1024,
            buffer_records: 4096,
            max_payload_bytes: MAX_PAYLOAD,
            read_batch_records: 64,
            queue_records: EVENT_QUEUE,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub core: CoreOptions,
    #[serde(default)]
    pub plugins: Vec<PluginSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub config: Config,
    pub admin_token: String,
    pub plugin_tokens: BTreeMap<String, String>,
}
pub fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}
/// Incrementally rejects oversized frames, including peers that never send newline.
pub async fn read_json<R: AsyncBufRead + Unpin, T: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> Result<Option<T>> {
    let mut frame = Vec::new();
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            if frame.is_empty() {
                return Ok(None);
            }
            bail!("truncated_frame")
        };
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(chunk.len());
        if frame.len() + n > MAX_WIRE {
            bail!("frame_too_large")
        };
        let done = chunk[n - 1] == b'\n';
        frame.extend_from_slice(&chunk[..n]);
        r.consume(n);
        if done {
            return Ok(Some(serde_json::from_slice(&frame)?));
        }
    }
}
pub async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, msg: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(msg)?;
    if bytes.len() + 1 > MAX_WIRE {
        bail!("frame_too_large")
    };
    bytes.push(b'\n');
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}
fn datagram<T: Serialize>(msg: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(msg)?;
    if bytes.len() > MAX_DATAGRAM {
        bail!("datagram_too_large: maximum encoded UDP message is {MAX_DATAGRAM} bytes")
    };
    Ok(bytes)
}
fn address(address: &str) -> Result<SocketAddr> {
    let addr: SocketAddr = address
        .parse()
        .context("address must be a loopback socket address")?;
    if !addr.ip().is_loopback() {
        bail!("address must be loopback")
    };
    Ok(addr)
}
pub enum ClientReader {
    Tcp(BufReader<tokio::net::tcp::OwnedReadHalf>),
    Udp(Arc<UdpSocket>),
}
pub enum ClientWriter {
    Tcp(tokio::net::tcp::OwnedWriteHalf),
    Udp(Arc<UdpSocket>),
}
pub struct ClientConnection;
impl ClientConnection {
    /// Sends one registration datagram/frame. Caller checks the welcome reply; no retries.
    pub async fn connect(
        addr: &str,
        transport: TransportKind,
        hello: &Hello,
    ) -> Result<(ClientReader, ClientWriter)> {
        let addr = address(addr)?;
        match transport {
            TransportKind::Tcp => {
                let socket =
                    tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
                        .await??;
                socket.set_nodelay(true)?;
                let (r, mut w) = socket.into_split();
                write_json(&mut w, hello).await?;
                Ok((ClientReader::Tcp(BufReader::new(r)), ClientWriter::Tcp(w)))
            }
            TransportKind::Udp => {
                let socket = Arc::new(
                    UdpSocket::bind(if addr.is_ipv4() {
                        "127.0.0.1:0"
                    } else {
                        "[::1]:0"
                    })
                    .await?,
                );
                socket.connect(addr).await?;
                socket.send(&datagram(hello)?).await?;
                Ok((ClientReader::Udp(socket.clone()), ClientWriter::Udp(socket)))
            }
        }
    }
}
impl ClientReader {
    pub async fn receive(&mut self) -> Result<Option<ServerMessage>> {
        match self {
            Self::Tcp(r) => read_json(r).await,
            Self::Udp(s) => {
                let mut bytes = vec![0; MAX_DATAGRAM + 1];
                let n = s.recv(&mut bytes).await?;
                if n > MAX_DATAGRAM {
                    bail!("datagram_too_large")
                };
                Ok(Some(serde_json::from_slice(&bytes[..n])?))
            }
        }
    }
}
impl ClientWriter {
    /// Explicit best-effort close for a UDP session. No delivery or retry guarantee.
    pub fn disconnect(&self) {
        if let Self::Udp(socket) = self {
            let _ = socket.try_send(b"{\"id\":0,\"op\":\"disconnect\",\"args\":null}");
        }
    }
    /// UDP success means only that the local socket accepted the datagram.
    pub async fn send(&mut self, request: &Request) -> Result<()> {
        match self {
            Self::Tcp(w) => write_json(w, request).await,
            Self::Udp(s) => {
                let bytes = datagram(request)?;
                let n = s.send(&bytes).await?;
                if n != bytes.len() {
                    bail!("partial_datagram")
                };
                Ok(())
            }
        }
    }
}
type Outgoing = (ServerMessage, oneshot::Sender<Result<()>>);
/// Decoded endpoint used by Core. Neither TCP nor UDP leaks into its business loop.
pub struct ServerConnection {
    pub hello: Hello,
    pub transport: TransportKind,
    incoming: mpsc::Receiver<Request>,
    outgoing: mpsc::Sender<Outgoing>,
    tasks: Vec<AbortHandle>,
}
impl Drop for ServerConnection {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort()
        }
    }
}
impl ServerConnection {
    /// Poll control messages even while an event stream continuously has records.
    pub fn try_receive(&mut self) -> Result<Option<Request>> {
        match self.incoming.try_recv() {
            Ok(request) => Ok(Some(request)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => bail!("transport closed"),
        }
    }
    pub async fn receive(&mut self) -> Option<Request> {
        self.incoming.recv().await
    }
    pub async fn send(&self, message: ServerMessage) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.outgoing
            .send((message, tx))
            .await
            .map_err(|_| anyhow::anyhow!("transport closed"))?;
        rx.await.context("transport closed")?
    }
    /// Adapter retained for callers with an already accepted TCP socket.
    pub async fn tcp(socket: TcpStream) -> Result<Self> {
        socket.set_nodelay(true)?;
        let (r, mut w) = socket.into_split();
        let mut r = BufReader::new(r);
        let hello = tokio::time::timeout(Duration::from_secs(5), read_json::<_, Hello>(&mut r))
            .await??
            .context("missing hello")?;
        let (itx, irx) = mpsc::channel(EVENT_QUEUE);
        let (otx, mut orx) = mpsc::channel::<Outgoing>(EVENT_QUEUE);
        let reader = tokio::spawn(async move {
            while let Ok(Some(request)) = read_json(&mut r).await {
                if itx.send(request).await.is_err() {
                    break;
                }
            }
        });
        let writer = tokio::spawn(async move {
            while let Some((message, ack)) = orx.recv().await {
                let result = write_json(&mut w, &message).await;
                let failed = result.is_err();
                let _ = ack.send(result);
                if failed {
                    break;
                }
            }
        });
        Ok(Self {
            hello,
            transport: TransportKind::Tcp,
            incoming: irx,
            outgoing: otx,
            tasks: vec![reader.abort_handle(), writer.abort_handle()],
        })
    }
}
pub struct ServerListener {
    address: SocketAddr,
    incoming: mpsc::Receiver<Result<ServerConnection>>,
    task: AbortHandle,
}
impl Drop for ServerListener {
    fn drop(&mut self) {
        self.task.abort()
    }
}
impl ServerListener {
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
    pub async fn accept(&mut self) -> Result<ServerConnection> {
        self.incoming.recv().await.context("listener closed")?
    }
    pub async fn bind(addr: &str, transport: TransportKind) -> Result<Self> {
        let addr = address(addr)?;
        let (tx, rx) = mpsc::channel(64);
        match transport {
            TransportKind::Tcp => {
                let listener = TcpListener::bind(addr).await?;
                let address = listener.local_addr()?;
                let task = tokio::spawn(async move {
                    let handshakes = Arc::new(tokio::sync::Semaphore::new(128));
                    while let Ok((socket, _)) = listener.accept().await {
                        let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                            continue;
                        };
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let result = ServerConnection::tcp(socket).await;
                            let _ = tx.send(result).await;
                        });
                    }
                });
                Ok(Self {
                    address,
                    incoming: rx,
                    task: task.abort_handle(),
                })
            }
            TransportKind::Udp => {
                let socket = Arc::new(UdpSocket::bind(addr).await?);
                let address = socket.local_addr()?;
                let task = tokio::spawn(async move {
                    let mut sessions = BTreeMap::<SocketAddr, mpsc::Sender<Request>>::new();
                    let mut bytes = vec![0; MAX_DATAGRAM + 1];
                    while let Ok((n, peer)) = socket.recv_from(&mut bytes).await {
                        if n > MAX_DATAGRAM || !peer.ip().is_loopback() {
                            continue;
                        };
                        sessions.retain(|_, s| !s.is_closed());
                        if let Some(session) = sessions.get(&peer) {
                            if let Ok(request) = serde_json::from_slice::<Request>(&bytes[..n]) {
                                let _ = session.try_send(request);
                            };
                            continue;
                        };
                        let Ok(hello) = serde_json::from_slice::<Hello>(&bytes[..n]) else {
                            continue;
                        };
                        if sessions.len() >= 1024 {
                            continue;
                        };
                        let (itx, irx) = mpsc::channel(EVENT_QUEUE);
                        let (otx, mut orx) = mpsc::channel::<Outgoing>(EVENT_QUEUE);
                        let outgoing_socket = socket.clone();
                        let writer = tokio::spawn(async move {
                            while let Some((message, ack)) = orx.recv().await {
                                let result = async {
                                    let bytes = datagram(&message)?;
                                    outgoing_socket.send_to(&bytes, peer).await?;
                                    Ok(())
                                }
                                .await;
                                let _ = ack.send(result);
                            }
                        });
                        let conn = ServerConnection {
                            hello,
                            transport: TransportKind::Udp,
                            incoming: irx,
                            outgoing: otx,
                            tasks: vec![writer.abort_handle()],
                        };
                        if tx.try_send(Ok(conn)).is_ok() {
                            sessions.insert(peer, itx);
                        }
                    }
                });
                Ok(Self {
                    address,
                    incoming: rx,
                    task: task.abort_handle(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bounded_frames_reject_oversize_and_truncation_without_waiting_for_newline() {
        let bytes = vec![b'a'; MAX_WIRE + 1];
        let mut r = BufReader::new(bytes.as_slice());
        assert!(read_json::<_, Value>(&mut r)
            .await
            .unwrap_err()
            .to_string()
            .contains("frame_too_large"));
        let mut r = BufReader::new(&b"{\"id\":1}"[..]);
        assert!(read_json::<_, Value>(&mut r)
            .await
            .unwrap_err()
            .to_string()
            .contains("truncated_frame"));
    }
    #[tokio::test]
    async fn json_framing_handles_multiple_frames_and_eof() {
        let mut r = BufReader::new(&b"{\"n\":1}\n{\"n\":2}\n"[..]);
        assert_eq!(
            read_json::<_, Value>(&mut r).await.unwrap().unwrap()["n"],
            1
        );
        assert_eq!(
            read_json::<_, Value>(&mut r).await.unwrap().unwrap()["n"],
            2
        );
        assert!(read_json::<_, Value>(&mut r).await.unwrap().is_none());
        let oversized = "x".repeat(MAX_WIRE);
        assert!(write_json(&mut tokio::io::sink(), &oversized)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn udp_send_rejects_encoded_oversize_locally() {
        let listener = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let hello = Hello {
            protocol: PROTOCOL.into(),
            plugin: "test".into(),
            token: "token".into(),
            events: false,
        };
        let (_r, mut w) = ClientConnection::connect(
            &listener.local_addr().unwrap().to_string(),
            TransportKind::Udp,
            &hello,
        )
        .await
        .unwrap();
        let request = Request {
            id: 0,
            op: "publish".into(),
            args: serde_json::json!({"payload":vec![255;MAX_DATAGRAM]}),
        };
        assert!(w
            .send(&request)
            .await
            .unwrap_err()
            .to_string()
            .contains("datagram_too_large"));
    }
    #[test]
    fn transport_is_tcp_by_default_and_config_only_accepts_two_roles() {
        assert_eq!(CoreOptions::default().transport, TransportKind::Tcp);
        assert_eq!("udp".parse::<TransportKind>().unwrap(), TransportKind::Udp);
        assert!("quic".parse::<TransportKind>().is_err());
        assert!(serde_json::from_str::<Role>("\"transform\"").is_err());
        assert_eq!(
            serde_json::from_str::<Role>("\"output\"").unwrap(),
            Role::Output
        );
    }
    #[tokio::test]
    async fn client_and_server_reject_non_loopback_addresses() {
        assert!(ServerListener::bind("0.0.0.0:0", TransportKind::Tcp)
            .await
            .is_err());
        let hello = Hello {
            protocol: PROTOCOL.into(),
            plugin: "test".into(),
            token: "token".into(),
            events: false,
        };
        assert!(
            ClientConnection::connect("192.0.2.1:1234", TransportKind::Udp, &hello)
                .await
                .is_err()
        );
    }
}
