use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{router, Auth};

router! {
    DistantManagerServerRouter:
        Auth -> Auth,
        ManagerResponse -> ManagerRequest,
}
