use clap::Subcommand;

mod client;
mod generate;
mod manager;
mod server;

#[derive(Debug, Subcommand)]
pub enum DistantSubcommand {
    /// Perform client commands
    #[clap(subcommand)]
    Client(client::ClientSubcommand),

    /// Perform manager commands
    #[clap(subcommand)]
    Manager(manager::ManagerSubcommand),

    /// Perform server commands
    #[clap(subcommand)]
    Server(server::ServerSubcommand),

    /// Perform generation commands
    #[clap(subcommand)]
    Generate(generate::GenerateSubcommand),
}
