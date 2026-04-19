use clap::{Args, CommandFactory};
use clap_complete::{generate, Shell};

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    pub shell: Shell,
}

pub async fn run(args: CompletionsArgs) -> anyhow::Result<()> {
    let mut cmd = crate::cli::commands::FlarionCli::command();
    let name = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, name, &mut std::io::stdout());
    Ok(())
}
