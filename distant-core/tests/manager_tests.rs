use distant_core::{DistantManager, DistantManagerClient, DistantManagerConfig};

#[tokio::test]
async fn should_be_able_to_manage_a_single_connection() {
    DistantManager::start()
}

#[tokio::test]
async fn should_be_able_to_manage_multiple_connections() {
    todo!();
}
