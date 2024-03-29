use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc;

type SendFn = Arc<Mutex<Box<dyn FnMut(&[u8]) -> io::Result<()> + Send>>>;
type RecvFn = Arc<Mutex<Box<dyn FnMut(&mut String) -> io::Result<()> + Send>>>;

/// Sends JSON messages over stdout
#[derive(Clone)]
pub struct MsgSender {
    send: SendFn,
}

impl<F> From<F> for MsgSender
where
    F: FnMut(&[u8]) -> io::Result<()> + Send + 'static,
{
    fn from(f: F) -> Self {
        Self {
            send: Arc::new(Mutex::new(Box::new(f))),
        }
    }
}

impl MsgSender {
    pub fn from_stdout() -> Self {
        let mut writer = std::io::stdout();
        Self::from(Box::new(move |output: &'_ [u8]| {
            writer.write_all(output)?;
            writer.flush()?;
            Ok(())
        }))
    }

    pub fn send_blocking<T>(&self, ser: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let msg = format!("{}\n", serde_json::to_string(ser)?);
        self.send.lock().unwrap()(msg.as_bytes())
    }
}

/// Receives JSON messages over stdin
#[derive(Clone)]
pub struct MsgReceiver {
    recv: RecvFn,
}

impl<F> From<F> for MsgReceiver
where
    F: FnMut(&mut String) -> io::Result<()> + Send + 'static,
{
    fn from(f: F) -> Self {
        Self {
            recv: Arc::new(Mutex::new(Box::new(f))),
        }
    }
}

impl MsgReceiver {
    pub fn from_stdin() -> Self {
        let reader = std::io::stdin();
        Self::from(move |input: &'_ mut String| {
            let _ = reader.read_line(input)?;
            Ok(())
        })
    }

    /// Spawns a thread to continually poll receiver for new input of the given type
    pub fn into_rx<T>(self) -> mpsc::Receiver<io::Result<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(1);

        thread::spawn(move || {
            loop {
                let res = self.recv_blocking();
                let is_eof = match res.as_ref() {
                    Err(x) => x.kind() == io::ErrorKind::UnexpectedEof,
                    Ok(_) => false,
                };

                // If there is nothing to listen for results, close our thread
                if tx.blocking_send(res).is_err() {
                    break;
                }

                // If stream has reached end, close our thread
                if is_eof {
                    break;
                }
            }
        });

        rx
    }

    pub fn recv_blocking<T>(&self) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        let mut input = String::new();

        // Continue reading input until we get a match, only if reported that current input
        // is a partial match
        let data: T = loop {
            // Read in another line of input
            self.recv.lock().unwrap()(&mut input)?;

            // Attempt to parse current input as type, yielding it on success, continuing to read
            // more input if error is unexpected EOF (meaning we are partially reading json), and
            // failing if we get any other error
            trace!(
                "Parsing into {} for {:?}",
                std::any::type_name::<T>(),
                input,
            );
            match serde_json::from_str(&input) {
                Ok(data) => break data,
                Err(x) if x.is_eof() => {
                    trace!(
                        "Not ready to parse as {}, so trying again with next update",
                        std::any::type_name::<T>(),
                    );
                    continue;
                }
                Err(x) => return Err(x.into()),
            }
        };

        Ok(data)
    }
}
