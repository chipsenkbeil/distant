use crate::{
    data::{Operation, Response},
    net::Transport,
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use log::*;
use orion::aead::SecretKey;
use std::{string::FromUtf8Error, sync::Arc};
use tokio::{io, net::TcpListener};

pub type Result = std::result::Result<(), Error>;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ConvertToIpAddrError(ConvertToIpAddrError),
    ForkError,
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub fn run(cmd: ListenSubcommand, opt: CommonOpt) -> Result {
    if cmd.daemon {
        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { run_async(cmd, opt, true).await })?;
            }
            Ok(Fork::Parent(pid)) => {
                info!("[distant detached, pid = {}]", pid);
                if let Err(_) = fork::close_fd() {
                    return Err(Error::ForkError);
                }
            }
            Err(_) => return Err(Error::ForkError),
        }
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, opt, false).await })?;
    }

    Ok(())
}

async fn run_async(cmd: ListenSubcommand, _opt: CommonOpt, is_forked: bool) -> Result {
    let addr = cmd.host.to_ip_addr(cmd.use_ipv6)?;
    let socket_addrs = cmd.port.make_socket_addrs(addr);

    debug!("Binding to {} in range {}", addr, cmd.port);
    let listener = TcpListener::bind(socket_addrs.as_slice()).await?;

    let port = listener.local_addr()?.port();
    debug!("Bound to port: {}", port);

    let key = Arc::new(SecretKey::default());

    // Print information about port, key, etc. unless told not to
    if !cmd.no_print_startup_data {
        publish_data(port, &key);
    }

    // For the child, we want to fully disconnect it from pipes, which we do now
    if is_forked {
        if let Err(_) = fork::close_fd() {
            return Err(Error::ForkError);
        }
    }

    // Wait for a client connection, then spawn a new task to handle
    // receiving data from the client
    while let Ok((client, _)) = listener.accept().await {
        // Grab the client's remote address for later logging purposes
        let addr_string = match client.peer_addr() {
            Ok(addr) => {
                let addr_string = addr.to_string();
                info!("<Client @ {}> Established connection", addr_string);
                addr_string
            }
            Err(x) => {
                error!("Unable to examine client's peer address: {}", x);
                "???".to_string()
            }
        };

        // Build a transport around the client
        let mut transport = Transport::new(client, Arc::clone(&key));

        // Spawn a new task that loops to handle requests from the client
        tokio::spawn(async move {
            loop {
                match transport.receive::<Operation>().await {
                    Ok(Some(request)) => {
                        trace!(
                            "<Client @ {}> Received request of type {}",
                            addr_string.as_str(),
                            request.as_ref()
                        );

                        let response = Response::Error {
                            msg: String::from("Unimplemented"),
                        };

                        if let Err(x) = transport.send(response).await {
                            error!("<Client @ {}> {}", addr_string.as_str(), x);
                            break;
                        }
                    }
                    Ok(None) => {
                        info!("<Client @ {}> Closed connection", addr_string.as_str());
                        break;
                    }
                    Err(x) => {
                        error!("<Client @ {}> {}", addr_string.as_str(), x);
                        break;
                    }
                }
            }
        });
    }

    Ok(())
}

fn publish_data(port: u16, key: &SecretKey) {
    // TODO: We have to share the key in some manner (maybe use k256 to arrive at the same key?)
    //       For now, we do what mosh does and print out the key knowing that this is shared over
    //       ssh, which should provide security
    println!(
        "DISTANT DATA {} {}",
        port,
        hex::encode(key.unprotected_as_bytes())
    );
}
