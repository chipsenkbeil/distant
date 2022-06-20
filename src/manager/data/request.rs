use super::Destination;
use distant_core::{data::DistantRequestData, DistantMsg};
use serde::{Deserialize, Serialize};
use structopt::StructOpt;

#[derive(Clone, Debug, Serialize, Deserialize, StructOpt)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Initiate a connection through the manager
    Connect {
        destination: Destination,

        /// Extra details specific to the connection
        #[structopt(short, long, parse(try_from_str = parse_key_val))]
        extra: Vec<(String, String)>,
    },

    /// Forward a request to a specific connection
    #[structopt(skip)]
    Request {
        id: usize,
        payload: DistantMsg<DistantRequestData>,
    },

    /// Retrieve information about a specific connection
    Info { id: usize },

    /// Kill a specific connection
    Kill { id: usize },

    /// Retrieve list of connections being managed
    List,

    /// Signals the manager to shutdown
    Shutdown,
}

fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn std::error::Error>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + 'static,
    U: std::str::FromStr,
    U::Err: std::error::Error + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}
