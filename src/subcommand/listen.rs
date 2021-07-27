use crate::{
    data::{Operation, Response, ResponsePayload},
    net::{Transport, TransportError},
    opt::{ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
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

pub fn run(cmd: ListenSubcommand) -> Result {
    if cmd.daemon {
        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { run_async(cmd, true).await })?;
            }
            Ok(Fork::Parent(pid)) => {
                eprintln!("[distant detached, pid = {}]", pid);
                if let Err(_) = fork::close_fd() {
                    return Err(Error::ForkError);
                }
            }
            Err(_) => return Err(Error::ForkError),
        }
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, false).await })?;
    }

    Ok(())
}

async fn run_async(cmd: ListenSubcommand, is_forked: bool) -> Result {
    let addr = cmd.host.to_ip_addr()?;
    let socket_addrs = cmd.port.make_socket_addrs(addr);
    let listener = TcpListener::bind(socket_addrs.as_slice()).await?;
    let port = listener.local_addr()?.port();
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

    // Begin our listen loop
    loop {
        // Wait for a client connection
        let (client, _) = listener.accept().await?;

        // Build a transport around the client
        let mut transport = Transport::new(client, Arc::clone(&key));

        // Spawn a new task that loops to handle requests from the client
        tokio::spawn(async move {
            loop {
                match transport.receive::<Operation>().await {
                    Ok(_request) => {
                        let response = Response::Error {
                            msg: String::from("Unimplemented"),
                        };

                        if let Err(x) = transport.send(response).await {
                            eprintln!("ERROR: {:?}", x);
                            break;
                        }
                    }
                    Err(x) => {
                        eprintln!("ERROR: {:?}", x);
                        break;
                    }
                }
            }
        });
    }

    #[allow(unreachable_code)]
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
