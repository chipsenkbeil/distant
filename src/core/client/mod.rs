mod lsp;
mod process;
mod session;
mod utils;

// TODO: Make wrappers around a connection to facilitate the types
//       of engagements
//
//       1. Command -> Single request/response through a future
//       2. Proxy -> Does proc-run and waits until proc-done received,
//                   exposing a sender for stdin and receivers for stdout/stderr,
//                   and supporting a future await for completion with exit code
//       3.
pub use lsp::*;
pub use process::{RemoteProcess, RemoteProcessError, RemoteStderr, RemoteStdin, RemoteStdout};
pub use session::*;
