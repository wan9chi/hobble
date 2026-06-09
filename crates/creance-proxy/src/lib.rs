//! CONNECT proxy used by Creance observation and enforcement.

use std::{
    collections::{BTreeSet, HashSet},
    fmt,
    str::FromStr,
    sync::Arc,
};

use tokio::{
    io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("empty request")]
    EmptyRequest,
    #[error("truncated CONNECT request")]
    Truncated,
    #[error("unsupported HTTP method: {0}")]
    InvalidMethod(String),
    #[error("malformed request line")]
    MalformedRequestLine,
    #[error("missing CONNECT port")]
    MissingPort,
    #[error("invalid CONNECT port: {0}")]
    InvalidPort(String),
    #[error("invalid CONNECT authority: {0}")]
    InvalidAuthority(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostPort {
    host: String,
    port: u16,
}

impl HostPort {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into().to_ascii_lowercase(),
            port,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HostPattern {
    Exact(String),
    WildcardSuffix(String),
}

impl HostPattern {
    pub fn exact(host: impl Into<String>) -> Self {
        Self::Exact(host.into().to_ascii_lowercase())
    }

    pub fn wildcard_suffix(suffix: impl Into<String>) -> Self {
        let suffix = suffix.into();
        Self::WildcardSuffix(suffix.trim_start_matches("*.").to_ascii_lowercase())
    }

    pub fn matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        match self {
            Self::Exact(exact) => host == *exact,
            Self::WildcardSuffix(suffix) => {
                host.len() > suffix.len()
                    && host.ends_with(suffix)
                    && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
            }
        }
    }
}

impl FromStr for HostPattern {
    type Err = ProxyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ProxyError::InvalidAuthority(value.to_string()));
        }

        if let Some(suffix) = value.strip_prefix("*.") {
            Ok(Self::wildcard_suffix(suffix))
        } else {
            Ok(Self::exact(value))
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Recorder {
    hosts: Arc<Mutex<BTreeSet<HostPort>>>,
}

impl Recorder {
    pub async fn record(&self, host: HostPort) {
        self.hosts.lock().await.insert(host);
    }

    pub async fn snapshot(&self) -> Vec<HostPort> {
        self.hosts.lock().await.iter().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub enum Policy {
    Enforce(HashSet<HostPattern>),
    ObserveLog(Recorder),
}

impl Policy {
    async fn authorize(&self, host: &HostPort) -> bool {
        match self {
            Self::Enforce(allowed) => allowed.iter().any(|pattern| pattern.matches(host.host())),
            Self::ObserveLog(recorder) => {
                recorder.record(host.clone()).await;
                true
            }
        }
    }
}

#[derive(Debug)]
pub struct Proxy {
    listener: TcpListener,
    local_addr: std::net::SocketAddr,
    policy: Arc<Policy>,
}

impl Proxy {
    pub async fn bind(
        addr: impl ToSocketAddrs,
    ) -> Result<(Self, std::net::SocketAddr), ProxyError> {
        Self::bind_with_policy(addr, Policy::ObserveLog(Recorder::default())).await
    }

    pub async fn bind_with_policy(
        addr: impl ToSocketAddrs,
        policy: Policy,
    ) -> Result<(Self, std::net::SocketAddr), ProxyError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        Ok((
            Self {
                listener,
                local_addr,
                policy: Arc::new(policy),
            },
            local_addr,
        ))
    }

    pub fn proxy_env(&self) -> Vec<(String, String)> {
        let value = format!("http://{}", self.local_addr);
        ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
            .into_iter()
            .map(|key| (key.to_string(), value.clone()))
            .collect()
    }

    pub async fn run(self, cancellation: CancellationToken) -> Result<(), ProxyError> {
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                accepted = self.listener.accept() => {
                    let (client, peer) = accepted?;
                    let policy = Arc::clone(&self.policy);
                    tokio::spawn(async move {
                        if let Err(error) = handle_client(client, policy).await {
                            tracing::debug!(%peer, %error, "proxy client closed with error");
                        }
                    });
                }
            }
        }
    }
}

