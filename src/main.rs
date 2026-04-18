#[tokio::main]
async fn main() -> anyhow::Result<()> {
    flarion::cli::commands::dispatch().await
}
