use crate::{
    data::{DirEntry, FileType, Request, RequestPayload, Response, ResponsePayload},
    net::Transport,
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use log::*;
use orion::aead::SecretKey;
use std::{string::FromUtf8Error, sync::Arc};
use tokio::{
    io::{self, AsyncWriteExt},
    net::TcpListener,
};
use walkdir::WalkDir;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ConvertToIpAddrError(ConvertToIpAddrError),
    ForkError,
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub fn run(cmd: ListenSubcommand, opt: CommonOpt) -> Result<(), Error> {
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

async fn run_async(cmd: ListenSubcommand, _opt: CommonOpt, is_forked: bool) -> Result<(), Error> {
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
                match transport.receive::<Request>().await {
                    Ok(Some(request)) => {
                        trace!(
                            "<Client @ {}> Received request of type {}",
                            addr_string.as_str(),
                            request.payload.as_ref()
                        );

                        // Process the request, converting any error into an error response
                        let response = Response::from_payload_with_origin(
                            match process_request_payload(request.payload).await {
                                Ok(payload) => payload,
                                Err(x) => ResponsePayload::Error {
                                    description: x.to_string(),
                                },
                            },
                            request.id,
                        );

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

async fn process_request_payload(
    payload: RequestPayload,
) -> Result<ResponsePayload, Box<dyn std::error::Error>> {
    match payload {
        RequestPayload::FileRead { path } => Ok(ResponsePayload::Blob {
            data: tokio::fs::read(path).await?,
        }),

        RequestPayload::FileReadText { path } => Ok(ResponsePayload::Text {
            data: tokio::fs::read_to_string(path).await?,
        }),

        RequestPayload::FileWrite {
            path,
            input: _,
            data,
        } => {
            tokio::fs::write(path, data).await?;
            Ok(ResponsePayload::Ok)
        }

        RequestPayload::FileAppend {
            path,
            input: _,
            data,
        } => {
            let mut file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .await?;
            file.write_all(&data).await?;
            Ok(ResponsePayload::Ok)
        }

        RequestPayload::DirRead { path, all } => {
            // Traverse, but don't include root directory in entries (hence min depth 1)
            let dir = WalkDir::new(path.as_path()).min_depth(1);

            // If all, will recursively traverse, otherwise just return directly from dir
            let dir = if all { dir } else { dir.max_depth(1) };

            // TODO: Support both returning errors and successfully-traversed entries
            // TODO: Support returning full paths instead of always relative?
            Ok(ResponsePayload::DirEntries {
                entries: dir
                    .into_iter()
                    .map(|e| {
                        e.map(|e| DirEntry {
                            path: e.path().strip_prefix(path.as_path()).unwrap().to_path_buf(),
                            file_type: if e.file_type().is_dir() {
                                FileType::Dir
                            } else if e.file_type().is_file() {
                                FileType::File
                            } else {
                                FileType::SymLink
                            },
                            depth: e.depth(),
                        })
                    })
                    .collect::<Result<Vec<DirEntry>, walkdir::Error>>()?,
            })
        }

        RequestPayload::DirCreate { path, all } => {
            if all {
                tokio::fs::create_dir_all(path).await?;
            } else {
                tokio::fs::create_dir(path).await?;
            }

            Ok(ResponsePayload::Ok)
        }

        RequestPayload::Remove { path, force } => {
            let path_metadata = tokio::fs::metadata(path.as_path()).await?;
            if path_metadata.is_dir() {
                if force {
                    tokio::fs::remove_dir_all(path).await?;
                } else {
                    tokio::fs::remove_dir(path).await?;
                }
            } else {
                tokio::fs::remove_file(path).await?;
            }

            Ok(ResponsePayload::Ok)
        }

        RequestPayload::Copy { src, dst } => {
            let src_metadata = tokio::fs::metadata(src.as_path()).await?;
            if src_metadata.is_dir() {
                for entry in WalkDir::new(src.as_path())
                    .min_depth(1)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| e.file_type().is_file() || e.path_is_symlink())
                {
                    let entry = entry?;

                    // Get unique portion of path relative to src
                    // NOTE: Because we are traversing files that are all within src, this
                    //       should always succeed
                    let local_src = entry.path().strip_prefix(src.as_path()).unwrap();

                    // Get the file without any directories
                    let local_src_file_name = local_src.file_name().unwrap();

                    // Get the directory housing the file
                    // NOTE: Because we enforce files/symlinks, there will always be a parent
                    let local_src_dir = local_src.parent().unwrap();

                    // Map out the path to the destination
                    let dst_parent_dir = dst.join(local_src_dir);

                    // Create the destination directory for the file when copying
                    tokio::fs::create_dir_all(dst_parent_dir.as_path()).await?;

                    // Perform copying from entry to destination
                    let dst_file = dst_parent_dir.join(local_src_file_name);
                    tokio::fs::copy(entry.path(), dst_file).await?;
                }
            } else {
                tokio::fs::copy(src, dst).await?;
            }

            Ok(ResponsePayload::Ok)
        }

        RequestPayload::Rename { src, dst } => {
            tokio::fs::rename(src, dst).await?;

            Ok(ResponsePayload::Ok)
        }

        RequestPayload::ProcRun { cmd, args, detach } => todo!(),

        RequestPayload::ProcConnect { id } => todo!(),

        RequestPayload::ProcKill { id } => todo!(),

        RequestPayload::ProcStdin { id, data } => todo!(),

        RequestPayload::ProcList {} => todo!(),
    }
}
