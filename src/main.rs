#[tokio::main]
async fn main() {
    if let Err(e) = questdrop::run().await {
        eprintln!("questdrop: {e}");
        std::process::exit(1);
    }
}