pub async fn parse_connect<R>(reader: &mut R) -> Result<HostPort, ProxyError>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Err(ProxyError::EmptyRequest);
    }

    let request_line = line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(ProxyError::MalformedRequestLine)?;
    let authority = parts.next().ok_or(ProxyError::MalformedRequestLine)?;
    let version = parts.next().ok_or(ProxyError::MalformedRequestLine)?;
    if parts.next().is_some() || !version.starts_with("HTTP/") {
        return Err(ProxyError::MalformedRequestLine);
    }
    if method != "CONNECT" {
        return Err(ProxyError::InvalidMethod(method.to_string()));
    }

    let host_port = parse_authority(authority)?;

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Err(ProxyError::Truncated);
        }
        if line == "\r\n" || line == "\n" {
            return Ok(host_port);
        }
    }
}

fn parse_authority(authority: &str) -> Result<HostPort, ProxyError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return Err(ProxyError::MissingPort);
        };
        return parse_host_port(host, port);
    }

    let Some((host, port)) = authority.rsplit_once(':') else {
        return Err(ProxyError::MissingPort);
    };
    if host.contains(':') {
        return Err(ProxyError::InvalidAuthority(authority.to_string()));
    }
    parse_host_port(host, port)
}

fn parse_host_port(host: &str, port: &str) -> Result<HostPort, ProxyError> {
    if host.is_empty() {
        return Err(ProxyError::InvalidAuthority(host.to_string()));
    }

    let port = port
        .parse::<u16>()
        .map_err(|_| ProxyError::InvalidPort(port.to_string()))?;
    Ok(HostPort::new(host, port))
}

