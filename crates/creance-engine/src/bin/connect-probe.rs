use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
};

fn main() -> std::io::Result<()> {
    let mut args = env::args_os();
    let _ = args.next();
    let Some(out) = args.next() else {
        eprintln!("usage: connect-probe <out-file> [allowed-port]");
        std::process::exit(2);
    };

    let proxy = env::var("HTTPS_PROXY")
        .or_else(|_| env::var("https_proxy"))
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HTTPS_PROXY is not set"))?;
    let proxy = proxy
        .strip_prefix("http://")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad proxy URL"))?;

    if let Some(port) = args.next() {
        connect(proxy, &format!("localhost:{}", port.to_string_lossy()), 200)?;
        connect(proxy, "evil.test:443", 403)?;
    } else {
        let _ = connect(proxy, "recorded.test:443", 0);
    }

    std::fs::write(PathBuf::from(out), b"probe\n")
}

fn connect(proxy: &str, authority: &str, expected_status: u16) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(proxy)?;
    write!(stream, "CONNECT {authority} HTTP/1.1\r\n\r\n")?;
    let mut response = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if expected_status != 0 {
        let response = String::from_utf8_lossy(&response);
        let expected = format!("HTTP/1.1 {expected_status} ");
        if !response.starts_with(&expected) {
            return Err(std::io::Error::other(format!(
                "expected {expected_status}, got {response:?}"
            )));
        }
    }
    Ok(())
}
