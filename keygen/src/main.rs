use common::identity::NodeSigningKey;

fn main() {
    let orchestrator_key = NodeSigningKey::generate();
    let admin_key = NodeSigningKey::generate();
    println!("ORCHESTRATOR_SIGNING_KEY=\"{}\"", orchestrator_key.to_hex());
    println!("ADMIN_SIGNING_KEY=\"{}\"", admin_key.to_hex());
}