async fn handle_client(client: TcpStream, policy: Arc<Policy>) -> Result<(), ProxyError> {
    let mut reader = BufReader::new(client);
    let host_port = match parse_connect(&mut reader).await {
        Ok(host_port) => host_port,
        Err(ProxyError::InvalidMethod(method)) => {
            let mut client = reader.into_inner();
            tracing::debug!(%method, "rejecting non-CONNECT request");
            client
                .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n")
                .await?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    let buffered = reader.buffer().to_vec();
    let mut client = reader.into_inner();

    if !policy.authorize(&host_port).await {
        tracing::debug!(%host_port, "CONNECT denied by policy");
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }

    let target_addr = host_port.to_string();
    let mut target = match TcpStream::connect(&target_addr).await {
        Ok(target) => target,
        Err(error) => {
            tracing::debug!(%host_port, %error, "CONNECT target dial failed");
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await?;
            return Ok(());
        }
    };

    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    if !buffered.is_empty() {
        target.write_all(&buffered).await?;
    }

    let (mut client_read, mut client_write) = client.into_split();
    let (mut target_read, mut target_write) = target.into_split();

    let client_to_target = async {
        let copied = io::copy(&mut client_read, &mut target_write).await;
        let shutdown = target_write.shutdown().await;
        copied.and(shutdown)
    };
    let target_to_client = async {
        let copied = io::copy(&mut target_read, &mut client_write).await;
        let shutdown = client_write.shutdown().await;
        copied.and(shutdown)
    };

    let _ = tokio::try_join!(client_to_target, target_to_client)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, BufReader},
        net::{TcpListener, TcpStream},
    };
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test]
    async fn parse_connect_accepts_host_and_ipv6() {
        let mut reader =
            BufReader::new(&b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n"[..]);
        assert_eq!(
            parse_connect(&mut reader).await.unwrap(),
            HostPort::new("example.com", 443)
        );

        let mut reader = BufReader::new(&b"CONNECT [::1]:8443 HTTP/1.1\r\n\r\n"[..]);
        assert_eq!(
            parse_connect(&mut reader).await.unwrap(),
            HostPort::new("::1", 8443)
        );
    }

    #[tokio::test]
    async fn parse_connect_rejects_bad_requests() {
        let cases = [
            (
                b"CONNECT example.com HTTP/1.1\r\n\r\n".as_slice(),
                "missing CONNECT port",
            ),
            (
                b"GET http://example.com/ HTTP/1.1\r\n\r\n".as_slice(),
                "unsupported HTTP method: GET",
            ),
            (
                b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n".as_slice(),
                "truncated CONNECT request",
            ),
        ];

        for (input, expected) in cases {
            let mut reader = BufReader::new(input);
            let error = parse_connect(&mut reader).await.unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
    }

    #[tokio::test]
    async fn connect_tunnels_bytes_and_rejects_cleartext() {
        let target = spawn_echo_server().await;
        let (proxy, proxy_addr) = Proxy::bind("127.0.0.1:0").await.unwrap();
        let cancellation = CancellationToken::new();
        let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(
                format!(
                    "CONNECT localhost:{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
                    target.port()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let response = read_headers(&mut client).await;
        assert_eq!(response, "HTTP/1.1 200 Connection Established\r\n\r\n");

        client.write_all(b"ping").await.unwrap();
        let mut echoed = [0; 4];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"GET http://x/ HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let response = read_headers(&mut client).await;
        assert!(response.starts_with("HTTP/1.1 405 Method Not Allowed"));

        cancellation.cancel();
        proxy_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn enforce_policy_denies_before_dial_and_observe_records_once() {
        let target = spawn_echo_server().await;
        let allowed = HashSet::from([HostPattern::exact("localhost")]);
        let (proxy, proxy_addr) = Proxy::bind_with_policy("127.0.0.1:0", Policy::Enforce(allowed))
            .await
            .unwrap();
        let cancellation = CancellationToken::new();
        let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(format!("CONNECT localhost:{} HTTP/1.1\r\n\r\n", target.port()).as_bytes())
            .await
            .unwrap();
        assert_eq!(
            read_headers(&mut client).await,
            "HTTP/1.1 200 Connection Established\r\n\r\n"
        );

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"CONNECT denied.test:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let response = read_headers(&mut client).await;
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));

        cancellation.cancel();
        proxy_task.await.unwrap().unwrap();

        let recorder = Recorder::default();
        let (proxy, proxy_addr) =
            Proxy::bind_with_policy("127.0.0.1:0", Policy::ObserveLog(recorder.clone()))
                .await
                .unwrap();
        let cancellation = CancellationToken::new();
        let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));

        for host in ["z.test", "a.test", "z.test"] {
            let mut client = TcpStream::connect(proxy_addr).await.unwrap();
            client
                .write_all(format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let _ = read_headers(&mut client).await;
        }

        assert_eq!(
            recorder.snapshot().await,
            vec![HostPort::new("a.test", 443), HostPort::new("z.test", 443)]
        );

        cancellation.cancel();
        proxy_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn proxy_env_values_parse_and_work() {
        let target = spawn_echo_server().await;
        let (proxy, proxy_addr) = Proxy::bind("127.0.0.1:0").await.unwrap();
        assert_eq!(proxy.proxy_env().len(), 4);
        for (key, value) in proxy.proxy_env() {
            assert!(
                ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"].contains(&key.as_str())
            );
            assert_eq!(value, format!("http://{proxy_addr}"));
        }

        let cancellation = CancellationToken::new();
        let proxy_env = proxy.proxy_env();
        let proxy_task = tokio::spawn(proxy.run(cancellation.clone()));
        let proxy_url = proxy_env
            .iter()
            .find_map(|(key, value)| (key == "HTTPS_PROXY").then_some(value))
            .unwrap();
        let proxy_addr = proxy_url.strip_prefix("http://").unwrap();

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(format!("CONNECT localhost:{} HTTP/1.1\r\n\r\n", target.port()).as_bytes())
            .await
            .unwrap();
        assert_eq!(
            read_headers(&mut client).await,
            "HTTP/1.1 200 Connection Established\r\n\r\n"
        );

        cancellation.cancel();
        proxy_task.await.unwrap().unwrap();
    }

    async fn spawn_echo_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let (mut reader, mut writer) = socket.split();
                    let _ = io::copy(&mut reader, &mut writer).await;
                });
            }
        });
        addr
    }

    async fn read_headers(stream: &mut TcpStream) -> String {
        let mut response = Vec::new();
        let deadline = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut byte = [0; 1];
                stream.read_exact(&mut byte).await.unwrap();
                response.push(byte[0]);
                if response.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        });
        deadline.await.unwrap();
        String::from_utf8(response).unwrap()
    }
}
