use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{router, Auth};

router!(DistantManagerServerRouter {
    auth_transport: Auth => Auth,
    manager_transport: ManagerResponse => ManagerRequest,
});
