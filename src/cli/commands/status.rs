use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub api_key: Option<String>,
    #[arg(long)]
    pub endpoint: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub client_config: Option<PathBuf>,
}

pub async fn run(_args: StatusArgs) -> anyhow::Result<()> {
    anyhow::bail!("flarion status not yet implemented (lands in Task 19)")
}
