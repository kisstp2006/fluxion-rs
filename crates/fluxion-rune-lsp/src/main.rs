use anyhow::Result;
use rune::languageserver;
use rune::Options;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args();
    args.next();

    for arg in args {
        match arg.as_str() {
            "--version" => {
                println!("fluxion-rune-lsp (rune 0.14)");
                return Ok(());
            }
            "language-server" => {}
            other => {
                anyhow::bail!("Unsupported argument: {}", other);
            }
        }
    }

    let context = rune_modules::default_context()?;
    let options = Options::from_default_env()?;

    if let Err(e) = languageserver::run(context, options).await {
        eprintln!("fluxion-rune-lsp error: {e}");
    }

    Ok(())
}
