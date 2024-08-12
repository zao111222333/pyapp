use std::path::PathBuf;

mod app;
mod py;

const PROMPT1: &str = "pyapp > ";
const PROMPT2: &str = " .... > ";
const TERMINATE_N: u8 = 2;

use clap::{builder::styling::Styles, Parser};

#[derive(Parser, Debug)]
#[command(version, about = "An example application", long_about = None)]
#[command(styles = CLAP_STYLING)]
struct Args {
    /// Execute file with arguments. If not specify, start interactive shell mode
    #[arg(value_name = "file arg1 arg2..", trailing_var_arg = true)]
    file_args: Vec<String>,
    /// Execute file in quiet mode
    #[arg(short = 'q', long = "quiet", default_value_t = false)]
    quiet_exec: bool,
}

pub const CLAP_STYLING: Styles = Styles::styled();

#[derive(Debug)]
enum ExecMode {
    /// Start in interactive shell mode
    InteractiveShell,
    /// Execute a file with arguments
    ExecFile { quiet: bool, path: PathBuf, args: Vec<String> },
}

impl From<Args> for ExecMode {
    #[inline]
    fn from(mut value: Args) -> Self {
        if value.file_args.is_empty() {
            ExecMode::InteractiveShell
        } else {
            ExecMode::ExecFile {
                quiet: value.quiet_exec,
                path: value.file_args.remove(0).into(),
                args: value.file_args,
            }
        }
    }
}

fn main() -> app::ExitCode {
    let args = Args::parse().into();
    app::run(args)
}
