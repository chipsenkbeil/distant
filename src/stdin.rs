use log::error;
use std::{
    io::{self, BufReader, Read},
    thread,
};
use tokio::sync::mpsc;

/// Creates a new thread that performs stdin reads in a blocking fashion, returning
/// a handle to the thread and a receiver that will be sent input as it becomes available
pub fn spawn_channel(buffer: usize) -> (thread::JoinHandle<()>, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(1);

    // NOTE: Using blocking I/O per tokio's advice to read from stdin line-by-line and then
    //       pass the results to a separate async handler to forward to the remote process
    let handle = thread::spawn(move || {
        let mut stdin = BufReader::new(io::stdin());

        // Maximum chunk that we expect to read at any one time
        let mut buf = vec![0; buffer];

        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    match String::from_utf8(buf[..n].to_vec()) {
                        Ok(text) => {
                            if let Err(x) = tx.blocking_send(text) {
                                error!("Stdin channel closed: {}", x);
                                break;
                            }
                        }
                        Err(x) => {
                            error!("Input over stdin is invalid: {}", x);
                        }
                    }
                    thread::yield_now();
                }
            }
        }
    });

    (handle, rx)
}
