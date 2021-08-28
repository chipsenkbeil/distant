// Generates a new tenant name
pub fn new_tenant() -> String {
    format!("tenant_{}{}", rand::random::<u16>(), rand::random::<u8>())
}
