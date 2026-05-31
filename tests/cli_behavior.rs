use ascii_cam::app::{Cli, Command};
use clap::Parser;

#[test]
fn serve_subcommand_is_recognized_by_the_cli_parser() {
    let cli = Cli::try_parse_from(["ascii-cam", "serve", "--token", "mytoken"])
        .expect("serve should parse as a subcommand");

    let Some(Command::Serve(args)) = cli.command else {
        panic!("serve should populate the serve subcommand");
    };
    assert_eq!(args.token.as_deref(), Some("mytoken"));
}
