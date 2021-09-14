macro_rules! extract_session_and_start {
    (
        $cmd:expr, 
        $session_ty:expr, 
        $session_file:expr, 
        $session_socket:expr, 
        $timeout:expr, 
        $start:expr
    ) => {{
        use distant_core::{PlainCodec, SessionInfo, SessionInfoFile, XChaCha20Poly1305Codec};
        match $session_ty {
            SessionInput::Environment => {
                let info = SessionInfo::from_environment()?;
                let addr = info.to_socket_addr().await?;
                let codec = XChaCha20Poly1305Codec::from(info.key);
                $start(
                    $cmd,
                    Session::tcp_connect_timeout(addr, codec, $timeout).await?,
                    $timeout,
                    None,
                )
                .await
            }
            SessionInput::File => {
                let info: SessionInfo = SessionInfoFile::load_from($session_file).await?.into();
                let addr = info.to_socket_addr().await?;
                let codec = XChaCha20Poly1305Codec::from(info.key);
                $start(
                    $cmd,
                    Session::tcp_connect_timeout(addr, codec, $timeout).await?,
                    $timeout,
                    None,
                )
                .await
            }
            SessionInput::Pipe => {
                let info = SessionInfo::from_stdin()?;
                let addr = info.to_socket_addr().await?;
                let codec = XChaCha20Poly1305Codec::from(info.key);
                $start(
                    $cmd,
                    Session::tcp_connect_timeout(addr, codec, $timeout).await?,
                    $timeout,
                    None,
                )
                .await
            }
            SessionInput::Lsp => {
                let mut data = LspData::from_buf_reader(&mut std::io::stdin().lock())
                    .map_err(io::Error::from)?;
                let info = data.take_session_info().map_err(io::Error::from)?;
                let addr = info.to_socket_addr().await?;
                let codec = XChaCha20Poly1305Codec::from(info.key);
                $start(
                    $cmd,
                    Session::tcp_connect_timeout(addr, codec, $timeout).await?,
                    $timeout,
                    Some(data),
                )
                .await
            }
            #[cfg(unix)]
            SessionInput::Socket => {
                $start(
                    $cmd,
                    Session::unix_connect_timeout($session_socket, PlainCodec::new(), $timeout)
                        .await?,
                    $timeout,
                    None,
                )
                .await
            }
        }
    }};
}

pub mod action;
pub mod launch;
pub mod listen;
pub mod lsp;
