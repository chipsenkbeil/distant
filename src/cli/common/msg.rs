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

#[cfg(test)]
mod tests {
    //! Tests for `MsgSender` and `MsgReceiver`: JSON-line-delimited message
    //! transport including send/receive, error propagation, clone behavior,
    //! partial message reassembly, and the async `into_rx` channel.

    use std::sync::Mutex;

    use serde::{Deserialize, Serialize};
    use test_log::test;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestMsg {
        name: String,
        value: u32,
    }

    // -------------------------------------------------------
    // MsgSender — basic send
    // -------------------------------------------------------
    #[test]
    fn sender_writes_json_with_newline() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let sender = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        let msg = TestMsg {
            name: "hello".to_string(),
            value: 42,
        };
        sender.send_blocking(&msg).unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(written.ends_with('\n'));

        // Parse the written JSON (minus trailing newline) back
        let parsed: TestMsg = serde_json::from_str(written.trim()).unwrap();
        assert_eq!(parsed, msg);
    }

    // -------------------------------------------------------
    // MsgSender — error propagation
    // -------------------------------------------------------
    #[test]
    fn sender_propagates_write_error() {
        let sender = MsgSender::from(move |_data: &[u8]| -> io::Result<()> {
            Err(io::Error::other("write failed"))
        });

        let msg = TestMsg {
            name: "test".to_string(),
            value: 0,
        };
        let result = sender.send_blocking(&msg);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "write failed");
    }

    // -------------------------------------------------------
    // MsgSender — clone sends to same target
    // -------------------------------------------------------
    #[test]
    fn sender_clone_shares_underlying_writer() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let sender = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        let sender2 = sender.clone();

        sender
            .send_blocking(&TestMsg {
                name: "first".to_string(),
                value: 1,
            })
            .unwrap();
        sender2
            .send_blocking(&TestMsg {
                name: "second".to_string(),
                value: 2,
            })
            .unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = written.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
    }

    // -------------------------------------------------------
    // MsgReceiver — basic receive
    // -------------------------------------------------------
    #[test]
    fn receiver_reads_json_line() {
        let msg = TestMsg {
            name: "world".to_string(),
            value: 99,
        };
        let json_line = format!("{}\n", serde_json::to_string(&msg).unwrap());
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);
        let json_line_clone = json_line.clone();

        let receiver = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str(&json_line_clone);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let result: TestMsg = receiver.recv_blocking().unwrap();
        assert_eq!(result, msg);
    }

    // -------------------------------------------------------
    // MsgReceiver — invalid JSON returns error
    // -------------------------------------------------------
    #[test]
    fn receiver_returns_error_on_invalid_json() {
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);

        let receiver = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str("not valid json\n");
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let result: io::Result<TestMsg> = receiver.recv_blocking();
        assert!(result.is_err());
    }

    // -------------------------------------------------------
    // MsgReceiver — partial JSON reassembles across calls
    // -------------------------------------------------------
    #[test]
    fn receiver_reassembles_partial_json() {
        let msg = TestMsg {
            name: "partial".to_string(),
            value: 7,
        };
        let json = serde_json::to_string(&msg).unwrap();
        // Split in half
        let mid = json.len() / 2;
        let part1 = json[..mid].to_string();
        let part2 = format!("{}\n", &json[mid..]);

        let call_count = Arc::new(Mutex::new(0u32));
        let call_count_clone = Arc::clone(&call_count);

        let receiver = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;
            match *count {
                1 => {
                    input.push_str(&part1);
                    Ok(())
                }
                2 => {
                    input.push_str(&part2);
                    Ok(())
                }
                _ => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data")),
            }
        });

        let result: TestMsg = receiver.recv_blocking().unwrap();
        assert_eq!(result, msg);
    }

    // -------------------------------------------------------
    // MsgReceiver — read error propagation
    // -------------------------------------------------------
    #[test]
    fn receiver_propagates_read_error() {
        let receiver = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> {
            Err(io::Error::other("read failed"))
        });

        let result: io::Result<TestMsg> = receiver.recv_blocking();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "read failed");
    }

    // -------------------------------------------------------
    // MsgReceiver::into_rx — produces channel of results
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn receiver_into_rx_produces_messages() {
        let msg = TestMsg {
            name: "async".to_string(),
            value: 42,
        };
        let json_line = format!("{}\n", serde_json::to_string(&msg).unwrap());

        let call_count = Arc::new(Mutex::new(0u32));
        let call_count_clone = Arc::clone(&call_count);

        let receiver = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut count = call_count_clone.lock().unwrap();
            *count += 1;
            if *count == 1 {
                input.push_str(&json_line);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let mut rx = receiver.into_rx::<TestMsg>();
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first, msg);

        // Second message should be an EOF error
        let second = rx.recv().await.unwrap();
        assert!(second.is_err());
        assert_eq!(second.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    // -------------------------------------------------------
    // MsgSender — multiple messages
    // -------------------------------------------------------
    #[test]
    fn sender_sends_multiple_messages() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let sender = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        for i in 0..3 {
            let msg = TestMsg {
                name: format!("msg{i}"),
                value: i,
            };
            sender.send_blocking(&msg).unwrap();
        }

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = written.trim().split('\n').collect();
        assert_eq!(lines.len(), 3);

        for (i, line) in lines.iter().enumerate() {
            let parsed: TestMsg = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.name, format!("msg{i}"));
            assert_eq!(parsed.value, i as u32);
        }
    }
}
